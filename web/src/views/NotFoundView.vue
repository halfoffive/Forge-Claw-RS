<script setup lang="ts">
import { useRouter } from 'vue-router'

const router = useRouter()
</script>

<template>
  <section class="notfound">
    <h1>404</h1>
    <p>页面不存在</p>
    <button type="button" @click="router.replace('/chat')">返回对话</button>
  </section>
</template>

<style scoped>
.notfound {
  min-height: 100svh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: var(--color-text);
  background: var(--color-bg);
}
.notfound h1 {
  margin: 0;
  font-size: 64px;
  font-weight: 700;
  color: var(--color-primary);
}
.notfound p {
  margin: 0;
  color: var(--color-muted);
}
.notfound button {
  margin-top: 8px;
  padding: 8px 16px;
  font-size: 14px;
  color: #fff;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
}
</style>
