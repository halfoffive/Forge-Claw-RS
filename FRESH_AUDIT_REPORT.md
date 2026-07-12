# ForgeClaw 多角度补充审计报告

> 报告编号：`FRESH_AUDIT_REPORT.md`
> 审计日期：2026-07-12
> 审计基线：当前 main HEAD `31c6db0`
> 前置审计：`REAUDIT_REPORT.md`（change-id `re-audit-after-fixes`，2026-07-11）
> 新增视角：Karpathy 代码质量准则、Frontend Design 审美准则

## 摘要

本次补充审计在 `REAUDIT_REPORT.md` 的基础上，引入 **Karpathy 代码质量**（简洁性、手术式改动、可验证目标）与 **Frontend Design 审美**（设计系统、排版、色彩、动效、空间构图）两个新视角，对当前 HEAD `31c6db0` 进行第三轮多角度审查。

### 统计

| 视角 | P0 | P1 | P2 | 合计 | NEW 数量（去重后） |
|------|----|----|----|------|-------------------|
| A Karpathy 代码质量 | 0 | 4 | 11 | 15 | 15 |
| B 前端设计审美 | 0 | 4 | 16 | 20 | 19 |
| C 安全与沙箱 | 4 | 7 | 9 | 20 | 2 |
| D 并发与性能 | 0 | 7 | 8 | 15 | 5 |
| E API/WS/编排器 | 1 | 10 | 16 | 27 | 9 |
| F 构建/CI/可维护性 | 1 | 4 | 10 | 15 | 5 |
| **合计（按角度去重前）** | **6** | **36** | **70** | **112** | **约 55** |

> 注：同一技术问题可能被多个视角同时提及（如 WS 超时未 abort 同时出现在 C/D/E），表中为各视角独立计数。跨视角去重后，**本次新增发现约 42 条**（P0=0，P1≈18，P2≈24）。

### 关键结论

1. **无新增 P0**：所有 P0 级问题（环境变量泄漏、沙箱缺失、命令注入、TOCTOU 等）均已在 `REAUDIT_REPORT.md` 中记录，当前 HEAD 未引入新的阻断性安全漏洞。
2. **Karpathy 视角发现大量可维护性问题**：CLI/server 重复造轮子、不可测试的 confirm 回调、静默 fallback、隐藏真实错误原因等，属于前两次审计未覆盖的“代码质量债”。
3. **前端设计审美是最大新增盲区**：WebUI 存在系统默认字体、`<pre>` 渲染长文本、无响应式策略、无设计系统、无 page-load 动效等问题，整体呈现“默认后台模板”观感。
4. **协议与并发残留风险集中**：WS 事件循环无超时、错误帧不发送、跨用户写回未复核、单连接 frame 并发无限制等协议层问题仍需修复。
5. **构建阻断性 P0 仍在**：`web/dist` 缺失导致本地 `cargo check/test` 全红，与前序审计结论一致。

### 健康度评估

- **安全性**：P0 未修复项仍高度危险，但无新增；P1 残留 7 项。
- **可维护性（新增评估）**：因 Karpathy 视角的加入，暴露较多重复逻辑、不可测试接口、隐藏假设，评级为 **需改进**。
- **前端体验（新增评估）**：设计方向缺失、排版保守、动效不足，评级为 **需大幅改进**。
- **并发/协议**：残留 P1 较多，评级为 **需改进**。
- **构建/CI**：P0 构建阻断仍未解决，评级为 **不健康**。

---

## 视角 A — Karpathy 代码质量

> 依据 Karpathy Guidelines 审查：Think Before Coding、Simplicity First、Surgical Changes、Goal-Driven Execution。

### A-001｜P1｜CLI 重复构造 server 侧默认沙箱
- **位置**：`crates/cli/src/commands.rs:61-128`
- **问题**：`build_orchestrator_confirm` 完整复制了 server 侧 `default_sandbox_with_specs` 的工具实例化、spec 描述与注册顺序，server 侧工具描述变更时 CLI 会漂移。
- **简化目标**：`Sandbox` 暴露 `with_confirmer()`，server 工厂返回 `(Sandbox, Vec<ToolSpec>)`，CLI confirm 模式直接复用默认沙箱并替换 confirmer。
- **可验证标准**：修改 server 默认工具描述后，CLI confirm 模式自动同步；新增测试断言两者工具列表一致。
- **重复性**：NEW

### A-002｜P1｜confirm 回调在 async 函数中阻塞读 stdin，不可测试
- **位置**：`crates/cli/src/commands.rs:87-101`
- **问题**：闭包直接 `io::stdin().read_line()`，阻塞当前 tokio worker；无法注入输入进行单元测试。
- **简化目标**：抽象为 `AsyncConfirmer` trait 或 channel 驱动，CLI 在 `spawn_blocking` 中读 stdin，测试通过 mock channel 注入 `true/false`。
- **可验证标准**：confirm 模式可单元测试且不阻塞 runtime。
- **重复性**：NEW

### A-003｜P1｜`Role::From<&str>` 对未知 role 静默回落为 User
- **位置**：`crates/llm/src/lib.rs:51-62`
- **问题**：上游 API 返回新 role 时，数据语义被篡改，违反“显式假设、不隐藏困惑”。
- **简化目标**：删除 `From<&str>`，改为返回 `Option<Role>` 或 `Result`；SSE 解析中遇到未知 role 时 warn 并跳过/报错。
- **可验证标准**：新增测试断言未知 role 不再静默变成 user。
- **重复性**：NEW

### A-004｜P1｜非法 profile/section 名被映射为 NotFound，隐藏真实原因
- **位置**：`crates/core/src/prompt/engine.rs:45-50,70-83`
- **问题**：路径遍历/非法名字被 `is_safe_name` 拒绝后，都被映射为 `ProfileNotFound` / `SectionNotFound`，调试和安全审计困难。
- **简化目标**：新增 `CoreError::InvalidName(String)`，让真实原因进入日志；API 层再决定是否对外脱敏为 404。
- **可验证标准**：测试验证 `../etc` 返回 `InvalidName` 而非 `ProfileNotFound`。
- **重复性**：NEW

### A-005｜P2｜`run_turn` 中直接用 struct literal 构造 ChatMessage
- **位置**：`crates/server/src/orchestrator.rs:336-341,444-449`
- **问题**：绕过 `ChatMessage::assistant` / `ChatMessage::tool` 构造器，破坏 `llm` crate 封装。
- **简化目标**：统一使用构造器；若参数不足则扩展构造器。
- **可验证标准**：grep 确认除反序列化外无 `ChatMessage { role: ... }` literal。
- **重复性**：NEW

### A-006｜P2｜`run_tool_exec` 在 async 函数中直接 `process::exit(1)`
- **位置**：`crates/cli/src/commands.rs:359-378`
- **问题**：跳过 tokio runtime 优雅关闭、临时目录清理与日志 flush，破坏函数返回 Result 的契约。
- **简化目标**：返回 `Err(anyhow!("tool failed: {err}"))`，由 `main()` 统一决定退出码。
- **可验证标准**：`run_tool_exec` 失败时 main 返回非零，但函数本身不调用 exit。
- **重复性**：NEW

### A-007｜P2｜`print_final_event` / `print_stream_event` 处理不可能分支
- **位置**：`crates/cli/src/commands.rs:144-153,296-302`
- **问题**：`run_once` 只返回 Complete/Error，`run_streaming` 在 Complete 处已 break，这些分支属于“对不可能场景做错误处理”。
- **简化目标**：删除不可能分支或用 `unreachable!()` 标注。
- **可验证标准**：编译通过，代码行数减少。
- **重复性**：NEW

