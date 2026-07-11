# 重新审计阶段 Checklist

> 用于验证重新审计产出是否满足 spec 要求。重新审计阶段不修复代码，仅产出报告与更新后的修复计划。

## 子代理审计独立性

- [x] 6 个视角（A-F）各自由独立子代理审计，无上下文串扰
- [x] 每个子代理仅阅读自己视角的代码，未越界修改

## 审计覆盖度（当前 HEAD be8ac66）

- [x] 视角 A 覆盖 `crates/core`、`crates/llm`、`crates/tools` 全部源码
- [x] 视角 B 覆盖并发原语（`Arc`/`Mutex`/`RwLock`）与缓存前缀稳定性（含补审 B-010~B-014）
- [x] 视角 C 覆盖路径逃逸、危险命令拦截、ticket 鉴权、用户隔离
- [x] 视角 D 覆盖 `crates/server` 全部源码 + tests（含被删测试的影响评估）
- [x] 视角 E 覆盖 `web/src/` 全部源码 + 构建配置（含 PromptsView 重写、ChatView ticket 流）
- [x] 视角 F 覆盖 `.github/workflows/` 与全部 `Cargo.toml` / `package.json` / `tsconfig`（含补审 F-009）

## 对照基线状态标注

- [x] 首轮 109 条发现（A-001~F-011）每条均有当前状态标注（FIXED/PARTIAL/NOT-FIXED/REGRESSED）
- [x] FIXED 条目有证据（代码位置或提交说明修复点）
- [x] PARTIAL 条目说明残留风险点
- [x] REGRESSED 条目附回归原因（哪个回退导致）
- [x] NEW 条目含位置、严重度、描述、修复方向

## 合并回归专项检测

- [x] `model.rs` 回退导致 `Session.messages` 重新 pub（A-002 回归）已检测并标注
- [x] `Cargo.toml`/`tsconfig.app.json` 回退对构建/安全的影响已评估
- [x] 被删的 3 个 orchestrator 测试覆盖的行为路径已识别（D-006/D-012 倒退）
- [x] `PromptsView.vue` 重写是否丢失功能或引入 XSS 已检测
- [x] ticket API 实现正确性（一次性、过期、跨用户复用）已验证

## 发现条目质量

- [x] 每条 NEW 发现包含：位置（文件:行号）、严重度（P0/P1/P2）、描述、修复方向
- [x] P0 条目（含 NEW）均有具体修复方向与受影响代码位置
- [x] 严重度分级符合 spec 定义（P0=安全/数据损坏/崩溃，P1=竞态/泄漏/panic，P2=优化/异味）

## 报告产出

- [x] `/workspace/REAUDIT_REPORT.md` 已生成
- [x] 报告含摘要（FIXED/PARTIAL/NOT-FIXED/REGRESSED/NEW 计数 + 与首轮对比的健康度评估）
- [x] 报告按视角分节，列出该视角全部发现（含状态标注）
- [x] 报告含合并回归专项小节（聚焦 `be8ac66` 的回退与删除）
- [x] 报告含仍需修复清单（NOT-FIXED + PARTIAL + REGRESSED + NEW），按 P0 → P1 → P2 排序
- [x] 报告含推荐下一轮修复顺序
- [x] 每条发现有唯一编号（沿用首轮编号 + NEW 编号）

## 修复任务清单

- [x] `tasks.md` 末尾「修复阶段任务」小节已填充（基于 REAUDIT_REPORT.md）
- [x] 修复任务按 P0 → P1 → P2 排序
- [x] 每条修复任务可追溯到 `REAUDIT_REPORT.md` 条目编号
- [x] 每条修复任务有 verifiable success criteria
- [x] 每条标注是否可并行
- [x] 每条标注状态来源（NOT-FIXED/PARTIAL/REGRESSED/NEW）与首轮 task 编号（若适用）
- [x] FIXED 条目未重复列入修复任务

## 边界约束

- [x] 重新审计阶段未修改任何源码（仅新增 `REAUDIT_REPORT.md` 与 spec 文档）
- [x] 未回滚用户既有改动（`be8ac66` 的修复与回退均保留）
- [x] 未删除首轮 `AUDIT_REPORT.md`（作为对比基线保留）
