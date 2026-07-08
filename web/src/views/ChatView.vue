<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'

import { ApiError, buildWsUrl, getWsTicket } from '@/api/client'
import type {
  Message,
  OrchestratorEvent,
  ToolCall,
  ToolResult,
  WsChatRequest,
} from '@/api/types'
import { useAuthStore } from '@/stores/auth'
import { useSessionStore } from '@/stores/session'

const auth = useAuthStore()
const session = useSessionStore()
const route = useRoute()

// 从会话列表点入时携带 ?session=id，自动加载历史。
watch(
  () => route.query.session,
  (id) => {
    const sid = typeof id === 'string' ? id : ''
    if (sid && sid !== session.currentId && auth.token) {
      session.openSession(auth.token, sid)
    }
  },
  { immediate: true },
)

const input = ref('')
const sending = ref(false)
const status = ref<'idle' | 'connecting' | 'streaming' | 'error'>('idle')
const errorMsg = ref('')
const messages = computed<Message[]>(() => session.currentMessages)

let ws: WebSocket | null = null

const scrollEl = ref<HTMLDivElement | null>(null)
async function scrollToBottom(): Promise<void> {
  await nextTick()
  if (scrollEl.value) scrollEl.value.scrollTop = scrollEl.value.scrollHeight
}

async function send(): Promise<void> {
  const text = input.value.trim()
  if (!text || sending.value) return
  if (!auth.token) {
    errorMsg.value = '未登录'
    return
  }

  input.value = ''
  session.pushMessage({ User: text })
  await scrollToBottom()

  sending.value = true
  status.value = 'connecting'
  errorMsg.value = ''

  // 占位 assistant 消息：delta 持续追加 text。
  const assistantIdx = session.currentMessages.length
  session.pushMessage({ Assistant: { text: '', tool_calls: [] } })
  await scrollToBottom()

  try {
    const ticket = await getWsTicket(auth.token)
    const url = buildWsUrl(ticket)
    ws = new WebSocket(url)
  } catch (e) {
    status.value = 'error'
    errorMsg.value = e instanceof ApiError ? e.message : '获取 WS ticket 失败'
    sending.value = false
    return
  }

  const payload: WsChatRequest = {
    message: text,
    ...(session.currentId ? { session_id: session.currentId } : {}),
  }

  ws.onopen = () => {
    status.value = 'streaming'
    ws?.send(JSON.stringify(payload))
  }
  ws.onmessage = (ev) => onFrame(ev.data as string, assistantIdx)
  ws.onerror = () => {
    status.value = 'error'
    errorMsg.value = 'WebSocket 连接错误'
  }
  ws.onclose = () => {
    sending.value = false
    if (status.value === 'streaming') status.value = 'idle'
  }
}

function onFrame(raw: string, assistantIdx: number): void {
  let event: OrchestratorEvent
  try {
    event = JSON.parse(raw) as OrchestratorEvent
  } catch {
    return
  }

  switch (event.type) {
    case 'delta': {
      const m = session.currentMessages[assistantIdx]
      if (m && 'Assistant' in m) {
        m.Assistant.text += event.text
      }
      scrollToBottom()
      break
    }
    case 'tool_call_start': {
      const call: ToolCall = { name: event.name, input: event.input }
      session.pushMessage({ Tool: [call, {}] })
      scrollToBottom()
      break
    }
    case 'tool_result': {
      // 回填最近一个同名且 result 为空的 Tool 消息。
      const result: ToolResult = event.result
      for (let i = session.currentMessages.length - 1; i >= 0; i--) {
        const m = session.currentMessages[i]
        if ('Tool' in m && m.Tool[0].name === event.name) {
          const [, prev] = m.Tool
          if (!prev.output && !prev.error) {
            m.Tool[1] = result
            break
          }
        }
      }
      break
    }
    case 'complete': {
      const m = session.currentMessages[assistantIdx]
      if (m && 'Assistant' in m) {
        m.Assistant.text = event.text || m.Assistant.text
        m.Assistant.tool_calls = event.tool_calls ?? []
      }
      status.value = 'idle'
      sending.value = false
      cleanupWs()
      break
    }
    case 'error': {
      errorMsg.value = event.message
      status.value = 'error'
      sending.value = false
      cleanupWs()
      break
    }
  }
}

function cleanupWs(): void {
  if (ws) {
    ws.onmessage = null
    ws.onerror = null
    ws.onclose = null
    if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
      ws.close()
    }
    ws = null
  }
}

onUnmounted(cleanupWs)

function startNew(): void {
  cleanupWs()
  session.newSession()
  status.value = 'idle'
  errorMsg.value = ''
}
</script>

