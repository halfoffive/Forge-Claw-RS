# Checklist

> change-id: `implement-forgeclaw-mvp`
> 逐项核对，通过的打勾。失败项在 tasks.md 新增修复任务。

## 脚手架
- [ ] Cargo workspace 含 5 crate（core/llm/tools/server/cli），`cargo build` 全绿
- [ ] `web/` Vue 项目 `pnpm install && pnpm build` 成功
- [ ] Rust 与前端依赖版本与 spec §3 一致（reqwest 启用 rustls-tls）

## 提示词引擎
- [ ] Section 支持 YAML frontmatter（id/title/level/enabled/order）
- [ ] `forgeclaw prompt compile --profile default` 输出含 identity/safety/tools/style 四章节
- [ ] 动态变量 `{{tools}}`/`{{model}}`/`{{cwd}}` 已替换
- [ ] `enabled=false` 章节被过滤
- [ ] 相同 Section 哈希命中缓存
- [ ] `cargo test -p forgeclaw-core` 通过

## LLM 适配器（缓存优先）
- [ ] `LlmClient` trait + `OpenAiClient`（兼容 DeepSeek）实现
- [ ] SSE 流式解析、超时、重试就绪
- [ ] 历史消息 append-only，system prompt 前缀字节稳定
- [ ] `forgeclaw chat "你好"` 流式返回
- [ ] `cargo test -p forgeclaw-llm` 通过

## 工具沙箱
- [ ] `Tool` trait + 5 内置工具（Shell/FileRead/FileWrite/Search/Grep）
- [ ] ShellTool 危险命令黑名单生效（`rm -rf /` 被拦截）
- [ ] FileWriteTool 拒绝写 `/etc`、`~/.ssh`，限定工作目录
- [ ] Sandbox 分层（critical block / confirm / allow）
- [ ] `forgeclaw tool exec shell -- ls -la` 在限定目录执行
- [ ] `cargo test -p forgeclaw-tools` 覆盖拦截场景

## Agent 编排与多子代理
- [ ] `AgentOrchestrator` 消息循环跑通（LLM→工具→回填→再调用）
- [ ] 多子代理协作：explore/research/review 隔离上下文仅回汇总
- [ ] axum REST 路由（chat/sessions/tools/prompts）就绪
- [ ] WebSocket `/ws/chat` 流式 token + 工具事件
- [ ] tower-http 中间件（Trace/Cors/Compression/超时）集成
- [ ] `forgeclaw chat "在当前目录建一个 hello.rs 并运行"` 全流程跑通
- [ ] `cargo test --test '*'` 集成测试通过

## 多用户鉴权
- [ ] 用户模型与会话隔离
- [ ] token/API Key 认证中间件
- [ ] 未认证访问 `/api/sessions` 返回 401
- [ ] 用户 A 查不到用户 B 会话
- [ ] 鉴权贯穿 REST 与 WebSocket

## CLI
- [x] clap 主命令 + 子命令（chat/run/prompt/tool/web/config）+ 全局选项
- [x] `chat` REPL + 斜杠命令 + 流式着色
- [x] `run` 单次任务
- [x] `prompt compile`/`list-sections`、`tool list`/`exec`、`web --port`、`config`
- [ ] 成功标准 1：`forgeclaw chat "用 Rust 写一个 hello world"` 流式返回并可写文件
- [x] 成功标准 2：沙箱执行（经 `forgeclaw-cli tool exec shell -- echo hello` 验证，直接工具执行路径）
- [x] 成功标准 3：`forgeclaw prompt compile --profile default` 输出完整 system prompt

## WebUI + rust-embed
- [ ] 5 页面（ChatView/SessionsView/PromptsView/ToolsView/SettingsView）就绪
- [ ] Pinia store（chat/session/prompt/config）实现
- [ ] ChatView 流式渲染 + ToolCallCard 折叠
- [ ] PromptsView Monaco 章节编辑器
- [ ] WebUI 视觉遵循 frontend-design SKILL（非模板化、单一签名元素、字体个性）
- [ ] rust-embed 嵌入 Vite 产物，`forgeclaw web` 单二进制提供前端
- [ ] 成功标准 4：`forgeclaw web` 启动浏览器可对话看工具链
- [ ] `pnpm build` + `vue-tsc --noEmit` 通过

## GitHub Actions CI/CD
- [ ] `ci.yml` PR/push 触发，Rust fmt/clippy/test + 前端 build/vue-tsc
- [ ] `release.yml` tag `v*` 触发，跨平台构建（linux/macos/windows × amd64/arm64，CGO-free）
- [ ] 使用 node22 最新 actions（checkout@v5/setup-node@v5[22]/pnpm-action-setup@v4/dtolnay/rust-toolchain@stable/Swatinem/rust-cache@v2/taiki-e/install-action）
- [ ] 不使用任何 node20 action
- [ ] 工作流 YAML 语法合法

## 设计技能与编码准则
- [ ] WebUI 实现遵循 frontend-design SKILL 设计流程
- [ ] 所有编码遵循 karpathy CLAUDE.md（先思考/最小化/外科手术式/目标驱动）
- [ ] 实现采用多子代理协作模式

## 测试与文档
- [ ] 单测覆盖 core/llm/tools 核心路径
- [ ] 集成测覆盖 CLI 全流程与 API 端到端
- [ ] README + docs/ 架构说明就绪
- [ ] 成功标准 5：`cargo test` 与 `pnpm build` 通过
