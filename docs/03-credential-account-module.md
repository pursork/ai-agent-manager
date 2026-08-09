# 03. 账号 / Provider 切换模块（Phase 1，最高优先级）

## 3.1 核心概念：Account、Provider、Profile 三层

沿用 `codex-skill` "account 与 provider 严格隔离成两个状态机"的原则，但在其上补一层面向用户的组合概念：

- **Account**（账号身份）：一份官方订阅登录凭据。Claude 侧 = 一个完整的 `CLAUDE_CONFIG_DIR` 目录；Codex 侧 = 一份 `auth.json`。
- **Provider**（后端/接口）：官方端点，或第三方/自建端点（CPA、DeepSeek V4 Flash……）。
- **Profile**（可切换的运行单元，面向用户）：一个 `(tool, account, provider)` 三元组，是用户在 GUI/CLI 里实际"点一下就切过去"的东西。

**架构边界依然不可协商**：写 Account 的代码模块永远不碰 Provider 的状态，反之亦然——Profile 只是"选择哪个 Account + 哪个 Provider 组合去启动一个终端会话"的运行期概念，不是把两者的存储合并。

## 3.2 关键发现：两个工具都支持"整配置目录重定向"

- **Claude Code**：官方支持的 `CLAUDE_CONFIG_DIR` 环境变量，设置后整个 `~/.claude` 等价目录（`settings.json`、`.credentials.json`、`projects/`……全部）被重定向到指定路径。这是**目录级别**的隔离。
- **Codex CLI**：`codex-skill` 现有代码里全程使用 `%CODEX_HOME%`，隐含 Codex 同样支持一个等价的整目录重定向环境变量。**这一点需要在正式开发前对照 Codex 官方文档二次确认其精确语义**（是否真的重定向包括 `config.toml` 在内的一切，还是只影响部分路径）——记入 `08-open-questions-risks.md`。

**如果 `CODEX_HOME` 语义与 `CLAUDE_CONFIG_DIR` 等价**，那么两个工具的 Account 切换可以统一成同一种更简单、更安全的模式：

> **不做"原地互换文件"，而是维护 N 个完全独立、预先物化好的配置目录（每个对应一个 Profile），切换 = 启动新终端进程时选择把哪个目录路径塞进对应的环境变量。**

这比 `codex-skill` 现在对 Codex 采用的"原地覆盖 `auth.json` + 快照回滚"更安全——因为**从不原地修改一个"当前唯一生效"的文件**，天然没有"写到一半"或"回滚失败留下半成品"的风险类别，若某个 Profile 目录本身损坏，换回上一个 Profile 只是换个路径，旧目录原封不动。

**Phase 1 的落地策略**：
- **Claude 侧**：直接采用"N 目录 + 启动时选路径"模式（零额外复杂度，就是标准用法）。
- **Codex 侧**：如果 `CODEX_HOME` 语义确认等价，同样迁移到"N 目录"模式；如果不等价（比如只重定向部分内容），则**继续沿用 `codex-skill` 现有的、已经过production验证的"原地写 `auth.json` + 快照/原子写/存活验证/回滚"机制**，原样移植到 Rust（不为了架构统一而放弃一个已经工作良好的实现）。两条路径在 `aam-switcher` 里对应两个不同的 trait 实现，由配置决定用哪个，不是二选一的一次性决定。

## 3.3 "切换"的作用范围：只影响新启动的进程

不论走哪种机制，**账号/Provider 切换在语义上只对"接下来新启动的 `claude`/`codex` 进程"生效**，不会让一个已经在运行的会话瞬间换账号——两个工具都只在进程启动时读取一次配置，不会中途热重载凭据。这是要在 GUI/CLI 交互文案里明确告知用户的行为边界，不能让用户误以为"切换"能让正在运行的终端会话立刻变身。

## 3.4 Provider 接入的可扩展接口

`codex-skill` 现有的 Provider 切换是"写死的 `if/switch` 分支"（CPA、DeepSeek 各一个分支），文档里明确说明这是"拍板过的、故意简单"的设计。`ai-agent-manager` 用 Rust 的优势往前走一步，定义一个 trait：

```rust
/// 一个第三方/自建 Provider 的接入方式。
/// CPA、DeepSeek V4 Flash 是 Phase 1 的两个具体实现；
/// trait 本身要求预留足够扩展性，但 Phase 1 只需要跑通这两个。
trait Provider {
    /// Provider 的稳定标识，如 "cpa" / "deepseek-v4-flash"
    fn id(&self) -> &str;

    /// 生成该 Provider 需要写入目标工具配置的内容
    /// （Claude: ANTHROPIC_BASE_URL/ANTHROPIC_API_KEY 或等价环境变量;
    ///  Codex: config.toml 的 model_provider 块，沿用 codex-skill 的 command-backed
    ///  bearer token 模式，密钥不落盘明文）
    fn materialize(&self, target: ToolKind) -> ProviderConfig;

    /// 存活验证：真实发一个请求确认这个 Provider 当下真的可用
    /// （沿用 codex-skill 的 `GET /v1/models` + 一次真实补全请求的验证方式）
    fn verify(&self, cfg: &ProviderConfig) -> Result<(), VerifyError>;
}
```

