# 07. 分阶段路线图

排序依据用户已确认的优先级：**凭证/账号切换 + 跨设备同步优先 → 会话进度追踪 → GUI 终端壳**。每个 Phase 明确交付物和验收标准，验收标准尽量对应 `00-overview.md` 里那 12 步验收场景的具体子集。

## Phase 0 — 项目脚手架

**交付物**：
- Rust workspace 结构（`crates/aam-core`、`aam-vault`、`aam-switcher`、`aam-sync`、`aam-memory`、`aam-cli`，`aam-gui` 先占位不实现）。
- `aam-core` 里的 `TransactionalWrite` 通用工具（`02.6` 定义的快照-写-验证-回滚基础设施）。
- CI 基本跑通（`cargo build` / `cargo clippy` / `cargo test` 在 Windows runner 上通过）。

**验收标准**：`cargo build --workspace` 成功，没有实际功能。

---

## Phase 1 — 账号 / Provider 切换（CLI 形态，Claude + Codex 双端）

对应文档：`03`。

**交付物**：
- `aam-switcher` 完成 Claude backend 和 Codex backend，**两者统一用"N 目录 + 启动时选 `CLAUDE_CONFIG_DIR`/`CODEX_HOME`"模型**（`CODEX_HOME` 语义已确认等价于整目录重定向，见 `08`，不再需要 codex-skill 原地写文件的回退分支）。
- Provider trait 落地两个具体实现：CPA、DeepSeek V4 Flash。
- `aam-cli` 的 `profile list/add/verify` + `aam claude`/`aam codex` 启动包装命令。
- Claude 侧存活验证命令 `claude auth status` 接入（`08` 已确认为官方命令，不再阻塞）。
- **Claude Profile 创建时的 Skills Junction/符号链接供给**（`03.7`、`09.3`）——结构性、无外部依赖，随账号切换一起交付。
- `aam-skills` crate 的 Phase 1 子集：`aam skills list/status` + 显式跨工具共享 `aam skills adopt <name> --share-with <targets>`（仅限已在规范位置的 skill，完整的"从任意位置纳管"流程留到 Phase 3，见 `09.8`）。

**验收标准**：覆盖验收场景第 1-6 步（设备 A 上，Claude 两个官方账号 + 第三方 API + Codex 两个官方账号 + 第三方 API，全部能通过 `aam` 命令一键切换并通过存活验证）。

---

## Phase 2 — WebDAV 加密同步（先同步 Phase 1 的凭据）

对应文档：`04`。

**交付物**：
- `aam-sync`：`devices.json.age` 的读写、多接收方 blob 加密/解密、设备加入/吊销流程。
- `aam-cli` 新增 `aam device join/list/revoke`、`aam sync push/pull`。
- Phase 1 里 `aam-vault` 存储的账号凭据接入同步（Provider 的密钥配置同理）。

**验收标准**：覆盖验收场景第 7 步的核心难点——"设备 B 未登录，通过云端加密共享凭据实现免登录使用官方订阅"。具体验收：全新设备 B，输入 WebDAV 地址+主密码，`aam sync pull` 后，`aam claude <官方账号1的profile>` 可以直接启动且通过存活验证，全程不触发 Claude 官方登录流程。

---

## Phase 3 — Session / Memory-Bank 追踪 + 同步

对应文档：`05`。分两个子阶段交付，同 Phase 2"先引擎、后接域对象、最后同步"的节奏。

### Phase 3a — 本地模型 + 发现/采集（已完成）

- `aam-memory`：`ProjectRecord`/`ProjectIndex`，直接读写 `project-tracker` 已经在维护的真实 `project-index.json`（桥接方式定案见 `08` #9），新字段 `serde(default)` 兼容旧记录。
- `aam-cli` 新增 `project list/show/resume`（本机视图）。
- **本机会话发现与采集**（`05.7`-`05.9`）：`aam session scan`（跨 Claude/Codex、跨已注册 Profile 的只读扫描）、`aam session adopt`（写本地索引，`discoverySource=scan`/`syncApproved=false`——**硬性约束：扫描出的内容默认不出本机**）、`aam session approve-sync`。`--summarize` 尚未实现（需要 `Provider` trait 先加一个通用的"文本补全"能力，见 `08` #17），Codex 会话的 `autoStatus` 目前留空。

