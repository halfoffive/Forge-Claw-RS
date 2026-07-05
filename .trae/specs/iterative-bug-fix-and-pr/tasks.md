# Tasks

> 本 tasks.md 按 AUDIT_REPORT.md 的 6 阶段优先级组织修复任务。每阶段完成后进入「循环审查」；P0/P1 清零且 CI 全绿后提交 PR。

## 阶段 1：阻断性 P0（WebUI 不可用 + CI 红）

- [x] Task 1: 修复 `web/index.html` 入口损坏（E-001）
  - 补全 `<html lang="zh-CN">`、`<meta charset>`、`<meta viewport>`、`<title>ForgeClaw</title>`、`<div id="app"></div>`、`<script type="module" src="/src/main.ts"></script>`
  - 修复目标：`pnpm build` 不再报入口缺失
  - 受影响：`web/index.html`
  - 可并行：是

- [x] Task 2: 修正 `web/package.json` 虚构版本号（E-015/E-016/E-017）
  - `typescript` 改 `~5.8.0`、`vite` 改 `^5.4.0`、`vue-router` 改 `^4.4.0`，更新 `pnpm-lock.yaml`
  - 修复目标：`pnpm install` 成功
  - 受影响：`web/package.json`、`web/pnpm-lock.yaml`
  - 可并行：是

- [x] Task 3: rust-embed 集成到 server crate（F-001）
  - 在 `crates/server/src/lib.rs` 新增 `#[derive(RustEmbed)] #[folder = "../../web/dist"] struct Asset;`
  - 写 `static_handler` 处理 `/{*path}` fallback：先 `Asset::get(path)`，未命中回 `index.html`
  - 在 `app()` 末尾 `.fallback(static_handler)`
  - 修复目标：`cargo check -p forgeclaw-server` 通过
  - 受影响：`crates/server/src/lib.rs`、`crates/server/Cargo.toml`
  - 可并行：否（与 Task 4 联动）

- [x] Task 4: CI 构建顺序修复（F-002）
  - `.github/workflows/ci.yml` 中 rust job 显式 `needs: frontend`
  - frontend job 用 `actions/upload-artifact@v4` 上传 `web/dist`
  - rust job 用 `actions/download-artifact@v4` 拉回后再跑 `cargo fmt/clippy/test`
  - 修复目标：GitHub Actions CI 全绿
  - 受影响：`.github/workflows/ci.yml`
  - 可并行：否（与 Task 3 联动）

## 阶段 2：安全 P0（开箱即破）

- [x] Task 5: ShellTool `env_clear()` 清理敏感环境变量（C-004）
  - `Command::new("sh").env_clear()`，仅注入 `PATH`、`HOME` 等允许变量，剔除 `*API_KEY*`/`*TOKEN*`/`FORGECLAW_USERS`/`*SECRET*`
  - 修复目标：新增测试 `printenv FORGECLAW_USERS` 返回空
  - 受影响：`crates/tools/src/shell.rs`
  - 可并行：是

- [x] Task 6: Config 文件权限 0600（C-007）
  - `Config::save()` 写入 `~/.forgeclaw/config.toml` 后 `set_permissions(0o600)`（Unix）
  - 修复目标：新增测试检查文件权限为 0o600
  - 受影响：`crates/cli/src/config.rs`
  - 可并行：是

- [x] Task 7: 默认绑定 127.0.0.1 + 默认 token 治理（C-008）
  - `run_web` 默认 `host = 127.0.0.1`；显式 `--host 0.0.0.0` 才暴露公网
  - 启动时检测 `change-me`/`local-token`/空 token 则拒绝启动并提示生成
  - `Config::default_for_init` 用 `Uuid::new_v4()` 生成随机 token 并打印一次
  - 修复目标：CLI 测试覆盖上述三条
  - 受影响：`crates/cli/src/commands.rs`、`crates/cli/src/config.rs`
  - 可并行：是

