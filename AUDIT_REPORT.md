# ForgeClaw 多视角审计报告

> change-id: `audit-bugs-and-optimizations`
> 审计日期: 2026-07-05
> 审计方式: 6 个独立子代理并行审计（视角 A-F），主代理汇总
> 审计范围: 全部 Rust crate（core/llm/tools/server/cli）+ Vue 3 WebUI + GitHub Actions + Cargo/pnpm 配置

## 摘要

| 视角 | 范围 | P0 | P1 | P2 | 小计 |
|------|------|----|----|----|----|
| A | Rust 类型/错误处理（core/llm/tools） | 0 | 3 | 15 | 18 |
| B | 并发与缓存正确性 | 1 | 5 | 8 | 14 |
| C | 安全沙箱与鉴权 | 9 | 8 | 5 | 22 |
| D | API/WebSocket/编排器 | 4 | 8 | 8 | 20 |
| E | 前端 WebUI | 8 | 6 | 10 | 24 |
| F | CI/CD 与构建配置 | 2 | 4 | 5 | 11 |
| **合计** | | **24** | **34** | **51** | **109** |

### 总体健康度评估

**红色（不可发布）**：当前代码在 P0 层面有 24 条严重缺陷，覆盖安全沙箱、鉴权、并发、协议、WebUI、CI 六大方向。其中：

- **WebUI 完全不可用**（E-001 入口 HTML 损坏 + E-002 五个核心 view 全缺 + F-001 rust-embed 未集成），`forgeclaw web` 命令对前端零响应
- **安全沙箱形同虚设**（C-001~C-005）：ShellTool 通过 `sh -c` 暴露完整 shell 能力，黑名单不完整且可被 `$(...)` 绕过，子进程继承全部环境变量直接泄漏 API Key 与全部用户 token
- **鉴权开箱即破**（C-007~C-008）：配置文件默认 0644 权限含明文 token，默认绑定 `0.0.0.0:8080` 且默认 token 为公开已知的 `change-me`/`local-token`
- **append-only 语义被破坏**（B-001）：SessionData 的 read-modify-write 模式在并发下互相覆盖，违反 spec 的 DeepSeek prefix-cache 硬要求
- **编排器死循环 + 错误吞没**（D-001/D-003/D-004）：`run_turn` 无轮次上限叠加工具错误信息丢失与 LLM Error 误判为 Complete，生产环境会无限烧 token

**亮点**：`History` 类型层面的 append-only 设计、`PromptEngine` cache key 覆盖完整输入、`reqwest` 纯 rustls 实现 CGO-free、CI 6 步齐全、actions 版本基本对齐 node22。

---

## 视角 A — Rust 类型/错误处理审计（core/llm/tools）

### P1

- **A-001** [`crates/tools/src/search.rs:89,179,185`] `SearchTool`/`GrepTool` 是 `async fn` 但内部用同步阻塞 IO（`WalkDir` + `std::fs::read_to_string`），卡住 tokio worker 线程 → 包入 `tokio::task::spawn_blocking` 或改用 `tokio::fs`
- **A-002** [`crates/core/src/model.rs:13-18`] `Session.messages: Vec<Message>` 是 `pub` 字段，外部可任意 mutate，与"append-only"设计意图冲突，类型系统未强制 → 改私有 + 提供 `append`/`messages()` 访问器，参照 `History` 设计
- **A-003** [`crates/tools/src/shell.rs:33-46`] 危险命令正则 `rm\s+-rf\s+/(?:\s|$|\*)` 不匹配 `rm -rf /home`/`/etc`/`/usr` 等子目录删除 → 扩展为 `rm\s+-rf\s+/(?:\S|$)` 或改白名单策略

### P2

