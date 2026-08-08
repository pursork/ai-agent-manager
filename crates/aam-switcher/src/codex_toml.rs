//! `config.toml` generation for Codex Providers -- field names, defaults,
//! and (crucially) omissions match `codex-skill`'s `Write-ProviderConfig`
//! exactly (verified against `codex-rs`'s `ConfigToml` struct source, not
//! just vendor docs -- see this crate's Phase 1 planning notes for the
//! full extraction and the `preferred_auth_method` cautionary tale).
//!
//! Deliberately **never written**, on purpose:
//! - `preferred_auth_method` -- not a real `ConfigToml` field
//!   (`#[schemars(deny_unknown_fields)]` rejects it outright).
//! - `forced_login_method` -- setting it to `"api"` while `auth.json` is
//!   still ChatGPT-logged-in makes Codex delete `auth.json` and log the
//!   user out on the next invocation, which conflicts with this project's
//!   whole point (switching back to the official subscription must always
//!   stay possible).

use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn build_codex_provider_toml(
    model: &str,
    model_provider_id: &str,
    provider_display_name: &str,
    base_url: &str,
    supports_websockets: bool,
    reasoning_effort: &str,
    plan_reasoning_effort: &str,
    model_catalog_json: Option<&Path>,
    auth_command: &str,
    auth_args: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("# Managed by ai-agent-manager (aam-switcher) -- see docs/03-credential-account-module.md\n");
    out.push_str(&format!("model = {}\n", toml_string(model)));
    out.push_str(&format!("model_provider = {}\n", toml_string(model_provider_id)));
    out.push_str(&format!(
        "model_reasoning_effort = {}\n",
        toml_string(reasoning_effort)
    ));
    out.push_str(&format!(
        "plan_mode_reasoning_effort = {}\n",
        toml_string(plan_reasoning_effort)
    ));
    if let Some(catalog) = model_catalog_json {
        out.push_str(&format!(
            "model_catalog_json = {}\n",
            toml_string(&catalog.display().to_string())
        ));
    }
    out.push('\n');

    out.push_str(&format!("[model_providers.{model_provider_id}]\n"));
    out.push_str(&format!("name = {}\n", toml_string(provider_display_name)));
    out.push_str(&format!("base_url = {}\n", toml_string(base_url)));
    out.push_str("wire_api = \"responses\"\n");
    out.push_str(&format!("supports_websockets = {supports_websockets}\n"));
    out.push_str("request_max_retries = 4\n");
    out.push_str("stream_max_retries = 5\n");
    out.push_str("stream_idle_timeout_ms = 300000\n");
    out.push('\n');

    out.push_str(&format!("[model_providers.{model_provider_id}.auth]\n"));
    out.push_str(&format!("command = {}\n", toml_string(auth_command)));
    let args_toml: Vec<String> = auth_args.iter().map(|a| toml_string(a)).collect();
    out.push_str(&format!("args = [{}]\n", args_toml.join(", ")));
    out.push_str("timeout_ms = 5000\n");
    out.push_str("refresh_interval_ms = 0\n");
    out.push('\n');

    out
}

/// TOML string literal escaping: `\` -> `\\`, `"` -> `\"`, always
/// double-quoted (mirrors `codex-skill`'s `ConvertTo-TomlString`).
fn toml_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_cpa_shaped_block() {
        let toml = build_codex_provider_toml(
            "gpt-5",
            "cliproxyapi",
            "CLIProxyAPI",
            "https://cpa.example.com",
            false,
            "high",
            "high",
            None,
            "powershell.exe",
            &["-File".into(), "helper.ps1".into()],
        );

        assert!(toml.contains("model_provider = \"cliproxyapi\""));
        assert!(toml.contains("[model_providers.cliproxyapi]"));
        assert!(toml.contains("wire_api = \"responses\""));
        assert!(toml.contains("[model_providers.cliproxyapi.auth]"));
        assert!(toml.contains("refresh_interval_ms = 0"));
        assert!(!toml.contains("preferred_auth_method"));
        assert!(!toml.contains("forced_login_method"));
    }

    #[test]
    fn escapes_backslashes_and_quotes_in_string_values() {
        let toml = build_codex_provider_toml(
            "m",
            "p",
            "P",
            "https://example.com",
            false,
            "high",
            "high",
            None,
            r#"C:\path\with"quote.exe"#,
            &[],
        );
        assert!(toml.contains(r#"command = "C:\\path\\with\"quote.exe""#));
    }

    #[test]
    fn includes_model_catalog_json_only_when_provided() {
        let without = build_codex_provider_toml(
            "m", "p", "P", "https://x", false, "high", "high", None, "cat", &[],
        );
        assert!(!without.contains("model_catalog_json"));

        let with = build_codex_provider_toml(
            "m",
            "p",
            "P",
            "https://x",
            false,
            "high",
            "high",
            Some(Path::new("/tmp/catalog.json")),
            "cat",
            &[],
        );
        assert!(with.contains("model_catalog_json"));
    }
}
