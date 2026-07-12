# Tasks

> 本 tasks.md 描述**补充审计阶段**的工作项。审计完成后产出的**修复任务**将作为新增条目追加到本文件末尾，由后续变更承接实际修复。
> 审计基线：当前 main HEAD `31c6db0`；对照基线：`REAUDIT_REPORT.md`（change-id: `re-audit-after-fixes`）。

## 审计阶段任务

- [x] Task 1: 视角 A — Karpathy 代码质量审计（简洁性、手术式改动、可验证目标）
  - 范围：全部 Rust crate + Web 前端
  - 重点关注：过度抽象、单次使用 helper、未请求的灵活性、无法验证的目标、顺手改进无关代码
  - 交付：结构化发现清单（位置、P0/P1/P2、描述、修复方向、是否与 REAUDIT 重复）

- [x] Task 2: 视角 B — 前端设计审美审计（设计系统、排版、色彩、动效、空间构图、UX）
  - 范围：`web/src/` 全部源码 + `web/index.html` + `web/style.css` + `web/package.json`
  - 重点关注：同质化 AI 审美、默认字体、无意义动画、布局平庸、缺乏氛围
  - 交付：结构化发现清单（位置、P0/P1/P2、描述、设计方向建议、是否与 REAUDIT 重复）

- [x] Task 3: 视角 C — 安全与沙箱审计
  - 范围：`crates/tools/src/{file,sandbox,shell}.rs`、`crates/server/src/{auth,api,ws,orchestrator}.rs`
  - 重点关注：路径逃逸、命令注入、鉴权绕过、ticket 安全、用户隔离
  - 交付：结构化发现清单（位置、P0/P1/P2、描述、修复方向、是否与 REAUDIT 重复）

- [x] Task 4: 视角 D — 并发与性能审计
  - 范围：`crates/core/src/prompt/`、`crates/server/src/{api,ws,orchestrator}.rs`、`crates/tools/src/search.rs`
  - 重点关注：竞态条件、资源泄漏、缓存策略、阻塞 I/O、锁粒度
  - 交付：结构化发现清单（位置、P0/P1/P2、描述、修复方向、是否与 REAUDIT 重复）

- [x] Task 5: 视角 E — API / WebSocket / 编排器协议审计
  - 范围：`crates/server/src/{api,ws,orchestrator,lib}.rs`、`crates/server/tests/`
  - 重点关注：生命周期管理、错误传播、协议一致性、消息循环、工具调度
  - 交付：结构化发现清单（位置、P0/P1/P2、描述、修复方向、是否与 REAUDIT 重复）

- [x] Task 6: 视角 F — 构建、CI/CD 与可维护性审计
  - 范围：`.github/workflows/`、`Cargo.toml`、`crates/*/Cargo.toml`、`web/package.json`、`web/vite.config.ts`
  - 重点关注：配置漂移、依赖版本、CI 冗余、测试覆盖、构建可复现性
  - 交付：结构化发现清单（位置、P0/P1/P2、描述、修复方向、是否与 REAUDIT 重复）

- [x] Task 7: 汇总各视角发现，生成 `FRESH_AUDIT_REPORT.md`
  - 去重、合并、按 P0/P1/P2 排序
  - 标注与 `REAUDIT_REPORT.md` 的重复项
  - 含摘要、按视角分节、重复项对照表、推荐修复顺序

- [x] Task 8: 生成优先级修复任务清单（追加到本 tasks.md 末尾）
  - 基于 `FRESH_AUDIT_REPORT.md` 生成
  - 每条标注：报告条目编号、修复目标、受影响文件、是否可并行、REAUDIT 重复引用

## Task Dependencies

- Task 1 / 2 / 3 / 4 / 5 / 6：互不依赖，**全部并行**
- Task 7 依赖 Task 1-6 全部完成
- Task 8 依赖 Task 7 完成

## 修复阶段任务

