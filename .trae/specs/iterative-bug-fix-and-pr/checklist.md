# 迭代修复 Bug 并提交 PR Checklist

## 修复实施

- [ ] 阶段 1 阻断性 P0 全部完成（index.html、版本号、rust-embed、CI 顺序）
- [ ] 阶段 2 安全 P0 全部完成（env_clear、0600、默认绑定/token、WS ticket、黑名单、跨用户 session）
- [ ] 阶段 3 并发/编排器 P0 全部完成（Arc<RwLock<History>>、max_turns、错误回填、Error 传播）
- [ ] 阶段 4 WebUI 业务 P0 全部完成（5 view、路由守卫、API 客户端/pinia、App.vue 导航）
- [ ] 阶段 5 P1 长尾全部完成（WS 生命周期、错误处理、HTTP 加固、类型安全、PromptEngine 并发、CI 加固）

## 循环审查

- [ ] 第一轮多视角再审查完成，子代理返回清单
- [ ] 第一轮新发现的 P0/P1 已修复
- [ ] 第二轮多视角再审查完成，P0 = 0 且 P1 = 0

## CI 与测试

- [ ] `cargo fmt --check` 通过
- [ ] `cargo clippy -- -D warnings` 通过
- [ ] `cargo test` 通过
- [ ] `pnpm install` 成功
- [ ] `pnpm build` 成功
- [ ] `pnpm typecheck` 通过

## PR 提交

- [ ] 分支 `fix/audit-findings` 已创建
- [ ] Conventional commit 已整理
- [ ] PR body 已生成（引用 AUDIT_REPORT.md、修复摘要、测试策略）
- [ ] `gh pr create` 成功并返回 PR URL

## 外部准则遵循

- [ ] 所有 Rust 改动符合 karpathy CLAUDE.md（先思考、最小化、外科手术式、目标驱动）
- [ ] 所有 WebUI 改动符合 frontend-design SKILL（非模板化、结构即信息、克制自省）
