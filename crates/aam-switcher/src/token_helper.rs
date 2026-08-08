//! Codex's "command-backed bearer token" mechanism
//! (`[model_providers.<id>.auth]`'s `command`/`args`): on each request,
//! Codex spawns a child process and captures its stdout as the bearer
//! token, so the token file never appears in `config.toml` itself.
//! Mirrors `codex-skill`'s `Write-ProviderTokenHelper` (see this crate's
//! Phase 1 planning notes for the extracted original).

use std::path::{Path, PathBuf};

/// Entropy label used both when *writing* the token (via
/// `aam_vault::SecretStore::new(config_dir, TOKEN_ENTROPY)`) and when
/// *reading* it back (the generated Windows helper script below). Must
/// match exactly on both sides -- DPAPI's `Unprotect` requires the same
/// entropy bytes that were passed to `Protect`.
pub const TOKEN_ENTROPY: &str = "aam-codex-provider-token-v1";

const TOKEN_FILE_NAME: &str = "provider-token.secret";
const HELPER_SCRIPT_NAME: &str = "provider-token-helper.ps1";

pub fn token_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join(TOKEN_FILE_NAME)
}

pub fn helper_script_path(config_dir: &Path) -> PathBuf {
    config_dir.join(HELPER_SCRIPT_NAME)
}

/// The Windows PowerShell helper script content. Reads a `-TokenFile`
/// *argument* (not stdin -- Codex only captures the child process's
/// stdout, it never pipes anything to stdin), DPAPI-decrypts it, and
/// writes the raw token bytes to stdout with **no** trailing newline
/// (`[Console]::Out.Write`, not `WriteLine` -- a trailing newline would
/// become part of the bearer token).
#[cfg(windows)]
pub fn helper_script_text() -> String {
    format!(
        r#"param(
    [Parameter(Mandatory = $true)]
    [string]$TokenFile
)
$ErrorActionPreference = 'Stop'
try {{
    Add-Type -AssemblyName System.Security
    $entropy = [Text.Encoding]::UTF8.GetBytes('{TOKEN_ENTROPY}')
    $cipher = [IO.File]::ReadAllBytes($TokenFile)
    $plain = [Security.Cryptography.ProtectedData]::Unprotect(
        $cipher, $entropy, [Security.Cryptography.DataProtectionScope]::CurrentUser
    )
    $stdout = [Console]::OpenStandardOutput()
    $stdout.Write($plain, 0, $plain.Length)
    $stdout.Flush()
}} catch {{
    [Console]::Error.WriteLine('Unable to read the provider bearer token.')
    exit 1
}}
"#
    )
}

/// The `[model_providers.<id>.auth]` `command`/`args` for this platform.
#[cfg(windows)]
pub fn auth_command(helper_script_path: &Path, token_file: &Path) -> (String, Vec<String>) {
    (
        "powershell.exe".to_string(),
        vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
            helper_script_path.display().to_string(),
            "-TokenFile".into(),
            token_file.display().to_string(),
        ],
    )
}

/// Unix has no separate helper script: `aam-vault`'s Unix backend is
/// already plaintext + `chmod 600` (`docs/02-architecture.md` §2.4's
/// declared, accepted asymmetry), so there's no ciphertext to decrypt --
/// `cat` the token file directly rather than writing a wrapper script
/// that would do the exact same thing.
#[cfg(unix)]
pub fn auth_command(_helper_script_path: &Path, token_file: &Path) -> (String, Vec<String>) {
    ("cat".to_string(), vec![token_file.display().to_string()])
}
