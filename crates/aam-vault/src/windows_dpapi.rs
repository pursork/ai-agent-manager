//! Windows DPAPI backend for [`crate::SecretStore`].
//!
//! Shells out to `powershell.exe` to call
//! `[Security.Cryptography.ProtectedData]::Protect`/`Unprotect` with
//! `DataProtectionScope::CurrentUser` — the exact primitive `codex-skill`
//! already uses in production (see the extraction in this project's Phase 1
//! planning notes). Shelling out rather than binding the Win32 API directly
//! trades a small amount of process-spawn overhead for reusing a
//! battle-tested code path instead of hand-rolling FFI against
//! `CryptProtectData`/`CryptUnprotectData`.
//!
//! The two helper scripts are written once per vault `root` (idempotent,
//! rewritten on every call to stay in sync with this source — they're a
//! few hundred bytes, the overhead is negligible) and invoked with the
//! payload piped through stdin/stdout so no plaintext or ciphertext ever
//! touches a command-line argument or an intermediate file.

use crate::VaultError;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PROTECT_SCRIPT: &str = r#"param(
    [Parameter(Mandatory = $true)]
    [string]$Entropy
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$entropyBytes = [Text.Encoding]::UTF8.GetBytes($Entropy)
$stdin = [Console]::OpenStandardInput()
$ms = New-Object System.IO.MemoryStream
$stdin.CopyTo($ms)
$plain = $ms.ToArray()
$cipher = [Security.Cryptography.ProtectedData]::Protect(
    $plain, $entropyBytes, [Security.Cryptography.DataProtectionScope]::CurrentUser
)
$stdout = [Console]::OpenStandardOutput()
$stdout.Write($cipher, 0, $cipher.Length)
$stdout.Flush()
"#;

const UNPROTECT_SCRIPT: &str = r#"param(
    [Parameter(Mandatory = $true)]
    [string]$Entropy
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$entropyBytes = [Text.Encoding]::UTF8.GetBytes($Entropy)
$stdin = [Console]::OpenStandardInput()
$ms = New-Object System.IO.MemoryStream
$stdin.CopyTo($ms)
$cipher = $ms.ToArray()
$plain = [Security.Cryptography.ProtectedData]::Unprotect(
    $cipher, $entropyBytes, [Security.Cryptography.DataProtectionScope]::CurrentUser
)
$stdout = [Console]::OpenStandardOutput()
$stdout.Write($plain, 0, $plain.Length)
$stdout.Flush()
"#;

fn scripts_dir(root: &Path) -> PathBuf {
    root.join(".ps-helpers")
}

fn ensure_scripts(root: &Path) -> Result<(PathBuf, PathBuf), VaultError> {
    let dir = scripts_dir(root);
    fs::create_dir_all(&dir)?;

    let protect_path = dir.join("protect.ps1");
    let unprotect_path = dir.join("unprotect.ps1");
    aam_core::atomic_write(&protect_path, PROTECT_SCRIPT.as_bytes())?;
    aam_core::atomic_write(&unprotect_path, UNPROTECT_SCRIPT.as_bytes())?;

    Ok((protect_path, unprotect_path))
}

fn run_powershell(script_path: &Path, entropy: &str, input: &[u8]) -> Result<Vec<u8>, VaultError> {
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script_path)
        .arg(entropy)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| VaultError::Backend(format!("failed to spawn powershell.exe: {e}")))?;

    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(input)
        .map_err(|e| VaultError::Backend(format!("failed to write to powershell stdin: {e}")))?;

    let output = child
        .wait_with_output()
        .map_err(|e| VaultError::Backend(format!("failed to wait for powershell.exe: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VaultError::Backend(format!(
            "DPAPI helper ({}) exited with {}: {}",
            script_path.display(),
            output.status,
            stderr.trim()
        )));
    }

    Ok(output.stdout)
}

pub(crate) fn protect(root: &Path, plaintext: &str, entropy: &str) -> Result<Vec<u8>, VaultError> {
    let (protect_script, _) = ensure_scripts(root)?;
    run_powershell(&protect_script, entropy, plaintext.as_bytes())
}

pub(crate) fn unprotect(root: &Path, ciphertext: &[u8], entropy: &str) -> Result<String, VaultError> {
    let (_, unprotect_script) = ensure_scripts(root)?;
    let bytes = run_powershell(&unprotect_script, entropy, ciphertext)?;
    String::from_utf8(bytes)
        .map_err(|e| VaultError::Backend(format!("decrypted secret is not valid UTF-8: {e}")))
}