<template>
  <section class="chat">
    <header class="bar">
      <div class="meta">
        <span class="dot" :data-status="status" />
        <span class="label">
          {{ status === 'connecting' ? '连接中…' : status === 'streaming' ? '流式中' : status === 'error' ? '错误' : '就绪' }}
        </span>
        <span v-if="session.currentId" class="sid">{{ session.currentId }}</span>
      </div>
      <button class="ghost" type="button" @click="startNew">新会话</button>
    </header>

    <div ref="scrollEl" class="messages">
      <p v-if="!messages.length" class="empty">发送一条消息开始对话</p>

      <template v-for="(m, i) in messages" :key="i">
        <div v-if="'User' in m" class="msg user">
          <div class="bubble">{{ m.User }}</div>
        </div>

        <div v-else-if="'Assistant' in m" class="msg assistant">
          <pre v-if="m.Assistant.text" class="bubble">{{ m.Assistant.text }}</pre>
          <span v-else class="bubble pending">…</span>
        </div>

        <div v-else class="msg tool">
          <div class="tool-card">
            <div class="tool-head">
              <span class="tool-name">{{ m.Tool[0].name }}</span>
              <span
                class="tool-state"
                :class="{ ok: !!m.Tool[1].output, err: !!m.Tool[1].error }"
              >
                {{ m.Tool[1].error ? '错误' : m.Tool[1].output ? '完成' : '执行中' }}
              </span>
            </div>
            <pre v-if="m.Tool[0].input != null" class="tool-input">{{ m.Tool[0].input }}</pre>
            <pre v-if="m.Tool[1].output" class="tool-out">{{ m.Tool[1].output }}</pre>
            <pre v-if="m.Tool[1].error" class="tool-err">{{ m.Tool[1].error }}</pre>
          </div>
        </div>
      </template>
    </div>

    <p v-if="errorMsg" class="err">{{ errorMsg }}</p>

    <form class="composer" @submit.prevent="send">
      <textarea
        v-model="input"
        class="input"
        rows="2"
        placeholder="输入消息，Enter 发送，Shift+Enter 换行"
        :disabled="sending"
        @keydown.enter.exact.prevent="send"
      />
      <button class="send" type="submit" :disabled="sending || !input.trim()">
        发送
      </button>
    </form>
  </section>
</template>

<style scoped>
.chat {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px var(--space);
  border-bottom: 1px solid var(--color-border);
  font-size: 13px;
}
.meta {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--color-muted);
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--color-muted);
}
.dot[data-status='streaming'] {
  background: var(--color-primary);
}
.dot[data-status='error'] {
  background: var(--color-danger);
}
.sid {
  font-family: var(--font-mono);
  font-size: 12px;
  opacity: 0.7;
}
.ghost {
  padding: 4px 10px;
  font-size: 13px;
  color: var(--color-text);
  background: transparent;
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  cursor: pointer;
}
.ghost:hover {
  border-color: var(--color-primary);
}
.messages {
  flex: 1;
  overflow-y: auto;
  padding: var(--space);
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.empty {
  margin: auto;
  color: var(--color-muted);
  font-size: 14px;
}
.msg {
  display: flex;
}
.msg.user {
  justify-content: flex-end;
}
.bubble {
  max-width: 80%;
  padding: 10px 14px;
  border-radius: var(--radius);
  font-size: 14px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  font-family: inherit;
}
.msg.user .bubble {
  background: var(--color-primary);
  color: #fff;
}
.msg.assistant .bubble {
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  color: var(--color-text);
}
.bubble.pending {
  color: var(--color-muted);
  font-style: italic;
}
.tool-card {
  width: 100%;
  max-width: 80%;
  padding: 10px 12px;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  font-size: 13px;
}
.tool-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}
.tool-name {
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--color-text);
}
.tool-state {
  font-size: 12px;
  color: var(--color-muted);
}
.tool-state.ok {
  color: var(--color-primary);
}
.tool-state.err {
  color: var(--color-danger);
}
.tool-input,
.tool-out,
.tool-err {
  margin: 4px 0 0;
  padding: 6px 8px;
  font-family: var(--font-mono);
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-word;
  background: var(--color-bg);
  border-radius: calc(var(--radius) - 2px);
}
.tool-err {
  color: var(--color-danger);
}
.err {
  margin: 0 var(--space);
  padding: 6px 10px;
  color: var(--color-danger);
  font-size: 13px;
}
.composer {
  display: flex;
  gap: 8px;
  padding: var(--space);
  border-top: 1px solid var(--color-border);
}
.input {
  flex: 1;
  resize: none;
  padding: 10px 12px;
  font-size: 14px;
  font-family: inherit;
  color: var(--color-text);
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  outline: none;
}
.input:focus {
  border-color: var(--color-primary);
}
.send {
  padding: 0 18px;
  font-size: 14px;
  font-weight: 500;
  color: #fff;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
}
.send:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
