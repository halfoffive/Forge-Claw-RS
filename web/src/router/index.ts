import { createRouter, createWebHashHistory } from 'vue-router'
import { getToken } from '../api/client'
import HomeView from '../views/HomeView.vue'

const routes = [
  { path: '/', component: HomeView },
  { path: '/login', component: () => import('../views/LoginView.vue') },
  {
    path: '/chat',
    component: () => import('../views/ChatView.vue'),
    meta: { requiresAuth: true },
  },
  {
    path: '/sessions',
    component: () => import('../views/SessionsView.vue'),
    meta: { requiresAuth: true },
  },
  {
    path: '/prompts',
    component: () => import('../views/PromptsView.vue'),
    meta: { requiresAuth: true },
  },
  {
    path: '/tools',
    component: () => import('../views/ToolsView.vue'),
    meta: { requiresAuth: true },
  },
  {
    path: '/settings',
    component: () => import('../views/SettingsView.vue'),
    meta: { requiresAuth: true },
  },
  { path: '/:pathMatch(.*)*', component: () => import('../views/NotFoundView.vue') },
]

const router = createRouter({
  history: createWebHashHistory(),
  routes,
})

function hasValidToken(): boolean {
  const token = getToken()
  return typeof token === 'string' && token.trim().length > 0
}

router.beforeEach((to) => {
  if (to.meta.requiresAuth && !hasValidToken()) {
    return { path: '/login', query: { redirect: to.fullPath } }
  }
})

export default router
