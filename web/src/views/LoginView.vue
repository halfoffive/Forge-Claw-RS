<script setup lang="ts">
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { useAuthStore } from '@/stores/auth'
import { ApiError } from '@/api/client'

const auth = useAuthStore()
const router = useRouter()
const route = useRoute()

const name = ref('')
const token = ref('')
const error = ref('')
const submitting = ref(false)

async function onSubmit(): Promise<void> {
  error.value = ''
  if (!name.value.trim() || !token.value.trim()) {
    error.value = '请填写用户名与 Token'
    return
  }
  submitting.value = true
  try {
    await auth.login(name.value.trim(), token.value.trim())
    const redirect = (route.query.redirect as string) || '/chat'
    router.replace(redirect)
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : '登录失败，请重试'
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <main class="login">
    <form class="card" @submit.prevent="onSubmit">
      <h1 class="title">ForgeClaw</h1>
      <p class="subtitle">登录以开始对话</p>

      <label class="field">
        <span>用户名</span>
        <input
          v-model="name"
          type="text"
          autocomplete="username"
          placeholder="your name"
          :disabled="submitting"
        />
      </label>

      <label class="field">
        <span>Token</span>
        <input
          v-model="token"
          type="password"
          autocomplete="current-password"
          placeholder="access token"
          :disabled="submitting"
        />
      </label>

      <p v-if="error" class="error">{{ error }}</p>

      <button type="submit" class="submit" :disabled="submitting">
        {{ submitting ? '登录中…' : '登录' }}
      </button>
    </form>
  </main>
</template>

<style scoped>
.login {
  min-height: 100svh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--space);
  background: var(--color-bg);
}
.card {
  width: 100%;
  max-width: 360px;
  display: flex;
  flex-direction: column;
  gap: var(--space);
  padding: calc(var(--space) * 2);
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
}
.title {
  margin: 0;
  font-size: 28px;
  font-weight: 600;
  color: var(--color-text);
  text-align: center;
}
.subtitle {
  margin: 0;
  text-align: center;
  color: var(--color-muted);
  font-size: 14px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
  color: var(--color-muted);
}
.field input {
  padding: 10px 12px;
  font-size: 14px;
  color: var(--color-text);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  outline: none;
}
.field input:focus {
  border-color: var(--color-primary);
}
.submit {
  margin-top: 4px;
  padding: 10px 12px;
  font-size: 14px;
  font-weight: 500;
  color: #fff;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
}
.submit:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.error {
  margin: 0;
  color: var(--color-danger);
  font-size: 13px;
}
</style>
