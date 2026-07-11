// 会话 store：维护会话列表 + 当前会话消息。
//
// 与 WS 解耦：ChatView 负责建立 WS 并把事件转成 Message 追加进 currentMessages；
// 本 store 负责 REST 拉取列表/详情与切换当前会话。

import { defineStore } from 'pinia'
import { ref } from 'vue'

import { getSession, listSessions } from '@/api/client'
import type { Message, SessionSummary } from '@/api/types'

export const useSessionStore = defineStore('session', () => {
  const sessions = ref<SessionSummary[]>([])
  const currentId = ref<string | null>(null)
  const currentMessages = ref<Message[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)

  async function fetchSessions(token: string): Promise<void> {
    loading.value = true
    error.value = null
    try {
      sessions.value = await listSessions(token)
    } catch (e) {
      error.value = (e as Error).message
      sessions.value = []
    } finally {
      loading.value = false
    }
  }

  async function openSession(token: string, id: string): Promise<void> {
    loading.value = true
    error.value = null
    try {
      const detail = await getSession(token, id)
      currentId.value = id
      currentMessages.value = detail.messages ?? []
    } catch (e) {
      error.value = (e as Error).message
    } finally {
      loading.value = false
    }
  }

  /** 新建空白会话（尚未持久化到后端，首条消息发送时由后端分配 id）。 */
  function newSession(): void {
    currentId.value = null
    currentMessages.value = []
    error.value = null
  }

  /** 由 ChatView 在收到 complete 帧后回写后端分配的 session_id。 */
  function setCurrentId(id: string): void {
    currentId.value = id
  }

  function pushMessage(msg: Message): void {
    currentMessages.value.push(msg)
  }

  /** 撤销最后一次 push（用于 WS 建立失败时清理用户消息和占位 assistant）。 */
  function popMessage(): void {
    currentMessages.value.pop()
  }

  /** 撤销最后 count 次 push。 */
  function popMessages(count: number): void {
    for (let i = 0; i < count; i++) currentMessages.value.pop()
  }

  function reset(): void {
    sessions.value = []
    currentId.value = null
    currentMessages.value = []
    error.value = null
  }

  return {
    sessions,
    currentId,
    currentMessages,
    loading,
    error,
    fetchSessions,
    openSession,
    newSession,
    setCurrentId,
    pushMessage,
    popMessage,
    popMessages,
    reset,
  }
})