### A-008｜P2｜`prompt_vars` 为取工具名构造完整沙箱后丢弃
- **位置**：`crates/cli/src/commands.rs:131-140`
- **问题**：过度包装，仅为了获取工具名列表而实例化所有工具。
- **简化目标**：直接复用 `Orchestrator` 已持有的 `tool_specs`，或在 `forgeclaw_tools` 暴露工具名列表函数。
- **可验证标准**：`prompt_vars` 不再调用 `Sandbox::default_for`。
- **重复性**：NEW

### A-009｜P2｜`config_path` 手写 HOME/USERPROFILE 回退
- **位置**：`crates/cli/src/config.rs:137-141`
- **问题**：跨平台性弱，且与 `tools/src/file.rs` 重复造轮子。
- **简化目标**：使用 `dirs::home_dir()` 统一处理 Windows/macOS/Linux。
- **可验证标准**：测试在 Windows/Linux 都能找到配置目录。
- **重复性**：NEW

### A-010｜P2｜`resolve_users` 与 `UserStore::from_env` 重复解析逻辑
- **位置**：`crates/cli/src/commands.rs:453-471`
- **问题**：`FORGECLAW_USERS` 解析逻辑复制粘贴，漂移风险。
- **简化目标**：把解析逻辑抽到 `forgeclaw_server::auth` 或 `forgeclaw_core`，CLI 直接调用。
- **可验证标准**：单一解析函数被 CLI 和 server 共享；测试覆盖空名/空 token 过滤。
- **重复性**：NEW

### A-011｜P2｜server 与 CLI 各有一个 `spec_for`
- **位置**：`crates/server/src/orchestrator.rs:503-512`、`crates/cli/src/commands.rs:119-128`
- **问题**：逻辑完全相同，重复实现。
- **简化目标**：给 `Tool` trait 增加 `to_spec(description: &str) -> ToolSpec` 默认方法。
- **可验证标准**：删除其中一个 `spec_for`，所有调用方编译通过。
- **重复性**：NEW

### A-012｜P2｜`client.ts` 未 catch `JSON.parse` 异常
- **位置**：`web/src/api/client.ts:95-101`
- **问题**：后端返回 HTML/纯文本错误时，抛出普通 `SyntaxError` 而非 `ApiError`，错误形态不一致。
- **简化目标**：将 `JSON.parse` 包进 try/catch，失败时按 `ApiError(res.status, text)` 抛出。
- **可验证标准**：mock 返回 `500 + 文本`，断言抛出 `ApiError(500, ...)`。
- **重复性**：NEW

### A-013｜P2｜前端预生成 `session_id` 的协议假设未显式化
- **位置**：`web/src/views/ChatView.vue:67-71`
- **问题**：后端据此创建会话，但协议仅在注释中说明，未在类型/API 契约中显式化。
- **简化目标**：`WsChatRequest` / `ChatRequestDto` 中显式要求 `session_id` 为可选 UUID；后端在 `Complete` 事件中回传 `session_id`。
- **可验证标准**：不预生成 id 也能正确关联会话。
- **重复性**：NEW

### A-014｜P2｜SSE 行结束符归一化创建两个临时 String
- **位置**：`crates/llm/src/client.rs:69-70`
- **问题**：`data.replace("\r\n", "\n").replace('\r', "\n")` 双重分配。
- **简化目标**：改用单次遍历归一化，或保留并加注释说明此路径非瓶颈。
- **可验证标准**：SSE 解析测试仍通过。
- **重复性**：NEW

### A-015｜P2｜`compile` 与 `list_sections` 重复加载与排序
- **位置**：`crates/core/src/prompt/engine.rs:96-124`
- **问题**：两者都执行 `load_all_sections` + `enabled_sorted`。
- **简化目标**：`compile` 直接调用 `self.list_sections(profile_name)` 获取已排序 sections。
- **可验证标准**：缓存命中/未命中测试通过，代码行数减少。
- **重复性**：NEW

### 视角 A 与 REAUDIT 重复项

| 本报告编号 | REAUDIT 编号 | 问题简述 |
|-----------|-------------|---------|
| R-A-001 | A-002 | `Session.messages` 为 pub 字段 |
| R-A-002 | C-004 | ShellTool 子进程继承全部环境变量 |
| R-A-003 | A-016 | `check` 缺失 command 时返回 Allow |
| R-A-004 | A-010 | WalkDir 错误被静默吞掉 |
| R-A-005 | A-NEW-002 | `spawn_blocking` 吞掉 JoinError |
| R-A-006 | A-012 | `trim_matches('"')` 处理 YAML 转义引号 |
| R-A-007 | B-004 / D-019 | PromptEngine 用 tokio::Mutex 包裹同步 IO |
| R-A-008 | B-008 / D-018 | `tool_specs.to_vec()` 每轮深拷贝 |
| R-A-009 | B-010 | `SessionData` 双源存储 |
| R-A-010 | E-NEW-001 | tool_result 按 name 回填 |
| R-A-011 | E-NEW-002 | ticket 获取失败时 popMessage 不一致 |
| R-A-012 | E-NEW-003 | error/close 未清理 assistant 占位 |
| R-A-013 | E-NEW-004 | 未使用 login 响应的 ticket |
| R-A-014 | E-NEW-005 | PromptsView 无保存 + `--mono` token 错误 |
| R-A-015 | A-008 | 5xx body 丢弃、429 未重试 |
| R-A-016 | C-016 / C-019 | `User` Debug 打印 token |
| R-A-017 | C-010 / C-020 | `find_by_token` 非常量时间查找 |

---

## 视角 B — 前端设计审美

> 依据 Frontend Design Guidelines 审查：设计方向、排版、色彩、动效、空间构图、背景质感。

### B-001｜P1｜整体美学方向缺失，界面为通用后台模板风格
- **位置**：`web/src/App.vue:36-61`、`web/src/views/*.vue`
- **问题**：侧边栏+主内容区、圆角卡片、纯色按钮、扁平布局与大量 admin 模板雷同；品牌仅文字 logo，无视觉符号、无氛围营造。
- **设计方向建议**：确立“锻造/熔炉/爪痕”工业炽热感或“代码符文”开发者气质母题，贯穿 logo、加载态、空状态、状态色、按钮形状。将 favicon 紫-蓝极光质感延伸至登录页与导航激活态。
- **重复性**：NEW

### B-002｜P1｜使用系统默认字体栈
- **位置**：`web/src/style.css:16-17`
- **问题**：`--font-sans: system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif` 明确使用了 guideline 禁止的默认字体；页面无特色字体。
- **设计方向建议**：引入 JetBrains Mono/Fira Code 用于代码/工具名，正文字体可选 Space Grotesk、Sora、Outfit 等几何无衬线。
- **重复性**：NEW

### B-003｜P1｜助手回复以 `<pre>` 渲染，破坏长文阅读
- **位置**：`web/src/views/ChatView.vue:234`
- **问题**：自然语言回复以等宽预格式化文本输出，行高、字间距、折行均不适合长文阅读。
- **设计方向建议**：助手气泡使用正文 `<p>` / 富文本渲染（`white-space: pre-wrap`），代码片段再用 `<pre><code>` 包裹并配深色代码块样式。
- **重复性**：NEW

### B-004｜P1｜布局为刚性对称双栏，无响应式策略
- **位置**：`web/src/App.vue:64-69`
- **问题**：`grid-template-columns: 220px 1fr` 固定侧边栏，无 `@media` 断点；移动端下基本不可用。
- **设计方向建议**：移动端采用抽屉式导航（`< 768px` 隐藏侧边栏，顶部显示汉堡菜单）；桌面端可尝试可变侧边栏或折叠图标模式。
- **重复性**：NEW