> 基于 `FRESH_AUDIT_REPORT.md` 生成。每条标注：报告条目编号、修复目标（verifiable）、受影响文件、是否可并行、是否与 `REAUDIT_REPORT.md` 重复。
> **本审计变更不执行修复**，下方任务由后续变更承接。

### 阶段 1：阻断性 P0（5 条，最高优先）

- [x] Task 9: [F-001 / E-001] [P0] 本地 `cargo check/test` 因 `web/dist` 缺失失败
  - 修复目标：干净克隆后无需先 `pnpm build` 即可通过 `cargo check -p forgeclaw-server` 与 `cargo test --workspace`。方案择一：① `web/dist/.gitkeep` 占位 + `web/.gitignore` 调整 `dist/*` + `!dist/.gitkeep`；② `crates/server/build.rs` 在 dist 不存在时创建空目录。新增 CI 步骤验证干净环境构建。
  - 受影响：`web/.gitignore`、`web/dist/.gitkeep`（新增）或 `crates/server/build.rs`、`crates/server/src/lib.rs:46-48`
  - 可并行：是（建议最先做，解除后续所有编译验证阻塞）
  - REAUDIT 重复：`REAUDIT_REPORT.md` F-001 / F-NEW-001

- [x] Task 10: [C-001] [P0] ShellTool 子进程继承全部环境变量
  - 修复目标：`crates/tools/src/shell.rs` 中 `Command::new("sh")` 调用 `.env_clear()`，仅注入白名单（`PATH`、`HOME`、`LANG`、`TERM` 等），显式排除含 `TOKEN`/`KEY`/`SECRET` 字样的变量；新增测试验证 `printenv FORGECLAW_USERS` 与 `printenv DEEPSEEK_API_KEY` 返回空，`printenv PATH` 非空。
  - 受影响：`crates/tools/src/shell.rs`
  - 可并行：是（与 Task 11/12/13 独立）
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-004

- [x] Task 11: [C-003] [P0] ShellTool 危险命令黑名单缺失
  - 修复目标：`crates/tools/src/shell.rs:38-64` 黑名单补全 `sudo`/`su`/`chown`/`chmod 777`/`cat /etc/passwd`/`cat ~/.ssh`/`env`/`mv`/`cp` 写 `/etc/`/`bash -i >& /dev/tcp`/`nc`/`mkfifo`/`curl|sh`/`wget|bash` 等；或改用命令白名单；新增测试覆盖每个新增绕过用例均被拦截。
  - 受影响：`crates/tools/src/shell.rs`
  - 可并行：是（与 Task 10 同文件，建议合并到同一变更）
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-001

- [x] Task 12: [C-004] [P0] FileWriteTool TOCTOU 路径逃逸
  - 修复目标：`crates/tools/src/file.rs:213-241` 消除 canonicalize 与 `tokio::fs::write` 之间的符号链接替换窗口：canonicalize 后使用规范化绝对路径直接写入（不再 re-resolve），或在写锁内完成检查+写入；新增测试验证符号链接替换逃逸被拒。
  - 受影响：`crates/tools/src/file.rs`
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-015

- [x] Task 13: [C-002] [P0] 引入 `landlock` 真沙箱（Linux）
  - 修复目标：`crates/tools/src/sandbox.rs` 在 Linux 下用 `landlock` crate 限制 FS 访问至 `working_dir`；其他平台文档明示限制并降级到 cwd 检查；新增集成测试验证 `cd / && touch /tmp/outside` 被拒。
  - 受影响：`crates/tools/Cargo.toml`、`crates/tools/src/sandbox.rs`、`crates/tools/src/shell.rs`
  - 可并行：是（长期任务，可拆独立变更）
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-005

### 阶段 2：本次新增高价值 P1（7 条）

