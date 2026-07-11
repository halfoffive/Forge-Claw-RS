# Tasks

> 本 tasks.md 描述**重新审计阶段**的工作项。重新审计完成后产出的**修复任务**将作为新增条目追加到本文件末尾，由后续变更承接实际修复。
> 重新审计针对当前 HEAD（`be8ac66`），对照首轮 `AUDIT_REPORT.md`（change-id: `audit-bugs-and-optimizations`）做状态标注。

## 重新审计阶段任务

- [x] Task 1: 视角 A — Rust 编译/类型/错误处理重新审计（`core` / `llm` / `tools`） ✅ FIXED 11 / PARTIAL 2 / NOT-FIXED 4 / REGRESSED 1 / NEW 2
- [x] Task 2: 视角 B — 并发与缓存正确性重新审计 ✅ FIXED 5 / PARTIAL 2 / NOT-FIXED 7 / REGRESSED 0 / NEW 3（含补审 B-010~B-014）
- [x] Task 3: 视角 C — 安全沙箱与鉴权重新审计 ✅ FIXED 10 / PARTIAL 3 / NOT-FIXED 9 / REGRESSED 0 / NEW 3
- [x] Task 4: 视角 D — API/WebSocket 协议与编排器重新审计 ✅ FIXED 12 / PARTIAL 4 / NOT-FIXED 4 / REGRESSED 0 / NEW 2
- [x] Task 5: 视角 E — 前端 WebUI 重新审计 ✅ FIXED 16 / PARTIAL 5 / NOT-FIXED 3（前提错误 N/A）/ REGRESSED 0 / NEW 6
- [x] Task 6: 视角 F — CI/CD 与构建配置重新审计 ✅ FIXED 5 / PARTIAL 1 / NOT-FIXED 2+3 残留 / REGRESSED 0 / NEW 5（含补审 F-009）
- [x] Task 7: 汇总各视角发现，生成 `REAUDIT_REPORT.md` ✅ `/workspace/REAUDIT_REPORT.md` 已生成（127 条发现：59 FIXED / 17 PARTIAL / 29 NOT-FIXED / 1 REGRESSED / 21 NEW，含合并回归专项 + P0→P1→P2 清单 + 5 阶段修复顺序）
- [x] Task 8: 生成更新后的修复任务清单（追加到本 tasks.md 末尾） ✅ 见下方「修复阶段任务」Task 9-32（4 P0 + 13 P1 + 7 P2 分组，共 24 条修复任务）

## Task Dependencies

- Task 1 / 2 / 3 / 4 / 5 / 6：互不依赖，**全部并行**（6 个独立子代理）
- Task 7 依赖 Task 1-6 全部完成
- Task 8 依赖 Task 7 完成

## 修复阶段任务

> 基于 `REAUDIT_REPORT.md` 中 NOT-FIXED + PARTIAL + REGRESSED + NEW 条目生成。每条标注：报告条目编号、状态来源、修复目标（verifiable）、受影响文件、是否可并行、首轮 task 编号（若适用）。
> **本重新审计变更不执行修复**，下方任务由后续变更承接。
> FIXED 条目不列入修复任务（首轮 57 条已修复项仅在报告中记录）。

### 阶段 1：阻断性 P0（4 条，最高优先）

- [x] Task 9: [C-004] [NOT-FIXED] [首轮 Task 13] ShellTool `env_clear()` 清理敏感环境变量
  - 修复目标：`crates/tools/src/shell.rs` 中 `Command::new("sh")` 调用 `.env_clear()` 后仅注入 allowlist（`PATH`、`HOME`、必要的 `TERM`）；新增测试验证 `printenv FORGECLAW_USERS` / `printenv DEEPSEEK_API_KEY` 返回空，`printenv PATH` 非空
  - 受影响：`crates/tools/src/shell.rs`
  - 可并行：是（独立文件，与 Task 10/11/12 互不冲突）
  - 状态来源：NOT-FIXED（首轮 P0，be8ac66 未触及）

