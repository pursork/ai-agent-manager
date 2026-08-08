# 00. 项目愿景与范围

## 一句话定义

**ai-agent-manager** 是一个本地优先（local-first）、跨设备、跨 Agent 工具（Claude Code + Codex CLI）的**凭据/账号/第三方 API 管理器 + 项目进度记忆库 + 多终端 GUI 外壳**，目标是让用户在任意设备、任意账号、任意后端下都能"连续、高效"地接续同一项研究/开发任务，而不是把连续性完全寄托在 Claude Code / Codex 自己脆弱的本地 session 状态上。

## 问题背景

用户日常在多台设备上交替使用 Claude Code 和 Codex CLI，每个工具下都有多个官方订阅账号，以及若干第三方 API/自建代理（DeepSeek V4 Flash、CPA/CLIProxyAPI 等）。随之而来的具体痛点：

1. **换设备要重新登录**——没有机制把已登录的官方订阅凭据安全地带到另一台设备。
2. **换账号/换后端后，会话"断片"**——Claude Code 的 `--resume` 依赖本地 session transcript 里的 extended-thinking 签名校验，这个校验和后端/生成时的上下文绑定，跨后端/长会话重建时会报 `400 Invalid signature in thinking block`，且截至目前（2.1.170）官方仍未修复（详见 `01-prior-art.md` 和本项目调研历史）。换句话说，**不能依赖 Agent 工具自身的 resume 机制作为跨设备/跨账号连续性的兜底**。
3. **记不清自己有哪些项目、做到哪一步**——已经用 `~/.claude/skills/project-tracker` 部分解决了单机场景，但没有跨设备版本。
4. **账号/Provider 切换目前是两套割裂的工具**——Codex 侧有 `codex-skill`（本机已验证好用），Claude 侧完全没有等价物。

## 验收场景（用户原始描述，逐字保留，作为端到端验收用例）

> 假设一个具体场景，设备 A 和设备 B：

| # | 设备 | 工具 | 账号/后端 | 动作 |
|---|---|---|---|---|
| 1 | A | Claude | 官方账号 1 | 辅助进行"研究 X" |
| 2 | A | Claude | 官方账号 2（另一个） | 继续 X |
| 3 | A | Claude | 第三方 API | 继续 X |
| 4 | A | Codex | 官方账号 1 | 辅助进行"研究 X" |
| 5 | A | Codex | 官方账号 2（另一个） | 继续 X |
| 6 | A | Codex | 第三方 API | 继续 X |
| 7 | **B（未登录 Claude，通过云端加密共享凭据实现免登录使用官方订阅）** | Claude | 官方账号 1 | 辅助进行"研究 X" |
| 8 | B | Claude | 官方账号 2（另一个） | 继续 X |
| 9 | B | Claude | 第三方 API | 继续 X |
| 10 | B | Codex | 官方账号 1 | 辅助进行"研究 X" |
| 11 | B | Codex | 官方账号 2（另一个） | 继续 X |
| 12 | B | Codex | 第三方 API | 继续 X |

**验收标准**：在这 12 步的任意一步切换点，用户都应该能：
(a) 免重复登录地拿到正确的凭据/账号/Provider；
(b) 明确知道"研究 X"这个项目上一次进展到哪、在哪台设备的哪个路径；
(c) 如果目标设备本地没有这个项目目录，得到清晰提示（"去设备 X 的路径 Y 继续"），而不是静默失败或错误地在当前设备重建一个空环境。

这 12 步验收用例贯穿 `03`~`07` 各设计文档，每个模块设计完都应该能回答"这一步骤在我的设计里具体怎么发生"。

## 目标（Goals）

- G1：Claude Code 与 Codex CLI 的账号/Provider 切换，做到和 `codex-skill` 同等的可靠性（快照-写-验证-回滚），并且两个工具用统一的一套工具/UI 操作。
- G2：账号凭据可以通过用户自己的 WebDAV 服务器，以零知识加密的方式跨设备同步，新设备免重复登录。
- G3：项目/会话进度信息（不是完整会话 transcript）跨设备同步，任意设备都能查到"我有哪些项目、做到哪、在哪台设备哪个路径"。
- G4：提供一个原生 Rust GUI（类 Termius 的多终端窗口），把以上能力可视化，同时保留纯 CLI 的可用性（GUI 是 CLI/核心库的外壳，不是唯一入口）。
- G5（可选，高风险，需用户显式逐项目开启）：完整项目工作目录跨设备同步。

## 非目标（Non-Goals）

- **不试图修复** Claude Code 的 extended-thinking signature 持久化 bug——那是 Anthropic 的责任，我们只是设计成不依赖它。
- **不做成 cc-switch 那样的"全家桶"**——不支持 Gemini CLI / Grok Build / OpenCode / OpenClaw / Hermes 等其他 Agent 工具，只服务 Claude Code + Codex CLI 这两个用户实际在用的工具（原因见 `01-prior-art.md`）。
- **不做真正的多人协作/多人编辑冲突解决**——WebDAV 同步面向"同一个人的多台设备"，不是团队协作工具，冲突处理策略可以简单粗暴（见 `04`）。
- **不托管服务器**——用户自带 WebDAV，本项目不提供、不运营任何云端服务。
- **默认不同步任何项目源代码/会话 transcript**——G3 只同步"进度元信息"，G5 的完整目录同步是显式 opt-in 的例外，且明确排除 session `.jsonl`（见 `05`）。

## 与既有项目的关系

`ai-agent-manager` 不是第三个平行系统，而是把已有两个项目的能力**吸收、泛化、并加上跨设备层**：

- **`C:\Users\16500\Desktop\codex-skill`**：其 account/provider 切换的工程模式（状态机隔离、快照-验证-回滚、command-backed token）被原样搬进 `03-credential-account-module.md`，作为 Rust 重实现的行为规范基准，并扩展出 Claude Code 侧的等价实现。这个旧项目未来可以被 `ai-agent-manager` 的 CLI 子命令取代，但取代之前继续保留、不动它。
- **`~/.claude/skills/project-tracker`**：其 `project-index.json` schema 和"用 SessionStart/SessionEnd hook 自动记录"的思路，是 `05-session-memory-bank-module.md` 的直接起点——本地部分不变，新增一层加密同步到 WebDAV 的跨设备索引。这个 skill 继续独立可用（对只在单机工作的场景仍然有效），`ai-agent-manager` 是它的超集，不是替代品的强制升级。

## 文档地图

- `01-prior-art.md` —— 参考的开源项目与设计教训
- `02-architecture.md` —— 总体模块划分与安全模型
- `03-credential-account-module.md` —— 账号/Provider 切换模块详设（Phase 1，最高优先级）
- `04-webdav-sync-security.md` —— 凭据跨设备同步的加密方案（Phase 2）
- `05-session-memory-bank-module.md` —— 会话/项目进度追踪模块（Phase 3）
- `06-gui-terminal-shell.md` —— GUI 终端壳（Phase 4-5）
- `07-roadmap.md` —— 分阶段里程碑与验收标准
- `08-open-questions-risks.md` —— 未决问题与风险台账
