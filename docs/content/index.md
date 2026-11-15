---
title: i18n-next-rs
description: Translations for Rust and Tauri apps in the nuxt-i18n-micro format, with the same files on the backend and the frontend.
navigation: false
---

```rust [main.rs]
use i18n_next::{I18n, t};

I18n::builder().dir("locales").locale("ru").fallback("en").install()?;

println!("{}", t!("cart.summary", name = user.name, count = n));

tauri::Builder::default()
    .plugin(tauri_plugin_i18n_next::Builder::new(i18n_next::global().clone()).build())
```

```ts [main.ts]
import { createTauriI18n } from 'tauri-plugin-i18n-next-api/vue'

app.use(await createTauriI18n({ router }))   // page chunks load before each navigation
// <i18n-t keypath="cart.total" :number="sum" />
```
