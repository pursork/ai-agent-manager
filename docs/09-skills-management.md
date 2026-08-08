# 09. Skills 跨工具共享与发现/纳管（G6）

## 9.1 背景：Skills 是什么，为什么需要单独一个模块

Claude Code 和 Codex CLI 都支持"Skills"——文件系统里的一个目录，含一个必填的 `SKILL.md`（`name`+`description` frontmatter + 正文指令），工具在相关时自动加载或由用户 `/skill-name` 主动触发。两边都基于同一个开放 "Agent Skills" 标准的最小子集，**结构级别互通**：一份只用 `name`/`description`/正文的 `SKILL.md` 两边都能读。但 Claude Code 在此基础上扩展了额外 frontmatter（谁来触发、是否用 subagent 执行、动态上下文注入等），这些扩展字段不保证被 Codex 理解，反之 Codex 若有自己的扩展字段也不保证被 Claude 理解——**结构兼容，高级特性不保证互通**，这是本模块所有"共享"设计的边界声明，不承诺 100% 无损互换。

存储位置官方现状（`08` 已核实）：

| 工具 | 个人/用户级位置 | 是否受账号切换机制影响 |
|---|---|---|
| Claude Code | `~/.claude/skills/`（或当前生效的 `CLAUDE_CONFIG_DIR` 下的 `skills/`——**未官方确认**是否跟随 `CLAUDE_CONFIG_DIR` 重定向，见 `08`） | 未知，`03.7` 用符号链接方案兼容两种可能 |
| Codex CLI | `$HOME/.agents/skills/`（**固定**，不受 `CODEX_HOME` 影响） | 否，天然全局共享 |

此外两边都还有项目级位置（Claude `.claude/skills/`，Codex `.agents/skills/`）——**项目级 skills 明确出本模块范围**，它们和项目代码一起走 git，不是 `aam-skills` 要管的对象（如果项目已经是 git 仓库，管理方式和 `05.4` 对"完整项目同步"的态度一致：交给 git，不重复造轮子）。

## 9.2 设计原则：明文/git，不进加密通道

**Skills 内容默认不含密钥**，本质是可分享的工作流/提示词。因此本模块的同步策略和 `04`（凭据）、`05`（会话进度）完全不同：

- `aam-skills` **不依赖 `aam-sync`**，不把 skill 内容加密成 blob 传 WebDAV。
- 检测到某个纳管中的 skill 目录本身就是（或藏在）一个 git 仓库时，`aam skills status` 直接建议用户 `git push`/`git pull` 自己同步，`aam-skills` 只负责"发现+提示"，不接管 git 操作本身。
- 这意味着 `aam-skills` 是 Phase 0 定的 crate 依赖图里一个**例外分支**：`aam-skills → aam-core`，不经过 `aam-vault`/`aam-sync`（`02.1`）。

## 9.3 核心机制：中心化仓库 + 多处符号链接/Junction

要解决两个问题：

1. **Claude 内部**：账号切换用"N 个 Profile 目录"模型（`03.2`），如果每个 Profile 各自一份 skills 副本，会越用越漂移。
2. **Claude 与 Codex 之间**：两者的 skills 目录本来就是完全不同的路径（`~/.claude/skills` vs `$HOME/.agents/skills`），互相看不到对方的 skills。

统一方案：

- 每个被 `aam-skills` 纳管的 skill，物理内容有且只有一份，存放在**规范位置**——直接定为 `~/.claude/skills/<name>`，不新建独立的 `~/.aam/skills-store` 之类的第三个目录，减少一层间接、也符合"Claude 用户本来就习惯去这里找 skills"的直觉。
- 需要在别处可见时，在目标位置建立指向规范位置的符号链接（Unix）/ Junction（Windows）：
  - Claude 某个 Profile 目录：`<profile-dir>/skills/<name>` → `~/.claude/skills/<name>`（`03.7`，Profile 创建时自动做，因为这是"同一个工具内部保持一致"，没有"要不要分享"的疑问）。
  - Codex：`$HOME/.agents/skills/<name>` → `~/.claude/skills/<name>`（**显式命令触发**，见 9.5，因为这是"要不要把这个 skill 给另一个工具用"的主动决定，不该被默认打开）。
- 链接的建立/替换全程走 `aam-core::TransactionalOp`（`02.6`）：操作前检查目标路径当前是什么（不存在/空目录/已有真实内容/已经是别的链接），失败要能回滚，不留半成品——尤其是"目标位置已经有真实目录内容"这种情况，绝不能不问用户就删了替换成链接，必须先给出提示。

## 9.4 Windows 平台注意事项（记入 `08`，Phase 1 要实测）

Windows 上"符号链接"（`mklink /D`，或 `CreateSymbolicLink` API）默认需要管理员权限或开启"开发者模式"；**Junction**（`mklink /J`）不需要这些权限，只是限制在同一本地卷内、且只能指向目录（不能指向文件）。鉴于本项目 Phase 0 已经在"需要管理员权限"这件事上吃过亏（VS Build Tools 安装那次），Windows 平台**默认用 Junction**，不要求用户开启开发者模式或提权；Unix 平台用普通符号链接。这一点在 `aam-skills` 的 Windows/Unix 双实现里要分开处理，不能假设一套 API 两边通用。

## 9.5 数据模型

`aam-skills` 维护一份本机 skills 台账（暂定 `~/.claude/skills/.aam-skills-index.json`，和规范仓库放一起，方便肉眼查看）：

