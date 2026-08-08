//! Where things live on disk: the user's real home directory, and this
//! project's own state directory (distinct from the *managed* tools'
//! directories -- `CLAUDE_CONFIG_DIR`/`CODEX_HOME` -- which belong to
//! Claude Code/Codex CLI, not to `ai-agent-manager` itself).

use std::path::PathBuf;

/// The current user's home directory (`%USERPROFILE%` on Windows, `$HOME`
/// on Unix). Used for paths that are meaningful to the *managed* tools
/// themselves, e.g. `~/.claude/skills` (`aam-skills`), which must track
/// the real home directory regardless of where `ai-agent-manager`'s own
/// state lives.
#[cfg(windows)]
pub fn user_home_dir() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").expect("USERPROFILE must be set on Windows"))
}

#[cfg(unix)]
pub fn user_home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME must be set on Unix"))
}

/// The root directory `ai-agent-manager` uses for its own state (Profile
/// registry, local vault, skills index, ...). Defaults to `~/.aam`,
/// overridable via `AAM_HOME` (mirrors `CLAUDE_CONFIG_DIR`/`CODEX_HOME`'s
/// override pattern, and makes tests hermetic without touching a real
/// home directory).
pub fn aam_home() -> PathBuf {
    if let Ok(dir) = std::env::var("AAM_HOME") {
        return PathBuf::from(dir);
    }
    user_home_dir().join(".aam")
}
