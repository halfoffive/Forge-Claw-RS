<script setup lang="ts">
import { NButton, NEmpty } from 'naive-ui'
import { useRouter } from 'vue-router'

const router = useRouter()
</script>

<template>
  <div class="not-found">
    <NEmpty description="页面不存在">
      <template #extra>
        <NButton @click="router.push('/chat')">返回首页</NButton>
      </template>
    </NEmpty>
  </div>
</template>

<style scoped>
.not-found {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 100%;
}
</style>
