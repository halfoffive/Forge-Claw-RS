<script setup lang="ts">
import { ref } from 'vue'

import { compilePrompt, listSections, saveSections } from '@/api/client'
import type { Section } from '@/api/client'
import { useAuthStore } from '@/stores/auth'

const auth = useAuthStore()
const profile = ref('default')
const body = ref('')
const compiled = ref('')
const sections = ref<Section[]>([])
const loading = ref(false)
const saving = ref(false)
const saved = ref(false)
const error = ref<string | null>(null)

function slugify(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^\w\s-]/g, '')
    .replace(/\s+/g, '-')
    .slice(0, 40)
}

function parseBody(text: string): Section[] {
  const chunks = text.split(/^## /m).filter((c) => c.trim().length > 0)
  return chunks.map((chunk, index) => {
    const newlineIndex = chunk.indexOf('\n')
    const title =
      newlineIndex === -1 ? chunk.trim() : chunk.slice(0, newlineIndex).trim()
    const bodyText =
      newlineIndex === -1 ? '' : chunk.slice(newlineIndex + 1).trim()
    const existing = sections.value[index]
    return {
      id: existing?.id ?? (slugify(title) || `section-${index}`),
      title,
      level: existing?.level ?? 'allow',
      enabled: existing?.enabled ?? true,
      order: existing?.order ?? index * 10,
      body: bodyText,
    }
  })
}

async function loadSections(): Promise<void> {
  if (!auth.token) return
  loading.value = true
  error.value = null
  try {
    const data = await listSections(auth.token, profile.value)
    sections.value = data as Section[]
    if (sections.value.length > 0) {
      body.value = sections.value
        .map((s) => `## ${s.title}\n${s.body}`)
        .join('\n\n')
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function doCompile(): Promise<void> {
  if (!auth.token) return
  loading.value = true
  error.value = null
  try {
    const res = await compilePrompt(auth.token, profile.value)
    compiled.value = res.prompt
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function doSave(): Promise<void> {
  if (!auth.token) return
  saving.value = true
  error.value = null
  saved.value = false
  try {
    const parsed = parseBody(body.value)
    await saveSections(auth.token, profile.value, parsed)
    sections.value = parsed
    saved.value = true
    setTimeout(() => {
      saved.value = false
    }, 1500)
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div class="prompts-view">
    <h2>提示词</h2>
    <div class="toolbar">
      <input
        v-model="profile"
        class="profile-input"
        type="text"
        placeholder="profile"
      />
      <button class="btn" type="button" @click="loadSections">加载章节</button>
      <button class="btn btn-primary" type="button" :disabled="saving" @click="doSave">
        {{ saving ? '保存中…' : saved ? '已保存' : '保存' }}
      </button>
      <button class="btn btn-primary" type="button" @click="doCompile">编译</button>
    </div>

    <p v-if="loading" class="loading">加载中…</p>
    <p v-if="error" class="error">{{ error }}</p>

    <div class="card">
      <h3 class="card-title">编辑器</h3>
      <textarea
        v-model="body"
        class="editor"
        placeholder="提示词内容..."
        rows="12"
      />
    </div>

    <div v-if="compiled" class="card">
      <h3 class="card-title">编译结果</h3>
      <pre class="compiled">{{ compiled }}</pre>
    </div>
  </div>
</template>

<style scoped>
.prompts-view {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  padding: var(--space);
  height: 100%;
  overflow-y: auto;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
}

.profile-input {
  width: 200px;
  padding: 6px 10px;
  font-size: 14px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  background: var(--color-bg);
  color: var(--color-text);
}

.btn {
  padding: 6px 14px;
  font-size: 14px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  background: var(--color-bg);
  color: var(--color-text);
  cursor: pointer;
  transition: background 0.15s;
}

.btn:hover {
  background: var(--color-surface);
}

.btn-primary {
  background: var(--color-primary);
  color: var(--color-on-primary);
  border-color: var(--color-primary);
}

.btn-primary:hover {
  opacity: 0.9;
  background: var(--color-primary);
}

.btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.profile-input:focus-visible,
.btn:focus-visible,
.editor:focus-visible {
  outline: 2px solid var(--color-primary);
  outline-offset: 2px;
}

.card {
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  padding: 12px;
  background: var(--color-surface);
}

.card-title {
  margin: 0 0 8px;
  font-size: 14px;
  font-weight: 600;
  color: var(--color-muted);
}

.editor {
  width: 100%;
  font-size: 13px;
  font-family: var(--font-mono, monospace);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  padding: 10px;
  background: var(--color-bg);
  color: var(--color-text);
  resize: vertical;
}

.compiled {
  margin: 0;
  white-space: pre-wrap;
  font-family: var(--font-mono, monospace);
  font-size: 13px;
  color: var(--color-text);
}

.loading {
  color: var(--color-muted);
}

.error {
  color: var(--color-danger, #d03050);
}

@media (prefers-reduced-motion: reduce) {
  .btn {
    transition: none;
  }
}
</style>
