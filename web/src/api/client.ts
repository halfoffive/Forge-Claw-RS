const DEFAULT_TIMEOUT_MS = 30_000
const TOKEN_KEY = 'forgeclaw_token'
const SERVER_URL_KEY = 'forgeclaw_server_url'

export class ApiError extends Error {
  status?: number
  code?: string

  constructor(message: string, status?: number, code?: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY)
}

export function setToken(token: string | null): void {
  if (token) localStorage.setItem(TOKEN_KEY, token)
  else localStorage.removeItem(TOKEN_KEY)
}

export function getServerUrl(): string {
  return localStorage.getItem(SERVER_URL_KEY) || ''
}

export function setServerUrl(url: string): void {
  localStorage.setItem(SERVER_URL_KEY, url)
}

function buildBaseUrl(): string {
  return getServerUrl().replace(/\/$/, '')
}

function buildUrl(path: string): string {
  const base = buildBaseUrl()
  if (!base) return path
  const sep = path.startsWith('/') ? '' : '/'
  return `${base}${sep}${path}`
}

function redirectToLogin(): void {
  localStorage.removeItem(TOKEN_KEY)
  window.location.href = '#/login'
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const url = buildUrl(path)
  const token = getToken()

  const headers = new Headers(options.headers)
  headers.set('Accept', 'application/json')
  if (!(options.body instanceof FormData)) {
    headers.set('Content-Type', 'application/json')
  }
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }

  const controller = new AbortController()
  const timeoutId = setTimeout(() => controller.abort('timeout'), DEFAULT_TIMEOUT_MS)

  try {
    const res = await fetch(url, {
      ...options,
      headers,
      signal: controller.signal,
    })

    if (res.status === 401) {
      redirectToLogin()
      throw new ApiError('Unauthorized', 401, 'UNAUTHORIZED')
    }

    if (!res.ok) {
      let message = `HTTP ${res.status}`
      try {
        const body = await res.json()
        if (body.error) message = body.error
      } catch {
        // ignore parse failure
      }
      throw new ApiError(message, res.status, 'HTTP_ERROR')
    }

    if (res.status === 204) {
      return undefined as T
    }
    return (await res.json()) as T
  } catch (err) {
    if (err instanceof ApiError) throw err
    if (err instanceof Error && err.name === 'AbortError') {
      throw new ApiError('Request timeout', undefined, 'TIMEOUT')
    }
    throw new ApiError(err instanceof Error ? err.message : String(err), undefined, 'NETWORK')
  } finally {
    clearTimeout(timeoutId)
  }
}

export const api = {
  get: <T>(path: string) => request<T>(path, { method: 'GET' }),
  post: <T>(path: string, body: unknown) => request<T>(path, { method: 'POST', body: JSON.stringify(body) }),
}
