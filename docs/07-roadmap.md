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

### Phase 3b — 跨设备聚合 + WebDAV 同步（待立项）

- `aam-memory` 索引通过 `aam-sync` 同步的跨设备版本；`aam-cli` 的 `project list/show/resume` 升级为真正的跨设备聚合视图。
- `aam session sync`：只推 `syncApproved=true` 的条目。
- 项目跨设备逻辑身份（`projectId`，见 `08` #8）、`Provider::complete()` 通用文本补全能力（`08` #17，`--summarize` 的前置依赖）在本子阶段立项时一并定案。
- **`aam-skills` 的 Phase 3 子集**（`09.6`、`09.7`）：`aam skills scan`（跨 `~/.claude/skills`/`$HOME/.agents/skills`/各 Profile 的只读发现）、完整版 `aam skills adopt`（含从非规范位置移动内容）、来源追踪与 `check-updates`/`update`（GitHub 来源 skill 的手动更新检查，默认不自动应用）。

**验收标准**：覆盖验收场景第 7-12 步中"记得住做到哪了"的部分——在设备 B 上执行 `aam project resume X`，能看到"上次在设备 A，Claude 官方账号2，一句话状态：...”，且给出正确的下一步操作提示（本地无该目录时的提示文案符合 `05.3`）；另外验证"扫描出的历史会话/skills 默认不出本机，需要显式批准/adopt 才会离开本机"这条本轮新增的核心约束。

---

## Phase 4 — GUI 外壳（不含内嵌终端）

对应文档：`06.6`。

**交付物**：
- `aam-gui`：账号/Provider 管理界面、跨设备项目浏览器，均为 Phase 1-3 CLI 能力的图形化封装。
- 终端启动动作先退化为"调起系统默认终端并预填好环境变量+命令"，不做真正嵌入。

**验收标准**：不用命令行，纯点击操作，能完成 Phase 1-3 CLI 能覆盖的全部场景。

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
