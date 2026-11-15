# План работ

Этапы идут в порядке зависимостей. Каждый заканчивается зелёным CI: native на linux, macOS и windows, MSRV 1.90 и wasm. Логи пишутся в `target/logs/<task>.log`. Устройство описано в [DESIGN.md](DESIGN.md), номера вида 8b.N — его разделы.

Правила для всех этапов:
- Новая публичная функция приходит вместе с бенчем (иначе `bench_inventory` красный), с проверкой аллокаций, если она на горячем пути, и со сценарием: действием в модели или историей, а не тестом на функцию.
- Регрессия инструкций в CI-джобе `perf` — баг этапа. Изменение `api/*.txt` видно в diff и проходит semver-проверку.
- Плейграунды растут вместе с этапами: новая фича сразу появляется в туре `playground-rust`, начиная с M7 — в `tauri-app`, с M9 — в `leptos-app`.
- `cargo mutants` запускается один раз, в финале (M10), по обоим крейтам и параллельно (`-j`). На этапах мутанты не гоняются: это дорого, а сценарии и аудит этапа ловят основное.

## Как идёт реализация: оркестрация агентов

Реализация начинается только по команде владельца («начинай M0», «делай M1–M3» и т.п.). Эта команда — согласие на запуск агентов и workflow в рамках названных этапов. Дальше эта сессия работает оркестратором: сама код не пишет, а планирует, запускает агентов, проверяет их работу и отвечает владельцу.

**1. Подготовка этапа.**
- Перечитать DESIGN.md (разделы этапа) и строку этапа в этом плане.
- Разбить этап на задачи с непересекающимися файлами. Общие файлы (`Cargo.toml` workspace, списки `mod` в `lib.rs`, CI) правит только оркестратор.
- На каждую задачу написать бриф: цель, файлы, разделы DESIGN, критерий готовности. В бриф входят правила проекта:
  - сценарии вместо тестов на функцию;
  - бенч на каждую публичную функцию;
  - проверка аллокаций на горячем пути;
  - `forbid(unsafe_code)`;
  - ни одной новой зависимости (2.3);
  - логи в `target/logs/<task>.log`, без обрезки;
  - никаких sleep в тестах, никаких retries.

**2. Сборка.**
- Агенты-исполнители работают параллельно, каждый в своём git worktree (`isolation: worktree`), если задачи независимы; последовательно, если одна ждёт другую. Одновременно — не больше 4.
- Каждый агент возвращает:
  - список изменённых файлов;
  - добавленные сценарии;
  - цифры бенчей и аллокаций;
  - путь к логам;
  - что не сделано и почему.

**3. Проверка оркестратором.** Отчёт агента — не доказательство. Оркестратор сам:
- сливает worktree;
- читает diff;
- прогоняет полный гейт этапа: `cargo fmt`, `clippy -D warnings`, `cargo nextest`, wasm-сборки и `wasm-bindgen-test`, compat-голдены, `alloc.rs`, `bench_inventory`, `perf` (gungraun), `size`, `api/` diff, для guest-js — `pnpm typecheck` / `build` / `test`.

Красное возвращается исполнителю с логом.

**4. Аудит.** Новые агенты, которые не видели ни брифов исполнителей, ни их выводов. Они получают код, DESIGN и цели этапа, как в rewrite-playbook nuxt-i18n-micro. Аудиторы не правят код. Направления, по агенту на каждое:

| Направление | Что ищет |
| --- | --- |
| корректность | расхождение с DESIGN и с поведением `@i18n-micro/core` (по голденам и сверх них), крайние случаи, паники |
| тесты | одиночные тесты вместо сценариев, дыры в моделях |
| производительность | лишние аллокации и блокировки на горячем пути, регрессии бенчей, мономорфизация, раздувающая `.wasm` |
| безопасность и надёжность | имена с фронта, протокол, `unwrap` / `panic` в библиотечном пути, переполнения, `__proto__` |
| размер и зависимости | прирост `.wasm` и native, `format!` / `unwrap` с сообщениями, появившиеся зависимости |
| API и доки | удобство, соответствие 4.x, доктесты, страницы docs этапа |

