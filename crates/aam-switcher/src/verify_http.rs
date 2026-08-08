//! Shared network verification helper for Provider implementations.

use crate::provider::VerifyError;
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
