# Tasks

> 本 tasks.md 描述**审计阶段**的工作项。审计完成后产出的**修复任务**将作为新增条目追加到本文件末尾，由后续变更承接实际修复。

## 审计阶段任务

- [x] Task 1: 视角 A — Rust 编译/类型/错误处理审计（`core` / `llm` / `tools`） ✅ 0 P0, 3 P1, 15 P2
- [x] Task 2: 视角 B — 并发与缓存正确性审计 ✅ 1 P0, 5 P1, 8 P2
- [x] Task 3: 视角 C — 安全沙箱与鉴权审计 ✅ 9 P0, 8 P1, 5 P2
- [x] Task 4: 视角 D — API/WebSocket 协议与编排器审计 ✅ 4 P0, 8 P1, 8 P2
- [x] Task 5: 视角 E — 前端 WebUI 审计 ✅ 8 P0, 6 P1, 10 P2
- [x] Task 6: 视角 F — CI/CD 与构建配置审计 ✅ 2 P0, 4 P1, 5 P2
- [x] Task 7: 汇总各视角发现，生成 `AUDIT_REPORT.md` ✅ `/workspace/AUDIT_REPORT.md` 已生成（109 条发现，按 6 视角分节 + 跨视角共性 10 项 + 6 阶段修复顺序）
- [x] Task 8: 生成优先级修复任务清单（追加到本 tasks.md 末尾） ✅ 见下方「修复阶段任务」

## Task Dependencies

- Task 1 / 2 / 3 / 4 / 5 / 6：互不依赖，**全部并行**
- Task 7 依赖 Task 1-6 全部完成
- Task 8 依赖 Task 7 完成

## 修复阶段任务

> 基于 `AUDIT_REPORT.md` 生成。每条标注：报告条目编号、修复目标（verifiable）、受影响文件、是否可并行。P0 优先。
> **本审计变更不执行修复**，下方任务由后续变更承接。

### 阶段 1：阻断性 P0（WebUI 不可用 + CI 红）

- [ ] Task 9: [E-001] 补全 `web/index.html` 入口骨架
  - 修复目标：`index.html` 含 `<html lang="zh-CN">`/`<meta charset>`/`<meta viewport>`/`<title>ForgeClaw</title>`/`<div id="app">`/`<script type="module" src="/src/main.ts">`，`pnpm build` 不再报入口缺失
  - 受影响：`web/index.html`
  - 可并行：是（独立文件）

- [ ] Task 10: [E-015/E-016/E-017] 修正虚构依赖版本号
  - 修复目标：`web/package.json` 中 `typescript` 改 `~5.6.0`、`vite` 改 `^5.4.0`、`vue-router` 改 `^4.4.0`，`pnpm install` 成功
  - 受影响：`web/package.json`、`web/pnpm-lock.yaml`
  - 可并行：是

- [ ] Task 11: [F-001] rust-embed 集成到 server crate
  - 修复目标：`crates/server/src/lib.rs` 新增 `#[derive(RustEmbed)] #[folder = "$CARGO_MANIFEST_DIR/../../web/dist"] struct Asset;` + `static_handler` 处理 `/{*path}` fallback（先查 `Asset::get` 未命中回 `index.html`），`app()` 末尾 `.fallback(static_handler)`；`cargo build` 通过且 `forgeclaw web` 启动后访问 `/` 返回 `index.html`
  - 受影响：`crates/server/src/lib.rs`、`crates/server/Cargo.toml`
  - 可并行：否（依赖 Task 12 同步修复 CI）

- [ ] Task 12: [F-002] CI 构建顺序：rust job `needs: frontend` + artifact 传递
  - 修复目标：`.github/workflows/ci.yml` 中 rust job 显式 `needs: frontend`，frontend job 用 `actions/upload-artifact@v4` 上传 `web/dist`，rust job `actions/download-artifact@v4` 拉回后再 `cargo fmt/clippy/test`；CI 全绿
  - 受影响：`.github/workflows/ci.yml`
  - 可并行：否（与 Task 11 联动）

### 阶段 2：安全 P0（开箱即破）

- [ ] Task 13: [C-004] ShellTool `env_clear()` 清理敏感环境变量
  - 修复目标：`crates/tools/src/shell.rs` 中 `Command::new("sh").env_clear()` 后仅注入 allowlist（`PATH`、`HOME`），剔除 `*API_KEY*`/`*TOKEN*`/`FORGECLAW_USERS`/`*SECRET*`；新增测试验证 `printenv FORGECLAW_USERS` 返回空
  - 受影响：`crates/tools/src/shell.rs`、`crates/tools/src/sandbox.rs`
  - 可并行：是