- [x] Task 10: [C-001] [PARTIAL] [首轮 Task 17] ShellTool 黑名单补全
  - 修复目标：`crates/tools/src/shell.rs:38-64` 黑名单补全 `sudo`/`su`/`chown`/`chmod 777`（无 -R）/`cat /etc/passwd`/`cat ~/.ssh`/`env`/`mv`/`cp` 写 `/etc/`/`bash -i >& /dev/tcp`/`nc`/`mkfifo`/`curl|sh`/`wget|bash`；新增测试覆盖每个新增绕过用例均被拦截
  - 受影响：`crates/tools/src/shell.rs`
  - 可并行：是（与 Task 9 同文件但不同代码块，建议合并到同一变更）
  - 状态来源：PARTIAL（已扩展但仍缺高危命令）

- [ ] Task 11: [C-005] [NOT-FIXED] [首轮 Task 18] 引入 `landlock` 真沙箱（Linux）
  - 修复目标：`crates/tools/src/sandbox.rs` 在 Linux 下用 `landlock` crate 限制 FS 访问至 `working_dir`；其他平台文档明示限制并降级到 cwd 检查；新增集成测试验证 `cd / && touch /tmp/outside` 被拒
  - 受影响：`crates/tools/Cargo.toml`、`crates/tools/src/sandbox.rs`、`crates/tools/src/shell.rs`
  - 可并行：是（长期任务，可拆独立变更）
  - 状态来源：NOT-FIXED（首轮 P0，未引入任何沙箱 crate）

- [x] Task 12: [F-001/F-NEW-001] [PARTIAL+NEW] [首轮 Task 11] 本地 `cargo check` 失败修复
  - 修复目标：`web/dist` 目录在仓库中可被 rust-embed 编译期读取。方案择一：① `web/dist/.gitkeep` 占位 + `web/.gitignore` 改 `dist/*` + `!dist/.gitkeep`；② `crates/server/build.rs` 在 dist 不存在时创建空目录。验证：干净克隆后 `cargo check -p forgeclaw-server` 无需先 `pnpm build` 即通过
  - 受影响：`web/.gitignore`、`web/dist/.gitkeep`（新增）或 `crates/server/build.rs`
  - 可并行：是（最小改动，建议最先做以解除本地编译阻断）
  - 状态来源：PARTIAL（rust-embed 集成代码完整）+ NEW（F-NEW-001 本地编译失败）

### 阶段 2：合并回归与并发 P1（6 条）

- [ ] Task 13: [A-002] [REGRESSED] [首轮 Task 31] `Session.messages` 改私有 + 访问器
  - 修复目标：`crates/core/src/model.rs:17` 的 `pub messages: Vec<Message>` 改 `pub(crate)` 或私有 + 提供 `pub fn messages(&self) -> &[Message]` 与 `pub fn append_message(&mut self, m: Message)`；全 crate 编译通过，无外部直接 mutate
  - 受影响：`crates/core/src/model.rs`、调用方 `crates/server/src/{api,ws,orchestrator}.rs`
  - 可并行：是
  - 状态来源：REGRESSED（be8ac66 显式回退 model.rs 到 main 版本）

- [x] Task 14: [B-NEW-001] [NEW] 新建 session 并发竞态修复
  - 修复目标：`crates/server/src/api.rs:172-181` 与 `ws.rs:172-188` 的"session 不存在时各自创建 history_arc"改为 `sessions.write().entry(session_id).or_insert_with(|| ...)` 原子"取或建"；新增并发测试：两请求同时用同一不存在 session_id，最终 history 与 session.messages 状态一致
  - 受影响：`crates/server/src/api.rs`、`crates/server/src/ws.rs`
  - 可并行：否（与 Task 15 联动，建议合并到同一变更）
  - 状态来源：NEW（B-001 修复盲区，新建 session 路径）

