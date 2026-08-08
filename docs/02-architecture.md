# 02. 总体架构

## 2.1 模块划分（Rust workspace，多 crate）

```
ai-agent-manager/
├── crates/
│   ├── aam-core/        # 共享类型、错误定义、指纹算法（沿用 codex-skill 的指纹思路）
│   ├── aam-vault/        # Credential Vault：本地凭据存储 + 主密码 KDF + 设备身份
│   ├── aam-switcher/     # Account/Provider Switcher：Claude + Codex 两个 backend
│   ├── aam-sync/         # WebDAV Sync Engine：加密 blob 的推拉、设备清单、冲突处理
│   ├── aam-memory/       # Session/Project Memory-Bank：本地索引 + 同步
│   ├── aam-skills/       # Skills 跨工具共享 + 发现/纳管（明文/git，不经 aam-sync）
│   ├── aam-cli/          # CLI 入口（Phase 1 交付形态，最先做）
│   └── aam-gui/          # iced GUI 外壳（Phase 4+，依赖以上全部作为库）
└── docs/                 # 本目录
```

依赖方向：`aam-gui` → `aam-cli` 的逻辑层（或共享一个 `aam-app` 门面 crate）→ `aam-switcher` + `aam-memory` + `aam-skills` → `aam-vault` + `aam-sync` → `aam-core`。`aam-skills` 是个例外分支：它**不经过** `aam-vault`/`aam-sync`，直接依赖 `aam-core`——因为 `09` 里已经定案 Skills 走明文/git 而不是加密 WebDAV 通道，没有理由绑定同步引擎。GUI 不直接碰文件系统/网络，一律通过下层 crate 的公开 API，保证"先做 CLI 能力，GUI 只是壳"这条 Roadmap 主线在代码结构上也成立。

## 2.2 五大模块职责

| 模块 | 职责 | 对应文档 |
|---|---|---|
| **Credential Vault** | 本地加密存储各账号凭据（Claude 的 `CLAUDE_CONFIG_DIR` 整目录 / Codex 的 `auth.json`），管理设备本地身份（age 密钥对），提供主密码解锁/加锁 API | `03`（存什么）+ `04`（怎么加密） |
| **Account/Provider Switcher** | 执行"切换到某账号/某 Provider"这个动作本身：快照→原子写→存活验证→失败回滚。对 Claude 和 Codex 各有一个 backend 实现，第三方 Provider 通过统一 trait 接入 | `03` |
| **WebDAV Sync Engine** | 把 Vault 里的加密 blob、Memory-Bank 的加密索引，推送/拉取到用户自己的 WebDAV；管理"设备清单"这个特殊的、决定谁能解密的控制文件 | `04` |
| **Session/Project Memory-Bank Tracker** | 维护 `project-index.json`（本地态，直接沿用 `project-tracker` 的 schema）+ 通过 Sync Engine 同步的跨设备版本；负责"这个项目上次在哪台设备哪个路径"的问答；负责本机会话的扫描发现与"显式批准才同步"的把关 | `05` |
| **Skills Manager** | 维护一份规范 Skills 仓库，通过符号链接/Junction 让 Claude 的多个 Profile 和 Codex 同时看到同一份 skills；负责本机 skills 的扫描发现、纳管、GitHub 来源的更新检查；不经过 WebDAV 加密通道，走明文/git | `09` |
| **GUI Shell** | iced 应用，多标签终端（`iced_term`）+ 侧边栏会话/账号选择器，调用以上模块的 API，不重复实现业务逻辑 | `06` |

## 2.3 数据流（一次典型的"切换账号并继续项目"操作）

