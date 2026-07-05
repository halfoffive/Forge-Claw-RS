import { defineStore } from 'pinia'
import { ref } from 'vue'
import { getServerUrl, setServerUrl as saveServerUrl } from '../api/client'

const MODEL_KEY = 'forgeclaw_model'

export const useSettingsStore = defineStore('settings', () => {
  const serverUrl = ref(getServerUrl() || '')
  const model = ref(localStorage.getItem(MODEL_KEY) || 'deepseek-chat')

  function setServerUrl(url: string) {
    serverUrl.value = url
    saveServerUrl(url)
  }

  function setModel(value: string) {
    model.value = value
    localStorage.setItem(MODEL_KEY, value)
  }

  return {
    serverUrl,
    model,
    setServerUrl,
    setModel,
  }
})