- **A-004** [`crates/llm/src/client.rs:190-191`] `OpenAiClient.timeout` 字段标 `#[allow(dead_code)]` 冗余 → 删除字段或实际使用
- **A-005** [`crates/llm/src/lib.rs:36`] `ChatMessage.role: String` 用裸字符串，无法阻止拼写错误 → 改 `enum Role { System, User, Assistant, Tool }` + `#[serde(rename_all = "lowercase")]`
- **A-006** [`crates/core/Cargo.toml:16`,`crates/llm/Cargo.toml:10,16`,`crates/tools/Cargo.toml:14`] 三个 crate 声明 `tracing` 依赖但源码无任何调用；`llm` 声明 `forgeclaw-core` 但无 `use` → 移除未用依赖或在关键路径补日志
- **A-007** [`crates/tools/src/shell.rs:125`] `?` 传播 `io::Error` 丢失命令文本上下文 → `.map_err(|e| anyhow!("shell execute failed (cmd={:?}): {}", command, e))`
- **A-008** [`crates/llm/src/client.rs:231-237`] 5xx 响应体被丢弃，错误上下文丢失；429 应可重试但被纳入 `is_client_error()` 直接返回 → 5xx 时读 body 入错误信息；429 单独判定重试
- **A-009** [`crates/llm/src/client.rs:170,174`] `parse_sse_stream` 只切 `\n\n`，不处理 `\r\n\r\n`/`\r\r`，CRLF 上游会卡死 → 用正则 `/\r?\n\r?\n/` 归一化
- **A-010** [`crates/tools/src/search.rs:89,179`] `filter_map(|e| e.ok())` 静默吞掉遍历错误 → `tracing::warn!` 或在 `ToolResult.error` 累计跳过计数
- **A-011** [`crates/core/src/prompt/profile.rs:51`] `parse_profile` 用 `HashMap` 收集 sections，相同 `order` 时相对顺序不确定，导致 cache key 不稳定 → 改 `BTreeMap` 或按 key 二次排序
- **A-012** [`crates/core/src/prompt/section.rs:79`] `trim_matches('"')` 不处理 YAML 转义引号 → 实现最小转义或文档明示不支持
- **A-013** [`crates/llm/src/lib.rs:44-71`] `ChatMessage` 缺 `tool` 角色构造器，调用方易遗漏 `tool_call_id` → 增加 `pub fn tool(content, tool_call_id)`
- **A-014** [`crates/llm/src/client.rs:87-90`] `parse_sse_events` 对非法 JSON `Err(_) => continue` 静默跳过 → `tracing::warn!` 或产出 `Event::Error`
- **A-015** [`crates/tools/src/file.rs:66-76`] `expand_tilde` 在 `HOME` 未设置时返回 `"~"` 字符串本身，后续路径拼接异常 → 用 `dirs::home_dir()` 或返回错误
- **A-016** [`crates/tools/src/shell.rs:75-81`] `ShellTool::check` 中 `command` 缺失时默认 `""` 返回 `Allow`，与 `execute` 报错不一致 → 缺失时返回 `Critical`/`Confirm`
- **A-017** [`crates/tools/src/shell.rs:48`] `Regex::new(...).expect(...)` 是生产代码唯一 panic 点 → 补单元测试覆盖或改 `lazy_static` + const 构造
- **A-018** [`crates/llm/src/client.rs:233-235`] 429 被纳入 `is_client_error()` 不重试，与"连接错误与 5xx 重试"策略不一致 → 429 单独拆出纳入重试集合（读 `Retry-After`）

---

## 视角 B — 并发与缓存正确性审计

### P0

- **B-001** [`crates/server/src/ws.rs:108-168` + `crates/server/src/api.rs:123-180`] **并发 read-modify-write 竞态破坏 append-only**。`chat_handler`（REST）与 `handle_text_frame`（WS）都采用「read lock → clone SessionData → 释放锁 → 跑 LLM（数十秒）→ write lock → insert 覆盖」模式。同一 session 并发请求时后完成者整段覆盖先完成者写入的 history，导致消息丢失、前缀字节不稳定。`ws.rs:111` 显式允许同 user_id 跨连接共享 session 使该竞态在生产可达路径 → 修复方向：把 `SessionData.history` 包裹为 `Arc<RwLock<History>>` 原地 append；或加 per-session 锁序列化 LLM 调用；或乐观版本号写回

### P1

- **B-002** [`crates/server/src/ws.rs:130-133,156`] spawn 任务 panic 被 `if let Ok(...)` 静默吞掉，不回写也不通知客户端，无 tracing → `match join.await` 三分支显式处理
- **B-003** [`crates/server/src/ws.rs:130-152`] WS 客户端断连后 LLM 任务继续运行（`tx.send` 用 `let _ =` 忽略），持续烧 token → `tx.send` 失败应中止传播，或主循环 break 后 `join.abort()`
- **B-004** [`crates/server/src/orchestrator.rs:74,114-119,142-147` + `crates/core/src/prompt/engine.rs:85-113`] `PromptEngine` 用 `tokio::Mutex` 包裹且 compile 内含同步阻塞文件 IO，慢盘下阻塞所有 compile 请求 → 文件读取改 `tokio::fs` 在锁外完成；或 cache 拆 `Arc<RwLock>` + `AtomicUsize`，compile 退化为 `&self`
- **B-005** [`crates/server/src/ws.rs:136-167`] `got_complete=false` 时仍 `sessions.insert` 写回 final_data，导致 `session.messages`（缺 assistant）与 `history.messages()`（完整）状态分裂 → 未收 Complete 不回写，或从 history 重建 session.messages
- **B-006** [`crates/server/src/api.rs:141-180`] `chat_handler` 在 LLM 失败时丢弃已 push 的 user 消息，下轮 LLM 看不到上轮用户输入 → 失败时仍 write lock 把 user 消息 append 到原始 SessionData

### P2