- [ ] Task 14: [C-007] Config 文件权限 0600
  - 修复目标：`crates/cli/src/config.rs` 的 `Config::save()` 写入后 `set_permissions(0o600)`（Unix），新增测试验证权限
  - 受影响：`crates/cli/src/config.rs`
  - 可并行：是

- [ ] Task 15: [C-008] 默认绑 `127.0.0.1` + 默认 token 治理
  - 修复目标：`crates/cli/src/commands.rs` 的 `run_web` 默认 `host = 127.0.0.1`；启动时检测 `change-me`/`local-token`/空 token 拒绝启动并提示生成；`Config::default_for_init` 用 `Uuid::new_v4()` 生成随机 token 并打印一次
  - 受影响：`crates/cli/src/commands.rs`、`crates/cli/src/config.rs`
  - 可并行：是

- [ ] Task 16: [C-006] WS token 改 `Sec-WebSocket-Protocol` 或一次性 ticket
  - 修复目标：`crates/server/src/ws.rs` 不再用 `?token=` query；实现一次性 ticket（`POST /api/auth/login` 返回 60s 短时 ticket，`WS /ws/chat?ticket=...` 用后即焚）；`TraceLayer` 用 `make_span_with` 脱敏 query
  - 受影响：`crates/server/src/ws.rs`、`crates/server/src/auth.rs`、`crates/server/src/lib.rs`
  - 可并行：是

- [ ] Task 17: [C-001/C-002/C-003] ShellTool 黑名单补全 + 锚点修复
  - 修复目标：`crates/tools/src/shell.rs` 黑名单覆盖 `sudo`/`su`/`eval`/`curl|sh`/`wget|bash`/`chmod 777`/`chown`/`cat /etc/passwd`/`cat ~/.ssh`/`env`/`mv`/`cp` 写 `/etc/`/`bash -i`/`nc`/`mkfifo`；正则去掉 `(?:^|\s)` 前瞻防 `$(...)` 绕过；`rm -rf /` 改 `rm\s+-rf\s+/(?:\S|$)` 拦截子目录；新增测试覆盖所有绕过用例
  - 受影响：`crates/tools/src/shell.rs`
  - 可并行：是

- [ ] Task 18: [C-005] 引入 `landlock` 真沙箱（Linux）
  - 修复目标：`crates/tools/src/sandbox.rs` 在 Linux 下用 `landlock` crate 限制文件系统访问范围至 `working_dir`；其他平台文档明示限制并降级到 cwd 检查；新增集成测试验证 `cd / && touch /tmp/outside` 被拒
  - 受影响：`crates/tools/Cargo.toml`、`crates/tools/src/sandbox.rs`、`crates/tools/src/shell.rs`
  - 可并行：是（长期任务，可拆独立变更）

- [ ] Task 19: [C-009/D-002] WS 跨用户 session 覆盖修复
  - 修复目标：`crates/server/src/ws.rs` 命中既存但 `user_id != current` 的 session_id 时生成新 `Uuid` 或返回错误帧关闭连接，与 REST `chat_handler` 的 404 行为对齐；新增测试覆盖跨用户场景
  - 受影响：`crates/server/src/ws.rs`、`crates/server/tests/auth_test.rs`
  - 可并行：是

### 阶段 3：并发与编排器 P0（数据损坏 + 烧 token）

- [ ] Task 20: [B-001] SessionData 改 `Arc<RwLock<History>>` 解决并发覆盖
  - 修复目标：`crates/server/src/api.rs` 与 `crates/server/src/ws.rs` 把 `SessionData.history` 包裹为 `Arc<RwLock<History>>`，原地 append 而非整体 clone-then-replace；新增并发测试：同一 session 两个并发请求不丢消息
  - 受影响：`crates/server/src/api.rs`、`crates/server/src/ws.rs`、`crates/server/src/orchestrator.rs`
  - 可并行：否（影响多个 P1 任务）

- [ ] Task 21: [D-001] `run_turn` 引入最大轮次
  - 修复目标：`crates/server/src/orchestrator.rs` 的 `run_turn` 加 `max_turns: usize` 参数（默认 25），超出返回 `OrchestratorEvent::Error { message: "max turns exceeded" }`；新增测试验证 25 轮后退出
  - 受影响：`crates/server/src/orchestrator.rs`
  - 可并行：是

