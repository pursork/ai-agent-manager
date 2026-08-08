# 01. 现有开源方案调研

本文档记录设计 `ai-agent-manager` 之前调研过的所有相关项目，含具体机制、真实的 star/活跃度数据（调研时间：2026-08-08），以及每个项目对我们设计的具体影响。所有数据来自 GitHub API 实测，不是印象。

## 1.1 `codex-skill`（本机已有，`C:\Users\16500\Desktop\codex-skill`）

单机 Codex CLI 账号 + Provider 切换工具，PowerShell（Win11）/ Bash（Debian13）双实现，是本项目**最重要的工程模式来源**。

### 核心设计：account 与 provider 严格隔离成两个状态机

- **Account 模块**（`402-Codex-Account-Switcher`）：只管 `%CODEX_HOME%\auth.json`（官方 ChatGPT OAuth 登录凭据），只认 `auth_mode == "chatgpt"`，拒绝任何其他模式。
- **Provider 模块**（`401-Codex-CPA-DeepSeek-Switcher`）：只管 `config.toml` 里的 `model_provider` / `model_providers.*` 块（第三方/自建端点，如 CPA=自建 CLIProxyAPI、DeepSeek V4 Flash）。
- **两者硬性不越界**：account 模块从不写 `config.toml`，provider 模块从不写 `auth.json`。这条边界在 SKILL.md 里被称为"不可协商的架构边界"。

**→ 对我们的意义**：`03-credential-account-module.md` 里 Claude/Codex 各自的 Account Switcher 与 Provider Switcher 必须是两个独立 Rust 模块/trait，不共享写路径。

### 核心设计：每次变更 = 快照 → 原子写 → 存活验证 → 失败自动回滚

标准操作序列（account 和 provider 模块都遵循）：

1. 前置检查：确认 Codex 进程未在运行（`Assert-CodexNotRunning`），避免写入时被进程覆盖。
2. **切换前先把"当前状态"存一份快照**（`Sync-CurrentAuth 'before-switch'`），保证切换失败也不会丢失原状态。
3. 额外做一次带时间戳的滚动备份（默认保留 30 份）。
4. **原子写**：先写临时文件，再 `File.Replace` 整体替换，不会出现"写到一半"的中间态。
5. **存活验证**：重新解析写入后的文件校验指纹匹配；再实际调用 `codex login status`（account）或对第三方端点发 `GET /v1/models` + `POST /v1/responses`（provider），确认新配置真的能用，不是"文件格式对但连不上"。
6. 任何一步失败 → 自动恢复切换前的备份 → 再次验证 → 报错退出。绝不留下"半成品"状态。

**→ 对我们的意义**：这是整个 `ai-agent-manager` 凭据模块唯一强制的操作范式，写进 `03` 作为规范，不允许"抄近路"直接覆盖文件。

### 账号身份识别与去重：指纹

导入一份 `auth.json` 或"ChatGPT 登录导出 JSON"时，解码其中 JWT（`id_token`/`access_token`）的 payload，交叉核对 `account_id`/`email`/`user_id`/`sub` 字段互相一致后，算出稳定指纹：

```powershell
$identity = @($userId.ToLowerInvariant(), $subject.ToLowerInvariant(), $email.ToLowerInvariant(), $accountId) -join '|'
$fingerprint = (Get-Sha256Hex $identity).Substring(0, 20)
```

每个账号按指纹存一份 `<fingerprint>.auth.dpapi`（Windows：DPAPI CurrentUser 加密）+ `<fingerprint>.meta.json`（标签、邮箱、套餐类型、过期时间等明文元数据）。Linux 版本对应是明文文件 + `chmod 600`，**项目文档里诚实注明这是弱于 DPAPI 的保护**，不假装两个平台一样安全。

**→ 对我们的意义**：`02-architecture.md` 的"本地态安全分层"直接采用这个思路——本地缓存按 OS 能力做最大努力加固（Windows DPAPI、Linux 文件权限），但**不能把本地态的安全强度当成跨设备同步的安全基础**，跨设备走独立的、与 OS 无关的加密（见 `04`）。指纹算法本身也值得直接复用到 Claude 侧账号识别。