### Phase 3a 附加项 — project-tracker 收编为附带 skill（已完成）

原定 Phase 3b 之前插入的一轮：`project-tracker` 确认暂时不能停用（`aam` 还没有接管 Claude Code hook 注册的能力，见 `08` 新增项），于是收编成 aam 自己维护的附带 skill，而不是放任它作为脱节的独立技能：

- `crates/aam-skills/bundled/project-tracker/`：`SKILL.md` + 改造后的 `track-session.ps1`/`backfill-index.ps1`，`include_str!` 嵌入二进制。
- `aam skills install-bundled [name] [--force]`：物化安装，默认不覆盖冲突内容，不碰 `settings.json` 的 hook 注册（那一步仍是手动的，`SKILL.md` 里有说明）。
- `aam whoami --tool <claude|codex>`：新的只读诊断命令，收编后的脚本靠它正确填 `deviceId`/`profileLabel`（`09.10`）。

### Phase 3b — 跨设备聚合 + WebDAV 同步（已完成）

- `aam-memory` 索引通过 `aam-sync` 同步的跨设备版本（共享 blob 多写者合并语义：拉取→过滤掉自己设备的旧记录→追加自己当前的记录→推送，`sync.rs`）；`aam-cli` 的 `project list/show` 拼接本机 + 镜像索引，成为跨设备聚合视图。
- `aam session sync`：只推 `syncApproved=true` 的条目。

### Phase 3 补完 — 完善收尾（已完成）

按顺序补完的四块中的三块（第四块——`aam` 接管 Claude Code hook 实时注册——明确排除在"完善 Phase 3"范围之外，独立留给 Phase 4 之后立项，见 `08` #18）：

- **跨设备 resume 兜底 + `projectId` 手动关联**（`08` #8）：`aam project resume` 在本机路径不存在时，改为在本机+镜像索引里找同名记录，给出"本机未找到目录（上次在设备 X）"的兜底提示，不再假设路径永远有效；`aam project link <path-a> <path-b>` 显式给两条记录关联同一个跨设备逻辑身份。
- **`Provider::complete()` + `--summarize`**（`08` #17）：`Provider` trait 加了通用的"文本补全"能力（Anthropic Messages API，协议选择过程见 `08` #17），`aam session adopt --summarize --profile <label>` 用它给采集到的会话生成一句话摘要。
- **`aam-skills` 的 Phase 3 子集**（`09.5`-`09.8`）：本机台账 `.aam-skills-index.json`、`aam skills scan`（跨规范仓库本身/`$HOME/.agents/skills`/各未链接 Profile 的只读发现）、完整版 `aam skills adopt`（含从非规范位置移动内容、`--source <git-url>[@ref]` 从 git 引入新 skill）、`check-updates`/`update`/`update --all-auto`（GitHub 来源 skill 的更新检查与应用，默认手动触发）。

**验收标准**：覆盖验收场景第 7-12 步中"记得住做到哪了"的部分——在设备 B 上执行 `aam project resume X`，能看到"上次在设备 A，Claude 官方账号2，一句话状态：...”，且给出正确的下一步操作提示（本地无该目录时的提示文案符合 `05.3`）；验证"扫描出的历史会话/skills 默认不出本机，需要显式批准/adopt 才会离开本机"这条核心约束；`aam-skills` 的 scan/adopt/check-updates/update 流程用真实本地 git 仓库端到端验证过（克隆、检测更新、`--all-auto` 应用、scan 与已链接 Profile 的去重）。**Phase 3 至此全部完成，下一步是 Phase 4。**

---

## Phase 4 — GUI 外壳（不含内嵌终端）

对应文档：`06.6`。

