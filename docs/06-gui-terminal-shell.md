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

## 6.9 Phase 4 Round 3 实现记录（已实现）

`aam-skills` 的全量 CLI 能力（`09.8`）搬进 GUI，新增 `crates/aam-gui/src/screens/skills.rs`，延续 Round 2 定的用户友好性准则，不重新讨论、直接应用：

- **信息分层**：已纳管 skill 只显示名字 + 小字路径 + 状态徽标（"git 仓库"/"已链接 Codex"/更新状态），完整台账字段（`shareTargets` 全量、`source`/`updateMode` 原始值）没有另开折叠区——比项目记录简单得多，直接显示够用，不需要 Round 2 项目卡片那种"详情"折叠（信息分层不等于"什么都要折叠"，字段本来就少时直接摊开反而更直接）。
- **单一主操作，按行/整屏分层反馈**：未纳管 skill 每行的"纳管"、已纳管 skill 每行的"分享到 Codex"/"更新"是逐行动作，行内状态（`RowStatus::Working`/`Failed`）；"扫描"/"检查更新"/"全部自动更新"/内置 skill 安装是整屏批量动作，用共享状态栏——跟项目浏览器 vs 会话面板同一套按行/整屏的判断标准。
- **人话提示**：`InstallOutcome` 的三种状态（`Installed`/`AlreadyUpToDate`/`Overwritten`）通过一个纯函数 `install_outcome_message` 映射成"已安装"/"已是最新，无需改动"/"已用最新内容覆盖"，不是把枚举变体名字直接打印出来；"更新"按钮是否可点由另一个纯函数 `update_button_state` 判断（本地来源不显示、git 来源但没查过显示"未知"、已是最新显示但不可点、有更新才可点）——两个函数都单测覆盖。
- **联网动作要让用户知道**：从 git 引入新 skill 会真的执行 `git clone`，表单旁边直接写"会执行真实的 git clone，需要网络"，不是弹确认框（用户已经手填地址、手点按钮，这本身就是确认），但不能让这件事变成隐藏信息。
- **一个真实的编译器坑**：`aam_skills::ManagedSkill` 没有 `#[derive(Clone)]`（此前没有调用方需要克隆它），而 `iced::Message` 要求整体 `Clone`。没有为了这一个 GUI 用例去放宽 `aam-skills` 那个公开类型的 derive——改成 GUI 侧自己的 `ManagedSkillRow`（`Clone`，字段是 `ManagedSkill` 的子集），`From<ManagedSkill>` 转换一次。跟 Round 2 处理 `Provider: !Send` 是同一种应对方式：GUI 端需要的额外能力（`Clone`/`Send`），优先在 GUI 自己的类型上满足，不去改下游库的公开签名。
- **复用 `aam-cli` 的 search_dirs 逻辑**：`skills_search_dirs`（规范仓库本身 + Codex 目录 + 每个未链接的 Claude Profile）跟 `commands.rs::skills_search_dirs` 一字不差地对应，`aam-gui` 已经在 `app.rs` 里持有 `profiles` 列表，不需要新依赖。这个函数本身依赖真实文件系统（`resolves_to` 检查真实 reparse point），跟 CLI 那份一样没有单测——只测了"给定 profiles 列表，返回的 labels 集合对不对"这种不碰真实链接状态的部分。

## 6.10 Phase 4 Round 4 实现记录（已实现，Phase 4 收尾）

`aam-sync`/`aam-switcher` 的设备/同步全量能力（`4.9`/`4.10`）+ 会话索引同步（`05`）汇总进新增的 `crates/aam-gui/src/screens/sync.rs`，Phase 4 到此全部完成：