- **B-007** [`crates/core/src/prompt/engine.rs:26,90-112`] cache HashMap 无上限，可被不同 vars 触发无限增长直至 OOM → 加 LRU 上限
- **B-008** [`crates/server/src/orchestrator.rs:244`] 每轮 LLM 调用 `tool_specs.to_vec()` 深拷贝整组 ToolSpec → `ChatRequest.tools` 改 `Option<Arc<[ToolSpec]>>`
- **B-009** [`crates/server/src/orchestrator.rs:114-119,142-147`] `compile_prompt`/`compile_system_prompt` 锁粒度过大覆盖整个 compile → 配合 B-004 拆细
- **B-010** [`crates/server/src/api.rs:34-40` + `crates/llm/src/lib.rs:152-193` + `crates/core/src/model.rs:13-26`] `SessionData` 维护两份消息存储（`session.messages` 与 `history`），易分裂 → 单一真源，`session.messages` 由 `history.messages()` 按需映射
- **B-011** [`crates/server/src/api.rs:243-253`] `compile_prompt` 接受任意 profile 名，错误信息可能泄漏文件系统路径 → profile 名白名单 + 错误脱敏
- **B-012** [`crates/server/src/api.rs:123-140` + `crates/server/src/ws.rs:108-122`] read lock 内深拷贝整个 SessionData 阻塞写者 → 配合 B-001 改 `Arc<RwLock<History>>` O(1) clone
- **B-013** [`crates/server/src/orchestrator.rs:128-140` + `crates/cli/src/commands.rs:131-140`] `prompt_vars` 在 orchestrator 与 cli 各有一份拷贝易漂移 → 提到 `forgeclaw_core` 作为单一真源
- **B-014** [`crates/server/src/orchestrator.rs:155-158,177-180`] 用 `history.is_empty()` 判定是否注入 system prompt，对"非空但缺 system 前缀"的 history 无保护 → 检查 `messages().first()` 是否为 system

---

## 视角 C — 安全沙箱与鉴权审计

### P0