- [x] Task 8: WS token 改一次性 ticket（C-006）
  - `crates/server/src/ws.rs` 不再读取 `?token=` query
  - 登录端点返回 60s 短时 ticket，`WS /ws/chat?ticket=...` 用后即焚
  - `TraceLayer` 用 `make_span_with` 脱敏 query string
  - 修复目标：WS 测试通过；日志不含完整 token
  - 受影响：`crates/server/src/ws.rs`、`crates/server/src/auth.rs`、`crates/server/src/lib.rs`
  - 可并行：是

- [x] Task 9: ShellTool 黑名单补全 + 绕过修复（C-001/C-002/C-003）
  - 覆盖 `sudo`/`su`/`eval`/`curl|sh`/`wget|bash`/`chmod 777`/`chown`/`cat /etc/passwd`/`cat ~/.ssh`/`env`/`mv`/`cp` 写 `/etc/`/`bash -i`/`nc`/`mkfifo`
  - 正则去掉 `(?:^|\s)` 前瞻，防止 `$(rm -rf /)`、`` `rm -rf /` `` 绕过
  - `rm -rf /` 改 `rm\s+-rf\s+/(?:\S|$)` 拦截子目录
  - 修复目标：新增危险命令黑名单测试全部通过
  - 受影响：`crates/tools/src/shell.rs`
  - 可并行：是

- [x] Task 10: WS 跨用户 session 覆盖修复（C-009/D-002）
  - `crates/server/src/ws.rs` 命中既存但 `user_id != current` 的 session_id 时返回错误帧关闭连接
  - 与 REST `chat_handler` 的 404 行为对齐
  - 修复目标：新增 WS 跨用户测试通过
  - 受影响：`crates/server/src/ws.rs`、`crates/server/tests/auth_test.rs`
  - 可并行：是

## 阶段 3：并发与编排器 P0（数据损坏 + 烧 token）

- [x] Task 11: SessionData 改 `Arc<RwLock<History>>`（B-001）
  - `crates/server/src/api.rs` 与 `ws.rs` 把 `SessionData.history` 包裹为 `Arc<RwLock<History>>`
  - 原地 append，不再整体 clone-then-replace
  - 修复目标：新增并发测试——同一 session 两个并发请求不丢消息
  - 受影响：`crates/server/src/api.rs`、`crates/server/src/ws.rs`、`crates/server/src/orchestrator.rs`
  - 可并行：否（影响多个 P1 任务）

- [x] Task 12: `run_turn` 引入最大轮次（D-001）
  - `crates/server/src/orchestrator.rs` 的 `run_turn` 加 `max_turns: usize` 参数（默认 25）
  - 超出后返回 `OrchestratorEvent::Error { message: "max turns exceeded" }`
  - 修复目标：新增死循环防护测试——26 轮调用后退出并返回 Error
  - 受影响：`crates/server/src/orchestrator.rs`
  - 可并行：是

- [x] Task 13: 工具错误信息回填 LLM（D-003）
  - `crates/server/src/orchestrator.rs` 中 `tool_msg.content` 在 `result.error.is_some()` 时改为 `format!("error: {}", e)` 或序列化整个 `ToolResult`
  - 修复目标：新增测试验证 LLM history 中收到错误描述
  - 受影响：`crates/server/src/orchestrator.rs`
  - 可并行：是

- [x] Task 14: LLM Error 不再误判 Complete（D-004）
  - `crates/server/src/orchestrator.rs` 的 `Event::Error` 分支直接 `return Ok(OrchestratorEvent::Error { message })`
  - 不再落入 `Complete { text: "" }` 路径
  - 修复目标：新增测试验证 LLM Error 正确传播
  - 受影响：`crates/server/src/orchestrator.rs`
  - 可并行：是

## 阶段 4：WebUI 业务 P0（无业务功能）

- [x] Task 15: 创建 5 个核心 view（E-002）
  - `web/src/views/` 下创建 `ChatView.vue`/`SessionsView.vue`/`PromptsView.vue`/`ToolsView.vue`/`SettingsView.vue`
  - 各自实现基础业务逻辑（ChatView 流式对话、SessionsView 列表、PromptsView 简单文本编辑、ToolsView 工具列表、SettingsView 配置）
  - 修复目标：5 个 view 文件存在且路由可访问
  - 受影响：`web/src/views/`
  - 可并行：是（5 个 view 可分人并行）

