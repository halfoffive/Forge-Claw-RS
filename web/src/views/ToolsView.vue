<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { NCard, NCode, NEmpty, NList, NListItem, NSpace, NSpin, NTag } from 'naive-ui'
import { api } from '../api/client'
import type { ToolInfo } from '../api/types'

const tools = ref<ToolInfo[]>([])
const loading = ref(false)
const error = ref<string | null>(null)

onMounted(async () => {
  loading.value = true
  try {
    const res = await api.get<{ tools: ToolInfo[] }>('/api/tools')
    tools.value = res.tools
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
})
</script>

<template>
  <div class="tools-view">
    <h2>工具列表</h2>
    <NSpin v-if="loading" />
    <NEmpty v-else-if="tools.length === 0" description="暂无工具" />
    <NList v-else>
      <NListItem v-for="tool in tools" :key="tool.name">
        <NCard :title="tool.name" size="small">
          <p class="description">{{ tool.description }}</p>
          <NSpace>
            <NTag size="small">参数</NTag>
            <NCode :code="JSON.stringify(tool.parameters, null, 2)" language="json" />
          </NSpace>
        </NCard>
      </NListItem>
    </NList>
    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>

<style scoped>
.tools-view {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.description {
  margin: 0 0 0.5rem;
}

.error {
  color: #d03050;
}
</style>