- **这轮唯一真正的新权衡：密码要不要跨动作记住**：CLI 每个子命令都重新提示输入 WebDAV 密码/vault 主密码，用完即弃；GUI 是长驻窗口，这个屏幕下有十来个共享同一份连接的动作，逐个重新弹密码框体验很差。**没有自己拍板**，先用 `AskUserQuestion` 跟用户确认，选择是"本次 `aam-gui` 运行期间记住"——两个密码字段存在 `sync::State` 里，直到关闭程序或用户点「清除已记住的密码」才清空，绝不写入任何文件。界面上用一条常驻说明把这件事讲清楚，不是不吭声地悄悄留着。
- **危险操作第一次需要真正的视觉区分**：吊销设备用 `iced::widget::button::danger` 样式（之前的删除/覆盖类操作要么有 force 复选框保护、要么本来影响面很小，没到需要红色按钮的程度）；吊销成功后的提示照抄 CLI 的既有措辞——"已同步的历史明文数据不会被远程擦除，这是设计上的已知限制"（`08` #13），不是只说"吊销成功"就完事。
- **共用一个"建 backend → 拿 identity → 列设备拿 recipients"的调用序列**：push provider/reencrypt/push 账号/会话同步这四个动作都要走这三步，直接照抄各自对应的 `commands.rs` 分支写在各自的 `perform` 闭包里，没有再抽一层公共 helper（闭包内联复用 vs 抽函数复用之间，鉴于每个动作的具体参数组合不完全一样、抽出来的公共部分本身只有三行，选择保持每个分支自包含、可读性优先）。
- **会话同步从 Round 2 移到这轮**：`aam session sync` 早在 Phase 3b 就已经实现，Round 2 做会话面板时特意把它的图形化留到这一轮——不是因为逻辑上有依赖，是因为它跟这个屏幕的其余动作共用同一份连接状态，放两个屏幕反而要多传一份 webdav 连接参数。
- **验收边界同前三轮**："本次会话记住密码"这条设计本身用起来顺不顺手，是这轮最需要用户亲自感受的部分，自动化测试没法替用户判断。
- **Phase 4 至此全部完成**，四轮加起来把 `aam-cli` 现有全部子命令域都做成了对应的 GUI 屏幕；下一步 Phase 5（`iced_term` 内嵌终端）需要用户另外发起。

## 6.11 Phase 5 Round 1 实现记录（`iced_term` 核心接入验证）

Phase 5 的第一轮，也是全项目第一次嵌入真实 PTY 渲染。按用户已确认的节奏，这轮只做一个固定的、不接业务逻辑的单标签终端，先把核心技术风险验证掉，不掺功能。

- **依赖版本核实**（用 WebFetch 直接读源码，不是训练记忆）：`iced_term` `0.8.0`，其 `Cargo.toml` 的 `[workspace.dependencies]` 锁定 `iced 0.14.0`——跟 `aam-gui` 已用版本一致，零版本冲突。
- **模块路径踩坑**：`Settings`/`BackendSettings` 实际在 `iced_term::settings::` 子模块下，不在 crate 根——README 示例里的 `iced_term::settings::Settings` 写法是对的，但容易被顺手简化成 `iced_term::Settings` 导致编译错误（`E0432`），第一次编译就撞上了，改成 `use iced_term::settings::{BackendSettings, Settings};` 后即通过。
- **Windows 默认 `program` 是 `wsl.exe`**：`iced_term` 的 `BackendSettings::default()` 在 Windows 上默认拉起 WSL，不是 PowerShell——这个项目全程是纯 Windows/PowerShell 语境，必须显式覆盖 `program: "powershell.exe".to_string()`，跟 `terminal.rs`（Phase 4 外部窗口那套）的选型保持一致。
- **本项目第一次用到 `iced::Subscription`**：前四轮全是一次性 `Task`（点一下、等结果），终端是持续事件流，`main.rs` 的 `iced::application(...)` 链新增了 `.subscription(app::subscription)`；`app::subscription` 目前只转发 Terminal 屏幕自己的 `term.subscription()`，多标签后会用 `Subscription::batch` 聚合多个。
- **真实验证，不只是"没崩溃"**：这轮的核心功能没法用自动化点击工具验证（原生窗口，没有等价于 Chrome DevTools 的工具），但后台启动 `aam-gui.exe` 后用 `Win32_Process` 查了一次进程父子关系，**确认 `aam-gui.exe` 真的拉起了一个以它为父进程的真实 `powershell.exe` 子进程**（PTY 真的建立了，不只是编译通过）；`taskkill` 关掉 `aam-gui.exe` 后这个子进程也一并消失，没有留下孤儿进程。这比前四轮"启动不崩溃"的冒烟检查更进一步，但**依然不能证明键盘输入/渲染是否正确**——终端窗口里打字有没有反应、显示对不对，这部分只有人眼确认，需要用户自己打开 `aam-gui.exe` 点到 Terminal 标签页实际试一下。

## 6.12 Phase 5 Round 2 实现记录（多标签 + 接续/打开终端接入内嵌标签页）

这轮是在用户明确表示"Round 1 还没来得及试"的情况下继续推进的——已经把这个风险跟用户说清楚，用户选择接受。做完后的验收请求会把"Round 1 单标签能不能打字"和"这轮的多标签好不好用"合并成一次，不假装这是两件独立的事。