### B-005｜P2｜缺少统一的设计系统/组件语言
- **位置**：`web/src/views/ChatView.vue:313-324`、`LoginView.vue:125-139` 等
- **问题**：按钮、输入框、卡片在不同 view 中各自实现，圆角、padding、字重、边框微不一致。
- **设计方向建议**：建立 `components/BaseButton.vue`、`BaseCard.vue` 等，统一 token 与变体。
- **重复性**：NEW

### B-006｜P2｜空状态与加载态无视觉表达
- **位置**：`web/src/views/ChatView.vue:225-226`、`SessionsView.vue:30,43`、`ToolsView.vue:39,51`
- **问题**：空状态均为纯文本；加载态仅有文字“加载中…”。
- **设计方向建议**：为关键空状态设计小体量插画/图标+引导文案；列表加载使用骨架屏或脉冲占位。
- **重复性**：NEW

### B-007｜P2｜页面标题层级缺乏字号对比
- **位置**：`web/src/views/SessionsView.vue:61-65`、`ToolsView.vue:69-73`、`SettingsView.vue:67-71`、`PromptsView.vue:57`
- **问题**：所有页面标题均为 `20px/600`，与正文 14px 跳跃过小。
- **设计方向建议**：建立 4 级字号比例（如 12/14/16/20/28px 或 `clamp()` 流体尺寸）。
- **重复性**：NEW

### B-008｜P2｜无流体/响应式排版系统
- **位置**：`web/src/style.css:19`
- **问题**：根元素固定 `font: 14px/1.5`，未使用 `clamp()` 或视口比例。
- **设计方向建议**：引入流体类型比例，例如 `font-size: clamp(13px, 0.85vw + 10px, 16px)`。
- **重复性**：NEW

### B-009｜P2｜主色为 clichéd 紫色，品牌辨识度弱
- **位置**：`web/src/style.css:10,36`
- **问题**：`--color-primary: #7c5cff` 是生成式 AI 工具最泛滥的品牌色之一。
- **设计方向建议**：转向与“ForgeClaw”命名更契合的炽橙/钢青/暗金配色，或降低饱和度并引入戏剧性暗色背景。
- **重复性**：NEW

### B-010｜P2｜语义色 tokens 不足
- **位置**：`web/src/style.css:3-12`
- **问题**：仅有 bg/surface/text/muted/border/primary/danger，缺少 success/warning/info。
- **设计方向建议**：扩展 `--color-success`、`--color-warning`、`--color-info`、`--color-highlight`。
- **重复性**：NEW

### B-011｜P2｜组件中存在硬编码色值
- **位置**：`web/src/views/ChatView.vue:357,444`、`LoginView.vue:130`、`NotFoundView.vue:40`
- **问题**：多处使用硬编码 `#fff`，未引用 CSS 变量。
- **设计方向建议**：所有颜色引用 CSS 变量；白色文字使用 `--color-inverse` 或 `--color-surface`。
- **重复性**：NEW

### B-012｜P2｜缺少 page-load 编排与入场动效
- **位置**：`web/src/App.vue:36-61`、`web/src/views/*.vue`
- **问题**：页面切换与组件挂载均为瞬间出现，无 staggered fade/slide。
- **设计方向建议**：为侧边栏、页面内容、列表项添加克制入场动画（opacity + translateY，stagger 30-50ms）；使用 `<TransitionGroup>`。
- **重复性**：NEW

### B-013｜P2｜hover 交互单一且保守
- **位置**：`web/src/App.vue:89-105`、`SessionsView.vue:101-103`、`PromptsView.vue:115-128`
- **问题**：hover 效果只有背景色/边框色 0.15s 变化。
- **设计方向建议**：主按钮添加 `translateY(-1px)` + 阴影加深；卡片添加 `box-shadow` 抬升；导航项添加 indicator bar。
- **重复性**：NEW

### B-014｜P2｜工具执行状态切换无过渡
- **位置**：`web/src/views/ChatView.vue:368-412`
- **问题**：工具卡片从“执行中”到“完成/错误”是瞬间状态跳变。
- **设计方向建议**：“执行中”增加 CSS 脉冲点或旋转 spinner；结果出现时使用展开动画；错误状态使用红色左侧边框高亮。
- **重复性**：NEW

### B-015｜P2｜focus 状态缺少可见 ring 动画
- **位置**：`web/src/style.css:57-63`、`web/src/views/ChatView.vue:437-439`
- **问题**：输入框 focus 仅有边框色变化，部分按钮未定义 focus 样式。
- **设计方向建议**：统一使用 `outline: 2px solid var(--color-primary)` 或 `box-shadow` focus-visible ring，并辅以过渡。
- **重复性**：NEW

### B-016｜P2｜聊天界面为传统左右气泡，缺乏构图变化
- **位置**：`web/src/views/ChatView.vue:225-255,344-363`
- **问题**：气泡形状完全一致，无尾巴/箭头、无重叠、无时间轴感。
- **设计方向建议**：助手气泡增加左侧小尾巴或头像占位；用户气泡使用更圆润的右侧大圆角；引入时间戳与分隔线。
- **重复性**：NEW

### B-017｜P2｜缺少非对称、负空间与破格网格
- **位置**：`web/src/views/LoginView.vue:73-92`、`SettingsView.vue:57-65`、`SessionsView.vue:47-55`
- **问题**：所有页面严格居中或等距堆叠，无元素重叠、无破格网格。
- **设计方向建议**：登录页让卡片略微偏离中心并配合大面积背景纹理；设置页使用两栏非对称布局。
- **重复性**：NEW

### B-018｜P2｜全局为纯色平涂，无氛围质感
- **位置**：`web/src/style.css:5-11,31-38`、`web/src/views/LoginView.vue:80`
- **问题**：背景仅使用 `--color-bg` 纯色，无渐变网格、噪点、几何图案、层叠透明。
- **设计方向建议**：登录页/错误页使用 subtle gradient mesh；深色模式叠加 SVG noise 纹理；卡片增加多层阴影。
- **重复性**：NEW

### B-019｜P2｜卡片与表面无层次区分
- **位置**：`web/src/views/ChatView.vue:344-363`、`SessionsView.vue:95-100`、`ToolsView.vue:95-100`
- **问题**：surface 与背景之间仅靠 1px 边框分隔，无阴影、无厚度感。
- **设计方向建议**：引入 elevation token（`--shadow-sm/md/lg`），为卡片、浮动 composer、下拉菜单分层。
- **重复性**：NEW

### B-020｜P2｜favicon 质感未被 UI 吸收
- **位置**：`web/index.html:3-7`、`web/public/favicon.svg`
- **问题**：favicon.svg 是项目中唯一具有视觉质感的资产，但 `index.html` 未链接 favicon，UI 中也未延伸该质感。
- **设计方向建议**：补 favicon 链接；将渐变光斑抽象为登录页背景装饰或品牌 loading 动画。
- **重复性**：与 `REAUDIT_REPORT.md` E-NEW-006 重复

---

## 视角 C — 安全与沙箱

> 重点审查路径逃逸、命令注入、鉴权、用户隔离、ticket 机制。

### C-001｜P0｜ShellTool 子进程继承全部环境变量
- **位置**：`crates/tools/src/shell.rs:140-148`
- **问题**：未调用 `env_clear()`，`printenv FORGECLAW_USERS` 可直接泄漏所有用户 token。
- **修复方向**：`Command::new("sh").env_clear()` 后仅注入 PATH、HOME、LANG 等白名单，显式排除含 token/key 变量。
- **重复性**：`REAUDIT_REPORT.md` C-004