Каждая находка оформляется так:
- `файл:строка`;
- воспроизведение, которое аудитор реально запустил;
- фактическое и ожидаемое поведение;
- серьёзность.

Находку без воспроизведения оркестратор проверяет сам и отбрасывает, если она не подтвердилась. Для глубокого аудита можно использовать скилл `hard-audit`.

**5. Исправление.** На каждую группу подтверждённых находок — агент-исполнитель (снова в worktree), с находками и воспроизведениями в брифе. После исправлений — шаг 3. Затем повторный аудит, только по изменённому. Цикл повторяется, пока аудит не вернёт ни одной подтверждённой находки. Если после трёх кругов находки остаются, оркестратор останавливается и пишет владельцу, что осталось и почему.

**6. Закрытие этапа.**
- Гейт зелёный (без мутантов — они в M10).
- Плейграунды обновлены, docs этапа написаны.
- В этом плане этап отмечен готовым, с итоговыми цифрами (ns/op горячего пути, аллокации, размер `.wasm`).
- Отчёт владельцу: что сделано, цифры, открытые вопросы.
- Коммит — по правилам владельца: новые коммиты, без amend и без force-push.

## v1

| # | Этап | Что входит | Готово, когда |
| --- | --- | --- | --- |
| M0 ✅ | Скелет (готово 2026-11-15: 4 круга аудита, 37 находок закрыты; ядро-проба +10 байт `.wasm`) | workspace (`rust-version = "1.90"`, edition 2024, resolver 3, `unsafe_code = "forbid"`, одна версия на все пакеты); CI-проверка «у `i18n-next` нет runtime-зависимостей при фичах по умолчанию» (`cargo tree -e normal`, 2.3); CI-джоб `size` (микроверсия, полная с тремя локалями, `.wasm` после `wasm-opt -Oz`, порог 2%, 2.4); criterion / gungraun / dhat каркас, `bench_inventory`, CI-джоб `perf`; `clippy.toml`, `.config/nextest.toml`, `.cargo/mutants.toml`, `[profile.mutants]`; два пустых крейта, `build.rs` и `permissions/` плагина, заготовка guest-js; `tests/support` (каркас: `Locales`, часы, guard); `ci.yml` (stable + 1.90 × linux / mac / windows через nextest; `wasm32-unknown-unknown` и `wasm32-wasip1`; каркас `wasm-bindgen-test` в Node и Chrome; `cargo public-api` → `api/`, `cargo-semver-checks`, `cargo-deny`, `cargo-machete`, тест `version_sync`) (8b.9, 8b.10, 8b.13); `renovate.json` (8b.12); `examples/locales/`, `examples/i18n.json`, каркас `playground-rust` с `--check`; `docs/` из каркаса `tauri-plugin-centrifugo/docs`, `docs.yml`, README, `CHANGELOG.md`, `.versionrc.json` | все джобы зелёные |
| M1 ✅ | Каталог, страницы, конфиг (готово 2026-11-15: 3 аудитора + 2 повторных, 8248 строк эталонов; `get` 12 нс, `t` 45 нс, `has` 11 нс, `set_locale` 122 нс; микро `.wasm` 60 КБ) | свой `json` (ридер сразу в плоскую карту + писатель строк) и свой `hash` (2.3), `tree`, `loader` (`dir` / `files` / `messages`, root + `pages/`, `pages(false)`, `page_alias`), слои `(locale, page)`, цепочки fallback, `template`, missing; `t` / `try_t` / `has` / `get` / `page` / `with_locale` / `set_locale` / `add_messages`; `config` (`i18n.json`, 8b.1); имена только из каталога, пути только из известных имён (8b.2); `scripts/compat.mjs`, голдены загрузки, слоёв и интерполяции; сырые байты файлов для payload (S6); `fixtures/json-test-suite` | `compat`, `catalog_model` (с действием `BadName`), дифференциальные тесты `json` против `serde_json` и JSONTestSuite, бенч `json` против serde-visitor и `hash` против `foldhash` / `rustc-hash`; размер микроверсии зафиксирован (ориентир до ~100 КБ); истории загрузки и плохих имён (native и wasip1), compat в wasm; бенчи всех функций этапа; lock-free чтение через `thread_local!` с кешем на 4 инстанса (бенч 1 / 4 / 16 потоков, спайк S1); свой хешер (2.3, S2); `alloc.rs` для `t` / `get` |
| M2 | Плюралы | `scripts/cldr.mjs` (правила) → `plural/rules.rs`; порт `pluralForm`, `iso` | `plural_props`; compat-голдены плюралов, native и wasm |
| M3 | Источники и проверки | linked, `lazy_dir` / `lazy_files` / `remote` + `ensure_async` / `missing_files` / `add_file`, `extend` для библиотек (8b.4), `evict_pages` / `pin_page` (8b.16), `reload`, `subscribe`, версия снимка; `watch` (8b.5); `check()` (8b.8); фича `tracing` (8b.17); `FakeFetch` в `tests/support`; `fuzz/` с целями `load_json`, `template`, `page_names` (8b.11) | `loading_model` (с `Evict`: вытеснение невидимо в ответах); действие `Extend` в `catalog_model`; бенчи вытеснения (горячий путь вкл/выкл, память на 1000 страниц); compat по linked; `concurrency`; `watch` на синтетических событиях и виртуальных часах; истории `check()`; `ensure_async` с настоящим `fetch` в headless Chrome; локальный fuzz без крэшей |
| M4 | `Text` и глобальный доступ | `Text` (аргументы во встроенном массиве на 4, S4), `Arg`, `parts` / `fold`, `html`, `or`, `hide_if_empty`, `plural_rule`, `page`, `Serialize`; `install`, `t!` / `tc!` / `tn!` / `td!` / `tdr!` / `key!`, `Key`, `scope` (`run`, `wrap`, `instance`), `negotiate`; ручной executor в `tests/support` | `text_model`, `scope_model`, `global_scope`, compat по `resolveText`; `wrap` с `wasm-bindgen-futures`; бенчи `Text`, global, `scope` |
| M5 | Данные CLDR и числа | `scripts/cldr.mjs` → `data/cldr` (+ автонимы, 8b.14; дедупликация региональных вариантов по родителю, интернирование строк; фича `currency-names`, S11), `build.rs` с `I18N_NEXT_LOCALES`, ленивый декод; `tn` / `tn_as`; `language_name`; разбор опций с ошибкой на неподдерживаемые; fuzz-цели `plural_rules` и `intl_options` | compat-голдены `NumberFormat` и автонимов (native и wasm); бенч; размер native и `.wasm`; решение о наборе локалей по умолчанию (цель ≤ 1.5 МБ, S11) |
| M6 | Даты и relative | `td` / `td_as` (паттерны комбинаций из ICU Node через `formatToParts`, S9 / S10); свой модуль `tz` (календарь, TZif v1–v3, POSIX-правила, контейнер Android tzdata, браузер через `js-sys`, фича `tzdb` с `scripts/tzdb.mjs` для Windows, 5.4); `clock`, `tdr`; ошибка при `build()` для `timeStyle: full / long` и `timeZoneName`; `.number()` / `.date()` / `.relative()` | compat-голдены `DateTimeFormat` / `RelativeTimeFormat` (U+202F → пробел, как `format()` V8); `tz` против `jiff` (dev-dependency) на всех зонах 1900–2100, фикстуры TZif, fuzz `tzif`; зона из `Intl` в Chrome; бенч; таблица размеров, включая таблицу паттернов |
| M7 | Плагин | `payload` (encode в крейте, с `locales` / `defaultLocale` / форматами), `protocol.rs` (`i18n-next://`: чанк и bundle с `ETag` / immutable, проверка имён и `Origin`, 6.1), `Builder` (+ `install`, `persist`, `watch` в dev), расширение `Manager`, `load` / `load_page` / `set_locale` / `get_locale` / `available_locales` / `pages`, событие `changed`, системная локаль из `navigator.languages` WebView и `negotiate` (без `sys-locale`), `persist` (8b.3), отчёт о промахах в dev (8b.7), `bind` / `Relabel` (решение отдельно от вызова Tauri), default-permission (8b.2); fuzz-цель `payload_apply` | `ipc_model` (с `BadName` и запросами протокола как чистой функции), истории плагина (перезапуск с `persist`, dev-отчёт, `watch` → окна), голден схемы payload; бенч IPC; каркас `examples/tauri-app` (меню, трей, второе окно, команда с `Text`) собирается в CI |
| M8 | guest-js | API, `connect` (данные через `fetch(...).json()` по протоколу с `<link rel="preload">`, запасной raw invoke; `setPage`, `preload`, кеш по версии, `<html lang dir>`, `missingHandler` → `report_missing` в dev, `maxPages` LRU, 8b.16), `/vue` → `createTauriI18n` (router guard, `locales` / `defaultLocale` из payload), форматы; рецепт и подключение `@i18n-micro/types-generator` (8b.6); `connect` с `@i18n-micro/react` / `preact` / `solid` (8b.20) | `tauri-app`: страницы через router guard, `<i18n-t>`, переключатель, inspector со списком промахов; `connect.model.test.ts` (fast-check, фиксированный seed, с `maxPages`), `vue-router.test.ts`, тесты с React / Preact / Solid; publint, attw, снимок `.d.ts`; Stryker локально; бенч `JSON.parse` |
| M9 | `tauri-guest` | `tauri_guest::connect` / `set_page` / `set_locale` (данные через `fetch` из `js-sys` по протоколу, без `web-sys`), `payload::apply`, подписка на `changed` | `wasm-bindgen-test` против заглушки `__TAURI_INTERNALS__` с payload от `payload::encode`; `examples/leptos-app` в двух режимах (trunk и Tauri, `localStorage` для persist в браузере), `trunk build` в CI |
| M10 | Плейграунды и цифры | доводка плейграундов (axum-сервер в `playground-rust`, Android-сборка `tauri-app`, размер `.wasm` на экране `leptos-app`; страница-бенч транспорта в inspector `tauri-app`, цифры WebView2 / WebKitGTK / Android WebView в таблицу, 6.1); `examples/nuxt-site` на тех же файлах (8b.19); `benches/compare` (rust-i18n, fluent, ICU4X); `scripts/bench-table.mjs` → `5.performance.md`; все страницы docs (en); полный локальный `cargo mutants` и fuzz по обоим крейтам | все плейграунды собираются в CI, тур `--check` зелёный, тексты `nuxt-site` совпадают с туром; docs без битых ссылок; ноль выживших мутантов вне `exclude_re` с причиной |

