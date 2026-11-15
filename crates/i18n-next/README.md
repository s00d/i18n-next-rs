# i18n-next

i18n for Rust on the same locale files as [nuxt-i18n-micro](https://github.com/s00d/nuxt-i18n-micro):
global and per-page translations, plurals and `Intl`-compatible formats, native and wasm,
with no runtime dependencies.

**Status: early.** Only the project skeleton exists so far; the API below `version()` is
being built stage by stage. Do not depend on it yet.

```rust
println!("i18n-next {}", i18n_next::version());
```

License: MIT.
