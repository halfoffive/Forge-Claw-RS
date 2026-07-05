<script setup lang="ts">
import { ref } from 'vue'
import { NButton, NCard, NInput, NSpace, NSpin } from 'naive-ui'
import { api } from '../api/client'
import type { Section } from '../api/types'

const profile = ref('default')
const body = ref('')
const compiled = ref('')
const sections = ref<Section[]>([])
const loading = ref(false)
const error = ref<string | null>(null)

async function listSections() {
  loading.value = true
  error.value = null
  try {
    sections.value = await api.get<Section[]>(`/api/prompts/sections?profile=${encodeURIComponent(profile.value)}`)
    if (sections.value.length > 0) {
      body.value = sections.value.map((s) => `## ${s.title}\n${s.body}`).join('\n\n')
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function compilePrompt() {
  loading.value = true
  error.value = null
  try {
    const res = await api.post<{ prompt: string }>('/api/prompts/compile', { profile: profile.value })
    compiled.value = res.prompt
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="prompts-view">
    <h2>提示词</h2>
    <NSpace align="center">
      <NInput v-model:value="profile" placeholder="profile" style="width: 200px" />
      <NButton @click="listSections">加载章节</NButton>
      <NButton type="primary" @click="compilePrompt">编译</NButton>
    </NSpace>

    <NSpin v-if="loading" />
    <p v-if="error" class="error">{{ error }}</p>

    <NCard title="编辑器">
      <NInput
        v-model:value="body"
        type="textarea"
        placeholder="提示词内容..."
        :autosize="{ minRows: 10, maxRows: 20 }"
      />
    </NCard>

    <NCard v-if="compiled" title="编译结果">
      <pre class="compiled">{{ compiled }}</pre>
    </NCard>
  </div>
</template>

<style scoped>
.prompts-view {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.compiled {
  margin: 0;
  white-space: pre-wrap;
  font-family: var(--mono);
}

.error {
  color: #d03050;
}
</style>
