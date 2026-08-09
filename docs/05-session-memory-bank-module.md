# 05. 会话 / 项目 Memory-Bank 模块（Phase 3）

> **实现状态（已全部完成）**：`aam-memory` crate + `aam project list/show/resume/link`、`aam session scan/adopt/approve-sync/sync` 全部已落地（本地模型、跨工具发现/采集、跨设备聚合视图、真正推到 WebDAV 的索引同步），细节见 `08` #8/#9/#10/#17。`aam session sync` 在 Phase 3b 落地，图形化版本（`crates/aam-gui/src/screens/sync.rs`）在 Phase 4 Round 4 补上——推迟到那一轮是因为它跟这个屏幕的其余动作（设备/Provider/账号同步）共用同一份 WebDAV 连接状态，不是因为逻辑本身有依赖。

## 5.1 起点：直接扩展今天已经在跑的 `project-tracker`

`~/.claude/skills/project-tracker`（`~/.claude/project-index.json` + SessionStart/SessionEnd hook）已经解决了单机场景。本模块 = 把它的 schema 和自动记录机制原样保留作为**本地缓存层**，在其上加一层**跨设备同步层**。不重新发明本地部分。

现有本地 schema（`project-index.json` 每条记录）：

```json
{
  "path": "D:\\研究\\lzy\\云边测试\\Gear-Sys",
  "name": "Gear-Sys",
  "lastSessionId": "286cce60-...",
  "lastActive": "2026-08-08T10:15:00+08:00",
  "created": "2026-07-01T09:00:00+08:00",
  "autoStatus": "验证研究工作文档并核实问题",
  "statusOverride": null,
  "authBackend": "oauth-subscription"
}
```

## 5.2 跨设备同步需要新增的字段

```json
{
  "...原有字段不变...": "...",
  "deviceId": "uuid-v4，写入这条记录时所在的设备",
  "toolKind": "claude | codex",
  "profileLabel": "写入时使用的 Profile（见 03），如 官方账号2 / DeepSeek",
  "fullSyncEnabled": false,
  "fullSyncStatus": null,
  "discoverySource": "live | scan",
  "syncApproved": true
}
```

- `discoverySource`：`live` = 正常使用中经由 hook 实时记录（今天 `project-tracker` 的现有行为）；`scan` = 通过 `aam session scan`/`adopt` 回溯采集出来的历史会话（见 5.7-5.9）。
- `syncApproved`：这条记录是否允许被 `aam session sync` 推送到 WebDAV。`live` 记录默认 `true`（沿用 G3 既定的"进度元信息默认同步"目标）；`scan` 来源的记录默认 **`false`**，必须用户显式 `aam session approve-sync` 才会变成 `true`。这是本轮新增的硬性原则：**"被发现"和"被同步"是两回事**，回溯采集不应该在用户没意识到的情况下把一堆历史项目路径/时间戳推到云端。

> `project-tracker` 已收编为 aam 的附带 skill（`aam skills install-bundled project-tracker`，见 `09.10`）。装的是收编后的新版 `track-session.ps1`/`backfill-index.ps1` 的话，`deviceId`/`profileLabel` 会通过 `aam whoami` 正确写入；仍在用旧版脚本、或从未通过 `aam claude <label>` 启动过会话，这两个字段就是 `serde(default)` 兜底的空值——两种状态都合法，`aam-memory` 读取时不区分对待。

- `deviceId` + `toolKind`：跨设备/跨工具场景下，"同一个项目路径 `X`"在设备 A 用 Claude 做过、在设备 B 用 Codex 做过，这两条记录**不合并成一条**，而是分别记录，但在 UI 上按项目"聚合展示"（同一个逻辑项目下面挂多条设备/工具/时间线记录）。项目的"逻辑身份"用什么做主键，是本模块需要在 Phase 3 立项时进一步定义的问题（候选：项目名 + 一个用户可选的稳定项目 ID，而不是物理路径，因为同一项目在不同设备上的绝对路径大概率不同）——记入 `08`。
- `fullSyncEnabled` / `fullSyncStatus`：见 5.4。

## 5.3 跨设备 Resume 的默认行为：只提示，不搬迁（已实现）

`aam project resume` 已经实现下面的流程图：本机+镜像拼接查找、路径存在性检查、找不到时打印设备提示而不是失效的 `cd`/`resume` 命令。**`enable-full-sync` 那一句建议这轮没加进提示文案**——它是 Phase 6 才有的功能，现在提会让用户以为已经能用。

沿用 `project-tracker` 已经确立的原则（"Claude 不能替用户 cd，只能给命令提示"），跨设备场景下同样**默认绝不自动搬迁/重建项目目录**：

```
用户在设备 B 问："继续 X 项目"
        │
        ▼
查跨设备索引，找到 X 项目最新一条记录 → 在设备 A，路径 D:\...\X
        │
        ▼
   当前设备（B）本地是否存在这个路径？
   ├─ 是（说明用户在两台设备上都手动建过同名目录，或已开启完整同步）
   │     → 正常给出 cd + resume 提示（同 project-tracker 现有逻辑）
   └─ 否 → 明确告知：
         "项目 X 上次在【设备 A】的【D:\...\X】继续，本机（设备 B）未找到该目录。
          请前往设备 A 继续，或使用 `aam project enable-full-sync X`
          为这个项目单独开启完整目录同步（见下）。"
```

