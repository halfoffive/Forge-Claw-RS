# ForgeClaw WebUI

ForgeClaw 的 Vue 3 + Vite + TypeScript 前端，构建产物通过 `rust-embed` 嵌入
`forgeclaw` 单二进制，由 `forgeclaw web` 启动后直接服务，无需独立部署。

## 功能

- **登录**：用户名 + Token 鉴权，token 持久化到 `localStorage`。
- **对话**：WebSocket 流式输出，支持 `delta` / `tool_call_start` /
  `tool_result` / `complete` / `error` 事件，工具调用以卡片形式展示。
- **会话**：列出历史会话，点击进入对应对话。
- **工具**：查看后端注册的工具清单与参数 schema。
- **设置**：配置后端服务器地址、登出。

## 开发

```bash
pnpm install
pnpm dev        # 启动 vite dev server（/api、/ws 代理到 localhost:8080）
pnpm build      # 类型检查 + 生产构建到 dist/
pnpm typecheck  # 仅 vue-tsc 类型检查
```

开发时需要后端在 `localhost:8080` 监听（见 `vite.config.ts` 的 proxy 配置）。

## 结构

```
src/
  api/
    types.ts    # API 类型定义
    client.ts   # fetch 封装 + WS ticket 辅助
  stores/       # pinia: auth / session / settings
  views/        # Login / Chat / Sessions / Tools / Settings / NotFound
  router/       # 路由 + 鉴权守卫
  App.vue       # 侧边栏布局
  main.ts       # 应用入口
```

`@/` 别名指向 `src/`（见 `tsconfig.app.json` 与 `vite.config.ts`）。