```json
{
  "skills": [
    {
      "name": "pdf-processing",
      "canonicalPath": "~/.claude/skills/pdf-processing",
      "managed": true,
      "shareTargets": ["codex", "claude:官方账号2"],
      "source": "local",
      "updateMode": "manual"
    },
    {
      "name": "some-community-skill",
      "canonicalPath": "~/.claude/skills/some-community-skill",
      "managed": true,
      "shareTargets": [],
      "source": "https://github.com/example/skill-repo@main",
      "updateMode": "manual"
    }
  ]
}
```

- `managed`：是否被 `aam-skills` 纳管（区分"发现了但用户还没 adopt"的 skill，见 9.6）。
- `shareTargets`：这个 skill 当前被显式共享到了哪些位置（Codex / 哪个 Claude Profile）。
- `source`：`"local"`（本机原创，不追踪更新）或 `"<git url>[@ref]"`（有上游可对比，见 9.7）。
- `updateMode`：`"manual"`（默认）或 `"auto"`——是否自动应用检测到的上游更新，和本项目其余"默认关闭、显式开启"的原则保持一致（`04.7`、`05.4` 都是同样的态度），不为这一项破例。

## 9.6 扫描发现与纳管（`aam skills scan` / `aam skills adopt`）——两段式，Phase 3

和 `05.7`-`05.9` 的会话发现完全对称的 UX：

1. `aam skills scan`：只读，扫描 `~/.claude/skills`、`$HOME/.agents/skills`、每个已注册 Claude Profile 目录下的 `skills/`（排除已经是指向规范位置的链接的项，那些已经在纳管中），打印一份"发现 N 个未纳管 skill"的清单。**不写任何台账，不建任何链接。**
2. `aam skills adopt <name> [--share-with codex[,claude:<profile>]]`：把某个发现到的 skill 正式纳入台账（`managed: true`），如果它当前物理位置不是规范位置（比如是在某个 Codex 专用目录里原创的），先把内容**移动**到 `~/.claude/skills/<name>`（用 `TransactionalOp`：快照原位置→移动→在原位置建回链接→验证→失败回滚），再按 `--share-with` 建立其余位置的链接。
   - `--share-with codex` 时，先解析目标 skill 的 `SKILL.md` frontmatter，如果发现非 `name`/`description` 的额外字段（大概率是 Claude 专属扩展），打印警告"该 skill 使用了 Claude 专属字段，Codex 不保证正确识别"，但不阻止操作（用户知情后仍可坚持）。

## 9.7 GitHub 来源的更新检查（`aam skills check-updates` / `aam skills update`）——Phase 3

只对 `source` 不是 `"local"` 的 skill 生效：

- `check-updates`：对每个有 `source` 记录的 skill 做一次 `git fetch`（源仓库通常比 skill 目录大，只 fetch 不 clone 全部工作区；如果 skill 只是源仓库里的一个子目录，用 sparse-checkout 或者干脆把整个源仓库克隆到一个缓存目录、规范位置只保留需要的子目录内容），比较本地内容 hash 和上游最新版本，列出"有更新可用"的 skill。
- `update <name>`：对指定 skill 应用上游最新内容——**默认必须显式调用这个命令才会真的改动本机文件**（`updateMode: manual`）；如果用户为某个 skill 单独或全局设置了 `updateMode: auto`，`aam` 可以在其他命令执行的间隙顺带做一次静默更新，但这是可选加强项，不是默认行为。
- 这不是一个真正的包管理器：不做依赖解析、不做语义化版本比较，只是"diff 一下、提示一下、需要的话手动应用"，符合 `00` 里"不重新发明 npm/cargo"的 Non-Goal 声明。

## 9.8 CLI 交付形态

```
aam skills list                                  # 列出已纳管 skill 及其链接目标（Phase 1）
aam skills status                                # 检测规范仓库/各链接目标状态是否健康，git 仓库则提示自行同步（Phase 1）
aam skills adopt <name> --share-with <targets>   # 显式跨工具/跨 Profile 共享（Phase 1，仅限已在规范位置的 skill）
aam skills scan                                  # 发现本机未纳管 skill（Phase 3）
aam skills adopt <name> [--share-with <targets>] # 完整纳管流程，含移动到规范位置（Phase 3 扩展 9.6）
aam skills check-updates                         # 检查 GitHub 来源 skill 是否有更新（Phase 3）
aam skills update <name>                         # 应用更新（Phase 3）
```

Phase 1 和 Phase 3 用同一个 `adopt` 子命令名，语义是渐进扩展（Phase 1 版本只处理"已经在规范位置、只是要不要多建几个链接"这个子集，Phase 3 版本补上"从非规范位置移动过来"的完整流程），不是两个不同命令，避免用户需要记两套命令名。

## 9.9 与其他模块的关系

- `03.7`：Claude Profile 创建时自动调用本模块的链接供给能力，是本模块在"工具内一致性"场景下的具体应用。
- `05.7`-`05.9`：会话的"扫描→本机记账→显式批准"三段式和本模块的"scan → adopt"是同一套 UX 哲学在两个不同数据域（会话 vs skills）的体现，`aam-memory` 和 `aam-skills` 是姊妹模块，但彼此没有代码依赖。
- `02.1`：`aam-skills` 依赖仅 `aam-core`，被 `aam-switcher`（Profile 创建时调用）和 `aam-cli`（暴露 `aam skills` 子命令）使用。
