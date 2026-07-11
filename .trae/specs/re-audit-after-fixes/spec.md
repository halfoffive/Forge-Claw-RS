# 修复后重新审计 Spec

> change-id: `re-audit-after-fixes`
> 编制日期: 2026-07-11
> 语言: 中文
> 前置变更: `audit-bugs-and-optimizations`（2026-07-05 首轮审计，产出 `AUDIT_REPORT.md` 109 条发现）

## Why

首轮审计（2026-07-05）发现 24 条 P0、34 条 P1、51 条 P2 共 109 条问题。随后提交 `be8ac66 fix: address audit findings (#2)`（2026-07-08）尝试修复，但该提交在合并过程中出现冲突与回退，具体表现：

- `crates/core/src/model.rs` 被显式回退到 main 版本（`Session.messages` 仍 `pub`，A-002 修复撤销）
- `Cargo.toml`、`tsconfig.app.json` 被回退
- 3 个 orchestrator 测试因"与 main 行为不兼容"被删除（测试覆盖倒退）
- `PromptsView.vue` 因 naive-ui 被移除而重写
- 新增 `/api/auth/ticket` 端点与前端 ticket 流（C-006 方向），但实现正确性未验证

当前代码状态为"部分修复 + 部分回退 + 可能的合并回归"，原 `AUDIT_REPORT.md` 对当前代码已过时。在进入下一轮修复或发布前，需要对**当前 HEAD 代码**做一次多角度重新审查，区分：已修复 / 部分修复 / 未修复 / 合并回归 / 新引入 bug。

## What Changes

- **新增** 6 视角并行重新审计（沿用首轮视角划分，针对当前 HEAD 代码）
  - 视角 A：Rust 编译/类型/错误处理（`core` / `llm` / `tools`）
  - 视角 B：并发与缓存正确性（append-only 历史、prefix-cache 字节稳定、`Arc/RwLock`）
  - 视角 C：安全沙箱与鉴权（路径逃逸、危险命令拦截、ticket API、用户隔离）
  - 视角 D：API/WebSocket 协议与编排器（ticket 鉴权流、WS 生命周期、工具调度、被删测试的影响）
  - 视角 E：前端 WebUI（Vue 3：ticket 流、路由守卫、PromptsView 重写、类型安全、流式渲染）
  - 视角 F：CI/CD 与构建配置（node22 约束、Cargo workspace、Vite 构建、被回退的 Cargo.toml/tsconfig）
- **新增** `REAUDIT_REPORT.md`：对首轮 109 条发现逐条标注当前状态（FIXED / PARTIAL / NOT-FIXED / REGRESSED / NEW），并记录合并回归与新 bug
- **新增** 更新后的修复任务清单（追加到本 tasks.md），仅保留/新增当前仍有效的问题
- **不在本变更范围**：实际修复代码。本变更仅产出重新审查报告与更新后的修复计划

## Impact

- 受影响 specs:
  - `audit-bugs-and-optimizations`（首轮审计，作为对比基线）
  - `implement-forgeclaw-mvp`（被审计对象的实现 spec）
- 受影响代码（只读审计，覆盖当前 HEAD `be8ac66`）:
  - `crates/core/src/`：`error.rs`、`lib.rs`、`model.rs`（重点：回退后的 messages 字段）、`prompt/{engine,profile,section,mod}.rs`
  - `crates/llm/src/`：`client.rs`、`lib.rs`
  - `crates/tools/src/`：`file.rs`、`sandbox.rs`、`search.rs`、`shell.rs`、`lib.rs`
  - `crates/server/src/`：`api.rs`、`auth.rs`（重点：新增 ticket 端点）、`orchestrator.rs`、`ws.rs`（重点：ticket 鉴权）、`lib.rs` + `tests/`（重点：被删的 3 个测试）
  - `crates/cli/src/`：`commands.rs`、`config.rs`、`main.rs`
  - `web/src/`：`App.vue`、`main.ts`、`router/index.ts`、`views/`（重点：PromptsView 重写、ChatView ticket 流）、`stores/`、`api/`
  - `.github/workflows/`：`ci.yml`、`release.yml`
  - `Cargo.toml`、`crates/*/Cargo.toml`（重点：回退后的版本）、`web/package.json`、`web/vite.config.ts`、`web/tsconfig.app.json`
- 审查产出: `REAUDIT_REPORT.md`（项目根目录）

## ADDED Requirements

### Requirement: 对当前 HEAD 重新审计

系统 SHALL 通过 6 个独立子代理并行审计**当前 HEAD（`be8ac66`）**代码，每个子代理仅负责一个视角，互不干扰。

