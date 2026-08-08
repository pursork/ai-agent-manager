# 08. 未决问题与风险台账

本文档汇总前面文档里标记为"待确认/待调研"的所有事项。

## 8.1 曾阻塞 Phase 1 的问题（已解决）

| # | 问题 | 结论 | 来源 |
|---|---|---|---|
| 1 | `CODEX_HOME` 环境变量的精确语义 | **确认等价于整目录重定向**：`config.toml`、`auth.json`、`sessions/YYYY/MM/DD/rollout-*.jsonl`（`codex resume` 依赖的会话记录）、`history.jsonl`、`hooks.json`、日志/缓存、`profile-name.config.toml` 全部在 `CODEX_HOME` 根下（默认 `~/.codex`）。**例外：Skills 不受影响**，固定从 `$HOME/.agents/skills`（+ 仓库级/管理员级/内置）读取，天然全局共享，不随 Profile 分裂——见 `09`。据此 `aam-switcher` 的 Codex backend 采用和 Claude 相同的"N 目录 + 启动时选路径"模型，不再需要 codex-skill 原地写文件的回退分支（`03.2`、`07` Phase 1 已更新）。 | [developers.openai.com/codex/config-advanced](https://developers.openai.com/codex/config-advanced)，[developers.openai.com/codex/skills](https://developers.openai.com/codex/skills) |
| 2 | Claude Code 侧"存活验证"该调用什么命令 | **确认存在官方命令**：`claude auth status`（默认 JSON，`--text` 可读格式），登录时退出码 0，未登录退出码 1。对应 GitHub Issue #1886（"Make status checkable from command line"）已于 **2.1.41 版本修复关闭**（`state_reason: completed`）。`03.5`/`03.6` 的存活验证步骤据此接入。 | [code.claude.com/docs/en/cli-reference](https://code.claude.com/docs/en/cli-reference)，[anthropics/claude-code#1886](https://github.com/anthropics/claude-code/issues/1886) |

## 8.2 Phase 1 内需要实测、但不阻塞开始编码的问题

| # | 问题 | 影响范围 | 应对方式 |
|---|---|---|---|
| 3 | `CLAUDE_CONFIG_DIR` 是否连带重定向 `skills/` 子路径 | `03.7`、`09.1`——`CLAUDE_CONFIG_DIR` 本身就不在官方文档里（社区验证行为），是否影响 skills 路径未知 | **非阻塞**：`03.7` 的符号链接/Junction 供给方案对两种结果都安全（若不重定向，链接是无害空操作；若重定向，链接解决了漂移问题），Phase 1 实现时顺带记录实测结果，不需要提前确认才能开工 |
| 4 | Windows 上用 Junction（而非需要提权/开发者模式的符号链接）供给 Skills 目录，是否在所有场景下都被 Claude Code / Codex CLI 正确识别为普通目录 | `09.4` | Phase 1 实现 `aam-skills` 的 Windows 分支时用真实的 `claude`/`codex` 进程实测一次（建 Junction 后启动进程，确认能读到里面的 `SKILL.md`），不能只测"Junction 建成功"就当作完成 |

## 8.3 阻塞 Phase 2 的问题

| # | 问题 | 影响范围 | 应对方式 |
|---|---|---|---|
| 5 | Rust WebDAV 客户端 crate 选型 | `04.8`，本轮未做 crate 级别调研 | Phase 2 立项时调研当时活跃度最高、支持 Basic/Digest/Bearer 多种鉴权方式的 crate |
| 6 | 用户自己的 WebDAV 服务器是否支持 `ETag`/`If-Match` 条件写入 | `04.6` 的冲突检测依赖服务器至少支持基本的条件请求，如果目标服务器不支持，需要退化到"先拉取比较 version 字段再写"的应用层方案（已经是当前设计的默认方案，不依赖服务器特性），这一条更多是"能不能锦上添花用服务器原生机制减少一次网络往返"的优化项，不是阻塞项 | 不阻塞，Phase 2 直接用应用层 version 比较，服务器特性作为后续优化 |
| 7 | `age`/`rage` crate 在 Windows 和 Linux 上的 passphrase 模式（scrypt 参数）是否完全一致，避免"设备 A 加密的东西设备 B 解不开"这种跨平台不一致问题 | `04.2` | Phase 2 开发时写一条跨平台集成测试（CI 里 Windows + Linux 两个 runner 互相加解密同一份测试数据） |

## 8.4 阻塞 Phase 3 的问题

| # | 问题 | 影响范围 | 应对方式 |
|---|---|---|---|
| 8 | "项目"的跨设备逻辑身份用什么做主键——物理路径在不同设备上大概率不同 | `05.2` | Phase 3 立项时定：候选方案是引入一个用户可选/可改的 `projectId`（首次记录时生成，允许用户手动关联"设备A的这个路径"和"设备B的那个路径"是同一个逻辑项目），而不是试图自动猜测 |
| 9 | `project-tracker`（现有单机技能）与 `aam-memory`（本模块）的具体桥接方式 | `07` Phase 3 交付物提到但未定案 | Phase 3 立项时二选一：(a) 改造 `project-tracker` 的 hook 脚本直接写 `aam-memory` 认的 schema；(b) `aam-memory` 提供一个后台文件监视/定期读取 `project-index.json` 做双写。倾向 (a)，因为避免两套自动化并存 |
| 10 | `~/.codex/session_index.jsonl` 元数据缓存是否真实存在、格式是否稳定 | `05.7` 想用它做"扫描 Codex 会话"的快速入口；来源是社区文章描述，非一等公民官方文档确认 | 不阻塞，Phase 3 实现时先探测该文件是否存在/可解析，不存在或解析失败就退化为直接扫描 `sessions/**/rollout-*.jsonl`（较慢但可靠） |

## 8.5 不阻塞任何 Phase，但需要长期关注的风险

| # | 风险 | 备注 |
|---|---|---|
| 11 | Claude Code extended-thinking 的 `Invalid signature in thinking block` 问题（anthropics/claude-code#63147 等），截至 2026-06-13 最新数据仍未修复 | `05.4` 已经明确排除同步 session `.jsonl` 来规避这个风险；建议每隔几个月回访一次 anthropics/claude-code 相关 issue 状态 |
| 12 | 主密码丢失=无法管理设备清单（`04.7`），这是设计上接受的权衡，不是 bug，但要在 Phase 2 的 GUI/CLI 首次设置流程里用足够醒目的方式告知用户 |  |
| 13 | 被吊销设备本地缓存的历史明文数据无法远程擦除（`04.4`） | 同上，产品文案层面的责任，不是技术要解决的问题 |
| 14 | Skills 的 GitHub 来源更新检查（`09.7`）本质是对源仓库做 `git fetch`，如果源仓库很大而 skill 只是其中一个子目录，全量克隆的存储/带宽成本可能不小 | 非阻塞，Phase 3 实现时优先尝试 sparse-checkout，退化方案是提示用户"这个 skill 来自一个较大的仓库，更新检查可能较慢" |

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
