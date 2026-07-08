// 设置 store：服务器地址（API base），持久化到 localStorage。
// 切换 server 后通过 setApiBase 同步给 api/client。

import { defineStore } from 'pinia'
import { ref } from 'vue'

import { setApiBase } from '@/api/client'

const SERVER_KEY = 'forgeclaw.server'

export const useSettingsStore = defineStore('settings', () => {
  const server = ref<string>(localStorage.getItem(SERVER_KEY) ?? '')

  // 初始化时同步一次到 client（空串 = 走相对路径/vite 代理）。
  setApiBase(server.value)

  function setServer(value: string): void {
    server.value = value.trim()
    localStorage.setItem(SERVER_KEY, server.value)
    setApiBase(server.value)
  }

  return { server, setServer }
})