**范围比 `06.6` 原文更大**：立项时用户明确要求"GUI 是功能的直接体现，所有功能尽量都加到 GUI 里"，不满足于最小化的"账号管理+项目浏览器"。鉴于这是全项目最大的单块工作（覆盖 `aam-cli` 现有全部子命令域），沿用 Phase 3 的节奏，分轮推进，每轮独立验收：

- **Round 1（已完成）**：`aam-gui` 骨架（`iced` 0.14，函数式 `application(boot, update, view)` API）+ Profile 管理屏（list/add/verify/挂载 Provider/打开终端登录）+ Provider 管理屏（list/add，API key 遮罩输入，从不显示已存 key）。终端拉起原语 `terminal.rs`：优先探测并使用 Windows Terminal（`wt.exe`），检测不到时退回 `powershell.exe`（这条退回路径必须永远可用），检测不到 `wt.exe` 时 GUI 展示一次性提示，用户点击可触发 `winget install` 协助安装（绝不静默自动装）。新终端窗口打开后直接自动执行命令，不退化成"只给命令"（`06.5` 的正当性依据）。
- **Round 2（已完成）**：项目浏览器（`aam-memory` 的 list/show/resume/link）+ 会话扫描/采纳/批准同步面板（`05.7`-`05.9`），"接续项目"复用 Round 1 的终端原语真正打开终端执行 resume。立项时用户强调"GUI 的核心是用户友好性"，这轮把这句话落成了几条具体设计约束（信息分层/单一主操作/人话错误提示/核心安全约束常驻可见/按行反馈/四屏统一视觉风格），细节见 `06.8`。
- **Round 3（已完成）**：Skills 管理面板（`aam-skills` 全量：list/status/scan/adopt(本地+git)/install-bundled/check-updates/update），沿用 Round 2 定下的用户友好性准则（信息分层、单一主操作、人话提示、按行 vs 整屏反馈、统一视觉语言），细节见 `06.9`。
- **Round 4（已完成）**：设备/同步管理面板（`aam-sync` 的 vault init/join/list/revoke/reencrypt，Provider/账号 push/pull，会话 sync——Round 2 时特意延后到这轮的部分，因为它跟本屏其余动作共用同一份 WebDAV 连接）。这是第一次要在 GUI 里长期持有真正的敏感信息（WebDAV 密码、vault 主密码）；跟用户确认后的取舍是"本次 `aam-gui` 运行期间记住"，界面上用常驻说明 + 显式「清除」按钮把这件事讲清楚，细节见 `04.9`。

**验收标准**：不用命令行，纯点击操作，能完成 Phase 1-3 CLI 能覆盖的全部场景。**Phase 4 至此全部完成**，下一步是 Phase 5（`iced_term` 内嵌终端）。

---

## Phase 5 — 内嵌终端（`iced_term`）

对应文档：`06.3`-`06.4`。

**交付物**：多标签/多面板真终端，`06.4` 描述的"点击项目 → 自动开新标签页并 resume"完整体验。

**验收标准**：完整走通 `00-overview.md` 12 步验收场景，全程只用 GUI，不需要手动敲命令行。

---

## Phase 6 — 可选的完整项目多端同步（高风险，opt-in）

对应文档：`05.4`。

**交付物**：单项目粒度的完整目录同步开关，先实现"最后写入者胜出 + 冲突文件重命名保留"策略。

**验收标准**：一个非 git 管理的研究目录，在设备 A 改动后，设备 B 手动触发同步能拿到最新文件；两端并发修改同一文件时不会静默丢失数据（至少有冲突文件留存）。

---

## 阶段间的依赖关系

```
Phase 0 ──▶ Phase 1 ──▶ Phase 2 ──▶ Phase 3 ──▶ Phase 4 ──▶ Phase 5
                                                      │
                                                      └──▶ Phase 6（可与 Phase 5 并行，
                                                            不阻塞 GUI 主线）
```

Phase 6 依赖 Phase 3（需要项目索引机制先存在）和 Phase 2（同步基础设施复用），但不依赖 Phase 4/5，理论上可以在 Phase 3 完成后就并行开发，只是 Roadmap 排在最后以匹配"高风险功能不抢先做"的原则。