### 第三方 Provider 的密钥保护：command-backed bearer token

API Key 从不以明文写入 `config.toml`。而是加密存一份 token 文件，`config.toml` 里配置 Codex 在每次请求时去"执行一个命令"取 token：

```toml
[model_providers.cliproxyapi.auth]
command = "powershell.exe"
args = ["-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File","<HelperFile>","-TokenFile","<CpaTokenFile>"]
timeout_ms = 5000
```

`<HelperFile>` 是运行期生成的小脚本，职责仅仅是解密 token 文件、把明文 token 打到 stdout。

**→ 对我们的意义**：这是"密钥不落盘明文，同时兼容目标程序只认'配置文件'这种集成方式"的通用模式，`03` 里 Claude 侧的第三方 Provider 接入（如果 Claude Code/CCR 支持等价的 command 型 auth）应该复用这个思路；如果不支持，退化方案是"配置文件里只放一个占位符，真正的 key 由我们的进程在启动时注入环境变量"。

### 明确指出的经验教训（写入我们的 `08-open-questions-risks.md`）

- 第三方厂商文档可能描述不存在的配置字段（真实事故：DeepSeek 文档写的 `preferred_auth_method` 字段其实不存在，Codex 的 TOML 解析器对未知字段是硬报错不是警告）——**配置 schema 必须对照目标程序的源码（如 `codex-rs`），不能只信厂商文档**。
- "只在本地缓存缺失时才检查更新"这种设计，会让用户卡在一个已损坏的旧版本上、连"检查更新"这个菜单项本身都因为旧版本崩溃而进不去——更新检查必须在任何可能因旧版本而崩溃的逻辑**之前**无条件跑一次。

### 不做的事（对我们同样重要）

`codex-skill` **完全不做**跨机器凭据同步（文档明确写"DPAPI 保险库无法迁移到另一台机器，需要在目标设备重新导入/重新登录"），也**完全不做**会话/项目追踪。这两块是 `ai-agent-manager` 相对于它的全部增量价值所在。

---

## 1.2 Claude Code 账号切换：官方机制 + 社区工具

### 官方机制：`CLAUDE_CONFIG_DIR`

Anthropic 官方支持的多实例隔离方式——设置这个环境变量后，Claude Code 的整个配置目录（相当于 `~/.claude`，含 `settings.json`、`.credentials.json`、`projects/` 等全部内容）都会指向自定义路径。这是 Claude 侧账号隔离的**唯一官方支撑点**，和 Codex 的"单文件 `auth.json` 互换"是本质不同的隔离粒度——**是整个目录级别，不是单文件级别**。

**→ 对我们的意义**：`03-credential-account-module.md` 里 Claude 的账号切换 = "维护 N 个完整的 `CLAUDE_CONFIG_DIR` 目录 + 切换时设置/改写这个环境变量（或者用启动包装器，每次按当前选中账号传入对应的 `CLAUDE_CONFIG_DIR`）"，而不是像 Codex 那样只互换一个凭据文件。这意味着 Claude 侧天然可以做到"账号之间连 `project-index.json`/hooks 配置都互相隔离"，也意味着**我们今天做的 `project-tracker` 技能，如果用户切换了 `CLAUDE_CONFIG_DIR`，需要在每个账号目录下各装一份**——这是 `05` 要处理的实际问题。

### 社区工具对比（GitHub 实测数据，2026-08-08）

