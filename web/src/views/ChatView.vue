<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import {
  NButton,
  NCard,
  NEmpty,
  NInput,
  NList,
  NListItem,
  NSpace,
  NSpin,
  NTag,
} from 'naive-ui'
import { useRoute, useRouter } from 'vue-router'
import { getServerUrl, getToken } from '../api/client'
import type { Message, OrchestratorEvent } from '../api/types'
import { useSessionStore } from '../stores/session'

const route = useRoute()
const router = useRouter()
const sessionStore = useSessionStore()

const input = ref('')
const sessionId = computed(() => (route.query.session_id as string) || undefined)
const messages = computed(() => sessionStore.messages)
const loading = ref(false)
const streaming = ref(false)
const error = ref<string | null>(null)

let ws: WebSocket | null = null

function messageText(msg: Message): string {
  if ('User' in msg) return msg.User
  if ('Assistant' in msg) return msg.Assistant.text
  if ('Tool' in msg) return `[${msg.Tool[0].tool}] ${msg.Tool[1].output || msg.Tool[1].error || ''}`
  return ''
}

function messageRole(msg: Message): string {
  if ('User' in msg) return 'user'
  if ('Assistant' in msg) return 'assistant'
  if ('Tool' in msg) return 'tool'
  return ''
}

function buildWsUrl(): string {
  const base = getServerUrl() || window.location.origin
  const url = new URL('/ws/chat', base)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  const token = getToken()
  if (token) url.searchParams.set('token', token)
  return url.toString()
}

function connect() {
  if (ws) return
  ws = new WebSocket(buildWsUrl())
  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data) as OrchestratorEvent
      if (data.type === 'delta') {
        const last = sessionStore.messages[sessionStore.messages.length - 1]
        if (last && 'Assistant' in last) {
          last.Assistant.text += data.text
        } else {
          sessionStore.appendMessage({ Assistant: { text: data.text, tool_calls: [] } })
        }
      } else if (data.type === 'tool_call_start') {
        sessionStore.appendMessage({
          Tool: [
            { id: '', tool: data.name, input: data.input },
            { output: '', error: null, duration_ms: 0 },
          ],
        })
      } else if (data.type === 'tool_result') {
        sessionStore.appendMessage({
          Tool: [
            { id: '', tool: data.name, input: {} },
            data.result,
          ],
        })
      } else if (data.type === 'complete') {
        streaming.value = false
      } else if (data.type === 'error') {
        streaming.value = false
        error.value = data.message
      }
    } catch {
      // ignore malformed frame
    }
  }
  ws.onerror = () => {
    streaming.value = false
    error.value = 'WebSocket error'
  }
  ws.onclose = () => {
    ws = null
    streaming.value = false
  }
}

function sendMessage() {
  const text = input.value.trim()
  if (!text || streaming.value || !ws) return
  sessionStore.appendMessage({ User: text })
  input.value = ''
  streaming.value = true
  error.value = null
  ws.send(JSON.stringify({ message: text, session_id: sessionId.value }))
}

function startNewChat() {
  sessionStore.clearCurrent()
  router.push('/chat')
}

watch(
  () => route.query.session_id as string | undefined,
  async (id) => {
    if (id) {
      await sessionStore.fetchSession(id)
    }
  },
  { immediate: true },
)

watch(
  messages,
  () => {
    nextTick(() => {
      const el = document.querySelector('.chat-messages')
      if (el) el.scrollTop = el.scrollHeight
    })
  },
  { deep: true },
)

connect()
onBeforeUnmount(() => {
  if (ws) {
    ws.close()
    ws = null
  }
})
</script>

<template>
  <div class="chat-view">
    <NSpace justify="space-between" align="center">
      <h2>对话</h2>
      <NButton size="small" @click="startNewChat">新对话</NButton>
    </NSpace>

    <NCard class="chat-card">
      <div class="chat-messages">
        <NEmpty v-if="messages.length === 0" description="开始新对话" />
        <NList v-else>
          <NListItem v-for="(msg, idx) in messages" :key="idx">
            <NSpace align="start">
              <NTag :type="messageRole(msg) === 'user' ? 'primary' : 'default'" size="small">
                {{ messageRole(msg) }}
              </NTag>
              <pre class="message-text">{{ messageText(msg) }}</pre>
            </NSpace>
          </NListItem>
        </NList>
      </div>
    </NCard>

    <NSpace class="chat-input" align="start">
      <NInput
        v-model:value="input"
        type="textarea"
        placeholder="输入消息..."
        :disabled="streaming"
        :autosize="{ minRows: 2, maxRows: 6 }"
        @keydown.enter.prevent="sendMessage"
      />
      <NButton type="primary" :loading="streaming" :disabled="!input.trim()" @click="sendMessage">
        发送
      </NButton>
    </NSpace>

    <NSpin v-if="loading" />
    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>

<style scoped>
.chat-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 1rem;
}

.chat-card {
  flex: 1;
  overflow: hidden;
}

.chat-messages {
  height: 100%;
  overflow-y: auto;
}

.message-text {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: var(--mono);
}

.chat-input {
  width: 100%;
}

.chat-input .n-input {
  flex: 1;
}

.error {
  color: #d03050;
}
</style>