- [ ] Task 15: [D-011/D-NEW-001/B-003] [PARTIAL+NEW] [首轮 Task 28] WS timeout 后 `join.abort()` 修复
  - 修复目标：`crates/server/src/ws.rs:230` 的 `tokio::time::timeout(TASK_TIMEOUT, join).await` 改 `tokio::select! { r = &mut join => {...}, _ = sleep(TASK_TIMEOUT) => { join.abort(); ... } }`，保留 JoinHandle 引用显式 abort；`SESSION_TIMEOUT(600s)` 触发路径同步 abort；新增测试验证 timeout 后同 session 请求不阻塞
  - 受影响：`crates/server/src/ws.rs`
  - 可并行：否（与 Task 14 同文件，建议合并）
  - 状态来源：PARTIAL（B-003/D-011 已有 timeout 但未 abort）+ NEW（D-NEW-001 detach 任务持锁）

- [ ] Task 16: [被删测试] [REGRESSED] [首轮 Task 36 关联] 重写 3 个被删 orchestrator 测试
  - 修复目标：`crates/server/tests/orchestrator_test.rs` 重写对齐当前契约：① `run_streaming_stops_when_receiver_dropped`（期望 `Ok(Error)` 而非 `Err`，守护 D-006/B-003）；② `run_once_propagates_llm_stream_error`（断言 `matches!(event, Error { .. })` + 验证 message 内容，守护 D-004）；③ `run_once_invalid_tool_arguments_returns_tool_result_error`（期望 message 含 `"invalid tool input"`，守护 D-012）；3 个测试均通过
  - 受影响：`crates/server/tests/orchestrator_test.rs`
  - 可并行：是
  - 状态来源：REGRESSED（be8ac66 删除 3 个测试，2 个高风险路径无替代守护）

- [x] Task 17: [C-017] [NOT-FIXED] 去除 server 模式 `auto_confirm()`
  - 修复目标：`crates/server/src/orchestrator.rs:537,565` 的 `default_sandbox_with_specs`/`restricted_sandbox_with_specs` 不再用 `auto_confirm()`；Confirm 级工具（含 `FileWriteTool`）需显式确认或按配置策略放行；新增测试验证 server 模式下 FileWriteTool 不被自动放行
  - 受影响：`crates/server/src/orchestrator.rs`
  - 可并行：是
  - 状态来源：NOT-FIXED（首轮 P1，未触及）

- [ ] Task 18: [C-016/C-019] [PARTIAL+NOT-FIXED] [首轮 Task 31] `User` Debug 掩盖 token
  - 修复目标：`crates/server/src/auth.rs:23` 的 `User` 手写 `Debug` impl 跳过 `token` 字段（或用 `secrecy::Secret<String>` 包裹）；全 crate `tracing::debug!(?user)` 不再打印 token；新增测试验证 `format!("{:?}", user)` 不含 token 值
  - 受影响：`crates/server/src/auth.rs`、`crates/server/Cargo.toml`（若引入 secrecy）
  - 可并行：是
  - 状态来源：PARTIAL（C-016 LoginResponse 已改 UserPublic）+ NOT-FIXED（C-019 User Debug 未手写）

### 阶段 3：安全加固 P1（5 条）

- [x] Task 19: [C-015] [NOT-FIXED] FileWriteTool TOCTOU 修复
  - 修复目标：`crates/tools/src/file.rs:213-241` 的 `is_within` canonicalize 与 `fs::write` 之间 TOCTOU 消除：canonicalize 后直接写规范化路径（不再 re-resolve），或在写锁内完成检查+写入；新增测试验证符号链接替换逃逸被拒
  - 受影响：`crates/tools/src/file.rs`
  - 可并行：是
  - 状态来源：NOT-FIXED（首轮 P1，未触及）