- **C-001** [`crates/tools/src/shell.rs:31-50`] ShellTool 危险命令黑名单严重不完整，缺失 `sudo`/`su`/`eval`/`curl|sh`/`wget|bash`/`chmod 777`（无 -R）/`chown`/`cat /etc/passwd`/`cat ~/.ssh/id_rsa`/`env`/`mv`/`cp`/`tee` 写 `/etc/`/`bash -i >& /dev/tcp`/`nc`/`mkfifo` → 放弃单一正则，改 `shell-words` 解析 argv + 关键字 allowlist；长期用 `landlock`/`bubblewrap`/`firejail` 真沙箱
- **C-002** [`crates/tools/src/shell.rs:35`] 正则锚点 `(?:^|\s)` 可被 `$(rm -rf /)`、`` `rm -rf /` ``、`echo $(rm -rf /)` 绕过（`rm` 前是 `(` 或 `` ` `` 不是空白）→ 去掉前瞻直接子串匹配，或解析 shell AST
- **C-003** [`crates/tools/src/shell.rs:36`] `rm\s+-rf\s+/(?:\s|$|\*)` 不拦截 `rm -rf /home`/`/usr`/`/var`/`/boot` → 改 `rm\s+-rf\s+/(?:\S|$)`
- **C-004** [`crates/tools/src/shell.rs:120-125`] `tokio::process::Command::new("sh")` 继承全部环境变量，包括 `DEEPSEEK_API_KEY`/`FORGECLAW_API_KEY`/`FORGECLAW_USERS`（含全部用户 token 明文）。LLM 仅需 `printenv FORGECLAW_USERS` 即可绕过鉴权 → `Command::env_clear()` 后仅注入 allowlist（`PATH`/`HOME`，剔除 `*API_KEY*`/`*TOKEN*`/`FORGECLAW_USERS`/`*SECRET*`）
- **C-005** [`crates/tools/src/shell.rs:120-125`] ShellTool 仅用 `current_dir` 限制 cwd，对文件系统/网络/进程无任何沙箱约束，可 `cd /` 后读写任意文件、`curl` 外联、`pkill` 杀进程 → 引入 `landlock` 限制文件系统范围 + `seccomp` 限制系统调用；或 `bubblewrap`/`firejail` 包裹
- **C-006** [`crates/server/src/ws.rs:44-50` + `crates/server/src/lib.rs:69`] WS token 通过 URL query（`/ws/chat?token=xxx`），`TraceLayer::new_for_http()` 默认把 `request.uri`（含 query）写入 span。任何能访问日志/代理日志/浏览器历史的人都能读取全部用户 token → 改用 `Sec-WebSocket-Protocol` 子协议传 token；或一次性 ticket（60s）；并 `make_span_with` 脱敏 query
- **C-007** [`crates/cli/src/config.rs:95-103`] `Config::save()` 用 `std::fs::write` 写入 `~/.forgeclaw/config.toml` 默认 0644，含 LLM `api_key` 与用户 token 明文，同主机其他用户可读 → 写入后 `set_permissions(0o600)`；或用 `keyring` crate 存入 OS keychain
- **C-008** [`crates/cli/src/commands.rs:418,453` + `crates/cli/src/config.rs:87-92`] `run_web` 默认绑定 `0.0.0.0:8080`，无配置时回退 `[("local", "local-token")]`，`Config::default_for_init` 用 `("local", "change-me")`。任何能访问端口者都能用公开已知 token 通过鉴权 → 默认绑 `127.0.0.1`；启动时检测默认 token 拒绝启动；`config init` 用 `Uuid::new_v4()` 生成随机 token
- **C-009** [`crates/server/src/ws.rs:108-122,166-168`] WS 处理器在 session_id 属于其他用户时**复用同一 session_id 创建新 SessionData 并回写**，覆盖原所有者会话。攻击者 B 猜中 A 的 session_id 发一条消息即可销毁 A 的会话。注释"视为新建（不泄漏、不混用）"与实现不符（覆盖式混用）→ WS 命中跨用户 session 时生成新 `Uuid` 或返回错误帧关闭连接，与 REST 行为对齐

### P1

- **C-010** [`crates/server/src/auth.rs:164`] `if user.token != req.token` 非常量时间比较，可时序探测 → `subtle::ConstantTimeEq` + 先比较长度
- **C-011** [`crates/server/src/auth.rs:156-167` + `crates/server/src/lib.rs:67`] `/api/auth/login` 无速率限制/锁定，可暴力短 token → `tower_governor` 按 IP+name 限流（5 次/分钟）
- **C-012** [`crates/server/src/api.rs:149,251,263`] `e.to_string()` 直接进 500 响应体，可能含 URL/api_key 片段/路径 → `tracing::error!(?e)` 落日志，客户端只返回 `"internal server error"`
- **C-013** [`crates/cli/src/commands.rs:504`] `run_config_set` 末尾 `println!("已设置 {key} = {value}")`，`key == "api_key"` 时把明文 key 打到 stdout → 复用 `config::mask_key`
- **C-014** [`crates/server/src/lib.rs:70`] `CorsLayer::permissive()` 设 `*` origin + 全 method + 全 header → `CorsLayer::new().allow_origin([...]).allow_methods([GET,POST]).allow_headers([AUTHORIZATION,CONTENT_TYPE])`
- **C-015** [`crates/tools/src/file.rs:194-223`] `FileWriteTool::execute` 先 `is_within` 检查（基于 `canonicalize`）再用原始未规范化 path 写入，存在 TOCTOU 窗口可被符号链接逃逸 → 用 `O_NOFOLLOW` 拒绝跟随符号链接；或对规范化路径写
- **C-016** [`crates/server/src/auth.rs:148-152,167`] `LoginResponse` 内嵌完整 `User`（含 token）返回，`User: Debug` 派生使后续任何 `tracing::debug!(?user)` 打印 token → 定义 `UserPublic { id, name }` 不含 token；用 `secrecy::Secret<String>` 包裹
- **C-017** [`crates/tools/src/sandbox.rs:85-87` + `crates/server/src/orchestrator.rs:431,459` + `crates/server/src/lib.rs:90`] server 模式装配 `auto_confirm()` 永远返回 true，所有 Confirm 级工具（含 `FileWriteTool`）被自动放行 → server 模式引入"确认队列"：Confirm 级产生 `OrchestratorEvent::ConfirmationRequired` 推前端 approve

### P2

- **C-018** [`crates/tools/src/file.rs:41-63`] `is_sensitive_path` 硬编码 Unix 路径，未覆盖 Windows `C:\Windows`/`AppData`/UNC → `cfg!(target_os = "windows")` 分支补充
- **C-019** [`crates/server/src/auth.rs:19-24`] `User: Debug` 派生会打印 token → 手写 `Debug` 跳过 token 或 `secrecy::Secret`
- **C-020** [`crates/server/src/auth.rs:78-80`] `find_by_token` 用 `HashMap::get` 非常量时间 → 配合 C-010 用 `subtle::ConstantTimeEq`
- **C-021** [`crates/tools/src/file.rs:66-76`] `expand_tilde` 信任 `HOME` 环境变量，可被 shell 注入操纵 → 用 `dirs::home_dir()`
- **C-022** [`crates/tools/src/search.rs:66,145`] `SearchTool`/`GrepTool` 的 `max` 无上限，可传巨值造成内存放大 → `.min(1000)` 服务端硬上限

---

## 视角 D — API/WebSocket 协议与编排器审计

### P0

- **D-001** [`crates/server/src/orchestrator.rs:238`] `run_turn` 的 `loop {}` 无轮次上限，LLM 持续调用工具或反复重试会无限循环直至 token 耗尽。`dispatch_subagent` 有 `for _round in 0..5` 上限但主入口无防护 → 引入最大轮次（如 25），超出返回 `OrchestratorEvent::Error`；同时给单轮 LLM/工具加超时
- **D-002** [`crates/server/src/ws.rs:108-122,167`] WS 跨用户访问既存 session_id 时 `sessions.insert(session_id, final_data)` **覆盖**原所有者会话，是数据破坏而非"不泄漏"。REST 同场景返回 404，WS 与 REST 行为不一致且 WS 更危险 → WS 命中跨用户 session 时 `ControlFlow::Break` 或发 Error 后 Continue，绝不覆盖（与 C-009 同一问题，跨视角命中）
- **D-003** [`crates/server/src/orchestrator.rs:319-326,340-345`] 工具执行失败时 `ToolResult { output: String::new(), error: Some(...) }`，但回填给 LLM 的 `tool_msg.content = result.output`（空字符串），`error` 字段从未进入 history。LLM 收到空 tool response 看不到失败原因无法修正，与 D-001 叠加加速死循环 → `tool_msg.content` 在 `result.error.is_some()` 时改为 `format!("error: {}", e)` 或序列化整个 `ToolResult`
- **D-004** [`crates/server/src/orchestrator.rs:276-281`] `Event::Error(message)` 仅 `warn!` + 推事件，**不 break 不 return**，循环继续，stream 结束后落入 `tcs.is_empty()` 分支返回 `Complete { text: "" }`，把 LLM 错误误判为"成功完成空回复"，错误被吞 → `Event::Error` 后 `return Ok(OrchestratorEvent::Error { message })`

### P1

- **D-005** [`crates/server/src/ws.rs:58-77`] WS 完全无心跳/ping-pong 与空闲超时，`Message::Ping`/`Pong`/`Binary` 落入 `_ => {}` 丢弃，半开连接永久挂着，spawn 任务与 sessions 内存泄漏 → 定时任务每 30s 发 Ping + `tokio::select!` 监听 pong 超时
- **D-006** [`crates/server/src/ws.rs:130-133,138-153`] 客户端断开后主循环 `break` + `drop(rx)`，但 spawn 的 `run_streaming` 任务仍在运行，`tx.send` 用 `let _ =` 忽略错误，LLM 循环继续烧 token（与 B-003 同根因）→ `join.abort()` 或 `tokio::select!` 监听 `tx.closed()`
- **D-007** [`crates/server/src/lib.rs:55-77`] 装配中无 `DefaultBodyLimit` 或 `ConcurrencyLimitLayer`，axum 默认不限制请求体大小，`/api/chat` 的 `message: String` 无上限，恶意客户端可巨型 JSON OOM → `.layer(DefaultBodyLimit::max(1 MiB))`，上传端点单独放宽
- **D-008** [`crates/server/src/lib.rs:72-75`] `TimeoutLayer::with_status_code(504, 300s)` 全局覆盖所有 HTTP 路由，LLM 慢响应 + 多轮工具循环易超 300s 被截断返回 504，但 orchestrator spawn 任务仍持锁运行（与 D-006 同源）→ REST 同步端点调短（120s）；流式端点排除在 TimeoutLayer 外
- **D-009** [`crates/server/src/api.rs:149,251,263` + `crates/server/src/ws.rs:91-95`] `e.to_string()` 直接作 500 响应体返回，含文件路径/IO 细节/上游 API URL/key 片段。WS `OrchestratorEvent::Error { message }` 同样直传前端（与 C-012 同一问题）→ 500 统一返回 `"internal server error"`，详细 `tracing::error!` 落日志
- **D-010** [`crates/server/src/lib.rs:70`] `CorsLayer::permissive()` 允许任意 origin/method/header（与 C-014 同一问题）→ 从配置读 origin 白名单
- **D-011** [`crates/server/src/ws.rs:138-168`] `handle_text_frame` 串行：主循环 `rx.recv()` 结束后才 `join.await`，若 spawn 任务卡住 `join.await` 永远阻塞，无法处理后续消息（与 D-006 同根因但影响可用性）→ `tokio::time::timeout(120s, join.await)` 或 `tokio::select!` 同时监听 `rx.recv()`/`socket.recv()`/`join`
- **D-012** [`crates/server/src/orchestrator.rs:372-377`] `parse_tool_input` 在 `serde_json::from_str` 失败时返回 `Value::Null` 喂给工具，工具可能 panic 或行为异常，错误信息又走 D-003 路径丢失 → 解析失败时直接构造 `ToolResult { error: Some("invalid arguments json: ...") }` 回填，跳过工具执行

### P2

- **D-013** [`crates/server/src/lib.rs:69-75`] 中间件顺序（栈式后 layer 先执行）：当前 TimeoutLayer → CompressionLayer → CorsLayer → TraceLayer → handler。CorsLayer 非最外层、TraceLayer 在最内层超时被截断的 span 不完整 → 调整为 `.layer(TimeoutLayer).layer(CompressionLayer).layer(TraceLayer).layer(CorsLayer)`
- **D-014** [`crates/server/tests/api_test.rs`] `chat_handler`（最核心端点）无集成测试覆盖 → 补 `post_api_chat_*` 端到端测试
- **D-015** [`crates/server/tests/auth_test.rs:331-359`] WS 实际消息流（升级后收发）未集成测试，仅断言升级前 426，依赖 axum 内部行为有 flaky 风险 → 用 `tokio-tungstenite` 起真实 WS 连接做集成测试
- **D-016** [`crates/server/src/api.rs:30,46`] `sessions: Arc<RwLock<HashMap>>` 纯内存，进程重启全丢；多实例水平扩展时各实例会话独立 → 短期 README 标注"单实例"；中期抽 `SessionStore` trait，内存/Redis 双实现
- **D-017** [`crates/server/src/ws.rs:44-50`] token 走 `?token=` query 会出现在 access log/proxy log/浏览器历史/Referer（与 C-006 同一问题）→ 短期 access_token + refresh_token，WS query token 5 分钟过期
- **D-018** [`crates/server/src/orchestrator.rs:244`] `tool_specs.to_vec()` 每轮 LLM 调用 clone（与 B-008 同一问题）→ `ChatRequest.tools` 改 `Option<Arc<[ToolSpec]>>`
- **D-019** [`crates/server/src/orchestrator.rs:114-119,142-147`] `compile_system_prompt` 与 `compile_prompt`/`list_sections` 都锁 `prompt_engine`，`compile` 是 `&mut self` 导致必须 Mutex（与 B-004/B-009 同一问题）→ 若 compile 无副作用改 `&self` + `RwLock`
- **D-020** [`crates/server/src/ws.rs:86-98`] WS 文本帧无大小限制，配合 D-007 无 body limit 可 DoS → 配置 `WebSocketUpgrade::max_message_size(256 KiB)`

---

## 视角 E — 前端 WebUI 审计

### P0

- **E-001** [`web/index.html:1-2`] HTML 入口严重不完整，整个文件只有 2 行：`<!doctype html>` 与被截断的 `<html lang="`，缺失 `<head>`/`<body>`/`<div id="app">`/`<script src="/src/main.ts">`。Vite 无法启动/构建，WebUI 完全不可用 → 补全完整 HTML 骨架（`lang="zh-CN"`、`<meta charset>`/`viewport`、`<div id="app">`、`<script type="module" src="/src/main.ts">`）
- **E-002** [`web/src/views/`] spec 要求的 5 个核心 view（ChatView/SessionsView/PromptsView/ToolsView/SettingsView）**全部缺失**，仅有脚手架占位 `HomeView.vue` → 创建 5 个 view 组件并实现业务逻辑
- **E-003** [`web/src/router/index.ts:6-8`] 路由表只注册 `{ path: '/', component: HomeView }`，缺 5 条业务路由与 404 fallback，无 `meta.requiresAuth` → 补 5 条懒加载路由 + `:pathMatch(.*)*` 兜底
- **E-004** [`web/src/router/index.ts:4-9`] 路由守卫完全缺失，无 `router.beforeEach`，未授权可直接访问任意路由 → 添加全局前置守卫校验 token
- **E-005** [`web/src/main.ts:1-10` & `web/src/`] 无任何 API 客户端封装，无统一 fetch wrapper，401 跳登录/错误处理/超时重试全部缺失 → 创建统一 HTTP 客户端封装
- **E-006** [`web/src/stores/.gitkeep`] `stores/` 仅空 `.gitkeep`，`main.ts` `app.use(createPinia())` 但无任何 store 实现 → 实现 `useAuthStore`/`useSessionStore`/`useSettingsStore` 等核心 store
- **E-007** [`web/src/App.vue:1-5`] `App.vue` 仅 `<router-view />`，无布局/侧边栏/导航菜单/Header，view 间无法跳转 → 在 App.vue 实现导航骨架
- **E-008** [`web/src/views/HomeView.vue:6`] 文案 `"WebUI scaffold. Full pages arrive in Task 7."` 证实当前仅为脚手架 → 5 个 view 实现后将 HomeView 改造为重定向到 `/chat` 或登录页