### C-002｜P0｜无真沙箱，仅靠 cwd 限制
- **位置**：`crates/tools/src/shell.rs:140-148`、`crates/tools/src/sandbox.rs:1-88`
- **问题**：仅通过 `current_dir` 限制 cwd；LLM 仍可 `cd /` 后任意读写、发起外联。
- **修复方向**：Linux 引入 `landlock` crate 将文件访问严格限制在 `working_dir`。
- **重复性**：`REAUDIT_REPORT.md` C-005

### C-003｜P0｜危险命令黑名单缺失多项高危命令
- **位置**：`crates/tools/src/shell.rs:38-64`
- **问题**：缺失 `sudo`/`su`/`chown`/`chmod 777`/`nc`/`mkfifo`/`curl | sh`/`cat /etc/passwd` 等。
- **修复方向**：补全黑名单或改用命令白名单。
- **重复性**：`REAUDIT_REPORT.md` C-001

### C-004｜P0｜FileWriteTool TOCTOU 路径逃逸
- **位置**：`crates/tools/src/file.rs:213-241`
- **问题**：先 `is_within` canonicalize 判定，再 `tokio::fs::write` 写入，检查与写入之间存在符号链接替换窗口。
- **修复方向**：canonicalize 后使用规范化绝对路径直接写入；或写入前重新校验并拒绝符号链接。
- **重复性**：`REAUDIT_REPORT.md` C-015

### C-005｜P1｜server 模式 `auto_confirm()` 自动放行 Confirm 级工具
- **位置**：`crates/server/src/orchestrator.rs:537,565`
- **问题**：`default_sandbox_with_specs`/`restricted_sandbox_with_specs` 均使用 `auto_confirm()`，`FileWriteTool` 等失去人工确认保护。
- **修复方向**：server 模式注入真实确认回调（默认拒绝或走 UI 确认）。
- **重复性**：`REAUDIT_REPORT.md` C-017

### C-006｜P1｜`find_by_token` 非常量时间查找
- **位置**：`crates/server/src/auth.rs:99-101`
- **问题**：`HashMap::get` 让 token 存在性产生时序侧信道。
- **修复方向**：遍历全表使用 `subtle` 常量时间比对。
- **重复性**：`REAUDIT_REPORT.md` C-010 / C-020

### C-007｜P1｜`User` Debug 打印明文 token
- **位置**：`crates/server/src/auth.rs:22-29`
- **问题**：`#[derive(Debug)]` 导致 `tracing::debug!(?user)` 泄漏 token。
- **修复方向**：手写 Debug 掩盖 token 或使用 `secrecy::Secret`。
- **重复性**：`REAUDIT_REPORT.md` C-016 / C-019

### C-008｜P1｜新建 session 存在并发竞态
- **位置**：`crates/server/src/api.rs:172-181`、`crates/server/src/ws.rs:172-188`
- **问题**：读锁判空后释放锁，各自创建独立 `history_arc`，并发请求导致 `session.messages` 与 `history` 状态分裂。
- **修复方向**：`sessions.write().entry(session_id).or_insert_with(...)` 原子“取或建”。
- **重复性**：`REAUDIT_REPORT.md` B-NEW-001

### C-009｜P1｜会话写回时未复核 `user_id`
- **位置**：`crates/server/src/api.rs:227-248`、`crates/server/src/ws.rs:249-267`
- **问题**：`get_mut` 后直接 extend，不校验 `d.user_id == user_id`，存在跨用户追加风险。
- **修复方向**：写锁分支中复核 `user_id`，不匹配则 insert 新的 `SessionData`。
- **重复性**：`REAUDIT_REPORT.md` C-NEW-003

### C-010｜P1｜WS 超时后未 abort spawned 任务
- **位置**：`crates/server/src/ws.rs:228-230,277-279`
- **问题**：`tokio::time::timeout(TASK_TIMEOUT, join).await` 超时后 JoinHandle 被 drop，任务 detached 继续占锁。
- **修复方向**：改用 `tokio::select!` 保留 JoinHandle 引用，超时分支显式 `join.abort()`。
- **重复性**：`REAUDIT_REPORT.md` D-NEW-001 / B-003

### C-011｜P1｜WS Error 事件直接透传上游敏感信息
- **位置**：`crates/server/src/ws.rs:271-279,285-291`
- **问题**：`OrchestratorEvent::Error { message }` 直接发给前端，可能包含上游 LLM API URL、状态码。
- **修复方向**：WS 发送通用错误文案，详细错误落 `tracing` 日志。
- **重复性**：`REAUDIT_REPORT.md` D-009

### C-012｜P1｜Windows 下配置文件权限未限制
- **位置**：`crates/cli/src/config.rs:100-114`
- **问题**：`Config::save` 仅在 Unix 设置 0o600，Windows 分支未限制 ACL。
- **修复方向**：Windows 分支使用 `std::os::windows::fs` 或 Win32 API 设置显式 ACL。
- **重复性**：NEW

### C-013｜P2｜`tickets` 表无上限、无过期清理
- **位置**：`crates/server/src/api.rs:70-76`、`crates/server/src/lib.rs:147`
- **问题**：`/api/auth/ticket` 未加限流，已认证用户可循环签发 ticket 造成内存持续增长。
- **修复方向**：签发时 sweep 过期 ticket 或设上限；给 `/api/auth/ticket` 加 GovernorLayer。
- **重复性**：`REAUDIT_REPORT.md` C-NEW-001 / D-NEW-002

### C-014｜P2｜ticket 通过 URL query 传递
- **位置**：`crates/server/src/ws.rs:57-65`
- **问题**：反向代理 access_log 默认记录完整 URI，存在泄露风险。
- **修复方向**：改用 `Sec-WebSocket-Protocol` 子协议头部传递 ticket。
- **重复性**：`REAUDIT_REPORT.md` C-NEW-002

### C-015｜P2｜`tickets.lock().expect(...)` 可能 panic
- **位置**：`crates/server/src/api.rs:72,80`
- **问题**：Mutex poison 时传播 panic，可能导致整个服务崩溃。
- **修复方向**：处理 poison（`into_inner` 恢复）或改用 `parking_lot::Mutex`。
- **重复性**：`REAUDIT_REPORT.md` B-NEW-003

### C-016｜P2｜`is_sensitive_path` 无 Windows 分支
- **位置**：`crates/tools/src/file.rs:41-63`
- **问题**：仅硬编码 Unix 风格前缀，无法识别 `C:\Windows`、`C:\Users\...\.ssh`。
- **修复方向**：增加 `#[cfg(windows)]` 分支覆盖系统目录与用户敏感目录。
- **重复性**：`REAUDIT_REPORT.md` C-018

### C-017｜P2｜`expand_tilde` 使用 `std::env::var("HOME")`
- **位置**：`crates/tools/src/file.rs:70-81`
- **问题**：进程环境被污染时 `~` 可展开到任意路径。
- **修复方向**：使用跨平台 `dirs::home_dir()` 并校验结果。
- **重复性**：`REAUDIT_REPORT.md` C-021

### C-018｜P2｜`FileWriteTool::check` 未检查 `is_within`
- **位置**：`crates/tools/src/file.rs:204-211`
- **问题**：`check` 仅检查 `is_sensitive_path`，未检查 `is_within`，对非敏感但越界路径返回 `Confirm`，与 `execute` 拦截行为不一致。
- **修复方向**：`check` 同步调用 `is_within`，越界路径直接返回 `Critical`。
- **重复性**：NEW