- [x] Task 20: [C-010/C-020] [PARTIAL+NOT-FIXED] [首轮 Task 29] `find_by_token` 常量时间查找
  - 修复目标：`crates/server/src/auth.rs:99-101` 的 `find_by_token` 改常量时间查找（遍历全部用户对每条 token 做 `subtle::ConstantTimeEq`，不短路）；新增测试验证不存在 token 与存在 token 的查找时间无显著差异
  - 受影响：`crates/server/src/auth.rs`
  - 可并行：是
  - 状态来源：PARTIAL（C-010 已引入 constant_time_eq 用于 middleware）+ NOT-FIXED（C-020 find_by_token 仍 HashMap::get）

- [ ] Task 21: [D-009] [PARTIAL] [首轮 Task 29] WS Error 消息脱敏
  - 修复目标：`crates/server/src/ws.rs:285-290` 的 `OrchestratorEvent::Error { message }` 经 `send_event` 时改传通用文案（如 `"internal error"`），原始 message 落 `tracing::error!`；新增测试验证前端收到的 error 不含上游 API URL/状态码
  - 受影响：`crates/server/src/ws.rs`
  - 可并行：是
  - 状态来源：PARTIAL（REST 侧已修，WS 侧未修）

- [ ] Task 22: [B-004/B-009/D-019] [NOT-FIXED] [首轮 Task 32] PromptEngine 并发优化
  - 修复目标：`crates/core/src/prompt/engine.rs` 的 `PromptEngine` 从 `tokio::Mutex` 拆为 `Arc<RwLock>` + `AtomicUsize`，`compile` 退化为 `&self`；cache 拆 `Arc<RwLock<HashMap>>` 短临界区；文件读取改 `tokio::fs` 在锁外完成；新增并发测试验证慢盘下不阻塞多个 compile 请求
  - 受影响：`crates/core/src/prompt/engine.rs`、`crates/server/src/orchestrator.rs`
  - 可并行：是
  - 状态来源：NOT-FIXED（首轮 P1，未触及）

- [ ] Task 23: [B-NEW-001 关联清理] [NEW] tickets HashMap 过期清理 + poison 处理
  - 修复目标：`crates/server/src/api.rs:42,71-89` 的 `tickets: Arc<std::sync::Mutex<HashMap>>` 在 `issue_ticket` 时 sweep 过期项（TTL 60s）或设上限；`tickets.lock().expect(...)` 改 `match` 处理 poison 或换 `parking_lot::Mutex`；新增测试验证过期 ticket 被清理、poison 不传播 panic
  - 受影响：`crates/server/src/api.rs`、`crates/server/src/auth.rs`
  - 可并行：是
  - 状态来源：NEW（B-NEW-002 无上限 + B-NEW-003 poison panic + C-NEW-001/D-NEW-002 同类）

### 阶段 4：前端 P1（2 条）

- [ ] Task 24: [E-005] [PARTIAL] [首轮 Task 26] API 客户端超时重试
  - 修复目标：`web/src/api/client.ts` 在现有 30s `AbortController` 超时基础上，对 5xx 与网络错误加指数退避重试（最多 2 次，间隔 500ms/1s）；401 不重试；新增单测验证重试行为
  - 受影响：`web/src/api/client.ts`
  - 可并行：是
  - 状态来源：PARTIAL（已封装超时与 401 处理，缺重试）

- [x] Task 25: [E-NEW-001] [NEW] tool_result 精确关联（后端加 `call_id`）
  - 修复目标：`crates/server/src/orchestrator.rs` 的 `tool_call_start` 事件增加 `call_id` 字段（取自 LLM 返回的 tool_call.id）；`crates/llm/src/lib.rs` 的 ToolCall 序列化保留 id；`web/src/views/ChatView.vue:142-156` 的 tool_result 回填改按 `call_id` 匹配；新增测试验证并发同名工具调用结果回填正确
  - 受影响：`crates/server/src/orchestrator.rs`、`crates/llm/src/lib.rs`、`web/src/views/ChatView.vue`、`web/src/api/types.ts`
  - 可并行：否（跨前后端，需协调）
  - 状态来源：NEW（前端按 name 匹配易错配）