**不做任何形式的"猜测性自动创建目录/拉代码"**——这条是从今天已经讨论过的教训（thinking-block 签名损坏问题）延伸出来的更大原则：**任何"自动"行为都必须是确定性的、可预测的，宁可多问用户一句，不做静默的环境重建**。

## 5.4 可选：完整项目多端同步（高风险，显式 opt-in）

这是 Roadmap 里明确排在最后（Phase 6）的功能，因为它触及真正的文件冲突问题，设计上要老实：

- **默认关闭**，必须由用户对**单个项目**显式执行 `aam project enable-full-sync <project>` 才开启，不存在"全局开关"。
- 同步范围：项目工作目录下的**源代码/文档文件**。**明确排除**：
  - `~/.claude/projects/**/*.jsonl`（Claude session transcript）——理由直接引用今天的调研结论：这类文件包含 extended-thinking 签名，同步到另一台设备后如果被读取/重建，会撞上尚未修复的 `Invalid signature in thinking block` 问题；就算不撞这个 bug，session transcript 本身也不是"项目内容"，属于工具自己的状态，不应该被我们的同步机制染指。
  - `~/.codex` 下的等价 session/日志文件。
  - 任何 `.git` 内部对象（如果项目本身是 git 仓库，交给 git 自己的同步机制/远程仓库处理，不用我们的 WebDAV 通道重复做这件事——**如果项目已经是 git 仓库，第一选择应该是引导用户直接用 git push/pull，完整目录同步只留给非 git 管理的研究类目录**，这一点要在 CLI/GUI 里做检测和提示）。
- 同步机制候选（Phase 6 立项时再细化，本轮只列出候选，不做最终决定）：
  1. **基于修改时间/哈希的单向"最后写入者胜出"**——实现简单，但对同一文件被两端并发修改的情况会静默丢失一方的修改，只适合"用户自己清楚不会真的并发编辑同一个文件"的使用习惯。
  2. **真正的双向 diff/三路合并**（借鉴 `rsync`/`rclone bisync`/`Syncthing` 的冲突文件重命名策略）——更安全，但工程量显著更大，且需要处理二进制文件、大文件等边界情况。
  - **建议 Phase 6 起步先做方案 1 + 冲突时重命名保留双份（`file.conflict-deviceB-<timestamp>.ext`）而不是直接覆盖丢失**，把"真正智能合并"作为更远期的加强项。
- 用户体验上，开启完整同步的项目，在 5.3 的流程图里会从"否"分支换到"是"分支——但**切换/合并本身仍然需要用户在成功同步后才认为"可以继续"**，不能默默假设同步已完成就直接开始改动文件。

## 5.5 与 `03`（账号/Provider 切换）模块的联动

Memory-Bank 记录里的 `profileLabel` 字段，是"resume 提示"里附加的关键信息——沿用今天 `project-tracker` 已经做的"`authBackend` 不一致时警告"逻辑，跨设备场景下要警告的范围更大：**Profile（账号+Provider组合）不一致，且这个项目历史记录里用过 extended thinking 时，明确提示有 resume 失败风险**，而不只是"后端不一致"这一种情况。

## 5.6 CLI 交付形态

**已实现**：

```
aam project list                       # 本机 + 已同步的跨设备镜像（拼接展示，见下）
aam project show <name>                # 模糊匹配，打印匹配到的每条记录详情（含 projectId）
aam project resume <name>              # 打印 cd + resume 命令，绝不代跑；Profile 缺失/非官方后端会警告，
                                        # 本机找不到目录时按 5.3 打印设备提示而不是失效的命令
aam project link <path-a> <path-b>     # 手动关联两条记录为同一逻辑项目（08 #8，见下）

aam session scan                       # 对已注册的每个 Profile 只读扫描，不改索引（5.7）
aam session adopt                      # 采集入库：写本地索引，syncApproved=false（5.8）；
                                        # --summarize 尚未实现，见 08 #17
aam session approve-sync <path...>     # 显式批准回溯采集的条目参与同步（5.9）
aam session approve-sync --all-scanned # 一次性批准所有 scan 来源、尚未批准的条目
aam session sync                       # 同步 Memory-Bank 索引：拉取共享 blob → 用本机当前
                                        # syncApproved 记录替换本机在共享集合里的那部分（不影响
                                        # 其他设备）→ 写入本机 ~/.aam/memory/remote-index.json
                                        # 镜像（project-index.json 本身永不被这一步写入，见 08 #9）
                                        # → 推回去
```

