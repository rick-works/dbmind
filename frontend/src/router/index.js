import { createRouter, createWebHashHistory } from 'vue-router'
import MainView from '../modules/data/MainView.vue'

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: '/main' },
    { path: '/main', component: MainView }
  ]
})

export default router