### P1

- **E-009** [`web/src/`] 流式渲染审计项无法落实：ChatView 完全缺失 → 实现 ChatView 时用 `ReadableStream`/`EventSource` 增量解析，`requestAnimationFrame` 节流，按 `seq` 字段排序避免乱序
- **E-010** [`web/src/`] WebSocket 清理审计项无法落实 → 在 ChatView/工具 view 的 `onBeforeUnmount` 显式 `ws.close()`，重连用指数退避最大 5 次
- **E-011** [`web/src/components/HelloWorld.vue:1-94`] Vite 默认模板"Get started"页，未被路由引用，属脚手架残留 → 删除 `HelloWorld.vue` 与 `src/assets/{hero.png,vite.svg,vue.svg}`
- **E-012** [`web/src/style.css:101-266`] Vite 模板样式（`.counter`/`.hero`/`#next-steps` 等），`#app` 写死 `width: 1126px` → 替换为项目实际设计 token，`#app` 不要写死固定宽度
- **E-013** [`web/README.md:1-3`] 仍是"Vue 3 + TypeScript + Vite"模板文案 → 更新为 ForgeClaw WebUI 说明
- **E-014** [`web/public/icons.svg:1-24`] 含 bluesky/discord/github/x 等 Vite 社交图标，与 ForgeClaw 无关 → 删除或替换