Claude 侧第三方接入的两条可选实现路径（详见 `01-prior-art.md` 1.3 节），Phase 1 先选**方案 B（直接改写 `ANTHROPIC_BASE_URL`/`ANTHROPIC_API_KEY` 等环境变量）**，因为它和"N 目录 + 环境变量注入"的账号切换模型是同一套机制、同一个启动包装器负责，不需要额外托管一个 CCR 常驻进程；如果后续需要 CCR 的智能路由能力（按任务类型/token 数路由），再作为可选的高级 Provider 实现追加，不阻塞 Phase 1。

## 3.5 标准操作序列（继承自 codex-skill，是本模块唯一强制规范）

无论是"N 目录选择"还是"原地写文件"哪种底层机制，对外暴露的操作语义必须统一遵守：

1. **前置检查**：确认目标工具的进程当前未在运行（避免运行中的进程和即将发生的切换操作互相干扰）。
2. **快照**：切换前的状态（不论是"记录当前选中的 Profile 是谁"还是"备份即将被覆盖的文件"）先保存。
3. **原子写**（仅"原地写文件"路径需要；"N 目录选择"路径这一步天然不存在，因为没有原地修改）。
4. **存活验证**：真实验证新 Profile 能用，不能只验证"文件格式对"。
   - Codex 侧：复用 `codex login status`（账号）/ 真实请求（Provider）。
   - **Claude 侧的等价存活检查命令待确认**——候选是 `claude` 的某个 status 子命令或 `/status` 等价的非交互调用方式，需要在开发 Phase 1 之前对照官方 CLI 文档核实（记入 `08`），不要凭猜测硬编码。
5. **失败自动回滚**：验证失败 → 恢复切换前状态 → 再次验证 → 报错退出，不留半成品。

## 3.6 Phase 1 的 CLI 交付形态（先于 GUI）

按 Roadmap 优先级，Phase 1 只交付 CLI，不做 GUI：

```
aam profile list                       # 列出所有已知 Profile（工具+账号+Provider 组合）
aam profile add --tool claude ...      # 引导式添加一个新账号/Provider 组合成 Profile
aam claude <profile-label> [-- ...]    # 用指定 Profile 的环境启动一个新的 claude 进程
aam codex <profile-label> [-- ...]     # 同上，Codex
aam profile verify <profile-label>     # 单独触发一次存活验证，不切换
```

这组命令行为上是 Codex 侧 `codex-skill` 和 Claude 侧尚不存在的等价物的统一入口，是后续 GUI（Phase 4，`06`）"新建终端标签页时选 Profile"这个交互的直接后端。

**工作目录**：`aam claude/codex <label>` 启动子进程时不设置、也不依赖 `current_dir`——`aam-cli` 的实现（`std::process::Command::new(binary)`）直接继承调用者 shell 当前的工作目录，跟直接跑 `claude`/`codex` 完全一样。用户还是"先 `cd` 到项目目录，再 `aam claude <label>`"，这是刻意的设计而不是遗漏：一来 `05.3` 已经明确"aam 不能替用户 cd"；二来 `project-tracker`（`09.10`）的 SessionStart hook 靠的正是进程真实的工作目录来判断项目路径，`aam` 如果自作主张改了 cwd，会直接破坏这个已经在用的机制。

## 3.7 Claude Profile 的 Skills 一致性

`3.2` 确认 Claude 侧账号切换用"N 目录 + 启动时选 `CLAUDE_CONFIG_DIR`"模型——这意味着每个 Profile 有自己独立的 `<profile-dir>/skills/`。这带来一个新问题：**用户精心积累的 Skills，会不会因为切换 Profile 就"看不见"了？**

- **官方文档没有回答这个问题**：`CLAUDE_CONFIG_DIR` 本身就不在官方文档里（社区验证的行为），更谈不上它是否连带重定向 `skills/` 子路径。这一点记入 `08`，Phase 1 要实测。
- **不管实测结果如何，设计上都统一处理**：每个 Claude Profile 目录创建时，`aam-switcher` 调用 `aam-skills`（`09`）提供的能力，把 `<profile-dir>/skills` 做成指向本机规范 Skills 仓库（`~/.claude/skills`）的符号链接（Unix）或 Junction（Windows）。
  - 如果实测发现 `CLAUDE_CONFIG_DIR` **不**重定向 skills（即 Claude 无论哪个 Profile 启动都固定读 `~/.claude/skills`）——这一步是无害的空操作，`<profile-dir>/skills` 本来就用不上。
  - 如果实测发现 `CLAUDE_CONFIG_DIR` **确实**重定向 skills——这一步避免了"每个 Profile 各自维护一份 skills 副本，改一个不改另一个，越用越漂移"的问题：所有 Profile 通过链接共享同一份物理内容。
- **Codex 侧不需要这一步**：`08` 已确认 Codex 的 Skills 固定从 `$HOME/.agents/skills` 读取，完全不受 `CODEX_HOME` 影响，天然是全局共享的，不会因为切换 Codex Profile 而分裂。
- **这个符号链接/Junction 操作本身也遵循 `02.6` 的 TransactionalOp 原则**：创建/替换前检查目标路径当前状态（可能是空目录、真实目录、已经是链接），失败要能回滚到"链接创建前"的状态，不留半成品。
- **Claude↔Codex 跨工具共享**（同一个 skill 同时被两个工具用）是同一套符号链接/Junction 机制的另一个应用，但那是显式命令触发（`aam skills adopt --share-with codex`），不是 Profile 创建时自动做的——因为不是所有 skill 都适合分享给另一个工具（可能用了 Claude 专属 frontmatter）。完整设计见 `09-skills-management.md`。
