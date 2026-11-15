<script setup lang="ts">
import type { ButtonProps } from '@nuxt/ui'

const { data: page } = await useAsyncData('landing', () => queryCollection('landing').path('/').first())

useSeoMeta({
  title: 'i18n-next-rs',
  description: page.value?.description,
  ogTitle: 'i18n-next-rs',
  ogDescription: page.value?.description,
})

const heroLinks: ButtonProps[] = [
  { label: 'Get started', to: '/getting-started/introduction', size: 'lg', trailingIcon: 'i-lucide-arrow-right' },
  { label: 'GitHub', to: 'https://github.com/s00d/i18n-next-rs', target: '_blank', size: 'lg', color: 'neutral', variant: 'outline', icon: 'i-simple-icons-github' },
]

const quickLinks: ButtonProps[] = [
  { label: 'Quick start', to: '/getting-started/quick-start', trailingIcon: 'i-lucide-arrow-right' },
  { label: 'Locale files', to: '/guides/locale-files', color: 'neutral', variant: 'outline' },
]
</script>

<template>
  <div>
    <UPageHero
      title="One translation catalog for Rust, Tauri and the browser"
      description="The nuxt-i18n-micro locale format in Rust: root and page chunks, plurals and Intl-compatible formats, with a Tauri plugin that feeds @i18n-micro on the frontend."
      :links="heroLinks"
    />

    <UPageSection
      title="Rust and the frontend, same files"
      description="Install once in Rust and call t! anywhere. The Tauri plugin hands the same catalog to @i18n-micro/vue, one page at a time."
      orientation="horizontal"
      :links="quickLinks"
    >
      <div class="min-w-0">
        <ContentRenderer v-if="page" :value="page" />
      </div>
    </UPageSection>
  </div>
</template>
