<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
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

const menuOpen = ref(false)

function openMenu(): void {
  menuOpen.value = true
}

function closeMenu(): void {
  menuOpen.value = false
}
</script>

<template>
  <router-view v-if="!showLayout" />
  <div v-else class="layout">
    <button
      class="menu-toggle"
      type="button"
      aria-label="打开导航"
      :aria-expanded="menuOpen"
      @click="openMenu"
    >
      <span />
      <span />
      <span />
    </button>

    <aside class="sidebar" :class="{ open: menuOpen }">
      <div class="brand">ForgeClaw</div>
      <nav class="nav">
        <router-link
          v-for="item in navItems"
          :key="item.name"
          :to="item.to"
          class="nav-item"
          active-class="active"
          @click="closeMenu"
        >
          {{ item.label }}
        </router-link>
      </nav>
      <div class="foot">
        <div v-if="auth.user" class="user">{{ auth.user.name }}</div>
        <button class="logout" type="button" @click="logout">登出</button>
      </div>
    </aside>

    <div class="overlay" :class="{ open: menuOpen }" @click="closeMenu" />

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

.menu-toggle {
  display: none;
  position: fixed;
  top: 12px;
  left: 12px;
  z-index: 60;
  width: 40px;
  height: 40px;
  padding: 0;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 5px;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  cursor: pointer;
}

.menu-toggle span {
  display: block;
  width: 18px;
  height: 2px;
  background: var(--color-text);
  border-radius: 1px;
}

.sidebar {
  display: flex;
  flex-direction: column;
  padding: var(--space);
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  z-index: 50;
}

.brand {
  font-size: 22px;
  font-weight: 700;
  color: var(--color-text);
  padding: 4px 8px 18px;
  letter-spacing: 0.04em;
}

.brand::after {
  content: '';
  display: block;
  width: 34px;
  height: 3px;
  margin-top: 10px;
  background: var(--color-primary);
  border-radius: 2px;
  box-shadow: 0 0 12px var(--color-primary-glow);
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
}

.nav-item {
  padding: 9px 12px;
  font-size: 14px;
  color: var(--color-muted);
  text-decoration: none;
  border-radius: var(--radius);
  transition: background 0.15s, color 0.15s;
}

.nav-item:hover {
  color: var(--color-text);
  background: var(--color-surface-elevated);
}

.nav-item.active {
  color: var(--color-primary);
  background: var(--color-surface-elevated);
  font-weight: 600;
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
  padding: 7px 12px;
  font-size: 13px;
  color: var(--color-muted);
  background: transparent;
  border: 1px solid var(--color-border);
  border-radius: var(--radius);
  cursor: pointer;
  transition: color 0.15s, border-color 0.15s;
}

.logout:hover {
  color: var(--color-danger);
  border-color: var(--color-danger);
}

.overlay {
  display: none;
}

.content {
  min-width: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

@media (max-width: 767px) {
  .layout {
    display: flex;
    grid-template-columns: none;
  }

  .menu-toggle {
    display: flex;
  }

  .sidebar {
    position: fixed;
    top: 0;
    left: 0;
    width: 260px;
    height: 100svh;
    transform: translateX(-100%);
    transition: transform 0.25s ease;
    box-shadow: 4px 0 30px rgba(0, 0, 0, 0.35);
  }

  .sidebar.open {
    transform: translateX(0);
  }

  .overlay {
    display: block;
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.25s ease;
    z-index: 40;
  }

  .overlay.open {
    opacity: 1;
    pointer-events: auto;
  }

  .content {
    padding-top: 56px;
  }
}
</style>
