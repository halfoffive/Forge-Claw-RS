<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { NButton, NLayout, NLayoutSider, NMenu, NSpace } from 'naive-ui'
import type { MenuOption } from 'naive-ui'
import { useAuthStore } from './stores/auth'

const route = useRoute()
const router = useRouter()
const auth = useAuthStore()

const hideNav = computed(() => route.path === '/login' || route.path === '/')

const menuOptions: MenuOption[] = [
  { label: '对话', key: '/chat' },
  { label: '会话', key: '/sessions' },
  { label: '提示词', key: '/prompts' },
  { label: '工具', key: '/tools' },
  { label: '设置', key: '/settings' },
]

const activeKey = computed(() => route.path)

function handleMenuSelect(key: string) {
  router.push(key)
}

function logout() {
  auth.logout()
  router.push('/login')
}
</script>

<template>
  <NLayout has-sider class="app-layout">
    <NLayoutSider
      v-if="!hideNav"
      bordered
      collapse-mode="width"
      :collapsed-width="64"
      :width="180"
      show-trigger
    >
      <NSpace vertical justify="space-between" class="sider-content">
        <NMenu
          :value="activeKey"
          :options="menuOptions"
          :collapsed-width="64"
          :collapsed-icon-size="22"
          @update:value="handleMenuSelect"
        />
        <NButton v-if="auth.isLoggedIn" class="logout-btn" ghost @click="logout">
          退出
        </NButton>
      </NSpace>
    </NLayoutSider>
    <NLayout class="main-layout">
      <main class="main-content">
        <router-view />
      </main>
    </NLayout>
  </NLayout>
</template>

<style scoped>
.app-layout {
  min-height: 100svh;
}

.sider-content {
  height: 100%;
  padding: 1rem 0;
}

.logout-btn {
  margin: 0 1rem;
}

.main-layout {
  padding: 1.5rem;
}

.main-content {
  max-width: 1200px;
  margin: 0 auto;
  width: 100%;
  height: 100%;
}
</style>
