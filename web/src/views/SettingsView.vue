<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'

import { useAuthStore } from '@/stores/auth'
import { useSettingsStore } from '@/stores/settings'

const settings = useSettingsStore()
const auth = useAuthStore()
const router = useRouter()

const server = ref(settings.server)
const saved = ref(false)

function save(): void {
  settings.setServer(server.value)
  saved.value = true
  setTimeout(() => (saved.value = false), 1500)
}

function logout(): void {
  auth.logout()
  router.replace('/login')
}
</script>

<template>
  <section class="page">
    <h1>设置</h1>

    <form class="form" @submit.prevent="save">
      <label class="field">
        <span>服务器地址</span>
        <input
          v-model="server"
          type="text"
          placeholder="留空使用默认（同源 / vite 代理）"
        />
        <small>例如 https://forgeclaw.example.com；留空则前端走相对路径。</small>
      </label>
      <div class="row">
        <button class="primary" type="submit">保存</button>
        <span v-if="saved" class="ok">已保存</span>
      </div>
    </form>

    <hr />

    <div class="account">
      <h2>账户</h2>
      <p v-if="auth.user">当前用户：<strong>{{ auth.user.name }}</strong></p>
      <button class="danger" type="button" @click="logout">登出</button>
    </div>
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
  color: var(--color-text);
}
.page h1 {
  margin: 0;
  font-size: 20px;
  font-weight: 600;
}
.form {
  display: flex;
  flex-direction: column;
  gap: var(--space);
  max-width: 480px;
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
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  outline: none;
}
.field input:focus {
  border-color: var(--color-primary);
}
.field small {
  font-size: 12px;
  color: var(--color-muted);
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.primary {
  padding: 8px 16px;
  font-size: 14px;
  font-weight: 500;
  color: #fff;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
}
.ok {
  font-size: 13px;
  color: var(--color-primary);
}
hr {
  width: 100%;
  border: none;
  border-top: 1px solid var(--color-border);
}
.account {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.account h2 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}
.account p {
  margin: 0;
  font-size: 14px;
  color: var(--color-muted);
}
.danger {
  align-self: flex-start;
  padding: 8px 16px;
  font-size: 14px;
  color: var(--color-danger);
  background: transparent;
  border: 1px solid var(--color-danger);
  border-radius: var(--radius);
  cursor: pointer;
}
</style>