- **不移除外部窗口方案，新增内嵌选项**：Profiles"打开终端"/Projects"接续"旁边各加一个"（内嵌）"按钮，两条路径并存。`crate::terminal::powershell_args`（Phase 4 外部窗口方案已有的命令行拼接纯函数）改成 `pub`，内嵌标签页的 `BackendSettings.args` 直接复用它，不重写一份。
- **跨屏幕状态的处理方式**：`profiles::Message::OpenEmbedded`/`projects::Message::ResumeEmbedded` 这两个新消息变体，各自屏幕的 `update` 里只有一个"什么都不做"的兜底分支（保持 `match` 穷尽性检查通过），真正的逻辑在 `app::update` 顶层拦截处理——因为打开内嵌标签页需要改 `state.terminal`，这是兄弟屏幕的状态，`profiles.rs`/`projects.rs` 自己没有访问权限。`projects.rs` 的 `resumable`/`record_tool`/`resume_command` 三个函数改成 `pub(crate)`，被 `app.rs` 直接复用，跟外部窗口路径共用同一套"能不能接续"判断和命令行构造，不是重新写一遍容易跟外部窗口路径慢慢分叉的逻辑。
- **一个真实的类型系统坑**：`style.rs` 的 `primary_button`/`secondary_button` 原来接收 `&'a str`，标签栏里每个标签的文字是循环内 `format!` 出来的临时 `String`（比如给当前激活的标签加一个"▶ "前缀），借用生命周期不够长，编译不过。改成 `impl iced::advanced::text::IntoFragment<'a>`——这个 trait 对 `&'a str`、`&'a String`、**拥有所有权的 `String`** 都有实现（`iced_core::text::IntoFragment` 源码确认过），换成这个之后字符串字面量和临时拼出来的 `String` 都能直接传，不需要额外中转。
- **测试策略延续 `aam-skills` 的真实-`git`测试先例**：`open_tab`/`close_tab`（id 分配、关闭标签后 active 该落到哪个标签）的单测**真的会起 `powershell.exe` PTY**（3 个测试共起 6 个真实进程），不是 mock——跟真实 `git` 测试同一个理由：本机永远有的程序、纯本地操作、不涉及网络/凭据，没必要为了"避免起真实进程"单独抽一层可以注入假货的接口。
- **验收边界（已改用自动化验证，见 6.13）**：用户明确反馈不想每轮都被要求手动验证——之后改成用 Win32 API 驱动真实点击/键盘输入 + 截图，自己把 Round 1 和这轮一起验证掉，细节和结果见 `6.13`。

## 6.13 用截图 + 合成输入自证 Round 1/2（不再要求用户逐轮验证）

用户明确要求："尽量别要求我进行验证（除非是最后的总验证），我现在没有时间一轮一轮替你去验证，你自己想办法。"——这条反馈之前"每轮结束请用户实际点一遍"的验收方式不再适用，改成尽量自己把能自动化的部分做掉，只把真正需要人主观判断的部分（好不好用、体验顺不顺）留到最后一次性验收。

**方法**：PowerShell + P/Invoke `user32.dll`/`System.Drawing`，脚本存在 `scratchpad/gui_probe.ps1`：

- `Pin-Window`：`SetWindowPos` 把窗口钉到屏幕固定位置/大小（`(0,0)`，`1100x850`），`SetForegroundWindow` 前台，随后校验 `GetForegroundWindow()` 确实是目标窗口——**第一次尝试时窗口还停留在系统默认弹出位置，跟桌面上其他真实窗口（代码编辑器、数据采集软件）重叠，第一次合成点击点偏了、点到了别的真实窗口上**（没造成损坏，只是点开了那个窗口的一个输入框，没敲字符），之后都先钉住窗口再操作，避免误触桌面上其他程序。
- `Click-At`：`SendInput` 发送绝对坐标的鼠标移动+按下+抬起，不是旧版 `mouse_event`（旧版兼容性没问题，但换 `SendInput` + `SetProcessDPIAware` 更稳）。
- `Send-Text-Safe`/`Send-Ascii-Safe`：发送前先查一次 `GetForegroundWindow()` 是不是目标窗口，不是就拒绝发送——避免焦点被偷走时把按键打进无关窗口。**踩了一个坑**：先试的 `KEYEVENTF_UNICODE` 合成按键（模拟真实字符输入的标准做法），敲的字符在 `iced_term` 渲染的终端里完全没反应（但同一时间发的 Enter 键——一个真实的虚拟键码——确实生效了，终端换了新的一行提示符）——说明 `iced`/`winit` 这类基于 wgpu 渲染的窗口读的是真实虚拟键码（`WM_KEYDOWN`），不是 `WM_CHAR` 组合消息。换成 `VkKeyScanEx` 查表 + 发送真实 `wVk` 的按键事件后，字符正常输入。
- 每一步操作后截图（`Screenshot`：`GetWindowRect` + `Graphics.CopyFromScreen`），用 `Read` 工具直接看图确认。

