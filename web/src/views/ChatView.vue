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

let pendingText = ''

function rollback(): void {
  if (!pendingText) return
  session.popMessage()
  session.popMessage()
  input.value = pendingText
  pendingText = ''
}

async function send(): Promise<void> {
  const text = input.value.trim()
  if (!text || sending.value) return
  if (!auth.token) {
    errorMsg.value = '未登录'
    return
  }

  pendingText = text
  input.value = ''
  session.pushMessage({ User: text })
  await scrollToBottom()

  sending.value = true
  status.value = 'connecting'
  errorMsg.value = ''

  const assistantIdx = session.currentMessages.length
  session.pushMessage({ Assistant: { text: '', tool_calls: [] } })
  await scrollToBottom()

  if (!session.currentId) {
    session.setCurrentId(crypto.randomUUID())
  }
  const currentId = session.currentId

  const payload: WsChatRequest = {
    message: text,
    ...(currentId ? { session_id: currentId } : {}),
  }

  try {
    const ticket = await getWsTicket(auth.token)
    const url = buildWsUrl(ticket)
    ws = new WebSocket(url)
  } catch (e) {
    rollback()
    status.value = 'error'
    errorMsg.value = e instanceof ApiError ? e.message : '获取 WS ticket 失败'
    sending.value = false
    return
  }

  ws.onopen = () => {
    status.value = 'streaming'
    ws?.send(JSON.stringify(payload))
  }
  ws.onmessage = (ev) => onFrame(ev.data as string, assistantIdx)
  ws.onerror = () => {
    status.value = 'error'
    errorMsg.value = 'WebSocket 连接错误'
    rollback()
    sending.value = false
  }
  ws.onclose = () => {
    if (status.value === 'streaming' || status.value === 'connecting') {
      errorMsg.value = '连接中断，回复可能不完整'
      rollback()
    }
    if (status.value !== 'error') {
      status.value = 'idle'
    }
    sending.value = false
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
      // F-01: ToolCall 字段对齐后端 { id, tool, input }。
      const call: ToolCall = {
        id: event.call_id,
        tool: event.name,
        input: event.input,
      }
      // F-03: 占位 result 须满足必填字段，待 tool_result 回填。
      session.pushMessage({ Tool: [call, { output: '', duration_ms: 0 }] })
      scrollToBottom()
      break
    }
    case 'tool_result': {
      // 按 call_id 回填对应 Tool 消息，支持并发同名工具调用。
      const result: ToolResult = event.result
      for (let i = session.currentMessages.length - 1; i >= 0; i--) {
        const m = session.currentMessages[i]
        if ('Tool' in m && m.Tool[0].id === event.call_id) {
          m.Tool[1] = result
          break
        }
      }
      break
    }
    case 'complete': {
      const m = session.currentMessages[assistantIdx]
      if (m && 'Assistant' in m) {
        m.Assistant.text = event.text || m.Assistant.text
      }
      pendingText = ''
      status.value = 'idle'
      sending.value = false
      cleanupWs()
      break
    }
    case 'error': {
      errorMsg.value = event.message
      status.value = 'error'
      rollback()
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
  session.setCurrentId(crypto.randomUUID())
  status.value = 'idle'
  errorMsg.value = ''
  pendingText = ''
  sending.value = false
}

// F-27: 中文输入法 composing 期间不触发发送。
function onEnter(e: KeyboardEvent): void {
  if (e.isComposing) return
  send()
}

type Segment =
  | { type: 'text'; content: string }
  | { type: 'code'; language: string; content: string }

// 将助手回复拆分为普通文本段落与代码块，分别渲染。
function parseSegments(text: string): Segment[] {
  const segments: Segment[] = []
  const regex = /```(\w*)\n?([\s\S]*?)```/g
  let lastIndex = 0
  let match: RegExpExecArray | null
  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIndex) {
      segments.push({ type: 'text', content: text.slice(lastIndex, match.index) })
    }
    segments.push({ type: 'code', language: match[1] || '', content: match[2] })
    lastIndex = regex.lastIndex
  }
  if (lastIndex < text.length) {
    segments.push({ type: 'text', content: text.slice(lastIndex) })
  }
  return segments
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
          <div class="bubble">
            <p class="text-segment">{{ m.User }}</p>
          </div>
        </div>

        <div v-else-if="'Assistant' in m" class="msg assistant">
          <div v-if="m.Assistant.text" class="bubble">
            <template v-for="(seg, si) in parseSegments(m.Assistant.text)" :key="si">
              <p v-if="seg.type === 'text'" class="text-segment">{{ seg.content }}</p>
              <div v-else class="code-wrap">
                <div v-if="seg.language" class="code-lang">{{ seg.language }}</div>
                <pre class="code-block"><code>{{ seg.content }}</code></pre>
              </div>
            </template>
          </div>
          <span v-else class="bubble pending">…</span>
        </div>

        <div v-else class="msg tool">
          <div class="tool-card">
            <div class="tool-head">
              <span class="tool-name">{{ m.Tool[0].tool }}</span>
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
        @keydown.enter.exact.prevent="onEnter"
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
  box-shadow: 0 0 8px var(--color-primary-glow);
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
  padding: 5px 12px;
  font-size: 13px;
  color: var(--color-text);
  background: transparent;
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  cursor: pointer;
  transition: border-color 0.15s, color 0.15s;
}
.ghost:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
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
  font-size: 15px;
  line-height: 1.65;
  margin: 0;
  font-family: inherit;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.text-segment {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
}
.msg.user .bubble {
  background: var(--color-primary);
  color: #1f2024;
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
.code-wrap {
  overflow: hidden;
  border-radius: calc(var(--radius) - 2px);
  border: 1px solid var(--color-border);
}
.code-lang {
  padding: 5px 10px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--color-muted);
  background: var(--color-surface-elevated);
  border-bottom: 1px solid var(--color-border);
}
.code-block {
  margin: 0;
  padding: 10px 12px;
  overflow-x: auto;
  font-family: var(--font-mono);
  font-size: 13px;
  line-height: 1.55;
  background: var(--color-bg);
  color: var(--color-text);
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
  font-size: 15px;
  font-family: inherit;
  color: var(--color-text);
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  outline: none;
}
.input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-glow);
}
.send {
  padding: 0 20px;
  font-size: 15px;
  font-weight: 600;
  color: #1f2024;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
  transition: box-shadow 0.15s;
}
.send:hover:not(:disabled) {
  box-shadow: 0 4px 18px var(--color-primary-glow);
}
.send:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

@media (max-width: 767px) {
  .bar {
    padding-left: 56px;
  }
  .bubble,
  .tool-card {
    max-width: 92%;
  }
  .composer {
    flex-direction: column;
  }
  .send {
    padding: 10px;
  }
}
</style>
