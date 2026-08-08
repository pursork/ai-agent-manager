//! Skills manager — Phase 1 subset (see `docs/09-skills-management.md`).
//!
//! Ships the **structural** piece only: provisioning symlinks/Junctions so
//! a Skill has one physical copy but is visible from multiple places
//! (a Claude Profile's `skills/` directory, or Codex's global
//! `$HOME/.agents/skills/`). Scanning for pre-existing unmanaged skills,
//! GitHub-source update tracking, and the full "adopt" flow that moves
//! content into the canonical location are Phase 3 (`09.6`, `09.7`).
//!
//! Deliberately has **no** dependency on `aam-sync` — Skills sync over
//! plain git, not the encrypted WebDAV channel (`09.2`).

mod link;
mod manage;
mod paths;

pub use link::{resolves_to, LinkError, ProvisionDirLink};
pub use manage::{
    list_managed_skills, provision_profile_skills_link, share_skill_with_codex, ManagedSkill,
    ShareError,
};
pub use paths::{claude_personal_skills_dir, codex_user_skills_dir};
