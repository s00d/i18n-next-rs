import { createApp } from 'vue'
import { createRouter, createWebHistory } from 'vue-router'
import { createTauriI18n } from 'tauri-plugin-i18n-next-api/vue'
import App from './App.vue'
import AboutPage from './pages/AboutPage.vue'
import IndexPage from './pages/IndexPage.vue'
import SettingsPage from './pages/SettingsPage.vue'

// Route names are the catalog's page names: the guard loads `locales/pages/<name>/` before
// the route shows (DESIGN 3.4). `index` has no page folder: the root alone answers it.
const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', name: 'index', component: IndexPage },
    { path: '/settings', name: 'settings', component: SettingsPage },
    { path: '/about', name: 'about', component: AboutPage },
  ],
})

const app = createApp(App)
app.use(router)
// locale, locales, defaultLocale and the named formats come from Rust (examples/i18n.json)
app.use(await createTauriI18n({ router, maxPages: 2 }))
await router.isReady()
app.mount('#app')
