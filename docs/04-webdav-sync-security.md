# 04. WebDAV 跨设备同步与加密方案（Phase 2）

## 4.1 设计目标

- 用户已有自己的 WebDAV 服务器（自建 Nextcloud 或商业 WebDAV），本项目不提供、不运营任何云端服务。
- 只需要记住**一个主密码**（类 Bitwarden），就能在新设备上恢复所有账号凭据 + 项目进度索引，免重复登录。
- **服务器零知识**：WebDAV 上只有密文，服务器（以及任何能读到 WebDAV 存储的第三方）拿到的东西没有主密码/设备私钥就无法解密。
- 支持**吊销单台设备**而不需要更换所有设备的主密码——这是选择"每设备独立密钥对 + 多接收方加密"而不是"所有设备共享同一把对称密钥"的核心原因。

## 4.2 密码学方案：主密码网关 + 每设备 age 密钥对

两层结构，缺一不可：

### 层 1：设备清单控制文件（`devices.json.age`）—— 主密码直接保护

- 用户设主密码时，本地用 **`age` 的 passphrase 模式**（内部走 scrypt KDF，是 `age`/`rage` crate 原生支持、已被广泛审计的标准格式，不需要我们自己拼装 Argon2id+AEAD）加密一个小 JSON 文件：

```json
{
  "vault_id": "uuid-v4",
  "devices": [
    {
      "device_id": "uuid-v4",
      "label": "设备A-Win11-台式机",
      "age_public_key": "age1...",
      "added_at": "2026-08-08T12:00:00+08:00",
      "revoked": false
    }
  ]
}
```

- 这是**唯一**需要主密码才能解密的文件，体积很小，改动频率低（只在加入/吊销设备时变化）。

### 层 2：真正的数据 blob —— age 多接收方加密

- Credential Vault 的凭据快照、Memory-Bank 的同步索引，都用 **`age` 多接收方加密**，接收方列表 = 层 1 里所有 `revoked: false` 的设备公钥。
- 每台设备的 age **私钥只存在本机**，本机态保护沿用 `02` 里定义的"本地态"分层（Windows DPAPI / Linux `chmod 600`），私钥本身从不上传 WebDAV。
- 结果：新设备想解密任何数据，必须先拥有一份"被写入接收方列表"的公钥——而这一步只能通过主密码解开层 1 来完成。

## 4.3 新设备加入流程（对应"设备 B 未登录，免登录使用官方订阅"这个验收场景）

1. 用户在设备 B 上首次运行 `ai-agent-manager`，输入 WebDAV 地址 + 主密码。
2. 本地拉取 `devices.json.age`，用主密码解密。
3. 本机生成一个新的 age 密钥对（私钥仅本地保存）。
4. 把 `{device_id: 新UUID, label: 用户输入或自动生成, age_public_key: 刚生成的公钥, revoked: false}` 追加进设备列表。
5. 用主密码重新加密 `devices.json.age`，推回 WebDAV。
6. 拉取所有凭据/索引 blob（此时它们的接收方列表还不包含设备 B 的公钥——**设备 B 此刻还解不开**），触发一次"重新加密"：用任意一台已在接收方列表里、且当前在线并同意执行这一步的设备（或设备 B 自己如果它碰巧能拿到旧接收方的私钥——通常不能，所以这一步现实中**必须由已授权设备来做**）重新加密全部 blob，接收方列表加入设备 B 的公钥。
7. 完成后设备 B 才能真正解密凭据/索引 blob，拿到 Claude/Codex 官方订阅凭据，免登录直接用。

**诚实的局限（写入威胁模型）**：第 6 步意味着——**如果设备 B 是当前唯一在线的设备（比如设备 A 已经很久没开机），新设备加入流程无法立刻完成"重新加密现有 blob"这一步**，只能先加入设备清单，等下次任意一台旧授权设备上线时自动补齐重新加密。Phase 2 实现里必须把这个"待补齐重新加密"的状态显式呈现给用户（例如 CLI/GUI 提示"设备 B 已加入清单，但凭据尚未对其可见，需等待另一台已授权设备上线同步一次"），不能假装它已经生效。

## 4.4 吊销设备

1. 任意一台持有主密码的设备，把目标 `device_id` 标记 `revoked: true`（或直接从列表移除），重新加密 `devices.json.age` 推回。
2. 下一次任何 blob 更新时，接收方列表自动排除被吊销设备的公钥。
3. **诚实的局限**：被吊销设备本地缓存的、吊销前已经解密过的明文（凭据、索引）不会被远程擦除——这本来就超出"文件同步"这个机制的能力范围。文档/UI 需要提示用户：如果吊销是因为设备丢失/被盗等安全事件，应同时在对应的 Claude/Codex 官方渠道主动吊销该账号的 session/token，`ai-agent-manager` 只能保证"以后不再收到新数据"，不能保证"清除已经泄露的旧数据"。

## 4.5 WebDAV 目录布局

```
/ai-agent-manager/
├── vault_id                    # 明文，仅用于识别是不是同一个 vault，不含任何秘密
├── devices.json.age            # 层1：设备清单，主密码保护
├── credentials/
│   ├── claude/
│   │   └── <account-fingerprint>.blob.age   # 层2：多接收方加密
│   └── codex/
│       └── <account-fingerprint>.blob.age
├── providers/
│   └── <provider-id>.blob.age
└── memory-bank/
    └── project-index.blob.age  # 05 模块的跨设备索引
```

每个 `.age` blob 附带一个不加密的元数据（版本号/时间戳/写入设备 id），用于 4.6 的冲突判断，元数据本身不含任何秘密，明文存放不影响零知识目标。

