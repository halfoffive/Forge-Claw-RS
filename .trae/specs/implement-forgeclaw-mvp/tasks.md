# Tasks

> change-id: `implement-forgeclaw-mvp`
> 实现遵循 karpathy CLAUDE.md（先思考/最小化/外科手术式/目标驱动）与 frontend-design SKILL（WebUI）。
> 无依赖任务可并行派发子代理，主代理汇总。

- [x] Task 0: 脚手架与依赖锁定
  - [ ] SubTask 0.1: 初始化 Cargo workspace（根 `Cargo.toml` + 5 crate：core/llm/tools/server/cli），workspace 划分原则：core 无 IO，其余依赖 core，cli 组装
  - [ ] SubTask 0.2: 初始化 `web/` Vue 3 + Vite 项目（package.json/vite.config.ts/tsconfig.json/index.html/src/main.ts/App.vue）
  - [ ] SubTask 0.3: 锁定 Rust 依赖版本（clap 4.6/tokio 1.52/axum 0.8/tower 0.5/tower-http 0.7/reqwest 0.13[+rustls-tls]/serde 1.0/serde_json 1.0/thiserror 2.0/anyhow 1.0/tracing 0.1/tracing-subscriber 0.3/uuid 1.23/chrono 0.4 + rust-embed/ rustyline）
  - [ ] SubTask 0.4: 锁定前端依赖版本（vue 3.5/vite 8.1/@vitejs/plugin-vue 6.0/vue-router 5.1/pinia 3.0/naive-ui 2.44/typescript 6.0[失败回退 ~5.9]/monaco-editor 0.55/@vueuse/core 14.3）
  - [ ] SubTask 0.5: 创建提示词目录 `prompts/sections/*.md` + `prompts/profiles/default.toml`（占位，章节内容由 Task 1 填充）
  - 验证：`cargo build` 全部 crate 成功；`pnpm install && pnpm build` 前端构建成功

- [x] Task 1: 提示词引擎（core/prompt）
  - [ ] SubTask 1.1: 实现 `model.rs` 领域模型（Session/Message/AssistantMsg/ToolCall/ToolResult/Section/SafetyLevel）
  - [ ] SubTask 1.2: 实现 `prompt/section.rs`：解析带 YAML frontmatter 的 Markdown Section 文件
  - [ ] SubTask 1.3: 实现 `prompt/profile.rs`：从 TOML 加载 Profile 与 Section 引用
  - [ ] SubTask 1.4: 实现 `prompt/engine.rs`：PromptEngine 编译流程（排序/过滤/变量注入 `{{tools}}{{model}}{{cwd}}`/哈希缓存）
  - [ ] SubTask 1.5: 编写 4 个章节源文件 identity/safety/tools/style，风格仿 CLAUDE-FABLE-5
  - 验证：`cargo test -p forgeclaw-core` 通过；编译输出含四章节且变量已替换
  - 依赖：Task 0

- [x] Task 2: LLM 适配器（llm）
  - [ ] SubTask 2.1: 定义 `LlmClient` trait（`async fn chat(&self, req: ChatRequest) -> Stream<Event>`）
  - [ ] SubTask 2.2: 实现 `OpenAiClient`（兼容 DeepSeek/GLM），reqwest + rustls-tls，SSE 流式解析、超时、重试
  - [ ] SubTask 2.3: 实现 cache-first append-only 历史：保证 system prompt 前缀字节稳定，仅追加新消息
  - 验证：`cargo test -p forgeclaw-llm` SSE 解析单测通过；`forgeclaw chat "你好"` 流式返回（需 API Key）
  - 依赖：Task 1

- [x] Task 3: 工具沙箱（tools）
  - [ ] SubTask 3.1: 定义 `Tool` trait（name/schema/execute）与 `ToolResult`
  - [ ] SubTask 3.2: 实现 `ShellTool` + 危险命令黑名单（`rm -rf /`/`git push --force` 等）+ 工作目录限制
  - [ ] SubTask 3.3: 实现 `FileReadTool`/`FileWriteTool`，敏感路径保护（/etc、~/.ssh 禁写），工作目录硬限制
  - [ ] SubTask 3.4: 实现 `SearchTool`/`GrepTool`
  - [ ] SubTask 3.5: 实现 `Sandbox` 装配（critical block / confirm 确认 / allow 放行）
  - 验证：`cargo test -p forgeclaw-tools` 覆盖拦截场景；`forgeclaw tool exec shell -- ls -la` 在限定目录执行；`rm -rf /` 被拦截
  - 依赖：Task 1

