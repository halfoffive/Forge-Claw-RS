# ForgeClaw 多角度补充审计 Spec

> change-id: `fresh-multi-angle-audit`
> 编制日期: 2026-07-12
> 语言: 中文
> 前置变更: `audit-bugs-and-optimizations`（2026-07-05）、`re-audit-after-fixes`（2026-07-11）

## Why

`re-audit-after-fixes` 已对当前 HEAD（`31c6db0`）做了 6 视角技术审计并产出 `REAUDIT_REPORT.md`。本次任务引入两个新视角——**Karpathy 代码质量准则**（反对过度设计、强调手术式改动、可验证目标）与 **Frontend Design 审美准则**（避免同质化 AI 界面、强调设计系统、动效与排版）——对同一代码基线再做一次补充审查，重点发现前两次审计未覆盖的：

- 过度抽象、过早泛化、无法验证的"灵活"设计
- 前端界面同质化、缺乏设计系统、动效与排版问题
- 跨视角综合遗漏（安全 + 简洁性、性能 + 可维护性）

本次审计**不替代** `re-audit-after-fixes`，而是其补充；产出独立报告，并与 `REAUDIT_REPORT.md` 去重。

## What Changes

- **新增** 5 个独立视角并行审计，每个视角由独立子代理负责
  - 视角 A：Karpathy 代码质量（简洁性、手术式改动、可验证目标）—— 跨全部 Rust crate 与 Web 前端
  - 视角 B：前端设计审美（设计系统、排版、色彩、动效、空间构图、UX）—— `web/src/`
  - 视角 C：安全与沙箱（路径逃逸、命令注入、鉴权、用户隔离、ticket 机制）—— `crates/tools/` / `crates/server/`
  - 视角 D：并发与性能（竞态、资源泄漏、缓存、阻塞 I/O）—— `crates/core/` / `crates/server/`
  - 视角 E：API / WebSocket / 编排器协议（生命周期、错误传播、协议一致性）—— `crates/server/`
  - 视角 F：构建、CI/CD 与可维护性（配置漂移、依赖、测试覆盖）—— 根目录配置与 workflows
- **新增** `FRESH_AUDIT_REPORT.md`：汇总各视角发现的 bug 与优化点，按严重度 P0/P1/P2 分级
- **新增** 修复任务清单（写入 `tasks.md`），按 P0 → P1 → P2 排序，并标注是否与 `REAUDIT_REPORT.md` 重复
- **不在本变更范围**：实际修复代码。本变更仅产出审查报告与修复计划

## Impact

- 受影响 specs:
  - `audit-bugs-and-optimizations`（基线审计）
  - `re-audit-after-fixes`（当前 HEAD 审计，本补充审计的对照基线）
- 受影响代码（只读审计）:
  - `crates/core/src/`：`error.rs`、`lib.rs`、`model.rs`、`prompt/{engine,profile,section,mod}.rs`
  - `crates/llm/src/`：`client.rs`、`lib.rs`
  - `crates/tools/src/`：`file.rs`、`sandbox.rs`、`search.rs`、`shell.rs`、`lib.rs`
  - `crates/server/src/`：`api.rs`、`auth.rs`、`orchestrator.rs`、`ws.rs`、`lib.rs` + `tests/`
  - `crates/cli/src/`：`commands.rs`、`config.rs`、`main.rs`
  - `web/src/`：`App.vue`、`main.ts`、`router/index.ts`、`views/`、`stores/`、`api/`、`style.css`
  - `.github/workflows/`：`ci.yml`、`release.yml`
  - `Cargo.toml`、`crates/*/Cargo.toml`、`web/package.json`、`web/vite.config.ts`
- 审查产出: `FRESH_AUDIT_REPORT.md`（项目根目录）

## ADDED Requirements

### Requirement: 多视角并行审计

系统 SHALL 通过 6 个独立子代理并行审计代码，每个子代理仅负责一个视角，互不干扰。

每个子代理 SHALL：
1. 阅读所负责视角的全部相关源码
2. 列出发现的 bug 与优化点，每条包含：位置（文件:行号）、严重度（P0/P1/P2）、问题描述、建议修复方向
3. 对 `REAUDIT_REPORT.md` 中已存在的问题进行去重标注（重复则引用原编号，不重复则为新发现）
4. 不修改任何代码，仅产出审查发现
5. 返回结构化发现清单给主代理