### 阶段 5：P2 长尾（按视角分批，滚动迭代）

- [ ] Task 26: [A-007/A-008/A-010/A-012/A-016/A-018/A-NEW-001/A-NEW-002] [PARTIAL/NOT-FIXED/NEW] [首轮 Task 34/35] Rust 类型与错误处理长尾
  - 修复目标：`shell.rs` io 错误含命令文本（A-007）；`client.rs` 5xx body 入错误 + 429 单独重试读 `Retry-After`（A-008/A-018）；`search.rs` `filter_map(|e| e.ok())` 加 `tracing::warn!` + 跳过计数（A-010）；`section.rs` `trim_matches('"')` 处理 YAML 转义或文档明示（A-012）；`shell.rs:89-102` `check` 缺失 command 时返回 `Critical`（A-016）；`shell.rs:53-54` `eval`/`exec` 收窄为 `eval\s+\$`/`exec\s+\$`（A-NEW-001）；`search.rs:108,212` `unwrap_or_default` 改 `unwrap_or_else` + warn（A-NEW-002）
  - 受影响：`crates/tools/src/{shell,search}.rs`、`crates/llm/src/client.rs`、`crates/core/src/prompt/section.rs`
  - 可并行：是
  - 状态来源：PARTIAL（A-007/A-008）+ NOT-FIXED（A-010/A-012/A-016/A-018）+ NEW（A-NEW-001/A-NEW-002）

- [ ] Task 27: [B-007/B-008/B-010/B-011/B-013/B-014] [NOT-FIXED/PARTIAL] [首轮 Task 34] 缓存与性能长尾
  - 修复目标：`engine.rs:26,101-122` cache HashMap 加 LRU 上限（如 256 条）防 OOM（B-007）；`orchestrator.rs:271-277` `tool_specs.to_vec()` 改 `ChatRequest.tools: Option<Arc<[ToolSpec]>>` 共享引用（B-008/D-018）；`api.rs:48-53` `SessionData` 改单一真源，`session.messages` 由 `history.messages()` 按需映射（B-010）；`api.rs:311-321` profile 名白名单（B-011 残留）；`prompt_vars` 提取到 `forgeclaw_core` 单一真源（B-013）；`orchestrator.rs:161,184` system prompt 注入改检查 `messages().first()` 是否为 system（B-014）
  - 受影响：`crates/core/src/prompt/engine.rs`、`crates/server/src/orchestrator.rs`、`crates/llm/src/lib.rs`、`crates/server/src/api.rs`、`crates/cli/src/commands.rs`、`crates/core/src/model.rs`
  - 可并行：是（B-010/B-013/B-014 可拆分独立子任务）
  - 状态来源：NOT-FIXED（B-007/B-008/B-010/B-013/B-014）+ PARTIAL（B-011）

- [ ] Task 28: [C-018/C-021/C-022] [NOT-FIXED] [首轮 Task 37] 跨平台与硬上限加固
  - 修复目标：`file.rs:41-63` `is_sensitive_path` 补 Windows 分支（`C:\Windows`、`%USERPROFILE%\.ssh` 等）（C-018）；`file.rs:70-81` `expand_tilde` 改 `dirs::home_dir()`（C-021）；`search.rs:66,151` `SearchTool`/`GrepTool` 的 `max` 加服务端硬上限（如 1000）（C-022）
  - 受影响：`crates/tools/src/{file,search}.rs`
  - 可并行：是
  - 状态来源：NOT-FIXED（首轮 P2，未触及）

- [ ] Task 29: [C-NEW-002/C-NEW-003] [NEW] ticket 安全加固
  - 修复目标：`ws.rs:59-62` ticket 改用 `Sec-WebSocket-Protocol` 子协议传递（C-NEW-002）；`ws.rs:249-267` + `api.rs:227-247` 写回 session 时校验 `d.user_id == user_id`，不匹配则 insert 新 SessionData（C-NEW-003）；新增测试验证跨用户写回不污染
  - 受影响：`crates/server/src/{ws,api}.rs`
  - 可并行：是
  - 状态来源：NEW（ticket API 引入的新问题）

