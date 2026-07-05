import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// https://vite.dev/config/
export default defineConfig({
  // Relative asset paths so the built bundle works when embedded into the
  // Rust binary via rust-embed (served from an arbitrary sub-path).
  base: './',
  plugins: [vue()],
  server: {
    proxy: {
      '/api': 'http://localhost:8080',
      '/ws': {
        target: 'http://localhost:8080',
        ws: true,
      },
    },
  },
})
