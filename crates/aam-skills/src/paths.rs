use std::path::PathBuf;

/// The canonical, single physical location for every skill this crate
/// manages (`docs/09-skills-management.md` §9.3: "直接定为
/// `~/.claude/skills/<name>`，不新建独立目录").
pub fn claude_personal_skills_dir() -> PathBuf {
    aam_core::user_home_dir().join(".claude").join("skills")
}

/// Codex's fixed, `CODEX_HOME`-independent user-scope skills location
/// (`docs/08-open-questions-risks.md` §8.1, confirmed via official docs:
/// Codex skills always come from `$HOME/.agents/skills`, never from
/// `CODEX_HOME`).
pub fn codex_user_skills_dir() -> PathBuf {
    aam_core::user_home_dir().join(".agents").join("skills")
}
