# ForgeClaw Bug 审查与优化审计 Spec

> change-id: `audit-bugs-and-optimizations`
> 编制日期: 2026-07-05
> 语言: 中文

## Why

ForgeClaw MVP 已完成首轮实现（5 个 Rust crate + Vue 3 WebUI + GitHub Actions），但实现期跨越多子代理并行编码，可能引入跨 crate 协作不一致、并发安全隐患、安全沙箱绕过、缓存前缀失效、鉴权泄漏、CI 配置漂移等严重 bug。在交付前需要一次**多角度、子代理并行**的审查，定位已存在的 bug 与可优化点，并形成可追踪的修复任务清单，避免带病发布。

## What Changes

- **新增** 多角度审查报告：从 6 个独立视角审计代码，每个视角由独立子代理负责，避免单视角盲区
  - 视角 A：Rust 编译/类型/错误处理（`core` / `llm` / `tools`）
  - 视角 B：并发与缓存正确性（append-only 历史、prefix-cache 字节稳定、`Arc/RwLock` 使用）
  - 视角 C：安全沙箱与鉴权（路径逃逸、危险命令拦截、API Key 校验、用户隔离）
  - 视角 D：API/WebSocket 协议与编排器（`server` crate：消息循环、工具调度、WS 生命周期）
  - 视角 E：前端 WebUI（Vue 3：路由、状态、流式渲染、类型安全）
  - 视角 F：CI/CD 与构建配置（GitHub Actions node22 约束、Cargo workspace、Vite 构建）
- **新增** `AUDIT_REPORT.md`：汇总各视角发现的 bug（按严重度 P0/P1/P2 分级）与优化建议
- **新增** 修复任务清单（写入 `tasks.md`），按 P0 → P1 → P2 顺序排序
- **不在本变更范围**：实际修复代码。本变更仅产出审查报告与修复计划，修复由后续变更承接

## Impact

- 受影响 specs: `implement-forgeclaw-mvp`（被审计对象）
- 受影响代码（只读审计）:
  - `crates/core/src/`：`error.rs`、`lib.rs`、`model.rs`、`prompt/{engine,profile,section,mod}.rs`
  - `crates/llm/src/`：`client.rs`、`lib.rs`
  - `crates/tools/src/`：`file.rs`、`sandbox.rs`、`search.rs`、`shell.rs`、`lib.rs`
  - `crates/server/src/`：`api.rs`、`auth.rs`、`orchestrator.rs`、`ws.rs`、`lib.rs` + `tests/`
  - `crates/cli/src/`：`commands.rs`、`config.rs`、`main.rs`
  - `web/src/`：`App.vue`、`main.ts`、`router/index.ts`、`views/`、`components/`、`stores/`
  - `.github/workflows/`：`ci.yml`、`release.yml`
  - `Cargo.toml`、`crates/*/Cargo.toml`、`web/package.json`、`web/vite.config.ts`
- 审查产出: `AUDIT_REPORT.md`（项目根目录）

## ADDED Requirements

### Requirement: 多视角并行审计

系统 SHALL 通过 6 个独立子代理并行审计代码，每个子代理仅负责一个视角，互不干扰，以避免单代理上下文污染与视角盲区。

每个子代理 SHALL：
1. 阅读所负责视角的全部相关源码
2. 列出发现的 bug 与优化点，每条包含：位置（文件:行号）、严重度（P0/P1/P2）、问题描述、建议修复方向
3. 不修改任何代码，仅产出审查发现
4. 返回结构化发现清单给主代理

#### Scenario: 子代理独立审计
- **WHEN** 主代理派发 6 个审计子代理
- **THEN** 各子代理在隔离上下文中完成各自视角的审计，返回发现清单

#### Scenario: 主代理汇总
- **WHEN** 全部子代理返回
- **THEN** 主代理去重、合并、按严重度排序，写入 `AUDIT_REPORT.md`

### Requirement: 严重度分级

所有发现 SHALL 按 P0/P1/P2 分级：

- **P0（严重）**：安全漏洞（沙箱逃逸、鉴权绕过、命令注入）、数据损坏（缓存前缀失效、消息历史被改写）、导致核心功能不可用的崩溃 bug
- **P1（高）**：并发竞态、资源泄漏（连接/句柄未释放）、错误处理缺失导致 panic、协议不一致
- **P2（中/低）**：性能优化点、代码异味、可读性改进、与 spec 不一致但不影响功能

#### Scenario: P0 必须给出修复方向
- **WHEN** 子代理标记某条发现为 P0
- **THEN** 该条目 SHALL 包含具体的修复方向与受影响代码位置

### Requirement: 审计报告产出

主代理 SHALL 汇总各视角发现，生成 `AUDIT_REPORT.md`，包含：

1. 摘要：P0/P1/P2 各多少条，总体健康度评估
2. 按视角分节，列出该视角全部发现
3. 跨视角共性问题汇总
4. 推荐修复顺序（P0 优先）

#### Scenario: 报告可被 tasks.md 引用
- **WHEN** 审查完成
- **THEN** `tasks.md` 中每个修复任务可追溯到 `AUDIT_REPORT.md` 中的具体条目编号

### Requirement: 修复任务清单

`tasks.md` SHALL 按严重度从高到低列出修复任务，每个任务标注：

- 对应 `AUDIT_REPORT.md` 条目编号
- 修复目标（verifiable success criteria）
- 受影响文件
- 是否可与其他任务并行

#### Scenario: P0 任务优先
- **WHEN** 生成 tasks.md
- **THEN** P0 任务排在 P1/P2 之前，且 P0 任务未完成前不进入 P1

## MODIFIED Requirements

无（本变更为只读审计 + 文档产出，不修改 `implement-forgeclaw-mvp` 的任何已有 requirement）。

## REMOVED Requirements

无。