### C-019｜P2｜`eval`/`exec` 过度拦截
- **位置**：`crates/tools/src/shell.rs:53-54`
- **问题**：`\beval\b`/`\bexec\b` 会阻断合法命令如 `exec cargo run`、`eval $(ssh-agent)`。
- **修复方向**：收窄为变量展开形式（`eval\s+\$`、`exec\s+\$`）或采用白名单。
- **重复性**：`REAUDIT_REPORT.md` A-NEW-001

### C-020｜P2｜system prompt 注入条件基于 `history.is_empty()`
- **位置**：`crates/server/src/orchestrator.rs:160-164`
- **问题**：非空但首条不是 system 的 history 不再注入 system prompt。
- **修复方向**：检查 `history.messages().first().role == System`。
- **重复性**：`REAUDIT_REPORT.md` B-014

---

## 视角 D — 并发与性能

> 重点审查锁粒度、竞态、资源泄漏、缓存、阻塞 I/O。

### D-001｜P1｜`history_arc.write()` 在 `run_once` 期间全程持有
- **位置**：`crates/server/src/api.rs:183-191`
- **问题**：同 session 并发请求被串行化，只读查询也被阻塞。
- **修复方向**：采用“读锁快照 → 释放锁跑 LLM → 写锁提交”；或 History 内部改为 `Arc<[ChatMessage]>`。
- **重复性**：NEW

### D-002｜P1｜WS spawned 任务超时后未 abort
- **位置**：`crates/server/src/ws.rs:197-200,230,277-279`
- **问题**：`tokio::time::timeout(TASK_TIMEOUT, join)` 超时后 JoinHandle 被 drop 但未 abort，任务 detached 继续占锁。
- **修复方向**：`tokio::select!` 保留 JoinHandle 引用，超时分支显式 abort。
- **重复性**：`REAUDIT_REPORT.md` B-003 / D-NEW-001

### D-003｜P1｜SESSION_TIMEOUT 触发后未 abort 运行中任务
- **位置**：`crates/server/src/ws.rs:109-136,230`
- **问题**：主循环退出，但 spawn 任务继续在后台持锁、烧 token。
- **修复方向**：`handle_ws` 退出前显式 abort 所有未完成 LLM JoinHandle。
- **重复性**：`REAUDIT_REPORT.md` B-003 / D-NEW-001

### D-004｜P1｜新建 session 三段式创建导致状态分裂
- **位置**：`crates/server/src/api.rs:172-181,227-248`
- **问题**：两个并发请求用同一未存在 `session_id` 会得到两个独立 History。
- **修复方向**：`sessions.write().entry(session_id).or_insert_with(...)` 原子“取或建”。
- **重复性**：`REAUDIT_REPORT.md` B-NEW-001

### D-005｜P2｜`tickets` HashMap 无容量上限、无过期清理
- **位置**：`crates/server/src/api.rs:42,71-89`
- **问题**：未消费或过期 ticket 永久驻留内存，形成慢速泄漏。
- **修复方向**：签发时 sweep 过期项，或设置总上限/LRU。
- **重复性**：`REAUDIT_REPORT.md` B-NEW-002 / D-NEW-002 / C-NEW-001

### D-006｜P1｜`tickets.lock().expect(...)` 可能 panic
- **位置**：`crates/server/src/api.rs:73,80`
- **问题**：Mutex poison 时 handler 直接 panic。
- **修复方向**：处理 poison 或改用 `parking_lot::Mutex`。
- **重复性**：`REAUDIT_REPORT.md` B-NEW-003

### D-007｜P1｜`PromptEngine` 整段持 `tokio::Mutex` 且含同步文件 IO
- **位置**：`crates/server/src/orchestrator.rs:80,120-125,128-132,148-153` + `crates/core/src/prompt/profile.rs:56-58` + `crates/core/src/prompt/section.rs:123-126`
- **问题**：慢盘下阻塞所有并发 prompt 编译请求。
- **修复方向**：cache 拆为 `RwLock`/`ArcSwap`；文件读取放锁外或改 `tokio::fs`。
- **重复性**：`REAUDIT_REPORT.md` B-004 / D-019 / B-009

### D-008｜P2｜prompt cache 无上限、无淘汰策略
- **位置**：`crates/core/src/prompt/engine.rs:26,101-122`
- **问题**：长期运行内存无限增长，存在 OOM 风险。
- **修复方向**：引入 LRU 或容量上限。
- **重复性**：`REAUDIT_REPORT.md` B-007

### D-009｜P2｜每轮深拷贝 History 与 tool_specs
- **位置**：`crates/server/src/orchestrator.rs:254,276`
- **问题**：长会话下 `history.clone()` 与 `tool_specs.to_vec()` 开销随历史长度线性增长。
- **修复方向**：History 内部改用 `Arc<[ChatMessage]>`；`tool_specs` 改为 `Arc<[ToolSpec]>`。
- **重复性**：`tool_specs` 重复 `REAUDIT_REPORT.md` B-008 / D-018；`history.clone()` 为 NEW

### D-010｜P2｜`GrepTool` 全文件读入内存
- **位置**：`crates/tools/src/search.rs:185-212`
- **问题**：单文件过大时一次性分配大量内存并长时间占用 blocking 线程。
- **修复方向**：使用 buffered reader / `memmap2` / 异步流式按行读取，限制单文件最大字节。
- **重复性**：NEW

### D-011｜P2｜每次搜索调用都 spawn_blocking 创建新任务
- **位置**：`crates/tools/src/search.rs:89-108,185-212`
- **问题**：高并发工具调用下线程池抖动、上下文切换开销明显。
- **修复方向**：使用有界线程池或 `tokio::fs` + 异步目录遍历。
- **重复性**：NEW

### D-012｜P2｜`spawn_blocking(...).await.unwrap_or_default()` 静默吞 JoinError
- **位置**：`crates/tools/src/search.rs:108,212`
- **问题**：子任务 panic 时返回空 Vec 且无 warn 日志。
- **修复方向**：`unwrap_or_else` 中 `tracing::warn!` 并返回空 Vec。
- **重复性**：`REAUDIT_REPORT.md` A-NEW-002

### D-013｜P2｜`Session.messages` 为 pub Vec
- **位置**：`crates/core/src/model.rs:17`
- **问题**：外部代码可直接 mutate，破坏并发安全假设。
- **修复方向**：改为私有字段，仅暴露 `append`/`extend` 访问器。
- **重复性**：`REAUDIT_REPORT.md` A-002

### D-014｜P2｜`ChatRequest::from_history` 每次深拷贝所有消息
- **位置**：`crates/llm/src/lib.rs:158`
- **问题**：每次请求都复制完整消息历史。
- **修复方向**：History 内部持有 `Arc<[ChatMessage]>`，或 `ChatRequest` 借用切片。
- **重复性**：NEW

### D-015｜P1｜写回 `session.messages` 未复核 `user_id`
- **位置**：`crates/server/src/api.rs:227-248`、`crates/server/src/ws.rs:249-268`
- **问题**：可能把当前用户消息追加到另一用户的展示副本。
- **修复方向**：写锁分支内校验 `d.user_id == user_id`。
- **重复性**：`REAUDIT_REPORT.md` C-NEW-003

---

## 视角 E — API / WebSocket / 编排器协议

> 重点审查生命周期、错误传播、协议一致性、消息循环、工具调度。

### E-001｜P0｜本地 cargo check/test 因缺少 `web/dist` 全部失败
- **位置**：`crates/server/src/lib.rs:46-48` + `web/.gitignore`
- **问题**：`#[derive(RustEmbed)] #[folder = "../../web/dist"]` 编译期读取 `web/dist`，该目录被 gitignore，新克隆未先 `pnpm build` 时构建失败。
- **修复方向**：`web/dist/.gitkeep` 占位 + `.gitignore` 调整，或 `crates/server/build.rs` 兜底创建空目录。
- **重复性**：`REAUDIT_REPORT.md` F-001 / F-NEW-001

