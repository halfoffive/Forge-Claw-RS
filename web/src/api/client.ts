// ForgeClaw API 客户端：fetch 封装 + WS ticket 辅助。
//
// 设计要点：
// - 所有 REST 请求自动注入 `Authorization: Bearer <token>`（由调用方传入）。
// - 30s 超时（AbortController），401 抛 ApiError 以便上层清理会话。
// - base 默认相对路径（依赖 vite 代理 /api → localhost:8080）；
//   settings store 可通过 setApiBase 切换到其他后端地址。
// - WS 走一次性 ticket：先 GET /api/auth/ticket，再通过 WebSocket 子协议头传递，
//   不在 URL 中暴露 ticket。

import type {
  LoginResponse,
  SessionDetail,
  SessionSummary,
  TicketResponse,
  ToolInfo,
} from './types'

/** 提示词章节。 */
export interface Section {
  id: string
  title: string
  level: string
  enabled: boolean
  order: number
  body: string
}

/** API 错误（携带状态码与后端消息）。 */
export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

/** 默认 30s 请求超时。 */
const DEFAULT_TIMEOUT_MS = 30_000

/** 模块级 API base，默认相对路径（走 vite 代理）。 */
let apiBase = ''

/** 设置 API base（由 settings store 调用）。传空串恢复相对路径。 */
export function setApiBase(base: string): void {
  apiBase = base.replace(/\/+$/, '')
}

/** 拼接完整 URL。 */
function buildUrl(path: string): string {
  if (!path.startsWith('/')) path = '/' + path
  return apiBase ? apiBase + path : path
}

/** 核心 fetch 封装：注入 Bearer、30s 超时、401 抛 ApiError。 */
async function request<T>(
  path: string,
  token: string,
  init: RequestInit = {},
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<T> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)

  const headers: Record<string, string> = {
    ...(init.headers as Record<string, string> | undefined),
  }
  if (init.body !== undefined && !headers['Content-Type']) {
    headers['Content-Type'] = 'application/json'
  }
  if (token) {
    headers['Authorization'] = `Bearer ${token}`
  }

  let res: Response
  try {
    res = await fetch(buildUrl(path), {
      ...init,
      headers,
      signal: controller.signal,
    })
  } catch (e) {
    if (controller.signal.aborted) {
      throw new ApiError(0, `请求超时（${timeoutMs}ms）：${path}`)
    }
    throw new ApiError(0, `网络错误：${(e as Error).message}`)
  } finally {
    clearTimeout(timer)
  }

  if (res.status === 401) {
    let msg = '未授权'
    try {
      msg = (await res.json())?.error ?? msg
    } catch {
      /* ignore */
    }
    // F-19: 401 时通知应用层清理登录态并跳转登录页（避免 client 直接依赖 store/router）。
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('forgeclaw:unauthorized'))
    }
    throw new ApiError(401, msg)
  }

  const text = await res.text()
  const body = text ? JSON.parse(text) : null
  if (!res.ok) {
    const msg = (body && (body.error || body.message)) || `HTTP ${res.status}`
    throw new ApiError(res.status, msg)
  }
  return body as T
}

/** `POST /api/auth/login`：用 name+token 换取登录态与首张 WS ticket。 */
export function login(
  name: string,
  token: string,
): Promise<LoginResponse> {
  return request<LoginResponse>('/api/auth/login', '', {
    method: 'POST',
    body: JSON.stringify({ name, token }),
  })
}

/** `GET /api/auth/ticket`：换取一次性 WS ticket（需 Bearer）。 */
export function getWsTicket(token: string): Promise<string> {
  return request<TicketResponse>('/api/auth/ticket', token).then((r) => r.ticket)
}

/** `GET /api/sessions`：列出当前用户会话摘要。 */
export function listSessions(token: string): Promise<SessionSummary[]> {
  return request<SessionSummary[]>('/api/sessions', token)
}

/** `GET /api/sessions/:id`：获取会话详情（含消息）。 */
export function getSession(token: string, id: string): Promise<SessionDetail> {
  return request<SessionDetail>(`/api/sessions/${encodeURIComponent(id)}`, token)
}

/** `GET /api/tools`：列出可用工具。 */
export function listTools(token: string): Promise<{ tools: ToolInfo[] }> {
  return request<{ tools: ToolInfo[] }>('/api/tools', token)
}

/** `POST /api/prompts/compile`：编译指定 profile 的 system prompt。 */
export function compilePrompt(
  token: string,
  profile: string,
): Promise<{ prompt: string }> {
  return request<{ prompt: string }>('/api/prompts/compile', token, {
    method: 'POST',
    body: JSON.stringify({ profile }),
  })
}

/** `GET /api/prompts/sections?profile=`：列出 profile 启用的章节。 */
export function listSections(
  token: string,
  profile: string,
): Promise<unknown[]> {
  return request<unknown[]>(
    `/api/prompts/sections?profile=${encodeURIComponent(profile)}`,
    token,
  )
}

/** `PUT /api/prompts/sections`：保存 profile 的章节。 */
export function saveSections(
  token: string,
  profile: string,
  sections: Section[],
): Promise<void> {
  return request<void>('/api/prompts/sections', token, {
    method: 'PUT',
    body: JSON.stringify({ profile, sections }),
  })
}

/**
 * 拼接 WebSocket URL：`ws(s)://host/ws/chat`。
 * host 取自 apiBase（若已配置绝对地址）或当前页面 origin。
 *
 * ticket 不再拼接到 URL，而是由调用方通过 `new WebSocket(url, ['forgeclaw', ticket])`
 * 作为子协议传递，避免被代理访问日志记录。
 */
export function buildWsUrl(): string {
  let origin: string
  if (apiBase && /^https?:\/\//.test(apiBase)) {
    origin = apiBase.replace(/^http/, 'ws')
  } else if (typeof window !== 'undefined') {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    origin = `${proto}//${window.location.host}`
  } else {
    origin = 'ws://localhost:8080'
  }
  return `${origin}/ws/chat`
}
