import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api, setToken, getToken } from '../api/client'
import type { LoginRequest, User } from '../api/types'

export const useAuthStore = defineStore('auth', () => {
  const token = ref<string | null>(getToken())
  const user = ref<User | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  const isLoggedIn = computed(() => !!token.value)

  async function login(credentials: LoginRequest) {
    loading.value = true
    error.value = null
    try {
      const res = await api.post<{ ok: boolean; user: User }>('/api/auth/login', credentials)
      if (!res.ok || !res.user) {
        throw new Error('Login failed')
      }
      user.value = res.user
      token.value = res.user.token
      setToken(res.user.token)
      return true
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
      return false
    } finally {
      loading.value = false
    }
  }

  function logout() {
    user.value = null
    token.value = null
    setToken(null)
  }

  return {
    token,
    user,
    loading,
    error,
    isLoggedIn,
    login,
    logout,
  }
})
