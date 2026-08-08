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
- `aam-switcher` 完成 Claude backend（`CLAUDE_CONFIG_DIR` 目录选择模式）+ Codex backend（先移植 `codex-skill` 现有的原地写文件模式，`CODEX_HOME` 语义确认后再评估是否切到目录选择模式）。
- Provider trait 落地两个具体实现：CPA、DeepSeek V4 Flash。
- `aam-cli` 的 `profile list/add/verify` + `aam claude`/`aam codex` 启动包装命令。
- Claude 侧存活验证命令确认并接入（阻塞项，见 `08`）。

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

对应文档：`05`。

**交付物**：
- `aam-memory`：本地索引（复用 `project-tracker` schema + 新增字段）+ 通过 `aam-sync` 同步的跨设备版本。
- `aam-cli` 新增 `project list/show/resume/sync`。
- 与 `~/.claude/skills/project-tracker` 的关系落地：要么 `project-tracker` 的 hook 直接写 `aam-memory` 能读的格式，要么提供一个一次性迁移/双写桥接（具体方式在本 Phase 立项时定，当前只确定"不能是用户手动维护两份"）。

**验收标准**：覆盖验收场景第 7-12 步中"记得住做到哪了"的部分——在设备 B 上执行 `aam project resume X`，能看到"上次在设备 A，Claude 官方账号2，一句话状态：...”，且给出正确的下一步操作提示（本地无该目录时的提示文案符合 `05.3`）。

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