- [ ] Task 22: [D-003] 工具错误信息回填 LLM
  - 修复目标：`crates/server/src/orchestrator.rs` 中 `tool_msg.content` 在 `result.error.is_some()` 时改为 `format!("error: {}", e)` 或序列化整个 `ToolResult`；新增测试验证 LLM 收到错误描述
  - 受影响：`crates/server/src/orchestrator.rs`
  - 可并行：是

- [ ] Task 23: [D-004] LLM Error 不再误判 Complete
  - 修复目标：`crates/server/src/orchestrator.rs` 的 `Event::Error` 分支 `return Ok(OrchestratorEvent::Error { message })`，不再落入 `Complete` 路径；新增测试验证 Error 事件正确传播
  - 受影响：`crates/server/src/orchestrator.rs`
  - 可并行：是

### 阶段 4：WebUI 业务 P0（无业务功能）

- [ ] Task 24: [E-002] 创建 5 个核心 view（ChatView/SessionsView/PromptsView/ToolsView/SettingsView）
  - 修复目标：`web/src/views/` 下创建 5 个 `.vue` 文件，各自实现基础业务逻辑（ChatView 流式对话、SessionsView 会话列表、PromptsView Monaco 编辑、ToolsView 工具列表、SettingsView 配置）
  - 受影响：`web/src/views/`
  - 可并行：是（5 个 view 可分人并行）

- [ ] Task 25: [E-003/E-004] 路由表 + 守卫
  - 修复目标：`web/src/router/index.ts` 补 5 条懒加载路由（`() => import(...)`）+ `:pathMatch(.*)*` 兜底 + `meta.requiresAuth`；添加 `router.beforeEach` 校验 token，未授权跳登录
  - 受影响：`web/src/router/index.ts`
  - 可并行：是

- [ ] Task 26: [E-005/E-006] API 客户端封装 + pinia store
  - 修复目标：`web/src/api/` 创建统一 HTTP 客户端（拦截 401 跳登录、统一错误码、超时重试）；`web/src/stores/` 实现 `useAuthStore`/`useSessionStore`/`useSettingsStore`
  - 受影响：`web/src/api/`、`web/src/stores/`
  - 可并行：是

- [ ] Task 27: [E-007] App.vue 导航骨架
  - 修复目标：`web/src/App.vue` 实现侧边栏/顶栏导航菜单，view 间可跳转，当前路由高亮
  - 受影响：`web/src/App.vue`、`web/src/style.css`
  - 可并行：是

### 阶段 5：P1 长尾（建议发布前修复）

- [ ] Task 28: [D-005/D-006/D-011 + B-002/B-003] WS 生命周期管理
  - 修复目标：`crates/server/src/ws.rs` 加心跳（每 30s 发 Ping + `tokio::select!` 监听 pong 超时 60s）+ 客户端断连后 `join.abort()` + `join.await` 用 `tokio::time::timeout(120s)` 包裹 + spawn panic 显式 `match` 三分支
  - 受影响：`crates/server/src/ws.rs`
  - 可并行：是

- [ ] Task 29: [C-010/C-011/C-012/C-013/D-009/D-012] 错误处理加固
  - 修复目标：`auth.rs` 用 `subtle::ConstantTimeEq` + `/api/auth/login` 加 `tower_governor` 限流；`api.rs`/`ws.rs` 的 500 响应体统一返回 `"internal server error"`，详细 `tracing::error!` 落日志；`commands.rs` 的 `config set api_key` 用 `mask_key`；`orchestrator.rs` 的 `parse_tool_input` 失败回填错误而非喂 null
  - 受影响：`crates/server/src/auth.rs`、`crates/server/src/api.rs`、`crates/server/src/ws.rs`、`crates/server/src/orchestrator.rs`、`crates/cli/src/commands.rs`
  - 可并行：是

- [ ] Task 30: [D-007/D-008/C-014/D-010/D-020] HTTP 加固
  - 修复目标：`lib.rs` 加 `DefaultBodyLimit::max(1 MiB)` + `WebSocketUpgrade::max_message_size(256 KiB)`；`TimeoutLayer` 调短至 120s；`CorsLayer::permissive()` 改白名单（从配置读 origin）；中间件顺序调整（CorsLayer 最外层）
  - 受影响：`crates/server/src/lib.rs`
  - 可并行：是

