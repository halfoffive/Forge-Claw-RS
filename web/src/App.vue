<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { useAuthStore } from '@/stores/auth'

const route = useRoute()
const router = useRouter()
const auth = useAuthStore()

// F-19: API 401 时 client.ts 派发此事件，这里统一清理登录态并跳转登录页。
function onUnauthorized(): void {
  auth.logout()
  router.replace('/login')
}
onMounted(() => window.addEventListener('forgeclaw:unauthorized', onUnauthorized))
onUnmounted(() => window.removeEventListener('forgeclaw:unauthorized', onUnauthorized))

// 公开页（登录/404）不显示侧边栏布局。
const showLayout = computed(() => !route.meta.public)

const navItems = [
  { name: 'chat', label: '对话', to: '/chat' },
  { name: 'sessions', label: '会话', to: '/sessions' },
  { name: 'tools', label: '工具', to: '/tools' },
  { name: 'prompts', label: '提示词', to: '/prompts' },
  { name: 'settings', label: '设置', to: '/settings' },
] as const

function logout(): void {
  auth.logout()
  router.replace('/login')
}
</script>

<template>
  <router-view v-if="!showLayout" />
  <div v-else class="layout">
    <aside class="sidebar">
      <div class="brand">ForgeClaw</div>
      <nav class="nav">
        <router-link
          v-for="item in navItems"
          :key="item.name"
          :to="item.to"
          class="nav-item"
          active-class="active"
        >
          {{ item.label }}
        </router-link>
      </nav>
      <div class="foot">
        <div v-if="auth.user" class="user">{{ auth.user.name }}</div>
        <button class="logout" type="button" @click="logout">登出</button>
      </div>
    </aside>
    <main class="content">
      <router-view />
    </main>
  </div>
</template>

<style scoped>
.layout {
  display: grid;
  grid-template-columns: 220px 1fr;
  height: 100svh;
  overflow: hidden;
}
.sidebar {
  display: flex;
  flex-direction: column;
  padding: var(--space);
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
}
.brand {
  font-size: 18px;
  font-weight: 700;
  color: var(--color-text);
  padding: 4px 8px 16px;
}
.nav {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
}
.nav-item {
  padding: 8px 12px;
  font-size: 14px;
  color: var(--color-muted);
  text-decoration: none;
  border-radius: var(--radius);
  transition: background 0.15s, color 0.15s;
}
.nav-item:hover {
  color: var(--color-text);
  background: var(--color-bg);
}
.nav-item.active {
  color: var(--color-primary);
  background: var(--color-bg);
  font-weight: 500;
}
.foot {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--color-border);
}
.user {
  font-size: 13px;
  color: var(--color-text);
  padding: 0 8px;
  word-break: break-all;
}
.logout {
  padding: 6px 12px;
  font-size: 13px;
  color: var(--color-muted);
  background: transparent;
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  cursor: pointer;
}
.logout:hover {
  color: var(--color-danger);
  border-color: var(--color-danger);
}
.content {
  min-width: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
</style>
