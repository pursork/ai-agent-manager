//! Shared network helpers for Provider implementations: liveness checks
//! (`verify_models_endpoint`) and text completion (`complete_via_messages_api`).

use crate::provider::{CompleteError, VerifyError};
use std::time::Duration;

/// Mirrors `codex-skill`'s `Test-ModelsEndpoint`: `GET {base_url}/models`
/// with Bearer auth, requires HTTP 200 + valid JSON + a `data[].id` entry
/// equal to `expected_model`.
pub fn verify_models_endpoint(
    base_url: &str,
    api_key: &str,
    expected_model: &str,
) -> Result<(), VerifyError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let response = ureq::get(&url)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Accept", "application/json")
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| VerifyError::Http(e.to_string()))?;

    let body: serde_json::Value = response
        .into_json()
        .map_err(|e| VerifyError::UnexpectedResponse(format!("response was not valid JSON: {e}")))?;

    let has_model = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .any(|item| item.get("id").and_then(|v| v.as_str()) == Some(expected_model))
        })
        .unwrap_or(false);

    if !has_model {
        return Err(VerifyError::UnexpectedResponse(format!(
            "model '{expected_model}' not found in {url} response's data[]"
        )));
    }

    Ok(())
}

/// Anthropic Messages API version this crate speaks -- required header,
/// per `platform.claude.com/docs/en/api/messages`.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Cap on the model's reply length -- `complete()`'s only caller wants a
/// one-line summary, not an essay.
const COMPLETE_MAX_TOKENS: u32 = 256;

/// Sends `prompt` as a single user message via the Anthropic Messages API
/// (`POST {base_url}/v1/messages`) and returns the first `text`-typed
/// content block. **`X-Api-Key` auth, not `Authorization: Bearer`** --
/// deliberately different from `verify_models_endpoint` above, which
/// checks a separate, OpenAI-shaped `/models` endpoint on the same
/// `base_url`; the two conventions coexist because a proxy like CPA is
/// designed to serve both Claude (Anthropic protocol) and Codex (OpenAI
/// Responses protocol, `codex_toml.rs`'s `wire_api = "responses"`) traffic
/// on one `base_url` (`docs/08-open-questions-risks.md` #17's design note
/// -- verified against Anthropic's own docs before writing this, not
/// assumed from `verify_models_endpoint`'s convention).
pub fn complete_via_messages_api(
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
) -> Result<String, CompleteError> {
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));

    let response = ureq::post(&url)
        .set("X-Api-Key", api_key)
        .set("anthropic-version", ANTHROPIC_VERSION)
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(30))
        .send_json(serde_json::json!({
            "model": model,
            "max_tokens": COMPLETE_MAX_TOKENS,
            "messages": [{"role": "user", "content": prompt}],
        }))
        .map_err(|e| CompleteError::Http(e.to_string()))?;

    let body: serde_json::Value = response
        .into_json()
        .map_err(|e| CompleteError::UnexpectedResponse(format!("response was not valid JSON: {e}")))?;

    parse_messages_api_response(&body)
}

/// Pure parsing step of [`complete_via_messages_api`], split out so it can
/// be unit tested against fixed response bodies without a network call --
/// mirrors `provider_sync`/`account_sync`'s general preference for
/// testing logic separately from the I/O that feeds it.
fn parse_messages_api_response(body: &serde_json::Value) -> Result<String, CompleteError> {
    let content = body
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| CompleteError::UnexpectedResponse("response has no 'content' array".to_string()))?;

    content
        .iter()
        .find(|block| block.get("type").and_then(|t| t.as_str()) == Some("text"))
        .and_then(|block| block.get("text"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            CompleteError::UnexpectedResponse("no text-typed content block in response".to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_normal_response() {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "修好了齿轮问题", "citations": []}],
            "stop_reason": "end_turn"
        });
        assert_eq!(parse_messages_api_response(&body).unwrap(), "修好了齿轮问题");
    }

    #[test]
    fn skips_non_text_blocks_to_find_the_text_one() {
        let body = serde_json::json!({
            "content": [
                {"type": "tool_use", "id": "x", "name": "y", "input": {}},
                {"type": "text", "text": "the actual summary"}
            ]
        });
        assert_eq!(parse_messages_api_response(&body).unwrap(), "the actual summary");
    }

    #[test]
    fn missing_content_array_errors() {
        let body = serde_json::json!({"stop_reason": "end_turn"});
        assert!(parse_messages_api_response(&body).is_err());
    }

    #[test]
    fn empty_content_array_errors() {
        let body = serde_json::json!({"content": []});
        assert!(parse_messages_api_response(&body).is_err());
    }

    #[test]
    fn only_non_text_blocks_errors() {
        let body = serde_json::json!({"content": [{"type": "tool_use", "id": "x", "name": "y", "input": {}}]});
        assert!(parse_messages_api_response(&body).is_err());
    }
}
