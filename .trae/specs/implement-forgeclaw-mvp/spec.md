# ForgeClaw MVP Spec

> change-id: `implement-forgeclaw-mvp`
> 编制日期: 2026-07-04
> 语言: 中文

## Why

当前开源生态中，没有一个方案能同时满足以下五点：CLAUDE-FABLE-5 式**结构化可组合系统提示词**、Claude Code 式**终端交互编程**、OpenHands/OpenClaw 式**工具沙箱与可拓展架构**、Hermes 式**分层安全防护**、Reasonix 式**缓存省钱（DeepSeek prefix-cache 对齐）**，并以 **Rust** 提供高性能与单二进制分发。

ForgeClaw 旨在填补此空白：一个 Rust 实现的 AI 编程 Agent，提供 CLI 与 WebUI 双入口，WebUI 通过 `rust-embed` 打包进同一二进制；默认对接 DeepSeek（OpenAI 兼容协议），采用 append-only 缓存优先循环以压低长会话成本；支持多用户鉴权；通过 GitHub Actions（node22 最新 actions）完成测试与构建。

## What Changes

- **新增** Rust Cargo workspace，划分 5 个 crate：`core` / `llm` / `tools` / `server` / `cli`
- **新增** 结构化提示词引擎：章节（Section）源文件带 frontmatter，Profile 组装，变量注入，按哈希缓存
- **新增** DeepSeek 原生 LLM 适配器：OpenAI 兼容协议，SSE 流式，**append-only 历史 + 字节稳定前缀**对齐 DeepSeek prefix-cache
- **新增** 工具沙箱：`Tool` trait + `ShellTool`/`FileReadTool`/`FileWriteTool`/`SearchTool`/`GrepTool`，工作目录硬限制 + 危险命令拦截 + 显式确认
- **新增** Agent 编排器：消息循环、工具调度、结果回填，支持**多子代理协作**（explore/research/review 等隔离子任务）
- **新增** clap CLI：`chat`/`run`/`prompt`/`tool`/`web`/`config` 子命令 + REPL + 流式着色
- **新增** axum 后端：REST + WebSocket，集成 `tower-http` 中间件
- **新增** 多用户鉴权：用户/会话隔离，API Key 或 token 认证
- **新增** Vue 3 + Vite WebUI：ChatView/SessionsView/PromptsView/ToolsView/SettingsView，通过 `rust-embed` 嵌入二进制
- **新增** 提示词章节源文件（`identity`/`safety`/`tools`/`style`），仿 CLAUDE-FABLE-5 章节化风格
- **新增** GitHub Actions：CI 测试 + 跨平台构建发布，使用 node22 最新 actions（node20 已弃用）

## Impact

- 受影响 specs: 无（全新项目）
- 受影响代码: 全新仓库 `/workspace`，当前仅有 `.gitignore`/`LICENSE`/`README.md`
- 设计参考:
  - `https://github.com/multica-ai/andrej-karpathy-skills/raw/refs/heads/main/CLAUDE.md`（编码行为准则：先思考、最小化、外科手术式改动、目标驱动）
  - `https://github.com/anthropics/skills/raw/refs/heads/main/skills/frontend-design/SKILL.md`（WebUI 独特视觉设计：非模板化、字体个性、结构即信息、克制与自省）
  - `https://raw.githubusercontent.com/elder-plinius/CL4R1T4S/09916a90583a320b3dde7ef5b9d8459ce0378a14/ANTHROPIC/CLAUDE-FABLE-5.md`（章节化系统提示词结构）
  - `https://reasonix.io/`（缓存优先循环、单二进制、plan+sandbox、子代理）

## ADDED Requirements

### Requirement: 结构化提示词引擎

系统 SHALL 提供基于章节（Section）的可组合系统提示词引擎，将提示词从代码中解耦为可编辑、可版本管理的源文件。

每个 Section 为一个带 YAML frontmatter 的 Markdown 文件，frontmatter 至少包含：`id`、`title`、`level`（critical/confirm/allow）、`enabled`、`order`。

