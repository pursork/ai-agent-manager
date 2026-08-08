use crate::codex_toml::build_codex_provider_toml;
use crate::provider::{Provider, ProviderConfig, ToolKind, VerifyError};
use crate::token_helper;
use crate::verify_http::verify_models_endpoint;

/// CPA (自建 CLIProxyAPI) -- one of the two Phase 1 Provider implementations
/// (`docs/03-credential-account-module.md` §3.4).
pub struct CpaProvider {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub supports_websockets: bool,
    pub reasoning_effort: String,
    pub plan_reasoning_effort: String,
}

impl Provider for CpaProvider {
    fn id(&self) -> &str {
        "cpa"
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
                    &self.model,
                    "cliproxyapi",
                    "CLIProxyAPI",
                    &self.base_url,
                    self.supports_websockets,
                    &self.reasoning_effort,
                    &self.plan_reasoning_effort,
                    None,
                    &command,
                    &args,
                );

                ProviderConfig {
                    codex_config_toml: Some(toml),
                    codex_token_helper_script: helper_script,
                    codex_token_file: Some(token_file),
                    ..Default::default()
                }
            }
        }
    }

    fn verify(&self, _cfg: &ProviderConfig) -> Result<(), VerifyError> {
        verify_models_endpoint(&self.base_url, &self.api_key, &self.model)
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }
}