- [x] Task 16: 路由表 + 守卫（E-003/E-004）
  - `web/src/router/index.ts` 补 5 条懒加载路由（`() => import(...)`）+ `:pathMatch(.*)*` 兜底
  - 添加 `meta.requiresAuth` 与 `router.beforeEach`，未授权重定向到登录页
  - 修复目标：未授权访问 `/chat` 等路由被重定向
  - 受影响：`web/src/router/index.ts`、`web/src/views/LoginView.vue`、`web/src/views/NotFoundView.vue`、`web/src/views/HomeView.vue`
  - 可并行：是

- [x] Task 17: API 客户端封装 + pinia store（E-005/E-006）
  - `web/src/api/` 创建统一 HTTP 客户端：拦截 401 跳登录、统一错误码、超时重试
  - `web/src/stores/` 实现 `useAuthStore`/`useSessionStore`/`useSettingsStore`
  - 修复目标：前端能登录并拉取 `/api/sessions` 列表
  - 受影响：`web/src/api/client.ts`、`web/src/api/types.ts`、`web/src/stores/auth.ts`、`web/src/stores/session.ts`、`web/src/stores/settings.ts`
  - 可并行：是

- [x] Task 18: `App.vue` 导航骨架（E-007）
  - `web/src/App.vue` 实现侧边栏/顶栏导航菜单，view 间可切换，当前路由高亮
  - 修复目标：用户可在 5 个 view 间跳转
  - 受影响：`web/src/App.vue`、`web/src/style.css`
  - 可并行：是

## 阶段 5：P1 长尾（建议发布前修复）

- [x] Task 19: WS 生命周期管理（D-005/D-006/D-011 + B-002/B-003）
  - `crates/server/src/ws.rs` 加心跳：每 30s 发 Ping + `tokio::select!` 监听 pong 超时 60s
  - 客户端断连后 `join.abort()`，`join.await` 用 `tokio::time::timeout(120s)` 包裹
  - spawn panic 显式 `match` 三分支处理
  - `crates/server/src/orchestrator.rs` 中 `tx.send` 失败时通过 `?` 传播，停止继续生成 token
  - 修复目标：新增 WS 心跳/断连/取消测试通过
  - 受影响：`crates/server/src/ws.rs`、`crates/server/src/orchestrator.rs`、`crates/server/tests/auth_test.rs`、`crates/server/tests/orchestrator_test.rs`
  - 可并行：是

- [ ] Task 20: 错误处理加固（C-010/C-011/C-012/C-013/D-009/D-012）
  - `auth.rs` token 比较改用 `subtle::ConstantTimeEq`；`/api/auth/login` 加 `tower_governor` 限流
  - `api.rs`/`ws.rs` 的 500 响应体统一返回 `"internal server error"`，详细 `tracing::error!` 落日志
  - `commands.rs` 的 `config set api_key` 用 `mask_key` 输出
  - `orchestrator.rs` 的 `parse_tool_input` 失败时构造错误 ToolResult 回填，不喂 null
  - 修复目标：相关单元/集成测试通过
  - 受影响：`crates/server/src/auth.rs`、`crates/server/src/api.rs`、`crates/server/src/ws.rs`、`crates/server/src/orchestrator.rs`、`crates/cli/src/commands.rs`
  - 可并行：是

- [x] Task 21: HTTP 加固（D-007/D-008/C-014/D-010/D-020）
  - `crates/server/src/lib.rs` 加 `DefaultBodyLimit::max(1 * 1024 * 1024)`
  - `WebSocketUpgrade::max_message_size(256 * 1024)`
  - `TimeoutLayer` 调短至 120s
  - `CorsLayer::permissive()` 改 origin 白名单（从配置读取）
  - 中间件顺序调整（CorsLayer 最外层）
  - 修复目标：集成测试拒绝超大请求；CORS 预检按白名单响应
  - 受影响：`crates/server/src/lib.rs`、`crates/server/src/api.rs`、`crates/server/src/ws.rs`、`crates/cli/src/config.rs`、`crates/cli/src/commands.rs`
  - 可并行：是