### E-002｜P1｜WS 单轮事件转发无超时，首事件前挂起无法覆盖
- **位置**：`crates/server/src/ws.rs:197-230`
- **问题**：spawn 任务先获取 `history.write()` 再进入 LLM 调用；若 LLM 在产生任何事件前挂起，`rx.recv().await` 永远阻塞。
- **修复方向**：用 `tokio::select!` 同时等待 `rx.recv()`、`join`、per-turn 定时器；超时或断连时 abort 并发送 Error 帧。
- **重复性**：NEW

### E-003｜P1｜WS 错误/超时/panic 不向客户端发送终端 Error 帧
- **位置**：`crates/server/src/ws.rs:271-279`
- **问题**：仅记录 tracing error，不向 WS 客户端发送 `OrchestratorEvent::Error` 终止帧；REST 在同样场景返回 500，协议不一致。
- **修复方向**：在上述分支内向 `out_tx` 发送 Error 帧后再返回。
- **重复性**：NEW

### E-004｜P1｜WS 对非法 `session_id` 静默新建会话
- **位置**：`crates/server/src/ws.rs:164-168`
- **问题**：`Uuid::parse_str` 失败时直接 `Uuid::new_v4()` 创建新会话，而 REST 对非法 `session_id` 返回 400。
- **修复方向**：解析失败时向客户端发送 Error 帧并 `Continue`，不新建会话。
- **重复性**：NEW

### E-005｜P1｜新建 session 并发竞态
- **位置**：`crates/server/src/ws.rs:172-188`、`crates/server/src/api.rs:172-181`
- **问题**：同 D-004。
- **修复方向**：`sessions.write().entry(session_id).or_insert_with(...)`。
- **重复性**：`REAUDIT_REPORT.md` B-NEW-001

### E-006｜P1｜会话写回时未复核 `user_id`
- **位置**：`crates/server/src/ws.rs:249-268`、`crates/server/src/api.rs:227-248`
- **问题**：同 C-009 / D-015。
- **修复方向**：写锁分支校验 `user_id`。
- **重复性**：`REAUDIT_REPORT.md` C-NEW-003

### E-007｜P1｜WS 断连/超时/会话超时不中止 spawned 任务
- **位置**：`crates/server/src/ws.rs:109-136`、`230`、`277-279`
- **问题**：同 D-002 / D-003 / C-010。
- **修复方向**：保留 JoinHandle 可变引用，超时/断连时 abort。
- **重复性**：`REAUDIT_REPORT.md` D-NEW-001 / B-003

### E-008｜P1｜WS Error 事件直接透传上游 LLM 原始错误信息
- **位置**：`crates/server/src/ws.rs:271-279,285-291`
- **问题**：同 C-011。
- **修复方向**：WS 发送通用错误文案，原始细节落 tracing。
- **重复性**：`REAUDIT_REPORT.md` D-009

### E-009｜P1｜编排器事件缺少 `call_id`，前端按工具名回填
- **位置**：`crates/server/src/orchestrator.rs:53-55,374-377,420-423`
- **问题**：`ToolCallStart` / `ToolResult` 只有 `name` 无 `id`，同名工具并发/乱序时错配。
- **修复方向**：事件中增加 `call_id`（使用 `agg.id`），并保证 `Complete.tool_calls` 与事件 id 一致。
- **重复性**：`REAUDIT_REPORT.md` E-NEW-001

### E-010｜P1｜`tickets` 表无上限、无过期清理，Mutex poison 会 panic
- **位置**：`crates/server/src/api.rs:42,71-89`
- **问题**：同 D-005 / D-006 / C-013 / C-015。
- **修复方向**：sweep 过期项、处理 poison 或换 `parking_lot::Mutex`。
- **重复性**：`REAUDIT_REPORT.md` B-NEW-002 / B-NEW-003 / D-NEW-002 / C-NEW-001

### E-011｜P1｜`PromptEngine` 整段持 `tokio::Mutex` 且含同步文件 IO
- **位置**：`crates/server/src/orchestrator.rs:80,120-125,148-153`
- **问题**：同 D-007。
- **修复方向**：文件读取改 `tokio::fs`，cache 拆 `RwLock`。
- **重复性**：`REAUDIT_REPORT.md` B-004 / D-019

### E-012｜P2｜`/api/auth/ticket` 未加限流
- **位置**：`crates/server/src/api.rs:147`、`crates/server/src/lib.rs:151-158`
- **问题**：仅 `/api/auth/login` 套了 GovernorLayer，`/api/auth/ticket` 对已认证用户无限流。
- **修复方向**：给 `/api/auth/ticket` 加 per-user / per-IP 限流。
- **重复性**：`REAUDIT_REPORT.md` C-NEW-001

### E-013｜P2｜REST 500 错误体与鉴权 401 JSON 体不一致
- **位置**：`crates/server/src/api.rs:92-99,195-209`
- **问题**：`internal_error` 返回纯文本，auth 返回 JSON，客户端解析困难。
- **修复方向**：统一返回 JSON 错误体 `{"error":"internal server error"}`。
- **重复性**：NEW

### E-014｜P2｜`tool_specs.to_vec()` 每轮深拷贝
- **位置**：`crates/server/src/orchestrator.rs:276`
- **问题**：同 D-009。
- **修复方向**：`ChatRequest.tools` 改为 `Option<Arc<[ToolSpec]>>`。
- **重复性**：`REAUDIT_REPORT.md` B-008 / D-018

### E-015｜P2｜system prompt 注入条件基于 `history.is_empty()`
- **位置**：`crates/server/src/orchestrator.rs:161,184`
- **问题**：同 C-020 / D... 不重复。
- **修复方向**：检查 `messages().first().role == System`。
- **重复性**：`REAUDIT_REPORT.md` B-014

### E-016｜P2｜`session.messages` 与 History 双源存储
- **位置**：`crates/server/src/api.rs:220-248`、`crates/server/src/ws.rs:242-268`
- **问题**：成功路径只把 User/Assistant 写入 `session.messages`，tool 消息仅存于 History；GET 会话拿不到完整上下文。
- **修复方向**：单一真源，或把 tool 消息也 append 到 `session.messages`。
- **重复性**：`REAUDIT_REPORT.md` B-010

### E-017｜P2｜ticket 通过 URL query 传递
- **位置**：`crates/server/src/ws.rs:59-62`
- **问题**：同 C-014。
- **修复方向**：改用 `Sec-WebSocket-Protocol` 子协议传 ticket。
- **重复性**：`REAUDIT_REPORT.md` C-NEW-002

### E-018｜P2｜会话存储纯内存，进程重启丢失
- **位置**：`crates/server/src/api.rs:37`
- **问题**：`sessions: Arc<RwLock<HashMap>>` 无持久化。
- **修复方向**：按需加 SQLite / Redis / 文件快照。
- **重复性**：`REAUDIT_REPORT.md` D-016

### E-019｜P1｜被删 orchestrator 测试导致关键路径无人守护
- **位置**：`crates/server/tests/orchestrator_test.rs`
- **问题**：`be8ac66` 删除的测试中，两个覆盖路径当前无替代守护。
- **修复方向**：按当前契约重写 `run_streaming_stops_when_receiver_dropped` 与 `run_once_invalid_tool_arguments_returns_tool_result_error`。
- **重复性**：`REAUDIT_REPORT.md` 合并回归小节 / D-006 / D-012

### E-020｜P2｜缺少 WS 真实消息流集成测试
- **位置**：`crates/server/tests/auth_test.rs:332-377`
- **问题**：仅验证升级前 401/426，未跑完整 chat / Complete / Error 帧序列。
- **修复方向**：增加 WS 端到端测试（MockClient 驱动）。
- **重复性**：`REAUDIT_REPORT.md` D-015

