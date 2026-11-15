---
title: i18n-next-rs
description: Translations for Rust and Tauri apps in the nuxt-i18n-micro format, with the same files on the backend and the frontend.
navigation: false
---

::code-group

```rust [src-tauri/src/lib.rs]
use i18n_next::{I18n, t};

let i18n = I18n::builder().dir("locales").locale("ru").fallback("en").build()?;

tauri::Builder::default()
    .plugin(tauri_plugin_i18n_next::Builder::new(i18n).build())   // installs the global instance
    .setup(|_app| {
        println!("{}", t!("cart.summary", name = "Ann", count = 3));   // t! works anywhere now
        Ok(())
    })
```

```ts [src/main.ts]
import { createTauriI18n } from 'tauri-plugin-i18n-next-api/vue'

app.use(await createTauriI18n({ router }))   // page chunks load before each navigation
// <i18n-t keypath="cart.total" :number="sum" />
```

::
