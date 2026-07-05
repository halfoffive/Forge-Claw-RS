<script setup lang="ts">
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const router = useRouter()
const auth = useAuthStore()

onMounted(() => {
  if (auth.isLoggedIn) {
    router.replace('/chat')
  } else {
    router.replace('/login')
  }
})
</script>

<template>
  <div />
</template>
