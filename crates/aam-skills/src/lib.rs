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

mod adopt;
mod bundled;
mod discover;
mod index;
mod link;
mod manage;
mod paths;
mod updates;

pub use adopt::{
    adopt_from_git, adopt_from_git_at, adopt_local_skill, adopt_local_skill_at, AdoptError, AdoptMoveError,
    AdoptSkillMove,
};
pub use bundled::{find_bundled_skill, BundledSkill, BUNDLED_SKILLS};
pub use discover::{scan_unmanaged_skills, DiscoveredSkill};
pub use index::{IndexError, SkillEntry, SkillsIndex};
pub use link::{resolves_to, LinkError, ProvisionDirLink};
pub use manage::{
    install_bundled_skill, install_bundled_skill_at, list_managed_skills, provision_profile_skills_link,
    share_skill_with_codex, InstallError, InstallOutcome, ManagedSkill, ShareError,
};
pub use paths::{claude_personal_skills_dir, codex_user_skills_dir};
pub use updates::{
    check_updates, check_updates_at, update_all_auto, update_all_auto_at, update_skill, update_skill_at,
    AutoUpdateOutcome, UpdateError, UpdateStatus,
};