- [ ] Task 30: [D-014/D-015/D-016] [PARTIAL/NOT-FIXED] [首轮 Task 36] 测试覆盖补全
  - 修复目标：`crates/server/tests/` 补 invalid session_id 400、REST 跨用户 chat 404 端到端测试（D-014）；用 `tokio-tungstenite` 起 WS 真实消息流集成测试（happy path + 跨用户 + 心跳 + 断连取消）（D-015）；评估 `sessions: Arc<RwLock<HashMap>>` 持久化方案（D-016 可延后）
  - 受影响：`crates/server/tests/`
  - 可并行：是
  - 状态来源：PARTIAL（D-014 已补并发与 500 测试）+ NOT-FIXED（D-015/D-016）

- [ ] Task 31: [E-009/E-010/E-019/E-023/E-NEW-002/E-NEW-003/E-NEW-004/E-NEW-005/E-NEW-006] [PARTIAL/NEW] [首轮 Task 26/38] 前端体验长尾
  - 修复目标：`ChatView.vue` 流式渲染加 `requestAnimationFrame` 节流（E-009）；mid-stream 断连加自动恢复（E-010）；`tsconfig.app.json` 显式声明 `strict: true`（E-019）；`vite.config.ts` 显式 `build.outDir: 'dist'`（E-023）；ticket 获取失败时清理 user 消息（E-NEW-002）；error/异常 close 路径清理 assistant 占位（E-NEW-003）；用 login 响应 ticket 作首连（E-NEW-004）；`PromptsView.vue` 补 `PUT /api/prompts/sections` 保存 + Markdown 预览 + 修正 `--font-mono` token（E-NEW-005）；`index.html` 补 favicon link（E-NEW-006）
  - 受影响：`web/src/views/{ChatView,PromptsView}.vue`、`web/src/stores/auth.ts`、`web/tsconfig.app.json`、`web/vite.config.ts`、`web/index.html`
  - 可并行：是（可拆分到多人）
  - 状态来源：PARTIAL（E-009/E-010/E-019/E-023）+ NEW（E-NEW-002~006）

- [ ] Task 32: [F-006/F-NEW-002/F-007/F-008/F-NEW-003/F-010/F-NEW-004/F-011/F-NEW-005] [NOT-FIXED/NEW] [首轮 Task 33/34] CI/CD 加固长尾
  - 修复目标：`release.yml` 删除顶层 `permissions`，在 build job 内声明 `contents: write`（F-006/F-NEW-002）；`Cargo.toml:26` `tokio features` 评估收窄（F-007）；`release.yml:110-116` Windows job 产物列表去冗余 `.tar.gz`（F-008/F-NEW-003）；`web/package.json:8,10` build 改纯 `vite build`（F-010/F-NEW-004）；`release.yml:78` rust-cache key 改 `${{ matrix.os }}-${{ matrix.target }}`（F-011/F-NEW-005）
  - 受影响：`.github/workflows/release.yml`、`Cargo.toml`、`web/package.json`
  - 可并行：是
  - 状态来源：NOT-FIXED（F-006/F-007）+ NEW（F-NEW-002~005 为同项残留折射）

### 修复阶段 Task Dependencies

- 阶段 1（Task 9-12）：互不依赖，可全部并行；Task 12 建议最先做以解除本地编译阻断
- 阶段 2（Task 13-18）：Task 14 与 Task 15 同文件建议合并；其余可并行；Task 16（测试）可在阶段 1 完成后即开始
- 阶段 3（Task 19-23）：互不依赖，可全部并行；建议在阶段 2 完成后启动
- 阶段 4（Task 24-25）：Task 25 跨前后端需协调；Task 24 独立
- 阶段 5（Task 26-32）：可在任何时间滚动迭代，按视角分批
