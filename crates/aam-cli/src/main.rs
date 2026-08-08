//! `aam` CLI entry point.
//!
//! Phase 0 placeholder: no subcommands yet. Phase 1 (see
//! `docs/03-credential-account-module.md` §3.6) adds `profile list/add/verify`
//! and `aam claude <profile>` / `aam codex <profile>`.

fn main() {
    println!("ai-agent-manager (aam) — Phase 0 scaffold, no commands implemented yet.");
    println!("  {}", aam_switcher::placeholder());
    println!("  {}", aam_memory::placeholder());
    println!("See docs/07-roadmap.md for what Phase 1 adds.");
}
