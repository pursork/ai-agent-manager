# 08. 未决问题与风险台账

本文档汇总前面文档里标记为"待确认/待调研"的所有事项。

## 8.1 曾阻塞 Phase 1 的问题（已解决）

| # | 问题 | 结论 | 来源 |
|---|---|---|---|
| 1 | `CODEX_HOME` 环境变量的精确语义 | **确认等价于整目录重定向**：`config.toml`、`auth.json`、`sessions/YYYY/MM/DD/rollout-*.jsonl`（`codex resume` 依赖的会话记录）、`history.jsonl`、`hooks.json`、日志/缓存、`profile-name.config.toml` 全部在 `CODEX_HOME` 根下（默认 `~/.codex`）。**例外：Skills 不受影响**，固定从 `$HOME/.agents/skills`（+ 仓库级/管理员级/内置）读取，天然全局共享，不随 Profile 分裂——见 `09`。据此 `aam-switcher` 的 Codex backend 采用和 Claude 相同的"N 目录 + 启动时选路径"模型，不再需要 codex-skill 原地写文件的回退分支（`03.2`、`07` Phase 1 已更新）。 | [developers.openai.com/codex/config-advanced](https://developers.openai.com/codex/config-advanced)，[developers.openai.com/codex/skills](https://developers.openai.com/codex/skills) |
| 2 | Claude Code 侧"存活验证"该调用什么命令 | **确认存在官方命令**：`claude auth status`（默认 JSON，`--text` 可读格式），登录时退出码 0，未登录退出码 1。对应 GitHub Issue #1886（"Make status checkable from command line"）已于 **2.1.41 版本修复关闭**（`state_reason: completed`）。`03.5`/`03.6` 的存活验证步骤据此接入。 | [code.claude.com/docs/en/cli-reference](https://code.claude.com/docs/en/cli-reference)，[anthropics/claude-code#1886](https://github.com/anthropics/claude-code/issues/1886) |
| 5 | Rust WebDAV 客户端 crate 选型（原 8.3 表格） | **确认不引入专门的 WebDAV crate**：GET/PUT/MKCOL/DELETE 都是普通 HTTP 方法 + Basic Auth，直接复用 Phase 1 已验证的 `ureq`（`ureq::request(method, url)` 支持任意方法）；PROPFIND 不需要，用 GET 404 判断"文件不存在"即可满足 push/pull。`04.8` 据此更新。 | Phase 2 实现时的技术选型（无需外部调研，`ureq` 是已在用的已验证依赖） |
| 15 | 账号凭据本身（Claude/Codex 官方登录态，而非 Provider 配置）该如何同步（原 8.3 表格） | **确认只同步单个凭证文件**（Claude `.credentials.json`、Codex `auth.json`，均已在本机真实文件上核实），不同步整个配置目录。**发现真实的技术差异并据此定案**：Claude 的 `accessToken` 实测不是 JWT，没有可解析的身份声明，WebDAV key 用本地 Profile 的 label；Codex 的 `auth.json` JWT 带身份声明，key 用 codex-skill 已验证算法（`SHA256(user_id\|subject\|email\|account_id)` 截 20 位十六进制，直接读取真实源码移植）派生的指纹，不随 token 刷新变化。新增 `accounts.json.age` 索引解决"没有 PROPFIND 就发现不了 vault 里有哪些账号"的问题。`04.10` 据此新增，`aam sync push-account/list-accounts/pull-account` 已实现。 | 本机真实文件核实 + `C:\Users\16500\.codex\codex-interface-manager\src\account.ps1`/`common.ps1` 源码 |
| 9 | `project-tracker`（现有单机技能）与 `aam-memory`（本模块）的具体桥接方式 | **确认选方案 (b) 的最简形式**：`aam-memory::ProjectIndex` 直接读写 `$HOME\.claude\project-index.json` 这个真实文件（本机核实 `track-session.ps1`/`backfill-index.ps1` 固定写这一个路径，不感知 `CLAUDE_CONFIG_DIR`），不改 hook 脚本、不复制一份。新字段全部 `serde(default)` 兼容旧记录。**连带发现一个真实 bug**：hook 脚本用 PowerShell `Out-File -Encoding utf8` 写文件，默认带 UTF-8 BOM（本机文件实测 `ef bb bf` 开头），`serde_json` 不会自动跳过，读取直接失败——`index.rs` 已加上剥离逻辑+回归测试。 | 本机真实 `project-index.json` 文件 + 实测运行 `aam project list` 复现的解析失败 |
| 10 | `~/.codex/session_index.jsonl` 元数据缓存是否真实存在、格式是否稳定 | **确认本机不存在**（`$CODEX_HOME` 根目录实测没有这个文件）。`scan_codex_sessions` 据此直接扫 `sessions/**/rollout-*.jsonl` 作为主路径，不实现从未被验证需要过的"优先读快速索引"分支。**连带核实**真实 rollout 文件的 `session_meta` 头部行结构：`payload.cwd`/`payload.id`，**没有** `model_provider` 字段（`05.7` 原文猜测的字段不存在），也没有类似 `ai-title` 的自动摘要字段（印证 `05.8`"Codex autoStatus 默认留空"是对的）。 | 本机真实 `~/.codex/sessions/**/rollout-*.jsonl` 文件 |

## 8.2 Phase 1 内需要实测、但不阻塞开始编码的问题

| # | 问题 | 影响范围 | 应对方式 |
|---|---|---|---|
| 3 | `CLAUDE_CONFIG_DIR` 是否连带重定向 `skills/` 子路径 | `03.7`、`09.1`——`CLAUDE_CONFIG_DIR` 本身就不在官方文档里（社区验证行为），是否影响 skills 路径未知 | **非阻塞**：`03.7` 的符号链接/Junction 供给方案对两种结果都安全（若不重定向，链接是无害空操作；若重定向，链接解决了漂移问题），Phase 1 实现时顺带记录实测结果，不需要提前确认才能开工 |
| 4 | Windows 上用 Junction（而非需要提权/开发者模式的符号链接）供给 Skills 目录，是否在所有场景下都被 Claude Code / Codex CLI 正确识别为普通目录 | `09.4` | Phase 1 实现 `aam-skills` 的 Windows 分支时用真实的 `claude`/`codex` 进程实测一次（建 Junction 后启动进程，确认能读到里面的 `SKILL.md`），不能只测"Junction 建成功"就当作完成 |

## 8.3 阻塞 Phase 2 的问题

| # | 问题 | 影响范围 | 应对方式 |
|---|---|---|---|
| 6 | 用户自己的 WebDAV 服务器是否支持 `ETag`/`If-Match` 条件写入 | `04.6` 的冲突检测依赖服务器至少支持基本的条件请求，如果目标服务器不支持，需要退化到"先拉取比较 version 字段再写"的应用层方案（已经是当前设计的默认方案，不依赖服务器特性），这一条更多是"能不能锦上添花用服务器原生机制减少一次网络往返"的优化项，不是阻塞项 | 不阻塞，Phase 2 直接用应用层 version 比较，服务器特性作为后续优化 |
| 7 | `age`/`rage` crate 在 Windows 和 Linux 上的 passphrase 模式（scrypt 参数）是否完全一致，避免"设备 A 加密的东西设备 B 解不开"这种跨平台不一致问题 | `04.2` | Phase 2 开发时写一条跨平台集成测试（CI 里 Windows + Linux 两个 runner 互相加解密同一份测试数据） |
| 16 | `aam sync reencrypt`/`push-account` 依赖本机 `ProviderRegistry`/`ProfileRegistry` 已知道的 id/label 才能覆盖——没有 PROPFIND，无法枚举"vault 上到底有哪些 blob"，如果某账号只在别的设备 push 过、本机从未 pull 过，本机跑 reencrypt 不会碰到它 | `04.9`/`04.10` | 非阻塞，`accounts.json.age`/未来的 provider 目录索引已经缓解"完全发现不了"的问题；真正的多设备联调放在 GUI 交付阶段做，届时验证这个限制的实际影响有多大 |

## 8.4 阻塞"真正的"跨设备项目身份关联（不阻塞已经交付的部分）的问题

| # | 问题 | 影响范围 | 应对方式 |
|---|---|---|---|
| 8 | "项目"的跨设备逻辑身份用什么做主键——物理路径在不同设备上大概率不同 | `05.2`；`aam project link`（手动关联，已实现）解决了"能不能关联"，没解决"要不要自动关联/怎么展示" | **手动关联已实现**：`aam project link <path-a> <path-b>` 给两条记录（可以跨本机/镜像）赋同一个 `projectId`，两边都没有就生成新的，一边有就沿用，两边不同就报错拒绝二选一。**仍未做**：自动匹配（不猜同名/同路径，风险大于价值）、展示层按 `projectId` 分组渲染成一行（现在 `aam project list` 依然是拼接展示，只是关联过的记录肉眼能核对）|
| 17 | `aam session adopt --summarize`（`05.8`）需要调用一个 Provider 生成摘要，但 `Provider` trait 之前只有 `materialize`/`verify`/`api_key`，没有"给一段文本，返回补全"的通用能力 | `05.8`；**已实现** | `Provider` trait 加了 `complete(prompt) -> Result<String, CompleteError>`。**协议选择先核实后动手**：Codex 端 `codex_toml.rs` 已经写死 `wire_api = "responses"`（OpenAI Responses API），跟 Claude 端查证的 Anthropic 官方文档（`platform.claude.com/docs/en/api/messages`：`POST {base_url}/v1/messages`，认证头是 `X-Api-Key` 不是 `Authorization: Bearer`）是两套不同协议，不能混用；`complete()` 选择 Anthropic Messages API，因为 `materialize()` 早就确定 `CpaProvider`/`DeepSeekProvider` 要能服务 Claude Code 就必须在 `{base_url}/v1/messages` 支持这个协议——是复用既有假设，不是新猜测。摘要输入是会话原始文件片段，不做结构化解析（`05.8`） |
| 18 | `aam` 接管 `project-tracker` 的实时记录能力（生成 hook 脚本 + 改写用户 `~/.claude/settings.json` 的 hooks 配置），而不只是收编脚本内容（`09.10` 已完成的部分） | `05.1` 的结论——`project-tracker` 目前是"实时记录"这一层唯一的实现，`aam` 没有常驻进程/hook 注册机制 | 非阻塞，独立于 Phase 3 立项：这会直接触碰用户当前生效的工具配置，风险类别与"不改 hook 脚本"这条既有边界（`08` #9）一致，值得单独设计"如何安全地改写别的工具的 settings.json"这件事本身（例如显式命令 + 改前展示 diff + 可回滚），不能在其他任务里顺手做——留给 Phase 4 之后独立立项 |
| 19 | `aam-skills` 的 Phase 3 子集——本机台账（`09.5`）、`scan`/完整版 `adopt`（含从非规范位置移动、`--source` 从 git 引入）（`09.6`）、`check-updates`/`update`（`09.7`）——是"完善 Phase 3"最后一块，需要 aam 自己的代码第一次真正 shell 出 `git` 命令 | `09.5`-`09.8`；**已实现** | 数据模型/发现扫描/移动纳管/git 更新检查全部落地（细节见 `09.5`-`09.8` 各节，已标注"已实现"及与原设计草稿的出入，如 `canonicalPath` 字段未做、`shareTargets` 目前只有 `codex` 一种）。CI 上 `updates.rs` 里真实调用 `git` 的测试一开始在 windows-latest 的 GitHub Actions runner 上失败，本机重复跑 3 次全绿——第一次误判为 git "dubious ownership" 保护（已顺手给所有 git 调用加 `-c safe.directory=*` 防御，无害但不是真正病因），**用 PAT 认证下载到实际 job 日志后确认真正原因**：`detects_and_applies_an_upstream_change` 测试里 `second_clone` 用不带 `--branch` 的 `git clone` 克隆本地裸仓库 `upstream`，而 `upstream` 的 HEAD 符号引用停留在 `git init --bare` 时的 `init.defaultBranch` 默认值——`seed` 仓库自己 `branch -M main` 只改了 `seed` 自己的分支名，从未更新远端裸仓库的 HEAD 指针；本机 git 的默认分支恰好也是 `main` 所以蒙混过关，CI runner 的 git 默认分支不同，导致 `git clone` 报 "remote HEAD refers to nonexistent ref"、`second_clone` 里没有可提交的本地 `main` 分支，紧接着 `push origin main` 因 "src refspec main does not match any" 失败。修复：`second_clone` 的 clone 命令也显式加 `--branch main`（和 `setup_git_skill` 里第一个 clone 一致），不依赖任何一台机器的 `init.defaultBranch` 配置。**Phase 3 至此全部完成**（连同 #8/#17/`aam project link`/`Provider::complete()`），下一步是 Phase 4 |

## 8.5 不阻塞任何 Phase，但需要长期关注的风险

| # | 风险 | 备注 |
|---|---|---|
| 11 | Claude Code extended-thinking 的 `Invalid signature in thinking block` 问题（anthropics/claude-code#63147 等），截至 2026-06-13 最新数据仍未修复 | `05.4` 已经明确排除同步 session `.jsonl` 来规避这个风险；建议每隔几个月回访一次 anthropics/claude-code 相关 issue 状态 |
| 12 | 主密码丢失=无法管理设备清单（`04.7`），这是设计上接受的权衡，不是 bug，但要在 Phase 2 的 GUI/CLI 首次设置流程里用足够醒目的方式告知用户 |  |
| 13 | 被吊销设备本地缓存的历史明文数据无法远程擦除（`04.4`） | 同上，产品文案层面的责任，不是技术要解决的问题 |
| 14 | Skills 的 GitHub 来源更新检查（`09.7`）本质是对源仓库做 `git fetch`，如果源仓库很大而 skill 只是其中一个子目录，全量克隆的存储/带宽成本可能不小 | 落地时 `adopt --source` 直接要求 `SKILL.md` 在仓库根目录（克隆后校验，不满足就报错），不支持"skill 是仓库里的一个子目录"这种形态，绕开了 sparse-checkout 的复杂度，问题不再存在于当前实现范围内；如果以后要支持子目录形态的 skill 仓库，这里记的 sparse-checkout 方案仍然是应对思路 |
| 20 | `aam-gui`（Phase 4）没有类似 Chrome DevTools 那样的原生窗口自动化点击工具，`cargo test` 覆盖不到"点击是否符合预期""表单排版是否合理"这类交互/视觉行为 | 从 Round 1 起，之后每个 GUI Round 都会遇到；`06.7` 已记录当前应对方式 | 长期做法：能拆成纯函数的逻辑（命令行拼接、表单校验、路径探测）一律拆出来单测，`Task`/`iced` 运行时相关的胶水代码不写"假测试"硬凑覆盖率；每轮结束前做"后台启动 `aam-gui.exe`→等几秒确认没崩溃→关闭"的自动化冒烟检查，真正的点击体验交给用户在自己机器上验收 |
| 21 | Windows Terminal 的 `winget install` 触发是 fire-and-forget（`06.7`），不追踪安装是否成功、也不在安装完成后自动刷新 `wt.exe` 检测状态 | 非阻塞，Round 1 范围内的已知简化 | 用户需要自己重启 `aam-gui` 才能让新装的 `wt.exe` 生效；如果以后觉得体验不够好，可以加一个"重新检测"按钮，但目前没必要为一次性的安装引导做更复杂的状态追踪 |

## 8.6 评估后明确不采纳的方案（存档理由，避免以后重复调研同一个问题）

- **`farion1231/cc-switch`**（125,510⭐，Rust+Tauri）：功能范围覆盖 8 款 Agent 工具、深度商业化（赞助商生态嵌入核心 README），与用户"只服务 Claude Code + Codex CLI，不做全家桶"的诉求不符。技术选型（Tauri）本身不是被否定的理由，`06.1` 选 iced 是出于 Windows 下终端组件可用性这个独立的技术判断。
- **`claude-task-master`**（27,945⭐）：面向"PRD 拆解成结构化任务列表"的完整任务管理系统，比用户实际需要的"记住做到哪了"重得多，引入的是一整套新工作流而不是轻量记忆机制。
- **`BMAD-METHOD`**（51,625⭐）：完整敏捷开发方法论（PRD→架构→story 全程留痕），同样过重；且 Codex CLI 支持有已知 bug（bmad-code-org/BMAD-METHOD#1782：全新安装后 Codex CLI 读不到 BMAD 上下文），与本项目"Claude+Codex 对等支持"的要求冲突。
- **`GreatScottyMac/roo-code-memory-bank`**（1,677⭐）：Memory Bank 模式的一个具体实现，但绑定 Roo Code/VSCode 插件生态，且最后更新停留在 2025-05，不适合纯 CLI 场景直接复用，只借用其"模式"本身（文件化、Agent 主动读写），不采用其代码。

## 8.7 本轮调研范围之外、留给后续文档迭代的话题

- CLI 参数/子命令的最终命名规范（本轮各文档里出现的 `aam ...` 命令是示意性质，未做正式的 CLI 设计评审）。
- 具体的 Rust 依赖版本锁定（`iced`/`iced_term`/`alacritty_terminal`/`age` 等的版本选择，等到对应 Phase 建 workspace 依赖时再定，避免文档写好后版本很快过期）。
- 错误处理/日志规范的统一设计（`aam-core` 里的错误类型体系，本轮文档只提到需要有，未展开）。
- `aam-skills` 是否要支持 skill 内容本身的语义化版本号（目前只做"diff 提示"，不做真正的版本协商），留给用量反馈后再评估要不要加。