Profile（TOML）声明启用的 Section 集合与模型提示。PromptEngine SHALL：
1. 读取 Profile，加载各 Section 源文件
2. 按 `order` 排序，过滤 `enabled=false`
3. 注入动态变量：`{{tools}}`（可用工具清单）、`{{model}}`（当前模型）、`{{cwd}}`（工作目录）
4. 拼接为最终 system prompt 字符串
5. 按 Section 内容哈希缓存编译结果

提示词章节风格 SHALL 仿照 CLAUDE-FABLE-5：分章节、产品信息注入、拒绝处理、分层安全、语气与格式规范、元规则（antml 块禁用等）。

#### Scenario: 编译默认 profile
- **WHEN** 执行 `forgeclaw prompt compile --profile default`
- **THEN** 输出包含 identity/safety/tools/style 四章节的完整 system prompt 字符串
- **AND** 动态变量 `{{tools}}` 被替换为实际工具清单

#### Scenario: 禁用章节
- **WHEN** 某 Section 的 frontmatter `enabled=false`
- **THEN** 编译结果不包含该章节内容

#### Scenario: 缓存命中
- **WHEN** 同一 Profile 的 Section 内容未变更，再次编译
- **THEN** 返回缓存的编译结果，不重新拼接

### Requirement: DeepSeek 原生缓存优先 LLM 适配器

系统 SHALL 提供 `LlmClient` trait 与默认的 `OpenAiClient`（兼容 DeepSeek/GLM 等 OpenAI 协议厂商），采用**缓存优先（cache-first）**设计以降低长会话成本。

核心约束（借鉴 Reasonix）：
- 历史消息 SHALL 以 **append-only** 方式增长，不修改既有消息的字节内容
- system prompt 前缀 SHALL 保持**字节稳定**（byte-stable），以命中 DeepSeek prefix-cache
- 每轮请求仅追加新增内容，既有前缀走缓存重放（输入 token 约按 1/5 计费）
- 适配器 SHALL 支持 SSE 流式解析、超时、重试

#### Scenario: 流式对话
- **WHEN** 用户执行 `forgeclaw chat "你好"`（已配置 API Key）
- **THEN** CLI 流式逐 token 输出回复

#### Scenario: 缓存前缀稳定
- **WHEN** 同一会话连续多轮对话
- **THEN** 既有消息字节序列不变，仅追加新消息，prefix-cache 命中率应保持高位

### Requirement: 工具沙箱与分层安全

系统 SHALL 提供工具沙箱，所有工具实现 `Tool` trait：`name()`、`schema()`、`async fn execute(&self, input: Value) -> ToolResult`。

内置工具：`ShellTool`、`FileReadTool`、`FileWriteTool`、`SearchTool`、`GrepTool`。

沙箱安全策略（Hermes 式分层）：
- **工作目录硬限制**：文件工具只能在指定工作目录及其子目录内读写
- **命令白/黑名单**：ShellTool 维护危险命令黑名单（`rm -rf /`、`git push --force` 等）
- **危险操作拦截**：critical 级操作直接 block，confirm 级需用户显式确认
- **敏感路径保护**：禁止写入 `/etc`、`~/.ssh` 等

#### Scenario: 沙箱内执行命令
- **WHEN** 执行 `forgeclaw tool exec shell -- ls -la`
- **THEN** 在限定工作目录内执行并返回结果

#### Scenario: 拦截危险命令
- **WHEN** Agent 尝试执行 `rm -rf /`
- **THEN** 操作被拦截，返回拒绝原因，不执行

### Requirement: Agent 编排与多子代理协作

系统 SHALL 提供 `AgentOrchestrator`，负责消息循环、工具调度、工具结果回填。

支持**多子代理协作**：主 Agent 可派发隔离子任务给 explore/research/review 等子代理，子代理拥有独立上下文与受限工具集，仅返回汇总结果给主 Agent，以节省主上下文与 token。

#### Scenario: 全流程编码
- **WHEN** 执行 `forgeclaw chat "在当前目录建一个 hello.rs 并运行"`
- **THEN** Agent 调用 FileWriteTool 创建文件、ShellTool 编译运行、回填结果并汇报

#### Scenario: 子代理派发
- **WHEN** 主 Agent 需要广泛代码探索
- **THEN** 派发 explore 子代理在隔离上下文中检索，主 Agent 仅接收汇总

### Requirement: CLI 交互