- [x] Task 14: [A-001 / A-002] [P1] CLI confirm 模式抽象为可复用、可测试的 `AsyncConfirmer`
  - 修复目标：删除 `crates/cli/src/commands.rs:61-128` 中对 server 侧默认沙箱的重复构造；`Sandbox` 暴露 `with_confirmer()` 方法，server 工厂返回 `(Sandbox, Vec<ToolSpec>)`，CLI confirm 模式直接复用默认沙箱并替换 confirmer；将阻塞 stdin 的闭包改为 `AsyncConfirmer` trait 或 channel 驱动（CLI 在 `spawn_blocking` 读 stdin，测试通过 mock channel 注入 `true/false`）。验证：server 默认工具描述变更后 CLI confirm 模式自动同步；confirm 模式可单元测试且不阻塞 runtime。
  - 受影响：`crates/cli/src/commands.rs`、`crates/tools/src/sandbox.rs`（可能）、`crates/server/src/orchestrator.rs`
  - 可并行：是
  - REAUDIT 重复：无（NEW）

- [ ] Task 15: [A-003] [P1] `Role::From<&str>` 对未知 role 静默回落
  - 修复目标：删除 `crates/llm/src/lib.rs:51-62` 的 `From<&str>`，改为返回 `Option<Role>` 或 `Result`；SSE 解析中遇到未知 role 时 warn 并跳过/报错。新增测试断言未知 role 不再静默变成 `User`。
  - 受影响：`crates/llm/src/lib.rs`、SSE 解析调用方
  - 可并行：是
  - REAUDIT 重复：无（NEW）

- [ ] Task 16: [A-004] [P1] 非法 profile/section 名被映射为 NotFound
  - 修复目标：`crates/core/src/prompt/engine.rs:45-50,70-83` 新增 `CoreError::InvalidName(String)`，让 `../etc` 等非法名字的真实原因进入日志；API 层再决定是否对外脱敏为 404。测试验证 `../etc` 返回 `InvalidName` 而非 `ProfileNotFound`。
  - 受影响：`crates/core/src/prompt/engine.rs`、`crates/core/src/error.rs`、`crates/server/src/api.rs`
  - 可并行：是
  - REAUDIT 重复：无（NEW）

- [ ] Task 17: [B-001 / B-002 / B-003 / B-004] [P1] 前端设计系统基础（方向、字体、响应式、助手渲染）
  - 修复目标：① 确立 ForgeClaw 工业/炽热或代码符文母题并应用于登录页与导航；② `web/src/style.css` 引入特色字体（如 Space Grotesk/Sora 正文 + JetBrains Mono 代码）；③ 助手回复改 `<p>`/`white-space: pre-wrap` 渲染，代码片段用 `<pre><code>` 高亮；④ `App.vue` 增加 `@media` 断点，`< 768px` 切换为抽屉式导航。验证：Lighthouse 可访问性无回归；移动端布局不溢出；助手长文本可读。
  - 受影响：`web/src/style.css`、`web/src/App.vue`、`web/src/views/ChatView.vue`、`web/index.html`、可能新增字体 CDN 或 npm 包
  - 可并行：是（纯前端，不依赖后端）
  - REAUDIT 重复：无（NEW）

- [ ] Task 18: [E-002 / E-003 / E-004] [P1] WS 协议一致性（事件循环超时、Error 帧、非法 session_id）
  - 修复目标：① `crates/server/src/ws.rs:197-230` 用 `tokio::select!` 同时等待 `rx.recv()`、`join`、per-turn 定时器，超时或断连时 abort 并发送 Error 帧；② 错误/超时/panic 分支向 `out_tx` 发送 `OrchestratorEvent::Error` 后再返回；③ 非法 `session_id` 解析失败时发送 Error 帧并 `Continue`，不新建会话。新增 WS 集成测试覆盖超时、Error 帧、非法 session_id。
  - 受影响：`crates/server/src/ws.rs`、`crates/server/tests/auth_test.rs`
  - 可并行：否（与 Task 22 同文件，建议合并）
  - REAUDIT 重复：无（NEW）