#### Scenario: 子代理独立审计
- **WHEN** 主代理派发 6 个审计子代理
- **THEN** 各子代理在隔离上下文中完成各自视角的审计，返回发现清单

#### Scenario: 主代理汇总
- **WHEN** 全部子代理返回
- **THEN** 主代理去重、合并、按严重度排序，写入 `FRESH_AUDIT_REPORT.md`

### Requirement: 严重度分级

所有发现 SHALL 按 P0/P1/P2 分级：

- **P0（严重）**：安全漏洞（沙箱逃逸、鉴权绕过、命令注入）、数据损坏、导致核心功能不可用的崩溃 bug
- **P1（高）**：并发竞态、资源泄漏、错误处理缺失导致 panic、协议不一致、显著影响可维护性的过度设计
- **P2（中/低）**：性能优化点、代码异味、可读性改进、前端审美/UX 改进

#### Scenario: P0 必须给出修复方向
- **WHEN** 子代理标记某条发现为 P0
- **THEN** 该条目 SHALL 包含具体的修复方向与受影响代码位置

### Requirement: Karpathy 视角审计

视角 A 子代理 SHALL 依据 Karpathy Guidelines 重点审查：

1. **假设与复杂度**：是否存在未说明的假设、过度抽象、为单次使用创造的 helper
2. **简洁性**：是否存在 200 行可缩为 50 行的实现；是否存在未请求的"灵活性"
3. **手术式改动**：是否存在对无关代码的"顺手改进"、格式化或注释修改
4. **可验证目标**：功能是否缺乏测试或无法验证成功标准

#### Scenario: 发现过度设计
- **WHEN** 子代理发现某函数/模块存在过度设计
- **THEN** 该条目 SHALL 给出简化后的目标形态与可验证标准

### Requirement: 前端设计视角审计

视角 B 子代理 SHALL 依据 Frontend Design Guidelines 重点审查：

1. **设计方向**：WebUI 是否有明确、大胆且一致的美学方向
2. **排版**：字体选择是否有特色、层级是否清晰，避免 Inter/Roboto/Arial 等默认字体
3. **色彩与主题**：是否使用 CSS 变量、主导色与强调色是否明确，避免 cliched 紫色渐变
4. **动效**：是否有 page-load 编排、hover/scroll 交互，避免零散无意义动画
5. **空间构图**：是否存在非对称、重叠、破格网格等有意思的布局
6. **背景与质感**：是否通过渐变、噪点、几何图案等营造氛围

#### Scenario: 发现同质化界面
- **WHEN** 子代理发现前端界面趋于默认 AI 审美
- **THEN** 该条目 SHALL 给出具体设计方向建议与参考实现思路

### Requirement: 审计报告产出

主代理 SHALL 汇总各视角发现，生成 `FRESH_AUDIT_REPORT.md`，包含：

1. 摘要：P0/P1/P2 各多少条，与 `REAUDIT_REPORT.md` 去重后新增多少条，总体健康度评估
2. 按视角分节，列出该视角全部发现
3. 与 `REAUDIT_REPORT.md` 的重复项对照表
4. 推荐修复顺序（P0 优先）

#### Scenario: 报告可被 tasks.md 引用
- **WHEN** 审查完成
- **THEN** `tasks.md` 中每个修复任务可追溯到 `FRESH_AUDIT_REPORT.md` 中的具体条目编号

### Requirement: 修复任务清单

`tasks.md` SHALL 按严重度从高到低列出修复任务，每个任务标注：

- 对应 `FRESH_AUDIT_REPORT.md` 条目编号
- 修复目标（verifiable success criteria）
- 受影响文件
- 是否可与其他任务并行
- 是否与 `REAUDIT_REPORT.md` 重复（若重复则引用原条目编号）

#### Scenario: P0 任务优先
- **WHEN** 生成 tasks.md
- **THEN** P0 任务排在 P1/P2 之前

## MODIFIED Requirements

无（本变更为只读审计 + 文档产出，不修改已有 requirement）。

## REMOVED Requirements

无。