`aam project list`/`show` 展示时把本机索引和 `remote-index.json` 镜像简单拼接，**不做自动的跨设备去重**——同一个逻辑项目在两台设备上默认显示成两行（各自的 `deviceId`/`profileLabel`）。`aam project link` 提供手动关联（`08` #8）：给两条记录（可以一条在本机、一条在镜像）打上同一个 `projectId`，不猜、不自动匹配同名/同路径——展示层暂时还是不按 `projectId` 分组渲染成一行，这两条记录只是"肉眼可核对已关联"，真正的分组渲染留给以后有需要再做。

**待实现**：

```
aam project enable-full-sync <name>    # 显式开启高风险的完整目录同步（Phase 6，见 5.4）
```

## 5.7 本机会话发现（`aam session scan`）

泛化今天 `project-tracker` 的 `backfill-index.ps1`（目前只扫 Claude 一个工具、一个固定的 `~/.claude`）成跨工具、跨已注册 Profile 的通用扫描：

- **扫描范围**：对 `aam profile list`（`03`）里注册的**每一个** Profile，分别去它对应的 `CLAUDE_CONFIG_DIR`/`CODEX_HOME` 下找会话记录——Claude 是 `<profile-dir>/projects/*/*.jsonl`（提取 `cwd` + 最新 `ai-title`，沿用 `backfill-index.ps1` 现有逻辑），Codex 是 `<profile-dir>/sessions/YYYY/MM/DD/rollout-*.jsonl`（提取 metadata 头部行的 `cwd`/时间戳/`model_provider`；如果 `<profile-dir>/session_index.jsonl` 存在就优先读它做快速索引，不存在则退化为直接扫 `sessions/**`，见 `08`）。
- **去重**：跳过本地索引里已经存在（不论 `discoverySource` 是 `live` 还是 `scan`）的 `(path, lastSessionId)` 组合。
- **只读**：`scan` 本身不写入 `project-index.json`，只打印/返回一份"发现了 N 条未纳管会话"的清单供用户过目，真正入库是下一步 `adopt`。这个"先看后动"的两段式设计和 `09` 里 Skills 的 `scan`/`adopt` 完全对称，是本轮统一确立的 UX 模式。

## 5.8 采集入库（`aam session adopt`）

把 `scan` 找到的会话（全部，或用户指定的子集）写入本地 `project-index.json`：

- 自动填充：`path`、`lastSessionId`、`lastActive`（取文件 mtime）、`deviceId`、`toolKind`、`profileLabel`（来自扫描时的 Profile 归属）、`discoverySource: "scan"`、`syncApproved: false`。
- `autoStatus`：
  - Claude 会话：能从 `ai-title` 记录里提取到，直接复用（今天 `project-tracker` backfill 已经这么做）。
  - Codex 会话：rollout 文件里没有等价的自动摘要字段（`08`）。**默认留空**（`autoStatus: null`，提示用户像 `statusOverride` 一样手动填一句话）。
  - 加 `--summarize --profile <label>` 时（**已实现**），改为调用 `<label>` 对应 Profile 的 Provider 生成摘要并写入 `autoStatus`——**硬性要求：必须显式给 `--profile`，缺了直接报错退出，绝不静默挑选**；`<label>` 对应的 Profile 必须挂了第三方 Provider，官方订阅没有 `Provider` 实现（`03.1`），选中官方订阅的 Profile 会明确报错而不是尝试用 OAuth 凭据拼一个 API 调用。摘要只对 `autoStatus` 已经是空的会话生成（主要是 Codex），不重新生成/覆盖 Claude 已有的 `ai-title`。**单条摘要失败不中断整个 `adopt`**——打印警告，这一条的 `autoStatus` 留空，继续处理其余会话。
    - 摘要的输入是会话原始文件（rollout/transcript）的前 6000 字符，不做结构化解析（Claude/Codex 两边 JSONL 结构不同，解析成本不值得，模型对半结构化日志的归纳能力足够），直接连同"用一句话概括这个会话在做什么"的指令一起发给 Provider 的 `complete()`（`Provider` trait 新增的通用文本补全能力，Anthropic Messages API 协议，`X-Api-Key` 认证——协议选择的核实过程见 `08` #17）。
- **这一步全程本机操作，不触碰网络/WebDAV**（除非 `--summarize` 选用的 Profile 恰好是需要联网调用的后端——但那是"调用 AI 生成摘要"这个动作本身要联网，跟"是否同步到 WebDAV"是两件独立的事，`adopt` 完成后 `syncApproved` 依然是 `false`）。

## 5.9 显式批准再同步（`aam session approve-sync`）

`aam session sync` 推送索引到 WebDAV 时，只推 `syncApproved: true` 的条目。`scan`/`adopt` 产生的条目默认 `false`，需要用户额外跑：

```
aam session approve-sync <name...>     # 按名字批准一批
aam session approve-sync --all-scanned # 一次性批准所有 scan 来源、目前还没批准的条目（明确知道自己在做什么才用）
```

批准后该条目的 `syncApproved` 改为 `true`，下次 `aam session sync` 才会把它带上云端。`live` 来源的条目不受这一步影响（一直是 `true`，这是既有 G3 目标的正常行为，不因为本轮新增设计而收紧）。
