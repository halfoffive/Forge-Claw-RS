// 鉴权 store：持久化 token/user 到 localStorage，提供 login/logout。
//
// 注意：后端 `/api/auth/login` 当前响应不含 ticket 字段（与任务契约略有出入），
// 这里仍按契约保留 ticket 字段读取（可选），实际 WS 连接走 getWsTicket() 重新换取。

import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { ApiError, login as apiLogin } from '@/api/client'
import type { User } from '@/api/types'

const TOKEN_KEY = 'forgeclaw.token'
const USER_KEY = 'forgeclaw.user'

function readUser(): User | null {
  try {
    const raw = localStorage.getItem(USER_KEY)
    return raw ? (JSON.parse(raw) as User) : null
  } catch {
    return null
  }
}

export const useAuthStore = defineStore('auth', () => {
  const token = ref<string>(localStorage.getItem(TOKEN_KEY) ?? '')
  const user = ref<User | null>(readUser())

  const isLoggedIn = computed(() => Boolean(token.value && user.value))

  async function login(name: string, rawToken: string): Promise<void> {
    const res = await apiLogin(name, rawToken)
    if (!res.ok || !res.user) {
      throw new ApiError(401, '登录失败：凭据无效')
    }
    token.value = rawToken
    user.value = { id: res.user.id, name: res.user.name }
    localStorage.setItem(TOKEN_KEY, rawToken)
    localStorage.setItem(USER_KEY, JSON.stringify(user.value))
  }

  function logout(): void {
    token.value = ''
    user.value = null
    localStorage.removeItem(TOKEN_KEY)
    localStorage.removeItem(USER_KEY)
  }

  return { token, user, isLoggedIn, login, logout }
})
