import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '../api/client'
import type { SessionSummary, SessionDetail, Message } from '../api/types'

export const useSessionStore = defineStore('session', () => {
  const sessions = ref<SessionSummary[]>([])
  const currentSession = ref<SessionDetail | null>(null)
  const messages = ref<Message[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)

  async function fetchSessions() {
    loading.value = true
    error.value = null
    try {
      sessions.value = await api.get<SessionSummary[]>('/api/sessions')
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  async function fetchSession(id: string) {
    loading.value = true
    error.value = null
    try {
      const detail = await api.get<SessionDetail>(`/api/sessions/${id}`)
      currentSession.value = detail
      messages.value = detail.messages
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err)
    } finally {
      loading.value = false
    }
  }

  function setMessages(msgs: Message[]) {
    messages.value = msgs
  }

  function appendMessage(msg: Message) {
    messages.value.push(msg)
  }

  function clearCurrent() {
    currentSession.value = null
    messages.value = []
  }

  return {
    sessions,
    currentSession,
    messages,
    loading,
    error,
    fetchSessions,
    fetchSession,
    setMessages,
    appendMessage,
    clearCurrent,
  }
})
