//! Unix backend for [`crate::SecretStore`]: plaintext + `chmod 600`.
//!
//! Deliberately weaker than the Windows DPAPI backend — this does **not**
//! resist a root-level or already-compromised-account attacker, only
//! ordinary other-user access on a shared machine. Documented as a known,
//! accepted asymmetry in `docs/02-architecture.md` §2.4, matching
//! `codex-skill`'s own stated Debian security model.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(crate) fn restrict_permissions(path: &Path) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)
}