| 项目 | ⭐ | 语言 | 最近推送 | 说明 |
|---|---|---|---|---|
| [`realiti4/claude-swap`](https://github.com/realiti4/claude-swap) | 1,609 | Python | 2026-08-04 | 目前最成熟/最多人用的 Claude 账号切换工具，额外做了限流自动轮换 + 用量看板 + 并行会话 |
| [`quinnjr/claude-code-profiles`](https://github.com/quinnjr/claude-code-profiles) | 59 | Shell | 2026-08-05 | 每个 profile = 完整独立配置目录（含 settings/credentials/MCP/CLAUDE.md/history） |
| `JakubKontra/claude-profile-manager` | 15 | Go | 2026-04-13 | 类似定位，热度低很多 |
| `ftery0/claude-account-switch` | 4 | JS | 2026-07-31 | 交互式向导，热度很低 |
| `guibes/claude-profile-switch` | 3 | Shell | 2026-04-21 | 热度很低 |

**→ 结论**：`claude-swap` 是目前事实上的标准参考实现，其"自动限流轮换"是个加分特性（我们 Roadmap 里可以作为 Phase 1 之后的加分项，非必需）；其余项目本质上都是同一个 `CLAUDE_CONFIG_DIR` 技巧的不同包装，没有额外值得抄的机制。

Anthropic 官方仓库里也有多个用户请求"原生多账号支持"的 issue（如 #20549、#35856、#44687），截至调研时均未被官方实现——说明这确实是一个真实存在、社区反复要求、官方尚未填补的空白，我们做这件事是有明确需求验证的。

---

## 1.3 第三方 API 路由：`musistudio/claude-code-router`（CCR）

**36,492⭐**，TypeScript，最近推送 2026-08-07（非常活跃）。定位是"本地控制面"：Claude Code 把请求发到本地 `http://127.0.0.1:3456`，CCR 在背后决定实际路由到哪个后端（OpenAI 兼容/Anthropic Messages/Gemini/OpenRouter/DeepSeek/SiliconFlow/Moonshot/Mistral/Z.AI 等），并支持按任务类型/token 数等规则动态路由，支持 Claude Code 里 `/model provider,model` 直接切换。

**→ 对我们的意义**：这是 Codex 侧 CPA/DeepSeek Provider 切换在 Claude 侧的等价物/灵感来源。两条可选路径写进 `03`：
- **方案 A（重用）**：`ai-agent-manager` 不重新发明路由层，账号/Provider 切换模块在"选择第三方 API"时，本质是"配置并托管一个本地 CCR 实例（或类似的代理），Claude Code 始终指向 `127.0.0.1:xxxx`"。
- **方案 B（自研）**：参照 codex-skill 的思路，直接改写 Claude Code 能识别的环境变量/配置（`ANTHROPIC_BASE_URL`、`ANTHROPIC_API_KEY` 等）做端点切换，不引入额外的路由进程。

两者的取舍（复杂度 vs 灵活度）留到 `03` 详细展开，此处只记录两个选项的存在。

---

## 1.4 `farion1231/cc-switch` —— 参考了但明确不采用其思路

**125,510⭐**，Rust + Tauri 2，跨平台桌面应用。官方定位："The All-in-One Manager for Claude Code, Claude Desktop, Codex, Gemini CLI, Grok Build, OpenCode, OpenClaw & Hermes Agent"。

**实测特征**：
- 支持 8 款不同的 Agent 工具（不只是 Claude+Codex）。
- README 首屏即为赞助商列表（Kimi/PackyCode/ZetaAPI 等 API 中转服务的联盟推广链接），商业化程度高。
- 功能范围覆盖账号/Provider 切换 + Skills 管理 + WSL 支持等，是一个"大而全"的商业化开源产品。

**用户明确评价**：过于臃肿、设计上不合理、过于复杂。调研后认同这个判断的依据是**范围蔓延**（8 款工具而不是用户实际用的 2 款）和**商业化耦合**（赞助商生态嵌入核心 README/流程），而不是 Tauri 这个技术选型本身有问题——事实上它证明了"Rust 后端 + 桌面应用"这条路径在这个问题域是可行的、有大量真实用户的。

**→ 对我们的意义**：`ai-agent-manager` 的核心设计边界声明——**只做 Claude Code + Codex CLI 两款工具，不做成插件市场/多工具全家桶，不引入任何商业化/赞助商机制**。技术选型上我们选纯 Rust（iced）而非 Tauri，是因为 Windows 下终端组件的可用性问题（见 `06`），不是因为否定 cc-switch 的 Tauri 选择本身。

---

## 1.5 会话/项目记忆：Memory Bank 模式 + 本项目已有的 `project-tracker`

社区里 Cline/Roo-Code 生态首创的"Memory Bank"模式——用几个约定俗成的 Markdown 文件（`progress.md`/`activeContext.md`/`decisionLog.md` 等）把项目状态持久化在工作目录里，让 Agent 每次开工先读、收工前更新。核心价值：**不依赖任何 Agent 工具自己的 session 状态，纯文件、纯约定，天然跨工具**（Claude Code 和 Codex 都能读写普通 Markdown/JSON 文件）。

具体参考实现（`GreatScottyMac/roo-code-memory-bank`，1,677⭐，但更新停留在 2025-05，偏 Roo Code/VSCode 插件生态，不直接适用于纯 CLI 场景）本身不直接复用，但**模式本身**——文件化、Agent 主动读写、不依赖工具内部状态——是 `05-session-memory-bank-module.md` 的方法论基础。

本机已有的 `~/.claude/skills/project-tracker`（今天完成）是这个模式的一个具体单机实现：`project-index.json`（项目名/路径/一句话状态/最后活跃时间/`authBackend`），配合 Claude Code 的 `SessionStart`/`SessionEnd` hook 自动维护。**`05` 的任务是把这个 schema 原样作为本地缓存层，加一层加密后同步到 WebDAV 的跨设备索引**，schema 需要新增 `deviceId`、`toolKind`（claude/codex）等字段。

**另外调研过、评估后不采纳的更重方案**（详细理由存档于 `08-open-questions-risks.md`）：`claude-task-master`（27,945⭐，PRD 拆解成结构化任务列表）、`BMAD-METHOD`（51,625⭐，完整敏捷开发方法论，PRD→架构→story 全程留痕）——两者都比用户实际需要的"记住做到哪了"重得多，属于"引入一整套开发方法论"而不是"轻量记忆机制"，且 BMAD 对 Codex CLI 的支持目前有已知 bug（issue #1782：Codex CLI 装完 BMAD 后读不到上下文）。

---

## 1.6 GUI 终端组件：`alacritty_terminal` / `iced_term` / `egui_term`

- **`alacritty_terminal`**：从 Alacritty 主程序拆分出的可复用终端仿真核心库（VTE 解析、PTY 管理、`Term<T>` 状态结构），多个第三方项目已经在其之上构建自定义终端而不用重写仿真逻辑。
- **`Harzu/iced_term`**：基于 `iced` + `alacritty_terminal` 的终端组件，**官方明确在 macOS / Linux / Windows 三端测试过**。
- **`Harzu/egui_term`**：同类组件但基于 `egui`，**官方明确"尚未在 Windows 上测试"**，且自述"仍在开发中，未提供完整终端特性"。

**→ 对我们的意义**：这是本轮唯一一处技术选型是由调研数据（而非用户偏好）决定的——鉴于用户主力设备是 Windows，`iced_term` 的 Windows 兼容性是明确验证过的、`egui_term` 不是，因此 `06-gui-terminal-shell.md` 选定 `iced` + `iced_term` 而非 `egui` 生态。两者都基于同一个 `alacritty_terminal` 底层，所以这个选择只影响 GUI 框架层，不影响终端仿真的正确性。

---

## 1.7 参考总表

| 项目 | ⭐（实测） | 采纳程度 |
|---|---|---|
| `codex-skill`（本机） | — | **核心工程模式全部采纳**，Rust 重实现的行为规范基准 |
| `realiti4/claude-swap` | 1,609 | 机制（`CLAUDE_CONFIG_DIR`）采纳，代码不直接复用（语言不同） |
| `musistudio/claude-code-router` | 36,492 | 作为 Provider 路由的可选依赖/灵感来源 |
| `farion1231/cc-switch` | 125,510 | **范围与技术选型均不采纳**，作为"反面参考"写入设计边界 |
| Memory Bank 模式 / `project-tracker` | — | 方法论 + schema 全部采纳，是 `05` 的直接起点 |
| `claude-task-master` | 27,945 | 不采纳（过重） |
| `BMAD-METHOD` | 51,625 | 不采纳（过重 + Codex 支持有已知 bug） |
| `alacritty_terminal` / `iced_term` | — | **直接作为依赖**，GUI 终端组件地基 |
