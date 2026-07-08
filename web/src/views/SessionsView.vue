<script setup lang="ts">
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'

import { useAuthStore } from '@/stores/auth'
import { useSessionStore } from '@/stores/session'

const auth = useAuthStore()
const session = useSessionStore()
const router = useRouter()

onMounted(() => {
  if (auth.token) session.fetchSessions(auth.token)
})

function open(id: string): void {
  router.push({ path: '/chat', query: { session: id } })
}
</script>

<template>
  <section class="page">
    <header class="head">
      <h1>会话</h1>
      <button class="refresh" type="button" :disabled="session.loading" @click="auth.token && session.fetchSessions(auth.token)">
        {{ session.loading ? '刷新中…' : '刷新' }}
      </button>
    </header>

    <p v-if="session.error" class="err">{{ session.error }}</p>

    <ul v-if="session.sessions.length" class="list">
      <li v-for="s in session.sessions" :key="s.id">
        <button class="item" type="button" @click="open(s.id)">
          <span class="id">{{ s.id }}</span>
          <span class="meta">
            <span v-if="s.message_count != null">{{ s.message_count }} 条消息</span>
            <span v-if="s.created_at">{{ new Date(s.created_at).toLocaleString() }}</span>
          </span>
        </button>
      </li>
    </ul>
    <p v-else-if="!session.loading" class="empty">暂无会话</p>
  </section>
</template>

<style scoped>
.page {
  padding: var(--space);
  display: flex;
  flex-direction: column;
  gap: var(--space);
  height: 100%;
  overflow-y: auto;
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.head h1 {
  margin: 0;
  font-size: 20px;
  font-weight: 600;
  color: var(--color-text);
}
.refresh {
  padding: 6px 12px;
  font-size: 13px;
  color: var(--color-text);
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  cursor: pointer;
}
.refresh:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.item {
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  padding: 12px;
  text-align: left;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  cursor: pointer;
}
.item:hover {
  border-color: var(--color-primary);
}
.id {
  font-family: var(--font-mono);
  font-size: 13px;
  color: var(--color-text);
}
.meta {
  display: flex;
  gap: 12px;
  font-size: 12px;
  color: var(--color-muted);
}
.empty,
.err {
  margin: 0;
  color: var(--color-muted);
  font-size: 14px;
}
.err {
  color: var(--color-danger);
}
</style>
