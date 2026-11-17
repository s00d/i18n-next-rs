# scripts

Node generators (Node 22, full ICU). Dependencies are pinned in `package.json`; `cldr-*` must match the CLDR of the Node used in CI (now 48). Renovate bumps them, and the compat job shows what changed.

| Script | Stage | What it does |
| --- | --- | --- |
| `compat.mjs` | M1 | runs the case matrix through the real `@i18n-micro/core` / `@i18n-micro/utils` and `Intl`, writes `fixtures/compat/__golden__/*.md` (`call → result`) |
| `casing.mjs` | M3 | writes the case-mapping data of linked-message modifiers (`tr` / `az` / `lt` / `el` rules, from Node's ICU, checked against V8) into `crates/i18n-next/src/linked/case_data.rs` |
| `cldr.mjs` | M2, M5 | extracts plural rules and number / date / relative data from `cldr-*` into `crates/i18n-next/src/plural/rules.rs` and `crates/i18n-next/data/cldr/` |
| `tzdb.mjs` | M6 | builds the compact tzdb for the `tzdb` feature (Windows) from IANA tzdata |