- [x] Task 19: [E-009] [P1] 编排器事件增加 `call_id`
  - 修复目标：`crates/server/src/orchestrator.rs` 的 `ToolCallStart`/`ToolResult` 事件增加 `call_id`（取自 LLM 返回的 `tool_call.id`）；`crates/llm/src/lib.rs` 的 `ToolCall` 序列化保留 id；`web/src/views/ChatView.vue` 的 tool_result 回填改按 `call_id` 匹配；更新 `web/src/api/types.ts`。新增测试验证并发同名工具调用结果回填正确。
  - 受影响：`crates/server/src/orchestrator.rs`、`crates/llm/src/lib.rs`、`web/src/views/ChatView.vue`、`web/src/api/types.ts`
  - 可并行：否（跨前后端，需协调）
  - REAUDIT 重复：`REAUDIT_REPORT.md` E-NEW-001

- [x] Task 20: [F-003] [P1] release 产物上传竞态
  - 修复目标：`.github/workflows/release.yml` 将 6 目标矩阵每个 job 都调用 `action-gh-release` 改为单汇总 job（`needs: build`）先 `download-artifact` 收集全部产物，再统一上传；验证多目标 release 产物完整无覆盖。
  - 受影响：`.github/workflows/release.yml`
  - 可并行：是
  - REAUDIT 重复：无（NEW）

### 阶段 3：并发/协议残留 P1（10 条）

- [ ] Task 21: [D-004 / E-005 / C-008] [P1] 新建 session 并发竞态
  - 修复目标：`crates/server/src/api.rs:172-181` 与 `crates/server/src/ws.rs:172-188` 的"session 不存在时各自创建 history_arc"改为 `sessions.write().entry(session_id).or_insert_with(|| ...)` 原子"取或建"；新增并发测试：两请求同时用同一不存在 session_id，最终 history 与 session.messages 状态一致。
  - 受影响：`crates/server/src/api.rs`、`crates/server/src/ws.rs`
  - 可并行：否（与 Task 22/23 同文件，建议合并）
  - REAUDIT 重复：`REAUDIT_REPORT.md` B-NEW-001

- [ ] Task 22: [D-015 / E-006 / C-009] [P1] 会话写回时复核 `user_id`
  - 修复目标：`crates/server/src/api.rs:227-248` 与 `crates/server/src/ws.rs:249-268` 写锁分支中校验 `d.user_id == user_id`，不匹配则 insert 新的 `SessionData`；新增测试验证跨用户写回不污染。
  - 受影响：`crates/server/src/api.rs`、`crates/server/src/ws.rs`
  - 可并行：否（与 Task 21/23 同文件，建议合并）
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-NEW-003

- [ ] Task 23: [D-002 / D-003 / E-007 / C-010] [P1] WS 超时/断连显式 abort spawned 任务
  - 修复目标：`crates/server/src/ws.rs:228-230` 的 `tokio::time::timeout(TASK_TIMEOUT, join).await` 改 `tokio::select! { r = &mut join => {...}, _ = sleep(TASK_TIMEOUT) => { join.abort(); ... } }`，保留 JoinHandle 引用显式 abort；`SESSION_TIMEOUT(600s)` 触发路径同步 abort；新增测试验证 timeout 后同 session 请求不阻塞。
  - 受影响：`crates/server/src/ws.rs`
  - 可并行：否（与 Task 21/22 同文件，建议合并）
  - REAUDIT 重复：`REAUDIT_REPORT.md` D-NEW-001 / B-003

- [ ] Task 24: [D-001] [P1] `history_arc.write()` 在 `run_once` 期间全程持有
  - 修复目标：`crates/server/src/api.rs:183-191` 采用"读锁快照 → 释放锁跑 LLM → 写锁提交"；或 `History` 内部改为 `Arc<[ChatMessage]>` 以支持 cheap clone。新增并发测试验证同 session 只读查询不被 LLM 调用阻塞。
  - 受影响：`crates/server/src/api.rs`、`crates/core/src/prompt/engine.rs`（若改 History 内部结构）
  - 可并行：是
  - REAUDIT 重复：无（NEW）

