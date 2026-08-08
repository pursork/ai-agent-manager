---
name: project-tracker
description: Lists the user's Claude Code projects and helps resume one by name. Use whenever the user asks things like "我有哪些项目", "xx 项目做到哪了", "继续 xx 项目", "resume xx project", "帮我 resume", "上次那个项目叫什么", or wants to see/update project status or find the cd + claude --resume command for a project they can't fully remember the path/name of.
---

# Project Tracker

Maintains and reads a lightweight index of every local Claude Code project so
the user never has to remember folder paths to pick up where they left off.

This copy ships with [ai-agent-manager](https://github.com/pursork/ai-agent-manager)
(`aam skills install-bundled project-tracker`) and is maintained alongside it
-- `aam-memory` reads this same `project-index.json` directly
(`docs/05-session-memory-bank-module.md` §5.1/`docs/08-open-questions-risks.md`
#9), so the two are meant to stay in sync rather than be two competing
trackers. The scripts additionally shell out to `aam whoami --tool claude`
to fill in `deviceId`/`profileLabel`, the cross-device fields `aam` needs;
if `aam` isn't installed/on PATH, those fields are simply left blank and
everything else keeps working exactly as before.

## Data sources

1. **`~/.claude/project-index.json`** — the source of truth. Each entry:
   ```json
   {
     "path": "D:\\研究\\lzy\\云边测试\\Gear-Sys",
     "name": "Gear-Sys",
     "lastSessionId": "286cce60-...",
     "lastActive": "2026-08-08T10:15:00+08:00",
     "created": "2026-07-01T09:00:00+08:00",
     "autoStatus": "验证研究工作文档并核实问题",
     "statusOverride": null,
     "authBackend": "oauth-subscription",
     "deviceId": "",
     "toolKind": "claude",
     "profileLabel": null
   }
   ```
   This file is kept up to date automatically by a SessionStart/SessionEnd
   hook (`scripts/track-session.ps1`) — you normally only need to **read**
   it, not maintain it by hand.

   `authBackend` records which backend was talking to Claude last time this
   project was touched: `oauth-subscription` (official `/login`), `api-key`,
   `auth-token`, `bedrock`, `vertex`, or `custom:<url>` (third-party/proxy
   endpoint via `ANTHROPIC_BASE_URL` -- this is what `aam`'s attached
   Providers set, so it's detected automatically, no aam-specific code
   needed here). `null` means unknown (pre-existing session from before this
   field existed). This is purely informational — see the resume-mode
   warning below for why it matters.

   `deviceId`/`profileLabel` are populated via `aam whoami` when available
   (empty string / `null` otherwise -- both are legitimate "not managed by
   aam yet" states, not errors).
2. **`~/.claude/sessions/*.json`** — Claude Code's own live registry of
   currently-running sessions (one file per running `claude` process), with
   real `cwd`, `pid`, `sessionId`, `status` (idle/waiting). Cross-reference
   this to tell the user if a project already has a session running, so you
   don't send them to open a duplicate one.

Display status = `statusOverride` if set, otherwise `autoStatus`, otherwise
"(尚无记录)".

## Modes

### 1. List projects ("我有哪些项目", "项目进度", `/projects`)
Read both files, join on path (case-insensitive), sort by `lastActive` desc.
Print a compact table: 项目名 / 路径 / 状态 / 最后活跃时间, and mark any
project whose path matches a running session with 🟢 (include its PID).

### 2. Resume a project ("继续 xx 项目", "resume xx", "回到 xx 项目")
1. Fuzzy-match the user's query against `name` and the trailing path segment
   of every entry (support partial / pinyin-ish / case-insensitive matches).
   If ambiguous, list the candidates and ask which one.
2. Check `~/.claude/sessions/*.json` for a live session on that path.
   - If one exists: tell the user it's already running (show terminal/PID
     info you have) and suggest switching to it instead of opening a new one.
   - If none: output the two commands the user should run, using the
     matched entry's `path` and `lastSessionId`:
     ```
     cd "<path>"
     claude --resume <lastSessionId>
     ```
     Mention that dropping the session id (`claude --resume` alone) opens
     Claude Code's interactive picker for that folder if they'd rather choose
     a different past session.
3. **Auth-backend mismatch warning.** Determine the *currently active*
   backend the same way `track-session.ps1`'s `Get-CurrentAuthBackend` does
   (check `CLAUDE_CODE_USE_BEDROCK` / `CLAUDE_CODE_USE_VERTEX` /
   `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY` env
   vars, default `oauth-subscription`). If it differs from the entry's
   recorded `authBackend` (and that field isn't `null`), warn the user
   *before* they run the resume commands: extended-thinking sessions resumed
   under a different backend than the one that generated them can fail with
   a `400 Invalid signature in thinking block` error (confirmed, recurring
   issue in both Claude Code and OpenAI Codex when switching backends/auth
   methods — it's a real risk, not hypothetical). Suggest switching back to
   the original backend first if possible, or warn that resume may fail.
4. You (Claude) cannot change the user's shell's working directory — always
   give the commands for the user to run, don't claim to have run `cd`.

### 3. Update a project's status ("记一下 xx 项目状态：...", "把 xx 项目标记为...")
Fuzzy-match as above, then edit that entry's `statusOverride` field in
`project-index.json` directly (read, modify, write back the whole file —
small file, no need for the PowerShell script for this).

### 4. Rename / cleanup ("把 xx 项目改名叫...", "删掉 xx 项目的记录")
Same pattern: locate the entry, edit `name` or remove the entry, write the
file back.

## First-time setup / re-seeding
If `project-index.json` looks empty or is missing entries the user expects,
run the one-time backfill (safe to re-run, only adds missing entries, never
overwrites existing ones):
```
powershell -NoProfile -ExecutionPolicy Bypass -File "~/.claude/skills/project-tracker/scripts/backfill-index.ps1"
```

Prefer `aam session scan` + `aam session adopt` instead if the user has
`aam` set up with multiple Profiles/Codex sessions to backfill from -- this
script only ever looks at the one fixed `~/.claude/projects` directory.

## Enabling the automatic hook (manual step, not done for you)
`aam skills install-bundled project-tracker` only places these files under
`~/.claude/skills/project-tracker` -- it deliberately does **not** edit
`~/.claude/settings.json` (aam never rewrites a tool's live config without
being explicitly asked to). To make tracking automatic, add to
`~/.claude/settings.json`'s `hooks` section:
```json
{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"~/.claude/skills/project-tracker/scripts/track-session.ps1\"" }] }],
    "SessionEnd":   [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"~/.claude/skills/project-tracker/scripts/track-session.ps1\"" }] }]
  }
}
```
(Merge into any existing `hooks` block rather than overwriting it.)

## Troubleshooting
- If new sessions aren't showing up automatically, check
  `~/.claude/skills/project-tracker/scripts/track-session.log` for hook
  errors, and confirm `~/.claude/settings.json` still has the `SessionStart`
  and `SessionEnd` hooks pointing at `track-session.ps1`.
- If a `claude --resume` the user just ran (per your suggestion) comes back
  with `400 ... Invalid signature in thinking block`, that's the known
  cross-backend/cross-account thinking-signature issue (see `authBackend`
  above) — tell the user plainly what happened and that switching back to
  the original backend before resuming is the most reliable workaround;
  it isn't something this skill can fix after the fact.
- `deviceId`/`profileLabel` staying blank even though `aam` is installed
  usually means either `aam sync init`/`aam device join` was never run (no
  device identity yet -- `deviceId` blank is then correct), or this Claude
  session wasn't launched via `aam claude <label>` (so there's no matching
  Profile's `CLAUDE_CONFIG_DIR` to look up -- `profileLabel` blank is then
  also correct). Neither is an error condition.
