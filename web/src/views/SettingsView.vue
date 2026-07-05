<script setup lang="ts">
import { ref } from 'vue'
import { NButton, NCard, NInput, NSpace } from 'naive-ui'
import { useSettingsStore } from '../stores/settings'

const settings = useSettingsStore()
const serverUrl = ref(settings.serverUrl)
const model = ref(settings.model)
const saved = ref(false)

function save() {
  settings.setServerUrl(serverUrl.value)
  settings.setModel(model.value)
  saved.value = true
  setTimeout(() => (saved.value = false), 2000)
}
</script>

<template>
  <div class="settings-view">
    <h2>设置</h2>
    <NCard title="服务端">
      <NSpace vertical>
        <label>
          服务器地址
          <NInput v-model:value="serverUrl" placeholder="http://localhost:8080" />
        </label>
        <label>
          模型
          <NInput v-model:value="model" placeholder="deepseek-chat" />
        </label>
        <NButton type="primary" @click="save">保存</NButton>
        <span v-if="saved" class="saved">已保存</span>
      </NSpace>
    </NCard>
  </div>
</template>

<style scoped>
.settings-view {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

label {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.saved {
  color: #18a058;
}
</style>