- [x] Task 25: [D-007 / E-011] [P1] `PromptEngine` 整段持 `tokio::Mutex` 且含同步文件 IO
  - 修复目标：`crates/core/src/prompt/engine.rs` 的 `PromptEngine` 从 `tokio::Mutex` 拆为 `Arc<RwLock>` + `AtomicUsize`，`compile` 退化为 `&self`；cache 拆 `Arc<RwLock<HashMap>>` 短临界区；文件读取改 `tokio::fs` 在锁外完成；新增并发测试验证慢盘下不阻塞多个 compile 请求。
  - 受影响：`crates/core/src/prompt/engine.rs`、`crates/server/src/orchestrator.rs`
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` B-004 / D-019 / B-009

- [ ] Task 26: [C-005] [P1] server 模式 `auto_confirm()` 自动放行 Confirm 级工具
  - 修复目标：`crates/server/src/orchestrator.rs:537,565` 的 `default_sandbox_with_specs`/`restricted_sandbox_with_specs` 不再用 `auto_confirm()`；Confirm 级工具（含 `FileWriteTool`）需显式确认或按配置策略放行；新增测试验证 server 模式下 FileWriteTool 不被自动放行。
  - 受影响：`crates/server/src/orchestrator.rs`
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-017

- [x] Task 27: [C-006] [P1] `find_by_token` 非常量时间查找
  - 修复目标：`crates/server/src/auth.rs:99-101` 改常量时间查找（遍历全部用户对每条 token 做 `subtle::ConstantTimeEq`，不短路）；新增测试验证不存在 token 与存在 token 的查找时间无显著差异。
  - 受影响：`crates/server/src/auth.rs`、`crates/server/Cargo.toml`（若引入 subtle）
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-010 / C-020

- [ ] Task 28: [C-007] [P1] `User` Debug 打印明文 token
  - 修复目标：`crates/server/src/auth.rs:22-29` 手写 `Debug` impl 跳过 `token` 字段（或用 `secrecy::Secret<String>` 包裹）；全 crate `tracing::debug!(?user)` 不再打印 token；新增测试验证 `format!("{:?}", user)` 不含 token 值。
  - 受影响：`crates/server/src/auth.rs`、`crates/server/Cargo.toml`（若引入 secrecy）
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-016 / C-019

- [x] Task 29: [C-011 / E-008] [P1] WS Error 事件直接透传上游敏感信息
  - 修复目标：`crates/server/src/ws.rs:271-279,285-291` 的 `OrchestratorEvent::Error { message }` 经 `send_event` 时改传通用文案（如 `"internal error"`），原始 message 落 `tracing::error!`；新增测试验证前端收到的 error 不含上游 API URL/状态码。
  - 受影响：`crates/server/src/ws.rs`
  - 可并行：否（与 Task 21/22/23 同文件，建议合并）
  - REAUDIT 重复：`REAUDIT_REPORT.md` D-009

- [x] Task 30: [C-012] [P1] Windows 下配置文件权限未限制
  - 修复目标：`crates/cli/src/config.rs:100-114` 的 `Config::save` 在 Windows 分支使用 `std::os::windows::fs` 或 Win32 API 设置显式 ACL，限制仅当前用户可读写；新增测试（至少文档化）验证保存后文件 ACL 正确。
  - 受影响：`crates/cli/src/config.rs`
  - 可并行：是
  - REAUDIT 重复：无（NEW）

- [x] Task 31: [E-019 / F-005] [P1] 重写被删 orchestrator 测试
  - 修复目标：`crates/server/tests/orchestrator_test.rs` 重写对齐当前契约：① `run_streaming_stops_when_receiver_dropped`（期望 `Ok(Error)` 而非 `Err`，守护 D-006/B-003）；② `run_once_propagates_llm_stream_error`（断言 `matches!(event, Error { .. })` + 验证 message 内容，守护 D-004）；③ `run_once_invalid_tool_arguments_returns_tool_result_error`（期望 message 含 `"invalid tool input"`，守护 D-012）；3 个测试均通过。
  - 受影响：`crates/server/tests/orchestrator_test.rs`
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` 合并回归小节 / D-006 / D-012

