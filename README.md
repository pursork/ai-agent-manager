# ai-agent-manager

Rust workspace for a cross-device, cross-account, cross-backend session continuity
tool for Claude Code and Codex CLI (credential vault, account/provider switching,
WebDAV-synced project memory bank, and an eventual `iced`-based terminal GUI).

Design docs and phased roadmap live in [`docs/`](./docs), starting with
[`docs/00-overview.md`](./docs/00-overview.md).

## Status

Phase 0 (workspace scaffold) — see [`docs/07-roadmap.md`](./docs/07-roadmap.md) for
the full phase breakdown and [`docs/08-open-questions-risks.md`](./docs/08-open-questions-risks.md)
for what's still blocking Phase 1.

## Workspace layout

| Crate | Role |
|---|---|
| `aam-core` | Shared types, error definitions, `TransactionalOp` (snapshot → apply → verify → rollback) |
| `aam-vault` | Local credential vault (Phase 2+) |
| `aam-switcher` | Account/Provider switching for Claude + Codex (Phase 1+) |
| `aam-sync` | WebDAV encrypted sync engine (Phase 2+) |
| `aam-memory` | Session/project memory-bank tracking (Phase 3+) |
| `aam-cli` | CLI entry point |
| `aam-gui` | `iced` GUI shell (Phase 4+) |

## Build

```sh
cargo build --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
```