## 4.6 冲突处理：不做真正的 diff/merge

凭据和索引这类小文件，采用**基于时间戳+版本号的整体覆盖**，不做真正的双向合并：

- 每个 blob 写入时带 `version`（单调递增）+ `updated_at` + `updated_by_device`。
- 拉取时如果远端 `version` 比本地缓存的更高 → 整体覆盖本地。
- 推送时如果远端 `version` 已经比本地准备推送的更高（说明另一台设备在此期间已经改过）→ **中止推送，先拉取合并**（对凭据/索引这种"通常只有一处在改"的数据，简单提示用户"检测到另一台设备的更新，请先同步"已经够用，不值得为这个场景实现真正的 CRDT/diff 算法）。
- 这条"不做真 diff"的原则**只适用于凭据和 Memory-Bank 索引**——`05` 里"可选的完整项目目录同步"是完全不同的、需要真正处理源代码冲突的场景，不复用这里的简化策略（那部分明确是高风险 opt-in 功能，见 `05` 和 `08`）。

## 4.7 威胁模型：明确写清楚"不做什么"

- **不信任 WebDAV 服务器本身**：即使服务器被攻破，攻击者只拿到密文和不含秘密的元数据/公钥。
- **不做无主密码找回**：忘记主密码 = 无法解密 `devices.json.age` = 无法加入新设备/吊销旧设备。已经加入的设备本地缓存的明文不受影响（它们的私钥不依赖主密码）,但要重新管理设备列表就做不到了。这个权衡需要在 GUI 首次设置主密码时用醒目文案告知用户，并建议用户自行找安全的地方记录主密码（本项目不做任何形式的密码托管/找回）。
- **不防护端点已被攻陷的设备**：如果攻击者已经拿到某台已授权设备的 age 私钥（本地态防护被突破），威胁模型上等同于拿到了合法接收方身份，这是本地态防护（`02`）的责任边界，不是本文档要解决的问题。
- **不做团队协作**：整套机制假设"所有设备清单里的设备都是同一个人的"，没有"多用户、分权限"的概念。

## 4.8 依赖的 Rust crate（Phase 2 实际选型）

- `age`：passphrase 模式（层1）+ 多接收方 X25519 模式（层2）均由官方 crate 原生支持，无需自己实现密码学原语。
- WebDAV 客户端：**不引入专门的 WebDAV crate**，复用 Phase 1 已经引入、已验证的 `ureq`（`aam-switcher::verify_http` 已经在用）——WebDAV 的 GET/PUT/MKCOL/DELETE 都是普通 HTTP 方法 + Basic Auth，`ureq::request(method, url)` 支持任意方法；PROPFIND（列目录）本轮不需要，用 GET 404 判断"文件不存在"即可满足 push/pull 的需求。减少一个新依赖的维护面（`08` 8.1 #5 已解决记录）。
- `rpassword`：主密码的隐藏式终端输入（不回显），比 Provider API key 的明文 stdin 输入更进一步——主密码是"一把解开一切"的密钥，理应隐藏输入。
- 测试策略：`aam-sync` 定义 `SyncBackend` trait，`WebDavBackend`（真实实现）之外另有 `LocalDirBackend`（同一套相对路径映射到本机目录），让 age 加解密往返、设备清单增删、版本冲突逻辑这些业务逻辑能在 CI 里被真正跑到，不需要连真实 WebDAV 服务器。

## 4.9 CLI 命令（Phase 2 第一批实现）

`aam-sync` 本身不知道"Provider"是什么（`02.1`：`aam-sync` 只依赖 `aam-core`，不能反过来依赖 `aam-switcher`），所以下面的 push/pull/reencrypt 命令里，"设备清单/加解密/冲突处理"部分调用 `aam-sync`，"这是一个 Provider 配置"这部分调用 `aam-switcher::provider_sync`（该模块因为 `aam-switcher` 本来就同时依赖 `aam-sync` 和 `aam-vault`，是承接这层业务绑定的正确位置）：

| 命令 | 说明 |
|---|---|
| `aam sync init --webdav-url <url> --webdav-user <user> --label <label>` | 在一个全新的 WebDAV 位置创建 vault，本机成为第一个设备（`4.3` 的第一台设备特例）。 |
| `aam device join --webdav-url <url> --webdav-user <user> --label <label>` | 加入一个已存在的 vault（`4.3` 步骤 1-5）。**不**自动完成步骤 6——加入后本机还不能解密已有 blob，需要已授权设备运行 `aam sync reencrypt`。 |
| `aam device list` / `aam device revoke <device-id>` | 查看/吊销设备（`4.4`）。 |
| `aam sync reencrypt --webdav-url <url> --webdav-user <user>` | `4.3` 步骤 6 的手动版：把本机已知的每个 Provider 配置用当前设备清单重新加密。已知限制：没有 PROPFIND 目录列举（`4.8`），只能覆盖本机本地已经注册过的 Provider id，见 `08` 待办事项。 |
| `aam sync push/pull --webdav-url <url> --webdav-user <user> --provider <id>` | 推送/拉取单个 Provider 的配置+密钥。`push` 会在推送前重新读取远端当前版本号（而不是信任本地缓存的版本号）作为 `push_if_not_stale` 的基准；`pull` 不需要主密码，只需要本机已有的设备私钥。 |

所有命令都要求显式传入 `--webdav-url`/`--webdav-user`（`07` 已确认的设计：不做"记住上次 vault"的隐式状态）；WebDAV 密码与 vault 主密码均通过 `rpassword` 隐藏输入，不作为命令行参数传入（避免出现在 shell 历史/进程列表里）。