### 阶段 4：P2 长尾（按视角分批，滚动迭代）

- [ ] Task 32: [A-005 ~ A-015 / A-013] [P2] Karpathy 代码质量长尾
  - 修复目标：① `crates/server/src/orchestrator.rs:336-341,444-449` 统一使用 `ChatMessage` 构造器（A-005）；② `crates/cli/src/commands.rs:359-378` 的 `run_tool_exec` 失败返回 `Err` 而非 `process::exit(1)`（A-006）；③ `crates/cli/src/commands.rs:144-153,296-302` 删除/标注不可能分支（A-007）；④ `crates/cli/src/commands.rs:131-140` 的 `prompt_vars` 不再为取工具名构造完整沙箱（A-008）；⑤ `crates/cli/src/config.rs:137-141` 改 `dirs::home_dir()`（A-009）；⑥ `crates/cli/src/commands.rs:453-471` 与 server 共享用户解析逻辑（A-010）；⑦ `crates/server/src/orchestrator.rs:503-512` 与 CLI `spec_for` 合并为 `Tool::to_spec` 默认方法（A-011）；⑧ `web/src/api/client.ts:95-101` catch `JSON.parse` 异常并按 `ApiError` 抛出（A-012）；⑨ `web/src/views/ChatView.vue:67-71` 与后端将 session_id 契约显式化到类型/API 中（A-013）；⑩ `crates/llm/src/client.rs:69-70` 单次遍历归一化 SSE 行结束符（A-014）；⑪ `crates/core/src/prompt/engine.rs:96-124` `compile` 复用 `list_sections`（A-015）。逐项新增/更新测试验证。
  - 受影响：`crates/server/src/orchestrator.rs`、`crates/cli/src/commands.rs`、`crates/cli/src/config.rs`、`crates/llm/src/client.rs`、`crates/core/src/prompt/engine.rs`、`web/src/api/client.ts`、`web/src/views/ChatView.vue`、可能新增共享模块
  - 可并行：是（可拆分到多个小变更）
  - REAUDIT 重复：部分条目与 `REAUDIT_REPORT.md` A-002/C-004/A-016/A-010/A-NEW-002/A-012/B-004/B-008/B-010/E-NEW-001~004/A-008/C-016/C-019/C-010/C-020 概念重叠，但本任务聚焦 Karpathy 简洁性

- [ ] Task 33: [B-005 ~ B-019] [P2] 前端设计审美长尾
  - 修复目标：① 建立 `web/src/components/BaseButton.vue`、`BaseCard.vue` 等统一设计系统组件（B-005）；② 为空状态/加载态设计插画/骨架屏（B-006）；③ 建立 4 级字号比例与流体排版（B-007/B-008）；④ 扩展语义色 tokens success/warning/info/highlight（B-010）；⑤ 消除硬编码色值（B-011）；⑥ 添加 page-load 编排与入场动效（B-012）；⑦ 丰富 hover/focus/工具状态过渡（B-013/B-014/B-015）；⑧ 聊天界面气泡差异化、时间戳、时间轴（B-016）；⑨ 登录/设置页非对称布局与破格网格（B-017）；⑩ 全局背景渐变/噪点/质感与 elevation 阴影系统（B-018/B-019）；⑪ 补 `index.html` favicon link 并延伸 favicon 质感（B-020）。
  - 受影响：`web/src/components/`（新增）、`web/src/style.css`、`web/src/App.vue`、`web/src/views/*.vue`、`web/index.html`
  - 可并行：是（纯前端，不依赖后端）
  - REAUDIT 重复：`REAUDIT_REPORT.md` E-NEW-006