每个子代理 SHALL：
1. 阅读所负责视角的全部相关源码（当前 HEAD 版本）
2. 对照首轮 `AUDIT_REPORT.md` 中该视角的发现，逐条标注当前状态：
   - **FIXED**：问题已不存在，代码正确修复
   - **PARTIAL**：部分修复，残留风险（需说明残留点）
   - **NOT-FIXED**：问题仍存在，与首轮一致
   - **REGRESSED**：修复后回退到更差状态（如 model.rs 回退）
   - **NEW**：首轮未发现，本次新引入（合并回归或新 bug）
3. 对每条 NEW 发现，包含：位置（文件:行号）、严重度（P0/P1/P2）、问题描述、建议修复方向
4. 不修改任何代码，仅产出审查发现
5. 返回结构化发现清单给主代理

#### Scenario: 子代理对照基线审计
- **WHEN** 主代理派发 6 个重新审计子代理，各附带首轮 `AUDIT_REPORT.md` 中对应视角的条目
- **THEN** 各子代理在隔离上下文中完成对照审计，返回带状态标注的发现清单

#### Scenario: 主代理汇总
- **WHEN** 全部子代理返回
- **THEN** 主代理去重、合并、按状态与严重度排序，写入 `REAUDIT_REPORT.md`

### Requirement: 合并回归专项检测

由于 `be8ac66` 提交存在显式回退（`model.rs`、`Cargo.toml`、`tsconfig.app.json`、删除 3 个 orchestrator 测试），重新审计 SHALL 专项检测这些回退是否导致：

- `Session.messages` 重新暴露 `pub` 字段（A-002 回归）
- 依赖版本回退是否影响构建或安全
- 被删测试覆盖的行为路径是否无人守护（D-014/D-015 倒退）
- `PromptsView.vue` 重写是否丢失功能或引入 XSS

#### Scenario: 回归项必须显式标注
- **WHEN** 子代理发现某条首轮已修复的问题在当前 HEAD 又出现
- **THEN** 该条目 SHALL 标注为 REGRESSED 并附回归原因（哪个提交/回退导致）

### Requirement: 严重度分级（沿用首轮）

所有发现（含 NEW）SHALL 按 P0/P1/P2 分级，定义与首轮一致：

- **P0（严重）**：安全漏洞、数据损坏、核心功能不可用崩溃
- **P1（高）**：并发竞态、资源泄漏、错误处理缺失导致 panic、协议不一致
- **P2（中/低）**：性能优化点、代码异味、可读性改进

#### Scenario: P0 必须给出修复方向
- **WHEN** 子代理标记某条发现（含 NEW）为 P0
- **THEN** 该条目 SHALL 包含具体修复方向与受影响代码位置

### Requirement: 重新审计报告产出

主代理 SHALL 汇总各视角发现，生成 `REAUDIT_REPORT.md`，包含：

1. 摘要：FIXED / PARTIAL / NOT-FIXED / REGRESSED / NEW 各多少条，当前总体健康度评估（对比首轮）
2. 按视角分节，列出该视角全部发现（含状态标注）
3. 合并回归专项小节（聚焦 `be8ac66` 的回退与删除）
4. 仍需修复的问题清单（NOT-FIXED + PARTIAL + REGRESSED + NEW），按 P0 → P1 → P2 排序
5. 推荐下一轮修复顺序

#### Scenario: 报告可被 tasks.md 引用
- **WHEN** 重新审查完成
- **THEN** `tasks.md` 中每个仍需修复的任务可追溯到 `REAUDIT_REPORT.md` 中的具体条目编号

### Requirement: 更新后的修复任务清单

`tasks.md` SHALL 基于 `REAUDIT_REPORT.md` 中"仍需修复"的问题，生成更新后的修复任务清单，每个任务标注：

- 对应 `REAUDIT_REPORT.md` 条目编号
- 状态来源（NOT-FIXED / PARTIAL / REGRESSED / NEW）
- 修复目标（verifiable success criteria）
- 受影响文件
- 是否可与其他任务并行
- 首轮 task 编号（若适用，用于追溯 `audit-bugs-and-optimizations` 的 Task 9-38）

#### Scenario: 已修复问题不重复列任务
- **WHEN** 某条首轮发现状态为 FIXED
- **THEN** 该条目 SHALL 从修复任务清单中移除，仅在报告中记录为 FIXED

## MODIFIED Requirements

无（本变更是独立的只读重新审计，不修改 `audit-bugs-and-optimizations` 的已有 requirement）。

## REMOVED Requirements

无。