### P2

- **E-015** [`web/package.json:24`] `typescript: ~6.0.2` 版本号异常（2025-08 最新稳定版为 5.x），`pnpm install` 会失败 → 改 `~5.6.0`
- **E-016** [`web/package.json:25`] `vite: ^8.1.1` 版本号异常（主流 5.x/6.x）→ 改 `^5.4.0` 或 `^6.0.0`
- **E-017** [`web/package.json:18`] `vue-router: ^5.1.0` 版本号异常（当前最新 4.x）→ 改 `^4.4.0`
- **E-018** [`web/package.json:13-15`] `@vueuse/core`/`monaco-editor`/`naive-ui` 三项依赖在 `src/` 下无任何引用 → 暂时移除，`monaco-editor` 体积大需评估按需引入
- **E-019** [`web/tsconfig.app.json:1-15`] 未显式开启 `strict`/`noImplicitAny`/`strictNullChecks` → 显式声明
- **E-020** [`web/src/router/index.ts:2`] `HomeView` 静态 import 未用懒加载 → 改 `() => import(...)` 实现代码分割
- **E-021** [`web/src/components/HelloWorld.vue:13`] hero 主图 `alt=""` 影响屏幕阅读器 → 给主图提供有意义的 `alt`（删除组件时随之消失）
- **E-022** [`web/index.html:2`] `<html lang="` 截断，无 charset/viewport/title → 与 E-001 一并修复
- **E-023** [`web/vite.config.ts:5-19`] `build.outDir` 未显式指定，若 rust-embed 期望固定路径可能错配 → 显式声明并与 Rust 端 `#[derive(RustEmbed)] #[folder = "..."]` 保持一致
- **E-024** [`web/src/router/index.ts:5`] 使用 `createWebHashHistory()` 对嵌入二进制场景合理（正面记录，无需修复）