- [x] Task 4: Agent 编排与多子代理（core + server）
  - [ ] SubTask 4.1: 实现 `AgentOrchestrator` 消息循环（LLM 调用→工具调度→结果回填→再调用）
  - [ ] SubTask 4.2: 实现多子代理协作：派发 explore/research/review 子任务，隔离上下文与受限工具，仅回汇总
  - [ ] SubTask 4.3: 实现 axum REST 路由（POST /api/chat、GET /api/sessions[/:id]、GET /api/tools、POST /api/prompts/compile、GET /api/prompts/sections）
  - [ ] SubTask 4.4: 实现 WebSocket `/ws/chat`（流式 token + 工具事件）
  - [ ] SubTask 4.5: 集成 tower-http（TraceLayer/CorsLayer/CompressionLayer/超时）
  - 验证：`forgeclaw chat "在当前目录建一个 hello.rs 并运行"` 全流程跑通；`cargo test --test '*'` 集成测试通过
  - 依赖：Task 2、Task 3

- [x] Task 5: 多用户鉴权（server）
  - [ ] SubTask 5.1: 实现用户模型与会话隔离（每用户独立会话/配置）
  - [ ] SubTask 5.2: 实现 token/API Key 认证中间件，未认证返回 401
  - [ ] SubTask 5.3: 鉴权贯穿 REST 与 WebSocket
  - 验证：未带 token 访问 `/api/sessions` 返回 401；用户 A 查不到用户 B 会话
  - 依赖：Task 4

- [x] Task 6: CLI（cli）
  - [x] SubTask 6.1: clap derive 主命令 + 子命令（chat/run/prompt/tool/web/config）+ 全局选项（-p/-m/-v）
  - [x] SubTask 6.2: `chat` REPL（rustyline）+ 斜杠命令（/tool ls、/model、/exit）+ 流式着色
  - [x] SubTask 6.3: `run` 单次任务、`prompt compile`/`list-sections`、`tool list`/`exec`、`web --port`、`config`
  - 验证：成功标准 §1.2 的 1–3（chat 流式写文件、tool exec 沙箱执行、prompt compile 输出）
  - 依赖：Task 4、Task 5

- [ ] Task 7: WebUI（web）+ rust-embed 嵌入
  - [ ] SubTask 7.1: Vue 路由 + 5 页面（ChatView/SessionsView/PromptsView/ToolsView/SettingsView）
  - [ ] SubTask 7.2: Pinia store（useChatStore/useSessionStore/usePromptStore/useConfigStore）
  - [ ] SubTask 7.3: ChatView 流式渲染 + ToolCallCard 工具调用卡片（可折叠）
  - [ ] SubTask 7.4: PromptsView Monaco 章节编辑器 + profile 组装
  - [ ] SubTask 7.5: WebUI 视觉设计遵循 frontend-design SKILL（brainstorm→plan→build→critique，避免三类 AI 默认外观，单一签名元素）
  - [ ] SubTask 7.6: server 用 rust-embed 嵌入 Vite 构建产物，`forgeclaw web` 直接提供前端
  - 验证：成功标准 §1.2 的 4（`forgeclaw web` 启动浏览器可对话看工具链）；`pnpm build` + `vue-tsc --noEmit` 通过；单二进制提供前端
  - 依赖：Task 4、Task 5

- [x] Task 8: GitHub Actions CI/CD
  - [ ] SubTask 8.1: `.github/workflows/ci.yml`：PR/push 触发，Rust（fmt --check/clippy -D warnings/test）+ 前端（pnpm build/vue-tsc --noEmit）
  - [ ] SubTask 8.2: `.github/workflows/release.yml`：tag `v*` 触发，跨平台构建（linux/macos/windows × amd64/arm64，CGO-free），上传单二进制产物
  - [ ] SubTask 8.3: 使用 node22 最新 actions（actions/checkout@v5、actions/setup-node@v5[node 22]、pnpm/action-setup@v4、dtolnay/rust-toolchain@stable、Swatinem/rust-cache@v2、taiki-e/install-action）
  - 验证：工作流 YAML 语法合法；本地 `act` 或推送后 CI 全绿；release 产物含多平台二进制
  - 依赖：Task 0（可与 Task 1-7 并行起步，但需项目结构就绪）

- [ ] Task 9: 测试与文档
  - [ ] SubTask 9.1: 单测覆盖 core/llm/tools 核心路径
  - [ ] SubTask 9.2: 集成测覆盖 CLI 全流程与 API 端到端
  - [ ] SubTask 9.3: README + docs/ 架构说明（仓库结构/架构图/提示词引擎/缓存优先/沙箱/鉴权/CI）
  - 验证：成功标准 §1.2 的 5（`cargo test` 与 `pnpm build` 通过）
  - 依赖：Task 1-8

# Task Dependencies

- Task 0 → 所有后续
- Task 1 → Task 2、Task 3
- Task 2 + Task 3 → Task 4
- Task 4 → Task 5、Task 6、Task 7
- Task 5 → Task 6（鉴权集成）、Task 7（WebUI 调用带认证）
- Task 0 → Task 8（CI 可早起步，但需结构就绪）
- Task 1-8 → Task 9

# 并行机会

- Task 1 完成后，Task 2 与 Task 3 可并行
- Task 4 完成后，Task 5、Task 6、Task 7 可部分并行（Task 6/7 依赖 Task 5 鉴权接口）
- Task 8 可在 Task 0 后与 Task 1-7 并行起草
