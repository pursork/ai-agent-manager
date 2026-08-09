# 06. GUI 终端壳（Phase 4-5）

## 6.1 技术选型（已确认）

**`iced` + [`iced_term`](https://github.com/Harzu/iced_term)**（基于 `alacritty_terminal`），不用 Tauri/webview。选型依据见 `01-prior-art.md` 1.6 节——`iced_term` 是明确测试过 Windows 的终端组件，同类的 `egui_term` 官方声明未测试 Windows。

`iced` 采用消息驱动（Elm 架构：`Model → update(Message) → view`），这与本项目本身"大量异步 I/O"（多个 PTY 输出流、后台 WebDAV 同步、凭据存活验证网络请求）的特性天然契合——`iced::Subscription` 机制可以把"某个终端标签页有新输出""后台同步完成""凭据验证结果返回"都统一建模成消息流入同一个 `update` 循环，不需要额外的回调/共享状态锁的手搓方案。

## 6.2 GUI 是壳，不是新的业务逻辑层

**硬性原则**：`aam-gui` crate 只做渲染和用户交互，所有实际操作（账号切换、Provider 切换、凭据同步、项目索引查询）全部调用 `aam-switcher` / `aam-sync` / `aam-memory` 暴露的公开 API——这些 API 在 Phase 1-3 已经通过 `aam-cli` 验证过可用。GUI 开发本身不应该发现"业务逻辑还没想清楚"这种问题，那说明前面阶段的文档/实现有缺口，要回头补，而不是在 GUI 层临时加逻辑。

## 6.3 应用结构草图

```
┌─────────────────────────────────────────────────────────┐
│  顶部：Profile 快速切换栏（当前聚焦终端标签页用的是哪个       │
│  账号+Provider，点击可切换 —— 调用 03 模块，遵循"只影响     │
│  新启动进程"的语义：切换会提示"是否新开一个标签页"，          │
│  不会尝试让当前运行中的 claude/codex 进程瞬间变身）           │
├───────────┬─────────────────────────────────────────────┤
│           │  标签页 1: [🟢 X项目 · Claude · 官方账号1]      │
│  侧边栏：   │  ┌───────────────────────────────────────┐  │
│  项目/会话  │  │                                       │  │
│  选择器     │  │     iced_term 渲染的终端内容              │  │
│  (来自 05   │  │     (对应一个真实 PTY 子进程,             │  │
│   模块的    │  │      启动时已注入正确的                  │  │
│   跨设备    │  │      CLAUDE_CONFIG_DIR/ANTHROPIC_*)      │  │
│   索引)     │  │                                       │  │
│           │  └───────────────────────────────────────┘  │
│  - 项目X    │  标签页 2: [🟡 Y项目 · Codex · DeepSeek]      │
│    🟢设备A  │  ...                                       │
│    ⚪设备B  │                                             │
│  - 项目Y    │                                             │
└───────────┴─────────────────────────────────────────────┘
```

## 6.4 关键交互流程：新建一个"接续项目 X"的终端标签页

1. 用户在侧边栏点击"项目 X"（数据来自 `aam-memory` 的跨设备索引，`05`）。
2. 若该项目路径在本机不存在 → 按 `05.3` 的流程弹出提示，不建标签页。
3. 若存在 → GUI 展示"上次用的 Profile 是【Claude·官方账号2】，是否沿用？"（也允许用户改选）。
4. 确认后，`aam-gui` 调用 `aam-switcher` 拿到该 Profile 对应的完整启动环境变量集合（`CLAUDE_CONFIG_DIR` 或 `CODEX_HOME` 指向哪个目录、`ANTHROPIC_BASE_URL`/`ANTHROPIC_API_KEY` 等）。
5. 用这组环境变量 + 项目路径作为 cwd，通过 `iced_term` 起一个新 PTY，实际执行 `claude --resume <lastSessionId>`（或 Codex 等价命令）。
6. 新标签页出现，标题栏显示"项目 X · 工具 · Profile"三元组，方便用户在多标签页间视觉区分（这一点是 Termius 类工具的核心 UX，必须做到——不同标签页对应完全不同的账号/后端时，视觉上要一眼分辨，避免用户在错误的账号下继续工作）。

## 6.5 与"只提示不代劳 cd"原则的关系

`project-tracker` 技能里"Claude 不能替用户 cd，只能给命令提示"是因为**Claude 作为对话式 Agent 无法控制用户已经打开的终端进程**。但 `ai-agent-manager` 的 GUI 是**我们自己起的终端**，不存在这个限制——GUI 里点击"接续项目 X"，是可以、也应该**真的直接开一个新终端标签页并执行 resume 命令**的，不需要退化成"打印一段命令给用户复制"。这一点在 GUI 语境下是合理的功能升级，不违背 `project-tracker` 场景下的原设计原则（两者面向的载体不同：一个是对话里的文字建议，一个是我们自己拥有完整控制权的终端宿主）。

## 6.6 Phase 4 / Phase 5 的拆分

- **Phase 4**：GUI 先只做"包裹已有 CLI 能力"——暂不嵌入真实终端（用"点击后打开系统终端并直接执行命令"过渡）。范围在立项时按用户要求扩大到覆盖全部 CLI 能力，分 Round 推进，见 `07-roadmap.md` Phase 4 一节。
- **Phase 5**：接入 `iced_term`，实现 6.3-6.4 描述的完整体验（真正嵌入的多标签终端 + 一键接续）。

这个拆分让"GUI 框架搭起来"和"终端嵌入这个技术难度更高的部分"解耦，Phase 4 结束时已经是一个可用的产品（哪怕还没有内嵌终端），不会因为 `iced_term` 遇到问题而阻塞整个 GUI 阶段的交付。

## 6.7 Phase 4 Round 1 实现记录（已实现）

- **`iced` 版本**：`0.14`（发布时最新稳定版，事先用 WebFetch 核对过 `docs.rs`/GitHub 源码确认过 API 形状，没有凭训练记忆硬编码——`iced` 大版本间 API 变动大，`Command` 早年改名成了 `Task`，`Application` trait 风格现在也不是唯一/推荐用法）。用的是函数式 builder：`iced::application(boot_fn, update_fn, view_fn).title(...).run()`，`boot_fn` 返回 `(State, Task<Message>)`，`update_fn` 返回 `Task<Message>`，不是老式的 `Application` trait 实现。
- **阻塞调用桥接**（`crates/aam-gui/src/task.rs`）：`ProfileRegistry`/`ProviderRegistry` 读写、`verify_login`（起外部进程）这些同步调用，用 `std::thread::spawn` + `iced::futures::channel::oneshot`（`iced` 自带 `futures` re-export，不需要额外引入 `tokio`）包成 `iced::Task`，喂给 `Task::perform`。
- **终端拉起原语**（`crates/aam-gui/src/terminal.rs`）：优先探测 `wt.exe`（PATH + Microsoft Store 包的 App Execution Alias 常见落点 `%LOCALAPPDATA%\Microsoft\WindowsApps`），找不到永远退回 `powershell.exe`——这条退回路径是硬性要求，不能因为没装 Windows Terminal 就整体失败。命令行拼接（`wt_args`/`powershell_args`）拆成纯函数，脱离真实进程/文件系统单测；真正探测/启动依赖真实环境，人工验证。`install_windows_terminal()`（`winget install`）只从 GUI 里一次性提示的按钮触发，绝不静默自动装。
- **窗口化 GUI 的可测试性边界**：这是本项目第一次交付真正的窗口应用，没有类似 Chrome DevTools 那样的原生窗口自动化点击工具，`cargo test` 覆盖不到交互行为。已做到的自动化验证：`build`/`clippy -D warnings`/`test` 全绿 + 手动后台启动 `aam-gui.exe`、等待几秒确认进程未崩溃、检查内存/句柄符合真实渲染窗口的特征，再关闭；点击/表单可用性这类主观体验，需要用户自己跑一下亲眼看。
- **模块划分**：`main.rs`（入口）/`app.rs`（顶层 State/Message/路由，含 Windows Terminal 缺失提示条）/`screens/{profiles,providers}.rs`（各自的局部状态+视图+更新逻辑，直接调用 `aam-switcher` 的 `claude_backend`/`codex_backend`/`ProfileRegistry`/`ProviderRegistry`/`provider_secret_store`——跟 `aam-cli::commands.rs` 的 `run_profile`/`run_provider` 调同一套 API，没有另起业务逻辑）。

## 6.8 Phase 4 Round 2 实现记录（已实现）

立项时用户明确要求"GUI 的核心是用户友好性，仔细思考这一点"——这轮把这句话落成了具体、可执行的设计约束，不只是把 CLI 参数堆成表单：

- **信息分层**：项目浏览器（`screens/projects.rs`）默认只显示决定"要不要接续"所需的信息（项目名、`工具·Profile` 徽标、最后活跃时间、本机能不能接续），`projectId`/`discoverySource`/`syncApproved`/`authBackend`/`deviceId` 收进每行的"详情"折叠区，不在主列表劈头列一堆。
- **单一主操作**：每张项目卡片只有一个突出的主按钮（"接续"），"详情"是次要操作；`style.rs` 的 `primary_button`/`secondary_button`（分别用 `iced::widget::button::primary`/`secondary` 预设样式）在四个屏幕（含 Round 1 的 Profiles/Providers，顺手做了统一）里都用同一套写法，不是新旧屏幕两种视觉语言。
- **人话错误提示**：目录不存在/Profile 未在本机注册这些情况，复用 `commands.rs::ProjectAction::Resume` 已经打磨过的文案语气（`resumable()`，纯函数，单测覆盖），不是把 Rust 错误类型直接 `format!("{:?}")` 甩出来。
- **核心安全约束常驻可见**：会话面板（`screens/sessions.rs`）顶部有一条不需要展开就能看到的说明条，"扫描/采集到的会话默认只留在本机，不会自动出现在其他设备"——`05.7`-`05.9` 的硬性约束，不是藏在某个按钮的 tooltip 里。
- **按行反馈**：项目浏览器的"接续"/"关联"是可能并发触发的逐行操作，回执（`ResumeStatus::Opening`/`Failed`）显示在对应行旁边；会话面板的扫描/采集/批准是整屏批量动作，继续用 Round 1 那种共享状态栏就够，不过度设计。
- **共享的启动环境逻辑**（`crates/aam-gui/src/launch.rs`，从 `screens/profiles.rs` 重构提出）：`resolve_provider`/`launch_env` 现在是 Profiles 和 Projects 两个屏幕共用的一个模块，不是各自维护一份。
- **`profiles`/`providers` 数据流**：从 Round 1"每个消费方自己镜像一份 + 手动同步消息"改成 `app::State` 直接持有、按 `&[Profile]`/`&[ProviderRecord]` 引用传给需要的屏幕（`view`/相关 `update` 分支的参数），给 Round 3/4 继续加屏幕铺路，不用每加一屏都抄一遍同步逻辑。
- **跨线程边界的一个真实坑**：`aam_switcher::Provider` trait 没有 `Send` 约束（也不必为了这一个用例去改这个已有 trait 的公开签名），所以不能在 GUI 主线程构造好 `Box<dyn Provider>` 再整体移进 `perform` 的后台线程闭包——会话面板的"生成摘要"改成把 `(ProviderRecord, api_key)`（纯 `Send` 数据）传进闭包，在闭包内部（已经在后台线程上）才用 `aam_switcher::build_provider` 现场构造，跟 `launch.rs` 的调用方式一致。
- **`iced::Pixels` 只有 `From<f32>`/`From<u32>`，没有 `From<u16>`**：`style.rs` 的间距常量因此是 `f32` 不是 `u16`（第一次写成 `u16` 时编译器直接报错，不是猜出来的）。
- **验收边界同 Round 1**：友好与否是主观判断，`cargo test` 只能保证逻辑正确、不崩溃，不能替用户点头——这轮结束后明确请用户自己跑一遍 `aam-gui.exe` 给实际反馈，不是自认为做完了就结束。