---

## 视角 F — CI/CD 与构建配置审计

### P0

- **F-001** [`crates/server/Cargo.toml:26` + `crates/server/src/lib.rs:55-77` + `crates/server/src/api.rs`] **rust-embed 仅声明依赖未实际集成**。`Cargo.toml` 写了 `rust-embed = { workspace = true }`，但 `crates/server/src/` 全量 Grep `rust-embed|RustEmbed|include_dir` 仅 Cargo.toml 命中 1 行，源码 0 命中。`lib.rs:55-77` 的 `app()` 只有 `/api/*` 与 `/ws/chat` 路由，无任何 fallback/nest_service/ServeDir 提供 `web/dist`。`vite.config.ts:6-8` 注释明确写 "built bundle works when embedded into the Rust binary via rust-embed" 但后端从未 `#[derive(RustEmbed)]`。`forgeclaw-cli web` 启动后浏览器访问 `/`、`/index.html`、`/assets/*` 全部 404，spec "WebUI 通过 rust-embed 嵌入" 落空 → 在 `crates/server/src/lib.rs` 增加 `#[derive(RustEmbed)] #[folder = "$CARGO_MANIFEST_DIR/../../web/dist"] struct Asset;`，写 `static_handler` 处理 `/{*path}` fallback（先查 `Asset::get(path)` 未命中回 `index.html`），在 `app()` 末尾 `.fallback(static_handler)`
- **F-002** [`.github/workflows/ci.yml:9-30` vs `32-60`] CI 的 rust job 与 frontend job 并行，无 `needs:` 关系。rust job 内无 `pnpm build`，也无 `needs: frontend`。一旦按 F-001 接入 `#[derive(RustEmbed)] #[folder = "web/dist"]`，CI 的 rust job 会因 `web/dist` 不存在编译失败（rust-embed 编译期读取该目录）。release.yml 已正确先 `pnpm build` 再 `cargo build`，ci.yml 未对齐 → 让 rust job `needs: frontend`，frontend 用 `actions/upload-artifact@v4` 上传 `web/dist`，rust job `download-artifact` 拉回；或把 frontend 步骤前置到 rust job 内

### P1

- **F-003** [`.github/workflows/release.yml:18-38`] 跨平台矩阵缺 Windows ARM64，未达 spec 的 6 组合（当前 5 项，缺 `aarch64-pc-windows-msvc`）→ 追加 `{ os: windows-latest, target: aarch64-pc-windows-msvc }`
- **F-004** [`.github/workflows/ci.yml` + `release.yml`] 缺 `concurrency` 与 `timeout-minutes`，同分支多次 push 并发跑浪费额度，job 无超时可能挂死 → ci.yml 加 `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`；release.yml 加 `cancel-in-progress: false`；每个 job 加 `timeout-minutes: 30`
- **F-005** [`.github/workflows/ci.yml`] CI 工作流无 `permissions:` 块，默认 token 权限取决于仓库设置 → ci.yml 顶层加 `permissions: { contents: read }`
- **F-006** [`.github/workflows/release.yml:8-9`] release 的 `permissions: contents: write` 写在工作流顶层对所有 job 生效 → 下移到 `build` job 内

### P2

- **F-007** [`Cargo.toml:26`] `tokio features = ["full"]` 过度，所有 crate 通过 `workspace = true` 引用无法按 crate 裁剪 → 评估实际子集（`["rt-multi-thread", "macros", "sync", "io-util", "net", "time"]`）或注释说明取舍
- **F-008** [`.github/workflows/release.yml:101-108`] release 上传产物列表对 Windows 冗余的 `.tar.gz` 行（Windows job 只产 `.zip`）→ 用 step-level 条件或两个独立 upload step
- **F-009** [`.github/workflows/ci.yml:42-44` + `release.yml:44-46`] `pnpm/action-setup@v5` 与 spec 字面要求的 `@v4` 不一致；若 v5 已发布且基于 node22 则合规，若不存在则 CI 报错升级 P0 → 确认 v5 可用，否则回退 `@v4`
- **F-010** [`.github/workflows/ci.yml:59-60` + `web/package.json:8,10`] `vue-tsc` 在 build（`vue-tsc -b`）与 typecheck（`vue-tsc --noEmit`）中重复执行，CI 时间浪费 → `build` 改纯 `vite build`，由 `typecheck` 单独保证类型
- **F-011** [`.github/workflows/release.yml:66-69`] rust-cache key 仅含 target 不含 OS，linux/macos/windows 共用同一 target key 时可能命中不兼容缓存 → `key: ${{ matrix.os }}-${{ matrix.target }}`

---

## 跨视角共性问题汇总

### 1. WS session 跨用户覆盖（C-009 / D-002 / B-001 部分相关）
WS handler 命中既存但 `user_id != current` 的 session_id 时复用 ID 创建新数据并 `insert` 覆盖原所有者会话。注释"视为新建（不泄漏、不混用）"与实现"覆盖式混用"矛盾。REST 同场景正确返回 404。**三视角独立命中同一问题**，是设计疏漏而非偶然 bug。

