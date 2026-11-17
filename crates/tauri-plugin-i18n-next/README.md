# tauri-plugin-i18n-next-api

Frontend of [`tauri-plugin-i18n-next`](https://crates.io/crates/tauri-plugin-i18n-next): the
translations of the Rust catalog in your WebView, for any [`@i18n-micro`](https://github.com/s00d/nuxt-i18n-micro)
instance. One set of locale files for Rust and JS; a locale switch in either switches both.

**Documentation: <https://s00d.github.io/i18n-next-rs/guides/tauri>**

## Install

```bash
npm install tauri-plugin-i18n-next-api
npm install @i18n-micro/vue vue          # for the /vue entry
```

The Rust side registers the plugin (`tauri_plugin_i18n_next::Builder::new(i18n).build()`) and the
capability allows `"i18n-next:default"`; see the guide.

## Vue

```ts
import { createTauriI18n } from 'tauri-plugin-i18n-next-api/vue'

app.use(router)
app.use(await createTauriI18n({ router }))   // a route's page loads before it shows
```

```vue
<script setup lang="ts">
import { useI18n } from '@i18n-micro/vue'
import { useI18nLink } from 'tauri-plugin-i18n-next-api/vue'
const { t } = useI18n()
const link = useI18nLink()
</script>

<template>
  <i18n-t keypath="cart.total" :number="42.5" />
  <button @click="link.setLocale('de')">{{ t('lang.de') }}</button>
</template>
```

## Any `@i18n-micro` instance (React, Preact, Solid, runtime)

```ts
import { createI18n } from '@i18n-micro/runtime'   // or the createI18n of @i18n-micro/react, /preact, /solid
import { connect } from 'tauri-plugin-i18n-next-api'

const i18n = createI18n({ locale: 'en' })
const link = await connect(i18n, { page: 'index' })
await link.setPage('settings')   // on navigation
await link.setLocale('de')       // Rust, native labels and every window
```

API: <https://s00d.github.io/i18n-next-rs/reference/javascript-api>.

License: MIT.