系统 SHALL 提供 `forgeclaw` CLI（clap derive），子命令：`chat`、`run`、`prompt`、`tool`、`web`、`config`。

`chat` 子命令 SHALL 进入 REPL，支持斜杠命令（`/tool ls`、`/model`、`/exit` 等）与流式着色输出。`run` 子命令 SHALL 单次执行任务。`prompt` 子命令 SHALL 支持 `compile` 与 `list-sections`。`tool` 子命令 SHALL 支持 `list` 与 `exec`。`web` 子命令 SHALL 启动 WebUI 后端。

#### Scenario: REPL 交互
- **WHEN** 执行 `forgeclaw chat`
- **THEN** 进入 REPL，可输入消息并接收流式回复，支持 `/exit` 退出

#### Scenario: 单次任务
- **WHEN** 执行 `forgeclaw run "重构 src/auth.rs 的错误处理"`
- **THEN** 单次执行任务并退出

### Requirement: WebUI 嵌入单二进制

系统 SHALL 提供 Vue 3 + Vite WebUI，并通过 `rust-embed` 将 Vite 构建产物嵌入 Rust 二进制，实现 `forgeclaw web` 启动即可访问，无需独立部署前端。

WebUI 页面：`/`（ChatView，流式 + 工具调用卡片）、`/sessions`、`/prompts`（Monaco 章节编辑）、`/tools`、`/settings`。

WebUI 视觉设计 SHALL 遵循 frontend-design SKILL：非模板化、字体个性、结构即信息、单一签名元素、克制与自省；遵循 karpathy CLAUDE.md 的最小化原则，不堆砌功能。

#### Scenario: 启动 WebUI
- **WHEN** 执行 `forgeclaw web --port 8080`
- **THEN** 浏览器可打开 WebUI，发起对话并查看工具调用链

#### Scenario: 单二进制分发
- **WHEN** 仅分发 `forgeclaw` 二进制
- **THEN** `forgeclaw web` 可直接提供前端，无需额外前端产物

### Requirement: 多用户鉴权

系统 SHALL 提供多用户鉴权：用户隔离（各用户会话/配置独立），支持 API Key 或 token 认证。WebUI 与 API 访问需通过认证。

#### Scenario: 未认证访问
- **WHEN** 未携带有效 token 访问 `/api/sessions`
- **THEN** 返回 401 未授权

#### Scenario: 认证后隔离
- **WHEN** 用户 A 登录后查询会话
- **THEN** 仅返回用户 A 的会话，不泄漏用户 B 数据

### Requirement: GitHub Actions CI/CD

系统 SHALL 提供 GitHub Actions 工作流，完成测试与跨平台构建发布。

约束：
- **使用 node22 最新 actions**（node20 已弃用）：`actions/checkout@v5`、`actions/setup-node@v5`（node 22）、`pnpm/action-setup@v4`、`dtolnay/rust-toolchain@stable`、`Swatinem/rust-cache@v2`、`taiki-e/install-action` 等
- Rust: `cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test`
- 前端: `pnpm install`、`pnpm build`、`vue-tsc --noEmit` 类型检查
- 跨平台构建：linux/macos/windows × amd64/arm64，CGO-free
- Release 工作流在 tag 推送时构建并上传单二进制产物

#### Scenario: PR 触发 CI
- **WHEN** 提交 PR
- **THEN** CI 运行 fmt/clippy/test/前端构建，全绿方可合并

#### Scenario: Release 构建
- **WHEN** 推送 `v*` tag
- **THEN** 跨平台构建并上传二进制产物到 Release

### Requirement: 启用设计技能与编码准则

实现阶段 SHALL 启用并遵循两份外部技能/准则：
- **frontend-design SKILL**：WebUI 实现时遵循其设计流程（brainstorm→explore→plan→critique→build→critique again），避免三类 AI 默认外观，做出独特且可辩护的设计选择
- **karpathy CLAUDE.md 准则**：所有编码遵循先思考、最小化、外科手术式改动、目标驱动执行

实现 SHALL 采用**多子代理协作**模式：将无依赖任务并行派发给子代理，主代理负责汇总与一致性校验。

## MODIFIED Requirements

无（全新项目）。

## REMOVED Requirements

无。
