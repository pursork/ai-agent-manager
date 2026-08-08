# 05. 会话 / 项目 Memory-Bank 模块（Phase 3）

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
  "fullSyncStatus": null
}
```

- `deviceId` + `toolKind`：跨设备/跨工具场景下，"同一个项目路径 `X`"在设备 A 用 Claude 做过、在设备 B 用 Codex 做过，这两条记录**不合并成一条**，而是分别记录，但在 UI 上按项目"聚合展示"（同一个逻辑项目下面挂多条设备/工具/时间线记录）。项目的"逻辑身份"用什么做主键，是本模块需要在 Phase 3 立项时进一步定义的问题（候选：项目名 + 一个用户可选的稳定项目 ID，而不是物理路径，因为同一项目在不同设备上的绝对路径大概率不同）——记入 `08`。
- `fullSyncEnabled` / `fullSyncStatus`：见 5.4。

## 5.3 跨设备 Resume 的默认行为：只提示，不搬迁

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

## 5.6 CLI 交付形态（Phase 3）

```
aam project list                       # 跨设备聚合视图（不止本机）
aam project show <name>                # 某项目在各设备上的完整时间线
aam project resume <name>              # 复用 project-tracker 逻辑，跨设备版
aam project enable-full-sync <name>    # 显式开启高风险的完整目录同步
aam project sync                       # 手动触发一次 Memory-Bank 索引的 WebDAV 同步
```
