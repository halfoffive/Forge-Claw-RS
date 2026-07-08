<script setup lang="ts">
import { onMounted, ref } from 'vue'

import { listTools } from '@/api/client'
import { useAuthStore } from '@/stores/auth'
import type { ToolInfo } from '@/api/types'

const auth = useAuthStore()
const tools = ref<ToolInfo[]>([])
const loading = ref(false)
const error = ref('')

onMounted(load)

async function load(): Promise<void> {
  if (!auth.token) return
  loading.value = true
  error.value = ''
  try {
    const data = await listTools(auth.token)
    // 后端实际返回 { tools: [...] }，兼容直接数组形态。
    tools.value = Array.isArray(data)
      ? data
      : (data as unknown as { tools?: ToolInfo[] }).tools ?? []
  } catch (e) {
    error.value = (e as Error).message
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <section class="page">
    <header class="head">
      <h1>工具</h1>
      <button class="refresh" type="button" :disabled="loading" @click="load">
        {{ loading ? '加载中…' : '刷新' }}
      </button>
    </header>

    <p v-if="error" class="err">{{ error }}</p>

    <ul v-if="tools.length" class="list">
      <li v-for="t in tools" :key="t.name" class="item">
        <h2 class="name">{{ t.name }}</h2>
        <p class="desc">{{ t.description }}</p>
        <details v-if="t.parameters != null">
          <summary>参数 schema</summary>
          <pre class="schema">{{ t.parameters }}</pre>
        </details>
      </li>
    </ul>
    <p v-else-if="!loading" class="empty">暂无工具</p>
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
}
.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.item {
  padding: 12px;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
}
.name {
  margin: 0 0 4px;
  font-family: var(--font-mono);
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text);
}
.desc {
  margin: 0;
  font-size: 13px;
  color: var(--color-muted);
  line-height: 1.5;
}
.schema {
  margin: 8px 0 0;
  padding: 8px;
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--color-text);
  background: var(--color-bg);
  border-radius: calc(var(--radius) - 2px);
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
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