- [ ] Task 31: [A-002/A-005/A-013/C-016/C-019] 类型安全
  - 修复目标：`Session.messages` 改私有 + 访问器；`ChatMessage.role` 改 `enum Role` + `pub fn tool()` 构造器；`User` 的 `Debug` 手写掩盖 token；`LoginResponse` 改 `UserPublic` 不含 token；用 `secrecy::Secret<String>` 包裹 token
  - 受影响：`crates/core/src/model.rs`、`crates/llm/src/lib.rs`、`crates/server/src/auth.rs`
  - 可并行：是

- [ ] Task 32: [B-004/B-009/D-019] prompt engine 并发优化
  - 修复目标：`PromptEngine::compile` 改 `&self` + `RwLock` 短临界区；cache 拆 `Arc<RwLock<HashMap>>` + `AtomicUsize`；文件读取改 `tokio::fs` 在锁外完成
  - 受影响：`crates/core/src/prompt/engine.rs`、`crates/server/src/orchestrator.rs`
  - 可并行：是

- [ ] Task 33: [F-003/F-004/F-005/F-006] CI 加固
  - 修复目标：`release.yml` 矩阵追加 `aarch64-pc-windows-msvc`；ci.yml/release.yml 加 `concurrency` + `timeout-minutes`；ci.yml 顶层 `permissions: { contents: read }`；release.yml 的 `permissions` 下移到 job 级
  - 受影响：`.github/workflows/ci.yml`、`.github/workflows/release.yml`
  - 可并行：是

### 阶段 6：P2 优化（滚动迭代）

- [ ] Task 34: [A-001/B-008/D-018/F-007] 性能优化
  - 修复目标：`SearchTool`/`GrepTool` 包 `spawn_blocking`；`ChatRequest.tools` 改 `Option<Arc<[ToolSpec]>>`；`tokio features` 评估子集
  - 受影响：`crates/tools/src/search.rs`、`crates/llm/src/lib.rs`、`crates/server/src/orchestrator.rs`、`Cargo.toml`
  - 可并行：是

- [ ] Task 35: [A-004/A-006/A-010/A-014] 代码异味清理
  - 修复目标：删除未用 `tracing`/`forgeclaw-core` 依赖或在关键路径补日志；删除 `OpenAiClient.timeout` 冗余字段；`walkdir` 错误 `tracing::warn!`；SSE 非法 JSON `tracing::warn!`
  - 受影响：`crates/*/Cargo.toml`、`crates/llm/src/client.rs`、`crates/tools/src/search.rs`
  - 可并行：是

- [ ] Task 36: [D-014/D-015] 测试覆盖
  - 修复目标：补 `chat_handler` 端到端测试（创建 session + 跨用户 404 + invalid session_id 400）；用 `tokio-tungstenite` 起 WS 集成测试（happy path + 跨用户覆盖 + 心跳 + 断连取消）
  - 受影响：`crates/server/tests/`
  - 可并行：是

- [ ] Task 37: [C-018/C-021] 跨平台加固
  - 修复目标：`is_sensitive_path` 补 Windows 分支；`expand_tilde` 改 `dirs::home_dir()`
  - 受影响：`crates/tools/src/file.rs`
  - 可并行：是

- [ ] Task 38: [E-011/E-012/E-013/E-014/E-018] 前端清理
  - 修复目标：删除 `HelloWorld.vue` 与 `src/assets/{hero.png,vite.svg,vue.svg}`；替换 `style.css` 为项目设计 token；更新 `README.md`；替换 `public/icons.svg`；移除未用依赖 `@vueuse/core`/`monaco-editor`/`naive-ui`
  - 受影响：`web/src/`、`web/public/`、`web/package.json`
  - 可并行：是

### 修复阶段 Task Dependencies

- 阶段 1（Task 9-12）互不依赖，可全部并行（Task 11 与 Task 12 联动需协调）
- 阶段 2（Task 13-19）依赖阶段 1 完成（CI 绿了才能验证修复），彼此可并行
- 阶段 3（Task 20-23）：Task 20 是基础，影响多个 P1；Task 21/22/23 可并行
- 阶段 4（Task 24-27）依赖阶段 1 的 E-001 与版本号修复，彼此可并行
- 阶段 5（Task 28-33）依赖阶段 2/3 的核心 P0 修复
- 阶段 6（Task 34-38）可在任何时间滚动迭代