- [ ] Task 34: [D-005 / D-006 / E-010] [P2] tickets 表过期清理与 poison 处理
  - 修复目标：`crates/server/src/api.rs:42,71-89` 的 `tickets: Arc<std::sync::Mutex<HashMap>>` 在 `issue_ticket` 时 sweep 过期项（TTL 60s）或设上限；`tickets.lock().expect(...)` 改 `match` 处理 poison 或换 `parking_lot::Mutex`；给 `/api/auth/ticket` 加 GovernorLayer；新增测试验证过期 ticket 被清理、poison 不传播 panic。
  - 受影响：`crates/server/src/api.rs`、`crates/server/src/lib.rs`、`crates/server/Cargo.toml`（若引入 parking_lot）
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` B-NEW-002 / B-NEW-003 / D-NEW-002 / C-NEW-001

- [ ] Task 35: [D-008 / D-009 / D-010 / D-011 / D-012 / D-013 / D-014] [P2] 性能与并发长尾
  - 修复目标：① `crates/core/src/prompt/engine.rs:26,101-122` cache 加 LRU 上限（如 256 条）防 OOM（D-008）；② `crates/server/src/orchestrator.rs:254,276` `History` 内部改 `Arc<[ChatMessage]>`，`ChatRequest.tools` 改 `Option<Arc<[ToolSpec]>>`（D-009/E-014）；③ `crates/tools/src/search.rs:185-212` `GrepTool` 用 buffered reader / `memmap2` / 异步流式读取并限制单文件最大字节（D-010）；④ `crates/tools/src/search.rs:89-108,185-212` 使用有界线程池或 `tokio::fs` 减少 `spawn_blocking` 抖动（D-011）；⑤ `crates/tools/src/search.rs:108,212` `unwrap_or_else` + `tracing::warn!` 处理 JoinError（D-012）；⑥ `crates/core/src/model.rs:17` `Session.messages` 改私有并暴露访问器（D-013）；⑦ `crates/llm/src/lib.rs:158` `ChatRequest::from_history` 借用切片或 Arc 避免深拷贝（D-014）。
  - 受影响：`crates/core/src/prompt/engine.rs`、`crates/server/src/orchestrator.rs`、`crates/llm/src/lib.rs`、`crates/tools/src/search.rs`、`crates/core/src/model.rs`
  - 可并行：是（D-013 与 Task 21/22/23 有潜在冲突，需协调）
  - REAUDIT 重复：`REAUDIT_REPORT.md` B-007 / B-008 / D-018 / A-002 / A-NEW-002 / A-002

- [ ] Task 36: [C-013 / C-014 / C-016 / C-017 / C-018 / C-019 / C-020] [P2] 安全沙箱长尾
  - 修复目标：① `crates/server/src/api.rs:70-76` ticket 签发加限流（C-013）；② `crates/server/src/ws.rs:57-65` ticket 改 `Sec-WebSocket-Protocol` 子协议传递（C-014）；③ `crates/tools/src/file.rs:41-63` 补 Windows 敏感路径分支（C-016）；④ `crates/tools/src/file.rs:70-81` `expand_tilde` 改 `dirs::home_dir()`（C-017）；⑤ `crates/tools/src/file.rs:204-211` `FileWriteTool::check` 同步调用 `is_within`（C-018）；⑥ `crates/tools/src/shell.rs:53-54` `eval`/`exec` 收窄为 `eval\s+\$`/`exec\s+\$`（C-019）；⑦ `crates/server/src/orchestrator.rs:160-164` system prompt 注入检查首条 role 是否为 System（C-020）。
  - 受影响：`crates/server/src/{api,ws}.rs`、`crates/tools/src/{file,shell}.rs`、`crates/server/src/orchestrator.rs`、`crates/tools/Cargo.toml`（若引入 dirs）
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-NEW-001 / C-NEW-002 / C-018 / C-021 / A-NEW-001 / B-014

- [ ] Task 37: [E-012 / E-013 / E-016 / E-017 / E-018 / E-020 / E-021 / E-022 / E-023 / E-024 / E-025 / E-026] [P2] API / WebSocket 协议长尾
  - 修复目标：① `crates/server/src/api.rs:147` `/api/auth/ticket` 加 GovernorLayer（E-012）；② `crates/server/src/api.rs:92-99,195-209` 统一 JSON 错误体（E-013）；③ `crates/server/src/api.rs:220-248` / `ws.rs:242-268` 解决 `session.messages` 与 History 双源存储（E-016）；④ ticket 改子协议传递（E-017，同 C-014）；⑤ 评估会话持久化方案（E-018，可延后）；⑥ `crates/server/tests/auth_test.rs` 增加 WS 真实消息流集成测试（E-020）；⑦ `crates/server/tests/api_test.rs` 补非法 session_id 400、跨用户 chat 404（E-021）；⑧ 新增并发新建 session 与跨用户写回竞态测试（E-022）；⑨ `crates/server/src/ws.rs:114-119` 对单连接 frame 处理加 Semaphore 限制（E-023）；⑩ `crates/server/src/lib.rs:168-171` REST `TimeoutLayer` 降至 120s（E-024）；⑪ `crates/server/src/orchestrator.rs:314` 对 tracing warn 中的上游 message 脱敏（E-025）；⑫ `crates/server/src/lib.rs:62-72` 启动时校验 `allowed_origins` 并 warn/error（E-026）。
  - 受影响：`crates/server/src/{api,ws,orchestrator,lib}.rs`、`crates/server/tests/{api,auth}_test.rs`
  - 可并行：是（部分与 Task 21/22/23 同文件，需协调）
  - REAUDIT 重复：`REAUDIT_REPORT.md` C-NEW-001 / D-014 / D-015 / B-010 / D-016

- [ ] Task 38: [F-002 / F-004 / F-006 / F-007 / F-008 / F-009 / F-010 / F-011 / F-012 / F-013 / F-014 / F-015] [P2] CI/CD 与可维护性长尾
  - 修复目标：① `.github/workflows/release.yml:8-9` 删除顶层 `permissions`，在 `jobs.build` 内声明 `contents: write`（F-002）；② README/CI 明确本地构建需先 `pnpm --dir web build` 或 build.rs 自动处理（F-004）；③ `Cargo.toml:26` 根 workspace 仅声明 `tokio = "1.52"`，各 crate 按需声明 feature（F-006）；④ `.github/workflows/release.yml:110-116` Windows job 产物列表去冗余 `.tar.gz`（F-007）；⑤ `web/package.json:8,10` build 改纯 `vite build`（F-008）；⑥ `.github/workflows/release.yml:78` rust-cache key 改 `${{ matrix.os }}-${{ matrix.target }}`（F-009）；⑦ `web/tsconfig.app.json` 显式声明 `strict: true`（F-010）；⑧ `web/vite.config.ts` 显式 `build.outDir: 'dist'`（F-011）；⑨ `Cargo.lock` 统一 `tower-http` 版本（F-012）；⑩ 统一 `web/tsconfig.app.json` 与 `tsconfig.node.json` linting 策略（F-013）；⑪ 关键第三方 actions 固定到 commit SHA 并配置 Dependabot（F-014）；⑫ 引入 `tokio-tungstenite` 编写 WS 端到端消息流测试（F-015）。
  - 受影响：`.github/workflows/{ci,release}.yml`、`Cargo.toml`、`web/package.json`、`web/tsconfig.app.json`、`web/vite.config.ts`、`Cargo.lock`、`crates/server/tests/`、`README.md`
  - 可并行：是
  - REAUDIT 重复：`REAUDIT_REPORT.md` F-006 / F-007 / F-008 / F-010 / F-011 / E-019 / E-023 / D-015

### 修复阶段 Task Dependencies

- 阶段 1（Task 9-13）：互不依赖，可全部并行；Task 9 建议最先做以解除本地编译阻断。
- 阶段 2（Task 14-20）：Task 18/19/20 跨前后端或 CI，需协调；Task 14/15/16/17 独立可并行。
- 阶段 3（Task 21-31）：Task 21/22/23/29 均修改 `ws.rs`，建议合并到同一变更；Task 24/25/26/27/28/30/31 独立可并行。
- 阶段 4（Task 32-38）：可在任何时间滚动迭代，按视角分批；Task 35 的 D-013 与阶段 3 的 model.rs 变更需协调。
