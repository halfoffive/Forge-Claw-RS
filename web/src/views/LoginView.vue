<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { NButton, NCard, NInput, NSpace } from 'naive-ui'
import { useAuthStore } from '../stores/auth'

const router = useRouter()
const auth = useAuthStore()

const name = ref('')
const token = ref('')

async function submit() {
  const ok = await auth.login({ name: name.value, token: token.value })
  if (ok) {
    router.push('/chat')
  }
}
</script>

<template>
  <div class="login-view">
    <NCard title="ForgeClaw 登录" class="login-card">
      <NSpace vertical>
        <NInput v-model:value="name" placeholder="用户名" />
        <NInput v-model:value="token" type="password" placeholder="Token" />
        <NButton type="primary" :loading="auth.loading" :disabled="!name || !token" @click="submit">
          登录
        </NButton>
        <p v-if="auth.error" class="error">{{ auth.error }}</p>
      </NSpace>
    </NCard>
  </div>
</template>

<style scoped>
.login-view {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 100%;
}

.login-card {
  width: 360px;
}

.error {
  color: #d03050;
}
</style>
