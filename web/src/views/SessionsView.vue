<script setup lang="ts">
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { NButton, NCard, NEmpty, NList, NListItem, NSpace, NSpin, NTag } from 'naive-ui'
import { useSessionStore } from '../stores/session'

const router = useRouter()
const sessionStore = useSessionStore()

function openSession(id: string) {
  router.push(`/chat?session_id=${id}`)
}

onMounted(() => {
  sessionStore.fetchSessions()
})
</script>

<template>
  <div class="sessions-view">
    <h2>会话列表</h2>
    <NSpin v-if="sessionStore.loading" />
    <NEmpty v-else-if="sessionStore.sessions.length === 0" description="暂无会话" />
    <NList v-else>
      <NListItem v-for="s in sessionStore.sessions" :key="s.id">
        <NCard size="small" hoverable @click="openSession(s.id)">
          <NSpace justify="space-between" align="center">
            <NSpace>
              <NTag size="small">{{ new Date(s.created_at).toLocaleString() }}</NTag>
              <span>消息数: {{ s.message_count }}</span>
            </NSpace>
            <NButton size="small" @click.stop="openSession(s.id)">打开</NButton>
          </NSpace>
        </NCard>
      </NListItem>
    </NList>
    <p v-if="sessionStore.error" class="error">{{ sessionStore.error }}</p>
  </div>
</template>

<style scoped>
.sessions-view {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.error {
  color: #d03050;
}
</style>