Релиз v1 — отдельно, по команде владельца: CHANGELOG, зелёный CI, тег без `v`, `gh release create`, `cargo publish --workspace`, `pnpm publish` без `--otp`.

## После v1

| # | Этап | Что входит | Готово, когда |
| --- | --- | --- | --- |
| M11 | Кодоген ключей | `i18n_next::build::keys` для `build.rs` приложения: `Key` / `PageKey` константы с doc-текстом и параметрами (8b.6) | голден сгенерированного файла; опечатка в ключе в `playground-rust` даёт ошибку компиляции (тест `trybuild`) |
| M12 | Расширения Intl | `tl` (`ListFormat`), `tc_ordinal`, `parse_number` (8b.15); названия языков на текущем языке по фиче | голдены против Node `Intl.ListFormat` / `PluralRules({ type: 'ordinal' })` |
| M13 | Экосистема | `i18n-next-macros` (проверка ключей при компиляции), если M11 мало; источник данных для `@i18n-micro/devtools-ui` (8b.7); русская версия docs (8b.18) | по пункту: `trybuild`-тесты макросов; devtools показывает каталог и промахи `tauri-app`; ru-сайт без битых ссылок |

Вне этого репозитория, по решению владельца: режим `@i18n-micro/cli` без Nuxt (`--dir`, `--config i18n.json`), см. 8b.8.