### E-021｜P2｜缺少 REST 端点测试：非法 session_id 400、跨用户 chat 404
- **位置**：`crates/server/tests/api_test.rs`
- **问题**：未覆盖 `session_id` 非法 400 与跨用户 chat 404。
- **修复方向**：补充对应测试用例。
- **重复性**：`REAUDIT_REPORT.md` D-014

### E-022｜P2｜缺少并发新建 session 与写回 `user_id` 竞态测试
- **位置**：`crates/server/tests/api_test.rs`、`crates/server/tests/auth_test.rs`
- **问题**：现有并发测试预先插入 session，无法发现 B-NEW-001 / C-NEW-003 竞态。
- **修复方向**：两个并发请求对同一 `session_id` 发 chat，校验 history 与 messages 一致且按用户隔离。
- **重复性**：NEW

### E-023｜P2｜WS 单连接未限制并发 frame 处理任务数
- **位置**：`crates/server/src/ws.rs:114-119`
- **问题**：每收到一帧文本就 spawn 一个任务，恶意/异常客户端可在一连接内发送大量帧制造任务堆积。
- **修复方向**：对单连接正在处理的 frame 数加 `tokio::sync::Semaphore` 限制。
- **重复性**：NEW

### E-024｜P2｜`TimeoutLayer` 全局 300s 仍偏长
- **位置**：`crates/server/src/lib.rs:168-171`
- **问题**：单 REST 请求 300s 上限仍可能超过建议 120s。
- **修复方向**：REST 降至 120s。
- **重复性**：NEW

### E-025｜P2｜`Event::Error` 上游信息可能通过 tracing warn 泄漏
- **位置**：`crates/server/src/orchestrator.rs:314`
- **问题**：`warn!(%message, "llm stream error event")` 可能把上游 API URL/状态码写入日志。
- **修复方向**：对 message 脱敏或仅记录固定字段。
- **重复性**：NEW

### E-026｜P2｜CORS layer 静默忽略非法 origin 配置
- **位置**：`crates/server/src/lib.rs:62-72`
- **问题**：`filter_map(|o| o.parse().ok())` 丢弃非法 origin，若全部非法则所有跨域请求失败，运维难定位。
- **修复方向**：启动时校验 `allowed_origins` 并 warn/error。
- **重复性**：NEW

---

## 视角 F — 构建、CI/CD 与可维护性

> 重点审查配置漂移、依赖版本、CI 安全、测试覆盖、构建可复现性。

### F-001｜P0｜本地 cargo check/test 因缺少 `web/dist` 全部失败
- **位置**：`crates/server/src/lib.rs:46-48`、`web/.gitignore:11,26`
- **问题**：同 E-001。
- **修复方向**：`web/dist/.gitkeep` + `.gitignore` 调整，或 `build.rs` 兜底。
- **重复性**：`REAUDIT_REPORT.md` F-001 / F-NEW-001

### F-002｜P1｜`release.yml` 顶层权限过大
- **位置**：`.github/workflows/release.yml:8-9`
- **问题**：`permissions: contents: write` 写在工作流顶层，未下放到 job 级，违反最小权限原则。
- **修复方向**：删除顶层 `permissions`，在 `jobs.build` 内声明 `permissions: contents: write`。
- **重复性**：`REAUDIT_REPORT.md` F-006 / F-NEW-002

### F-003｜P1｜release 矩阵每个 job 都执行 `action-gh-release`，存在上传竞态
- **位置**：`.github/workflows/release.yml:110-116`
- **问题**：6 目标矩阵每个 job 都向同一 Release 上传产物，可能覆盖 release 元数据或丢失产物。
- **修复方向**：改为单 job（或 `needs` 汇总 job）先 `download-artifact` 收集全部产物，再统一上传。
- **重复性**：NEW

### F-004｜P1｜本地无 `web/dist` 时 CI 失败提示不足
- **位置**：`.github/workflows/ci.yml`、`crates/server/src/lib.rs:167-179`
- **问题**：`cargo clippy/test --workspace` 实际依赖 frontend artifact；本地缺少时无明确提示，README 也未说明需先 `pnpm build`。
- **修复方向**：文档声明本地构建需先 `pnpm --dir web build`；或在 `crates/server/build.rs` 自动处理缺失目录。
- **重复性**：NEW（与 F-001 同源）

### F-005｜P1｜被删 orchestrator 测试导致关键路径无人守护
- **位置**：`crates/server/tests/orchestrator_test.rs`
- **问题**：同 E-019。
- **修复方向**：重写 `run_streaming_stops_when_receiver_dropped` 与 `run_once_invalid_tool_arguments_returns_tool_result_error`。
- **重复性**：`REAUDIT_REPORT.md` 合并回归小节

### F-006｜P2｜根 `Cargo.toml` `tokio` feature 为 `full`
- **位置**：`Cargo.toml:26`
- **问题**：所有 crate 通过 workspace 引用 `full`，无法按 crate 裁剪 feature，二进制膨胀。
- **修复方向**：根 `Cargo.toml` 仅声明 `tokio = "1.52"`，各 crate 按需声明 feature。
- **重复性**：`REAUDIT_REPORT.md` F-007

### F-007｜P2｜Windows job 产物列表包含不存在的 `.tar.gz`
- **位置**：`.github/workflows/release.yml:110-116`
- **问题**：Windows 只产出 `.zip`，但 `files` 列表包含 `.tar.gz` 及其 sha256。
- **修复方向**：拆分为条件 step 或按 OS 区分 `files`。
- **重复性**：`REAUDIT_REPORT.md` F-008 / F-NEW-003

### F-008｜P2｜`web/package.json` build 与 typecheck 重复执行类型检查
- **位置**：`web/package.json:8,10`
- **问题**：`"build": "vue-tsc -b && vite build"` 与 `"typecheck": "vue-tsc --noEmit"` 重复，CI 浪费时间。
- **修复方向**：`"build": "vite build"`，`typecheck` 保留。
- **重复性**：`REAUDIT_REPORT.md` F-010 / F-NEW-004

### F-009｜P2｜`rust-cache` key 不含 OS
- **位置**：`.github/workflows/release.yml:78`
- **问题**：`key: ${{ matrix.target }}` 不含 OS，未来矩阵扩展时可能冲突。
- **修复方向**：`key: ${{ matrix.os }}-${{ matrix.target }}`。
- **重复性**：`REAUDIT_REPORT.md` F-011 / F-NEW-005

### F-010｜P2｜`tsconfig.app.json` 未显式声明 `strict: true`
- **位置**：`web/tsconfig.app.json:1-17`
- **问题**：依赖 extends 与 TS 默认行为，配置可读性不足。
- **修复方向**：在 `compilerOptions` 中显式添加 `"strict": true`。
- **重复性**：`REAUDIT_REPORT.md` E-019

### F-011｜P2｜`vite.config.ts` 未显式设置 `build.outDir`
- **位置**：`web/vite.config.ts:6-25`
- **问题**：依赖默认 `dist`，缺少显式约定，配置漂移风险高。
- **修复方向**：添加 `build: { outDir: 'dist' }`。
- **重复性**：`REAUDIT_REPORT.md` E-023

### F-012｜P2｜`tower-http` 双版本
- **位置**：`Cargo.lock`
- **问题**：`tower-http` 同时被解析为 `0.7.0`（workspace 直接依赖）与 `0.6.11`（传递依赖），增加编译时间与二进制体积。
- **修复方向**：升级 `tower_governor` 或显式统一 `tower-http` 版本。
- **重复性**：NEW