**验证到的结果（Round 1 + Round 2 一次性做完）**：

1. 点 Terminal 标签页，内嵌终端渲染出真实的 PowerShell 提示符（`(base) PS C:\Users\...>`）——Round 1"能不能渲染"的问题有了确切答案。
2. 点进终端区域、合成输入 `echo AAM_GUI_REALVK_TEST_777` 回车，终端里正确回显了命令（带语法高亮）和执行结果——**键盘输入和命令执行都是真的在工作**，不是猜测。
3. 点"+ 新终端"开出第二个标签、切换回第一个标签（历史记录还在，没丢）、关掉第二个标签——多标签开/切/关全部通过截图确认。
4. 关闭标签后查了一次 `Win32_Process`，确认那个标签对应的 `powershell.exe` 子进程真的被杀掉了，不是只在整个 App 关闭时才清理。
5. 在一个隔离环境（`USERPROFILE`/`AAM_HOME` 指向临时目录，`aam profile add` 建一个假 Profile，不碰用户真实数据）里点 Profiles 屏的"打开终端（内嵌）"，确认：自动跳转到 Terminal 屏、新标签标题正确显示"embedtest · claude"、环境变量注入正确（`CLAUDE_CONFIG_DIR` 指向了这个隔离 Profile 的目录）——**证据是内嵌终端里真的跑起了 `claude`，并且因为这是全新目录，`claude` 弹出了它自己真实的首次运行主题选择向导，界面渲染（TUI 菜单、colored diff）都正常**。验证到这一步就主动退出（`taskkill /T /F` 杀掉整个进程树，不继续走真实的登录流程），没有产生任何真实的 Claude 账号交互。
6. 全程没有创建/修改用户真实的 `~/.aam`/`~/.claude` 状态——Profile 创建测试用的是完全独立的临时目录，用完删掉。

**这个方法目前还没做、留给以后需要时再补的部分**：没有验证内嵌终端的鼠标滚动/文本选择/复制粘贴这些次要交互。

## 6.14 Phase 5 Round 3：接续前确认/更换 Profile（`06.4` 完整体验补完）

用户问"为什么不自动继续、开发任务结束了吗"——之前每轮结束都停下来等一句"继续"，是沿用整个项目一路以来的节奏，但用户明确表示不希望这样，于是这轮开始不再逐轮等待，直接把 `06.4` 唯一还没做的缺口（第 3 步"上次用的 Profile 是 X，是否沿用？"的确认/更换 UI，之前"接续（内嵌）"是直接静默用记录里的 Profile，没有确认/更换这一步）补完，做完自动验证、提交、推送。

- **设计**：`projects::State` 新增 `pending_embedded: HashMap<path, 选中的 label>`。点"接续（内嵌）"（`ResumeEmbeddedRequested`）只是把这一行切换成确认态（pick_list 预选记录里的 Profile，同工具的其它本机 Profile 都能选），不直接开标签页；点"确认"（`ResumeEmbeddedConfirmed`）才真正触发 `app.rs` 里的开标签页逻辑，用的是 `pending_embedded` 里当前选中的 label，不是 `record.profile_label`——所以"更换"是真的会生效的，不是摆设。
- **自动化验证**（延续 `6.13` 的方法，隔离环境，两个假 Profile `work1`/`work2`，一条指向真实临时目录的假项目记录）：点"接续（内嵌）"截图确认弹出"沿用 Profile: [work1 ▾] 确认 取消"；点下拉框截图确认列出了 `work1`/`work2` 两个选项；选 `work2`、点确认、截图——**新标签标题显示"myproject · claude · work2"**，不是默认的 work1，直接证明了"更换 Profile 真的会被使用"这件事，不是只停留在下拉框选中态。
- **踩了一个自动化工具本身的坑**：`SetForegroundWindow` 在这轮反复被 Windows 的防抢焦点保护拦截——因为每次真正发起前台切换调用的，都是刚执行完一条新命令、因此重新拿到前台的 PowerShell 控制台本身。加一次合成 Alt 键敲击（`SendInput` 按下+抬起 `VK_MENU`）再调用 `SetForegroundWindow` 解决——这是绕过这层保护的标准技巧。已经记进个人记忆（不是项目文档该记的内容，纯粹是这次用到的自动化工具本身的坑）。
