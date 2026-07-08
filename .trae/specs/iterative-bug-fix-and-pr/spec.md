# 迭代修复 Bug 并提交 PR Spec

> change-id: `iterative-bug-fix-and-pr`
> 编制日期: 2026-07-05
> 语言: 中文

## Why

`audit-bugs-and-optimizations` 已产出 109 条发现（P0 24 / P1 34 / P2 51）。一次性批量修复风险高、回归难控，且部分问题相互关联（如 rust-embed 集成与 CI 构建顺序、WS 生命周期与并发覆盖）。需要启用 karpathy CLAUDE.md 与 frontend-design SKILL，以**分阶段修复 + 子代理多角度再审查 + 循环迭代**的方式，把 P0/P1 清零，最终通过 gh CLI 提交 Pull Request。

## What Changes

- **启用并遵循**两份外部指南：
  - `https://github.com/multica-ai/andrej-karpathy-skills/raw/refs/heads/main/CLAUDE.md`（编码行为准则）
  - `https://github.com/anthropics/skills/raw/refs/heads/main/skills/frontend-design/SKILL.md`（WebUI 设计流程）
- **按 AUDIT_REPORT.md 的 6 阶段优先级**修复所有 P0/P1 问题
- **每完成一个阶段/批次后**，派发不少于 3 个独立子代理进行多角度再审查
- **循环「修复 → 再审查 → 修复新发现」**，直到：
  - P0 计数 = 0
  - P1 计数 = 0
  - CI 全绿（`cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test`、`pnpm build`、`pnpm typecheck`）
- **整理 commit** 并使用 gh CLI 创建 Pull Request

## Impact

- 受影响 specs: `implement-forgeclaw-mvp`、`audit-bugs-and-optimizations`
- 受影响代码：
  - `crates/core/src/`、`crates/llm/src/`、`crates/tools/src/`、`crates/server/src/`、`crates/cli/src/`
  - `web/src/`、`web/index.html`、`web/package.json`、`web/vite.config.ts`
  - `.github/workflows/ci.yml`、`.github/workflows/release.yml`
- 审查产出：更新后的 `AUDIT_REPORT.md`（标记已修复条目）、最终再审查报告

## ADDED Requirements

### Requirement: P0/P1 清零

系统 SHALL 修复 `AUDIT_REPORT.md` 中所有 P0/P1 问题，并经过至少两轮独立子代理再审查确认无新增 P0/P1。

#### Scenario: 最终审查无严重问题
- **WHEN** 主代理组织多子代理对修复后代码进行最终审查
- **THEN** P0 计数 = 0 且 P1 计数 = 0
- **AND** CI 全绿

### Requirement: 遵循外部技能与准则

所有 Rust 编码 SHALL 遵循 karpathy CLAUDE.md 的「先思考、最小化、外科手术式改动、目标驱动执行」准则。所有 WebUI 改动 SHALL 遵循 frontend-design SKILL 的「brainstorm→explore→plan→critique→build→critique again」流程，避免三类 AI 默认外观。

#### Scenario: 编码审查
- **WHEN** 子代理审查修复代码
- **THEN** 不应出现与 karpathy/frontend-design 准则明显冲突的改动（如过度抽象、默认模板化 UI、无验证目标）

### Requirement: 循环审查

系统 SHALL 每完成一个修复阶段后，派发不少于 3 个独立子代理（安全、并发/编排器、类型/错误处理或前端）进行再审查，汇总新发现并进入下一轮修复。

#### Scenario: 一轮迭代
- **WHEN** 阶段 N 修复完成
- **THEN** 子代理返回新发现清单
- **AND** 若有 P0/P1，则创建新修复任务继续修复

### Requirement: PR 提交

P0/P1 清零且 CI 通过后，系统 SHALL 使用 gh CLI 创建 Pull Request，PR 描述引用 `AUDIT_REPORT.md` 与修复摘要。

#### Scenario: 创建 PR
- **WHEN** 执行 `gh pr create`
- **THEN** PR 包含标题、修复摘要、测试策略、相关 spec 链接

## MODIFIED Requirements

无。

## REMOVED Requirements

无。