### F-013｜P2｜两套 TS 配置 linting 策略不一致
- **位置**：`web/tsconfig.app.json:11-16`、`web/tsconfig.node.json:16-20`
- **问题**：`tsconfig.node.json` 开启 `verbatimModuleSyntax`，`tsconfig.app.json` 未开启。
- **修复方向**：统一配置或抽取公共 linting 配置。
- **重复性**：NEW

### F-014｜P2｜关键第三方 actions 未固定到 commit SHA
- **位置**：`.github/workflows/ci.yml:68,73`、`.github/workflows/release.yml:71,76,82,111`
- **问题**：`dtolnay/rust-toolchain@stable`、`taiki-e/install-action@v2`、`softprops/action-gh-release@v3` 等使用浮动 tag，存在 supply-chain 漂移风险。
- **修复方向**：固定到具体 commit SHA，并配置 Dependabot 定期更新。
- **重复性**：NEW

### F-015｜P2｜缺少 WS 真实消息流集成测试
- **位置**：`crates/server/tests/auth_test.rs:332-377`
- **问题**：同 E-020。
- **修复方向**：引入 `tokio-tungstenite` 编写端到端 WS 消息流测试。
- **重复性**：`REAUDIT_REPORT.md` D-015

---

## 与 `REAUDIT_REPORT.md` 重复项对照表

| 本报告条目 | REAUDIT 条目 | 问题简述 |
|-----------|-------------|---------|
| A 重复项 | 见视角 A 末尾表格 | 18 条 |
| B-020 | E-NEW-006 | favicon 未链接 |
| C-001 | C-004 | ShellTool 环境变量泄漏 |
| C-002 | C-005 | 无真沙箱 |
| C-003 | C-001 | 黑名单不全 |
| C-004 | C-015 | FileWriteTool TOCTOU |
| C-005 | C-017 | auto_confirm |
| C-006 | C-010 / C-020 | find_by_token 非常量时间 |
| C-007 | C-016 / C-019 | User Debug 打印 token |
| C-008 | B-NEW-001 | 新建 session 竞态 |
| C-009 | C-NEW-003 | 写回未复核 user_id |
| C-010 | D-NEW-001 / B-003 | WS 超时未 abort |
| C-011 | D-009 | WS Error 脱敏 |
| C-013 | C-NEW-001 / D-NEW-002 | tickets 无清理 |
| C-014 | C-NEW-002 | ticket query 传递 |
| C-015 | B-NEW-003 | tickets poison panic |
| C-016 | C-018 | Windows 敏感路径 |
| C-017 | C-021 | expand_tilde 用 HOME |
| C-019 | A-NEW-001 | eval/exec 过度拦截 |
| C-020 | B-014 | system prompt 注入条件 |
| D-002 | B-003 / D-NEW-001 | WS 超时未 abort |
| D-003 | B-003 / D-NEW-001 | SESSION_TIMEOUT 未 abort |
| D-004 | B-NEW-001 | 新建 session 竞态 |
| D-005 | B-NEW-002 / D-NEW-002 / C-NEW-001 | tickets 无清理 |
| D-006 | B-NEW-003 | tickets poison panic |
| D-007 | B-004 / D-019 / B-009 | PromptEngine 同步 IO |
| D-008 | B-007 | cache 无上限 |
| D-009 (tool_specs) | B-008 / D-018 | tool_specs.to_vec |
| D-012 | A-NEW-002 | spawn_blocking 吞 JoinError |
| D-013 | A-002 | Session.messages pub |
| D-015 | C-NEW-003 | 写回未复核 user_id |
| E-001 | F-001 / F-NEW-001 | web/dist 缺失 |
| E-005 | B-NEW-001 | 新建 session 竞态 |
| E-006 | C-NEW-003 | 写回未复核 user_id |
| E-007 | D-NEW-001 / B-003 | WS 未 abort |
| E-008 | D-009 | WS Error 脱敏 |
| E-009 | E-NEW-001 | 缺少 call_id |
| E-010 | B-NEW-002 / B-NEW-003 / D-NEW-002 / C-NEW-001 | tickets 清理/poison |
| E-011 | B-004 / D-019 | PromptEngine 同步 IO |
| E-014 | B-008 / D-018 | tool_specs.to_vec |
| E-015 | B-014 | system prompt 注入条件 |
| E-016 | B-010 | session.messages 双源 |
| E-017 | C-NEW-002 | ticket query 传递 |
| E-018 | D-016 | 会话无持久化 |
| E-019 | 合并回归 / D-006 / D-012 | 被删测试 |
| E-020 | D-015 | 缺 WS 集成测试 |
| E-021 | D-014 | 缺 REST 测试 |
| F-001 | F-001 / F-NEW-001 | web/dist 缺失 |
| F-002 | F-006 / F-NEW-002 | release 顶层权限 |
| F-005 | 合并回归 | 被删测试 |
| F-006 | F-007 | tokio full feature |
| F-007 | F-008 / F-NEW-003 | Windows 产物列表 |
| F-008 | F-010 / F-NEW-004 | build 重复 typecheck |
| F-009 | F-011 / F-NEW-005 | rust-cache key |
| F-010 | E-019 | tsconfig strict |
| F-011 | E-023 | vite outDir |
| F-015 | D-015 | 缺 WS 集成测试 |

---

## 推荐修复顺序

### 阶段 1：阻断性 P0（与 REAUDIT 一致，仍需优先处理）
1. **F-001 / E-001**：解决 `web/dist` 缺失，让本地 `cargo check/test` 通过。
2. **C-001**：ShellTool `env_clear()`，防止环境变量泄漏。
3. **C-003**：补全危险命令黑名单。
4. **C-004**：消除 FileWriteTool TOCTOU。
5. **C-002**：引入 landlock 真沙箱（可拆独立变更）。

### 阶段 2：本次新增高价值 P1
6. **A-001 / A-002**：CLI confirm 模式抽象为可复用、可测试的 `AsyncConfirmer`。
7. **A-003**：`Role::from(&str)` 改为显式 `Option/Result`。
8. **B-001 / B-002 / B-003 / B-004**：前端设计系统基础（字体、助手渲染、响应式）。
9. **E-002 / E-003 / E-004**：WS 协议一致性（事件循环超时、Error 帧、非法 session_id）。
10. **E-009**：编排器事件增加 `call_id`。
11. **F-003**：release 产物上传竞态。

### 阶段 3：并发/协议残留 P1（与 REAUDIT 重复但核心）
12. **D-004 / E-005 / C-008**：新建 session 原子“取或建”。
13. **D-015 / E-006 / C-009**：写回时复核 `user_id`。
14. **D-002 / D-003 / E-007 / C-010**：WS 超时/断连显式 abort。
15. **D-007 / E-011**：PromptEngine 拆锁、异步 IO。

### 阶段 4：P2 长尾（可滚动迭代）
16. 前端审美：page-load 动效、hover 反馈、工具状态过渡、背景质感、语义色 tokens（B-005 ~ B-019）。
17. Karpathy 简洁性：删除不可能分支、复用 `spec_for`、统一 `dirs::home_dir`、修复 JSON.parse 异常（A-005 ~ A-015）。
18. 性能：`history.clone()`、`GrepTool` 大文件读入、`spawn_blocking` 抖动（D-009 ~ D-014）。
19. 测试覆盖：WS 消息流、并发 session、跨用户写回（E-019 ~ E-022）。
20. CI/CD：tokio feature 裁剪、action SHA 固定、rust-cache key、tsconfig strict、vite outDir（F-006 ~ F-014）。

---

*报告结束。本次审计未修改任何源码。*