### 2. 500 响应体直接泄漏 `e.to_string()`（C-012 / D-009）
`chat_handler`/`compile_prompt`/`list_sections` 与 WS `OrchestratorEvent::Error { message }` 都把 `anyhow::Error` 的 Display 直传客户端，可能含文件路径/IO 细节/上游 API URL/key 片段。

### 3. `CorsLayer::permissive()`（C-014 / D-010）
允许任意 origin/method/header，当前 Bearer token 鉴权暂不直接可利用，但未来引入 cookie 凭据会立即变成 CSRF 漏洞。

### 4. WS 无生命周期管理（B-003 / D-005 / D-006 / D-011）
无心跳、无空闲超时、客户端断连后 spawn 任务不 abort、`join.await` 无超时。半开连接与断连后 LLM 任务继续运行在弱网环境会放大成资源泄漏与费用失控。**4 个视角从不同侧面命中同一根因**。

### 5. `tool_specs.to_vec()` 每轮深拷贝（B-008 / D-018）
`run_turn` 内每次 LLM 调用都 clone 整组 ToolSpec（含 JSON schema），多轮工具循环下无谓分配。

### 6. `compile_prompt` 锁粒度过大（B-004 / B-009 / D-019）
`PromptEngine` 用 `tokio::Mutex` 包裹且 compile 内含同步阻塞文件 IO，高并发下 REST `/api/prompts/compile` 与首次 `run_once`/`run_streaming` 串行化。

### 7. token 在 URL query（C-006 / D-017）
WS token 通过 `?token=` 传递，会出现在 access log/proxy log/浏览器历史/Referer header。

### 8. `let _ =` 吞错风格（D-001 / D-003 / D-004 / B-002 / B-003）
orchestrator 中 5 处 `let _ = tx.send(...)`，LLM stream Error、tool 执行错误、parse 失败的关键信号都被静默丢弃，调用方永远拿到 `Complete`，监控告警无从触发。

### 9. 默认凭证与文件权限（C-007 / C-008 / C-013）
开箱即用的部署近乎裸奔：配置文件默认 0644 含明文 token、默认绑定 `0.0.0.0:8080`、默认 token 公开已知（`change-me`/`local-token`）、`config set api_key` 把明文打到 stdout。

### 10. WebUI 完全不可用（E-001 ~ E-008 / F-001 / F-002）
HTML 入口损坏、5 个核心 view 全缺、rust-embed 未集成、CI 构建顺序错误。`forgeclaw web` 命令对前端零响应，spec "WebUI 嵌入单二进制" 完全未达成。

---

## 推荐修复顺序

### 阶段 1：阻断性 P0（必须最先修复，否则 WebUI 不可用 + CI 红）

1. **E-001** 补全 `index.html` 入口骨架
2. **F-001** rust-embed 集成：`#[derive(RustEmbed)]` + `static_handler` + `.fallback`
3. **F-002** CI 构建顺序：rust job `needs: frontend` + artifact 传递
4. **E-015/E-016/E-017** 修正 `typescript@6`/`vite@8`/`vue-router@5` 虚构版本号（否则 `pnpm install` 失败）

### 阶段 2：安全 P0（必须发布前修复，否则开箱即破）

5. **C-004** ShellTool `env_clear()` 清理敏感环境变量
6. **C-007** Config 文件权限 0600
7. **C-008** 默认绑 `127.0.0.1` + 默认 token 拒绝启动 + `config init` 生成随机 token
8. **C-006** WS token 改 `Sec-WebSocket-Protocol` 或一次性 ticket
9. **C-001/C-002/C-003** ShellTool 黑名单补全 + 锚点修复（短期缓解，长期靠 C-005）
10. **C-005** 引入 `landlock`/`bubblewrap` 真沙箱
11. **C-009/D-002** WS 跨用户 session 覆盖修复

### 阶段 3：并发与编排器 P0（必须发布前修复，否则数据损坏 + 烧 token）

12. **B-001** SessionData 改 `Arc<RwLock<History>>` 解决并发覆盖
13. **D-001** `run_turn` 引入最大轮次
14. **D-003** 工具错误信息回填 LLM
15. **D-004** LLM Error 不再误判 Complete

### 阶段 4：WebUI 业务 P0（必须发布前修复，否则无业务功能）

16. **E-002** 创建 5 个核心 view
17. **E-003/E-004** 路由表 + 守卫
18. **E-005/E-006** API 客户端封装 + pinia store
19. **E-007** App.vue 导航骨架

### 阶段 5：P1 长尾（建议发布前修复，可分批）

20. WS 生命周期（D-005/D-006/D-011 + B-002/B-003）
21. 错误处理（C-010/C-011/C-012/C-013/D-009/D-012）
22. HTTP 加固（D-007/D-008/C-014/D-010/D-020）
23. 类型安全（A-002/A-005/A-013/C-016/C-019）
24. prompt engine 并发（B-004/B-009/D-019）
25. CI 加固（F-003/F-004/F-005/F-006）

### 阶段 6：P2 优化（可滚动迭代）

26. 性能优化（A-001/B-008/D-018/F-007）
27. 代码异味（A-004/A-006/A-010/A-014）
28. 测试覆盖（D-014/D-015）
29. 跨平台（C-018/C-021）
30. 前端清理（E-011/E-012/E-013/E-014/E-018）
