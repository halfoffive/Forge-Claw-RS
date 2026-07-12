# 补充审计阶段 Checklist

> 用于验证补充审计产出是否满足 spec 要求。审计阶段不修复代码，仅产出报告与修复计划。

## 子代理审计独立性

- [x] 6 个视角（A-F）各自由独立子代理审计，无上下文串扰
- [x] 每个子代理仅阅读自己视角的代码，未越界修改

## 审计覆盖度

- [x] 视角 A 覆盖全部 Rust crate + Web 前端的简洁性/可验证目标
- [x] 视角 B 覆盖 `web/src/` 全部源码 + 入口与构建配置
- [x] 视角 C 覆盖路径逃逸、命令注入、鉴权、ticket、用户隔离
- [x] 视角 D 覆盖并发原语、缓存、性能、阻塞 I/O
- [x] 视角 E 覆盖 `crates/server` 协议、生命周期、错误传播、测试
- [x] 视角 F 覆盖 CI/CD、workspace 配置、依赖版本、测试覆盖

## 发现条目质量

- [x] 每条发现包含：位置（文件:行号）、严重度（P0/P1/P2）、描述、修复方向
- [x] P0 条目均有具体修复方向与受影响代码位置
- [x] 严重度分级符合 spec 定义
- [x] 与 `REAUDIT_REPORT.md` 的重复项已标注原编号

## Karpathy 视角专项

- [x] 发现至少一条过度设计/过度抽象问题（P1/P2）
- [x] 发现至少一条无法验证成功标准的问题（P1/P2）
- [x] 对每条 Karpathy 视角发现给出简化后的目标形态

## 前端设计视角专项

- [x] 对设计方向、排版、色彩、动效、空间构图、背景质感均有覆盖
- [x] 对同质化/默认 AI 审美问题给出具体设计方向建议

## 报告产出

- [x] `FRESH_AUDIT_REPORT.md` 已生成
- [x] 报告含摘要（P0/P1/P2 计数 + 与 REAUDIT 去重后新增计数 + 健康度评估）
- [x] 报告按视角分节
- [x] 报告含与 `REAUDIT_REPORT.md` 的重复项对照表
- [x] 报告含推荐修复顺序
- [x] 每条发现有唯一编号（如 `A-001`）

## 修复任务清单

- [x] `tasks.md` 末尾「修复阶段任务」小节已填充
- [x] 修复任务按 P0 → P1 → P2 排序
- [x] 每条修复任务可追溯到 `FRESH_AUDIT_REPORT.md` 条目编号
- [x] 每条修复任务有 verifiable success criteria
- [x] 每条标注是否可并行
- [x] 每条标注是否与 `REAUDIT_REPORT.md` 重复

## 修复阶段验证（阶段 1 P0）

- [x] Task 9 验证：`cargo check -p forgeclaw-server` 在干净环境下通过
- [x] Task 10 验证：`cargo test -p forgeclaw-tools shell::tests::env_leakage_prevented` 通过
- [x] Task 11 验证：`cargo test -p forgeclaw-tools shell::tests::blocks_new_dangerous_patterns` 通过
- [x] Task 12 验证：`cargo test -p forgeclaw-tools file::tests::write_symlink_replaced_to_outside_blocked` 通过
- [x] Task 13 验证：`cargo check --workspace` 通过（landlock 不影响 Windows/macOS 编译）
- [x] Task 13 验证：Linux 交叉编译 `cargo check -p forgeclaw-tools --target x86_64-unknown-linux-gnu` 通过

## 修复阶段验证（阶段 2 P1）

- [x] Task 14 验证：`cargo test -p forgeclaw` 通过（AsyncConfirmer + CLI confirm 复用沙箱）
- [x] Task 15 验证：`cargo test -p forgeclaw-llm` 通过（TryFrom + 未知 role 不回落）
- [x] Task 16 验证：`cargo test -p forgeclaw-core` 与 `cargo test -p forgeclaw-server` 通过（InvalidName）
- [x] Task 17 验证：`bun run build` 与 `bun run typecheck` 通过（前端设计系统）
- [x] Task 18+19 验证：`cargo test -p forgeclaw-server` 通过（WS 协议 + call_id）
- [x] Task 20 验证：`release.yml` 结构正确（单汇总 job 上传产物）

## 修复阶段验证（阶段 3 P1）

- [x] Task 21/22/23/29 验证：`cargo test -p forgeclaw-server` 通过（并发、user_id、超时 abort、Error 脱敏）
- [x] Task 24 验证：`cargo test -p forgeclaw-server` 通过（history 读锁快照不阻塞 LLM）
- [x] Task 25 验证：`cargo test -p forgeclaw-core` 与 `cargo test -p forgeclaw-server` 通过（PromptEngine 并发）
- [x] Task 26 验证：`cargo test -p forgeclaw-server` 通过（server 模式不自动放行 Confirm）
- [x] Task 27/28 验证：`cargo test -p forgeclaw-server` 通过（常量时间 token + Debug 脱敏）
- [x] Task 30 验证：`cargo test -p forgeclaw` 通过（Windows ACL）
- [x] Task 31 验证：`cargo test -p forgeclaw-server` 通过（orchestrator 测试重写）

## 边界约束

- [x] 审计阶段未修改任何源码（仅新增 `FRESH_AUDIT_REPORT.md` 与 spec 文档）
- [x] 未回滚用户既有改动
- [x] 未删除已有审计报告（`AUDIT_REPORT.md`、`REAUDIT_REPORT.md` 保留）
