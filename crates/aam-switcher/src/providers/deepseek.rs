use crate::codex_toml::build_codex_provider_toml;
use crate::provider::{CompleteError, Provider, ProviderConfig, ToolKind, VerifyError};
use crate::token_helper;
use crate::verify_http::{complete_via_messages_api, verify_models_endpoint};
use serde_json::json;

const CATALOG_FILE_NAME: &str = "deepseek-v4-flash-models.json";
const MINIMUM_CLIENT_VERSION: &str = "0.144.0";
const BASE_INSTRUCTIONS: &str = "You are Codex, an agentic coding assistant working in the user's repository. Inspect the workspace, use tools carefully, make the requested changes, verify them, and report concrete results. Follow AGENTS.md and user instructions when present.";

/// DeepSeek V4 Flash -- the second Phase 1 Provider implementation
/// (`docs/03-credential-account-module.md` §3.4).
pub struct DeepSeekProvider {
    pub base_url: String,
    pub api_key: String,
    pub reasoning_effort: String,
    pub plan_reasoning_effort: String,
}

impl DeepSeekProvider {
    const MODEL: &'static str = "deepseek-v4-flash";

    pub fn new(base_url: String, api_key: String, reasoning_effort: String, plan_reasoning_effort: String) -> Self {
        Self {
            base_url,
            api_key,
            reasoning_effort,
            plan_reasoning_effort,
        }
    }
}

impl Provider for DeepSeekProvider {
    fn id(&self) -> &str {
        "deepseek-v4-flash"
    }

    fn materialize(&self, target: ToolKind) -> ProviderConfig {
        match target {
            ToolKind::Claude => ProviderConfig {
                env_vars: vec![
                    ("ANTHROPIC_BASE_URL".into(), self.base_url.clone()),
                    ("ANTHROPIC_API_KEY".into(), self.api_key.clone()),
                ],
                ..Default::default()
            },
            ToolKind::Codex { config_dir } => {
                let token_file = token_helper::token_file_path(&config_dir);
                let catalog_path = config_dir.join(CATALOG_FILE_NAME);

                #[cfg(windows)]
                let (command, args, helper_script) = {
                    let helper_path = token_helper::helper_script_path(&config_dir);
                    let (command, args) = token_helper::auth_command(&helper_path, &token_file);
                    (command, args, Some(token_helper::helper_script_text()))
                };
                #[cfg(unix)]
                let (command, args, helper_script) = {
                    let (command, args) = token_helper::auth_command(&token_file, &token_file);
                    (command, args, None)
                };

                let toml = build_codex_provider_toml(
                    Self::MODEL,
                    "deepseek",
                    "DeepSeek",
                    &self.base_url,
                    false, // hardcoded false for DeepSeek, not settings-driven -- matches codex-skill
                    &self.reasoning_effort,
                    &self.plan_reasoning_effort,
                    Some(&catalog_path),
                    &command,
                    &args,
                );

                ProviderConfig {
                    codex_config_toml: Some(toml),
                    codex_token_helper_script: helper_script,
                    codex_token_file: Some(token_file),
                    codex_extra_files: vec![(catalog_path, deepseek_catalog_json().into_bytes())],
                    ..Default::default()
                }
            }
        }
    }

    fn verify(&self, _cfg: &ProviderConfig) -> Result<(), VerifyError> {
        verify_models_endpoint(&self.base_url, &self.api_key, Self::MODEL)
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn complete(&self, prompt: &str) -> Result<String, CompleteError> {
        complete_via_messages_api(&self.base_url, &self.api_key, Self::MODEL, prompt)
    }
}

/// The `model_catalog_json` sidecar file content for DeepSeek V4 Flash
/// (`codex-skill`'s `Write-DeepSeekCatalog`) -- Codex's config schema for
/// a custom model catalog entry, not TOML.
pub fn deepseek_catalog_json() -> String {
    let catalog = json!({
        "models": [{
            "slug": DeepSeekProvider::MODEL,
            "prefer_websockets": false,
            "support_verbosity": true,
            "default_verbosity": "low",
            "apply_patch_tool_type": "freeform",
            "web_search_tool_type": "text",
            "input_modalities": ["text"],
            "supports_image_detail_original": false,
            "truncation_policy": { "mode": "tokens", "limit": 10000 },
            "supports_parallel_tool_calls": true,
            "tool_mode": null,
            "multi_agent_version": "v2",
            "use_responses_lite": false,
            "include_skills_usage_instructions": false,
            "auto_review_model_override": null,
            "context_window": 1_048_576,
            "max_context_window": 1_048_576,
            "effective_context_window_percent": 95,
            "auto_compact_token_limit": null,
            "comp_hash": "3000",
            "reasoning_summary_format": "experimental",
            "default_reasoning_summary": "none",
            "display_name": "DeepSeek-V4-Flash",
            "description": "DeepSeek V4 Flash for the Responses API.",
            "default_reasoning_level": "high",
            "supported_reasoning_levels": [
                { "effort": "low", "description": "Fast responses with lighter reasoning" },
                { "effort": "high", "description": "Deeper reasoning for harder tasks" },
                { "effort": "max", "description": "Maximum reasoning depth, slowest" }
            ],
            "shell_type": "shell_command",
            "visibility": "list",
            "minimal_client_version": MINIMUM_CLIENT_VERSION,
            "supported_in_api": true,
            "availability_nux": null,
            "upgrade": null,
            "priority": 1,
            "model_messages": {
                "instructions_template": BASE_INSTRUCTIONS,
                "instructions_variables": {
                    "personality_default": "",
                    "personality_friendly": "",
                    "personality_pragmatic": ""
                },
                "approvals": null
            },
            "experimental_supported_tools": [],
            "supports_search_tool": true,
            "default_service_tier": null,
            "supports_reasoning_summaries": true,
            "base_instructions": BASE_INSTRUCTIONS
        }]
    });

    serde_json::to_string_pretty(&catalog).expect("catalog is always valid JSON")
}

pub fn catalog_file_name() -> &'static str {
    CATALOG_FILE_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_json_is_valid_and_has_expected_slug() {
        let text = deepseek_catalog_json();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed["models"][0]["slug"],
            serde_json::Value::String("deepseek-v4-flash".to_string())
        );
        assert_eq!(
            parsed["models"][0]["minimal_client_version"],
            serde_json::Value::String(MINIMUM_CLIENT_VERSION.to_string())
        );
    }
}