```
用户在 GUI（或 CLI）选择 "项目 X → 设备B → Claude 官方账号2"
        │
        ▼
┌───────────────────┐   1. 解锁本地 Vault（主密码，仅设备首次/超时后需要）
│  aam-vault         │──────────────────────────────────┐
└───────────────────┘                                    │
        │ 2. 取出 "Claude 官方账号2" 对应的 CLAUDE_CONFIG_DIR 快照
        ▼
┌───────────────────┐
│  aam-switcher       │  3. 快照当前状态 → 原子写目标账号目录 → 调用等价于
│  (Claude backend)  │     `claude /status` 的存活检查 → 失败则自动回滚
└───────────────────┘
        │ 4. 切换成功
        ▼
┌───────────────────┐
│  aam-memory         │  5. 查 project-index：项目 X 的 lastActive 记录在
└───────────────────┘     "设备 A，路径 D:\...\X"
        │
        ▼
   本地路径存在？
   ├─ 是 → 直接给出 cd + claude --resume 提示（同今天的 project-tracker 逻辑）
   └─ 否 → 明确提示"该项目工作目录在设备 A，本设备未同步完整目录，
            请前往设备 A 的 D:\...\X 继续"，不做任何静默重建
```

## 2.4 安全模型分层

安全设计明确分成两层，**不混用同一套强度假设**（这条原则直接继承自 `codex-skill` "DPAPI vs `chmod 600` 诚实分层"的教训）：

### 本地态（Local State）

单台设备上，未同步、静止不动的凭据缓存：

- **Windows**：DPAPI（`CurrentUser` scope）加固，沿用 `codex-skill` 已验证的模式。
- **Linux**：文件权限 `chmod 600` 起步，文档中明确注明这不抵抗 root/磁盘快照级别的威胁。
- 本地态的加固强度**因 OS 而异是被接受的、写清楚的**，不假装两个平台等价安全。

### 同步态（Sync State）—— 与 OS 无关

一旦数据要离开单台设备（推送到 WebDAV），一律走统一的、不依赖任何 OS 特性的加密方案，核心是：

- 用户的**主密码**只用来解锁一个很小的"设备清单控制文件"（谁的公钥被授权）。
- 真正的凭据/会话索引 blob，用 **age 多接收方加密**（加密给"当前所有未被吊销设备"的公钥列表），服务器（WebDAV）全程零知识，只存密文。
- 每台设备有自己独立的 age 密钥对（私钥仅本地态保存，受上面"本地态"那层保护），**不是所有设备共享同一把对称密钥**——这样才能做到"吊销单台设备"而不需要更换所有设备的密码。

完整方案（KDF 参数、控制文件格式、设备加入/吊销流程、WebDAV 目录布局）在 `04-webdav-sync-security.md` 展开，这里只定架构层面的分层原则。

## 2.5 设备身份模型（概要）

- 每台设备首次运行时生成一个本地 `device_id`（UUID v4，非秘密）+ 一个本地 age X25519 密钥对（私钥受本地态保护）。
- "加入 Vault"= 用主密码解密设备清单控制文件 → 把自己的 `device_id` + 公钥追加进去 → 重新用主密码加密写回 → 触发一次同步，让所有 blob 的加密接收方列表包含新设备。
- "吊销设备" = 从设备清单里标记 `revoked: true` 并移除其公钥 → 之后所有新写入的 blob 不再加密给它 → 建议（不强制）用户同时更换主密码，因为被吊销设备本地可能仍缓存着吊销前的明文数据，这属于诚实披露的已知局限（见 `04` 威胁模型）。

## 2.6 跨模块的一致操作哲学

不论是账号切换（`03`）、凭据同步（`04`）、项目索引更新（`05`），还是 Skills 符号链接/Junction 供给（`09`），全部遵循同一条从 `codex-skill` 继承来的原则：

> **任何会修改本地持久状态的操作，必须是"快照 → 原子写 → 校验 → 失败自动回滚"，不允许中间态可见，不允许静默失败。**

这条原则在 `aam-core` 里应该落成一个可复用的工具函数/trait（例如 `TransactionalWrite`），供 `aam-switcher`、`aam-sync`、`aam-memory` 三个模块共用，避免各自重复实现、行为不一致。