- [x] Task 22: 类型安全（A-002/A-005/A-013/C-016/C-019）
  - `Session.messages` 改私有 + 提供 `append`/`messages()` 访问器
  - `ChatMessage.role` 改 `enum Role { System, User, Assistant, Tool }` + `#[serde(rename_all = "lowercase")]`
  - 增加 `ChatMessage::tool(content, tool_call_id)` 构造器
  - `User` 手写 `Debug` 掩盖 token；`LoginResponse` 改用 `UserPublic { id, name }` 不含 token
  - 用 `secrecy::Secret<String>` 包裹 token
  - 修复目标：编译通过，新增测试验证 token 不泄漏
  - 受影响：`crates/core/src/model.rs`、`crates/llm/src/lib.rs`、`crates/server/src/auth.rs`
  - 可并行：是

- [x] Task 23: PromptEngine 并发优化（B-004/B-009/D-019）
  - `PromptEngine::compile` 改 `&self` + `RwLock` 短临界区
  - cache 拆为 `Arc<RwLock<HashMap>>` + `AtomicUsize`
  - 文件读取改 `tokio::fs::read_to_string` 在锁外完成
  - 修复目标：并发 compile 测试不阻塞、cache hit 路径无锁争用
  - 受影响：`crates/core/src/prompt/engine.rs`、`crates/server/src/orchestrator.rs`
  - 可并行：是

- [x] Task 24: CI 加固（F-003/F-004/F-005/F-006）
  - `release.yml` 矩阵追加 `aarch64-pc-windows-msvc`
  - `ci.yml`/`release.yml` 加 `concurrency` + `timeout-minutes`
  - `ci.yml` 顶层 `permissions: { contents: read }`
  - `release.yml` 的 `permissions` 下移到 job 级
  - 修复目标：release.yml 语法正确，不再并发跑重复 CI
  - 受影响：`.github/workflows/ci.yml`、`.github/workflows/release.yml`
  - 可并行：是

## 阶段 6：循环审查

- [ ] Task 25: 第一轮多视角再审查
  - 委派 3 个独立子代理（安全、并发/编排器、前端/类型）审查修复后代码
  - 每个子代理返回 P0/P1/P2 发现清单
  - 修复目标：获得新发现清单

- [ ] Task 26: 修复第一轮新发现的 P0/P1
  - 针对新发现创建修复任务并实施
  - 修复目标：P0/P1 计数下降

- [ ] Task 27: 第二轮多视角再审查
  - 再次委派 3 个独立子代理审查
  - 修复目标：P0 = 0，P1 = 0

- [ ] Task 28: 运行完整 CI
  - `cargo fmt --check`
  - `cargo clippy -- -D warnings`
  - `cargo test`
  - `pnpm install && pnpm build && pnpm typecheck`
  - 修复目标：全部通过

## 阶段 7：PR 提交

- [ ] Task 29: 整理 commit 与分支
  - `git checkout -b fix/audit-findings`
  - `git add` 相关文件
  - 使用 conventional commit：`fix: ...` / `feat: ...` / `refactor: ...`
  - 修复目标：`git status` 干净，commit 历史清晰

- [ ] Task 30: 使用 gh CLI 创建 PR
  - `gh pr create --title "fix: address audit findings" --body-file pr-body.md`
  - PR body 引用 `AUDIT_REPORT.md`、修复摘要、测试策略、相关 spec 链接
  - 修复目标：返回 PR URL

## Task Dependencies

- 阶段 1（Task 1-4）内部：Task 1/2 优先；Task 3/4 联动；其余可并行
- 阶段 2/3/4 彼此可并行，但均依赖阶段 1 CI 通过
- 阶段 5 依赖阶段 2/3/4 完成
- 阶段 6 依赖阶段 5 完成
- 阶段 7 依赖阶段 6 P0/P1 清零 + CI 全绿
