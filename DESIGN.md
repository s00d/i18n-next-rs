# i18n-next-rs: study и design

Статус: черновик, ревизия 9. Кода нет. План работ — в [PLAN.md](PLAN.md).

Что изменилось в ревизии 9:
- **Транспорт** проверен в настоящем WebView (S15): данные идут по custom protocol `fetch().json()` с `ETag` и версией в URL, IPC остаётся для управления и как запасной путь (6.1).
- **Политика зависимостей** (2.3): у `i18n-next` нет runtime-зависимостей — свой JSON-ридер, хешер и модуль `tz`; `serde`, `tracing`, `chrono`, `time` — фичи; `sys-locale` и `notify` убраны.
- **Микроверсия и размер** (2.4, S16): ориентир — реальный `.wasm` после `wasm-opt`, ядро 27 КБ; фичи режут крейт; CI-гейт размера.
- **Оркестрация реализации** агентами — в PLAN.md.

Ревизия 8:
- **Раздел 12**: спайки проверили 14 спорных решений измерениями. Выбрано `thread_local!`-чтение; паттерны дат берутся у ICU в Node при генерации (DTPG не портируется); часовые пояса в браузере через `js-sys`; имена валют вынесены в фичу; `timeStyle: full / long` — ошибка в v1.

Ревизия 7:
- **Раздел 8b**: 20 дополнительных механизмов — `i18n.json`, безопасность имён с фронта, persist, `extend` для библиотек, `watch`, типобезопасные ключи, отчёт о промахах, `check()`, public API и semver, cargo-deny, fuzz, Renovate, релиз в связке, автонимы, `ListFormat` / ordinal / разбор чисел, вытеснение страниц, `tracing`, ru-docs, Nuxt-плейграунд, React / Preact / Solid.

Ревизия 6:
- **Плейграунды**: Rust (консольный тур и axum), Tauri (Vue), Leptos (браузер и Rust-фронт в Tauri) на общем каталоге локалей (раздел 8a).

Ревизия 5:
- **Производительность как правило**: `forbid(unsafe_code)`, горячий путь без блокировок и лишних аллокаций, бенч на каждую публичную функцию (с проверкой инвентаря), детерминированные гейты в CI — число инструкций (gungraun) и число аллокаций (dhat) (разделы 2.2, 7.7).
- **Документация** по образцу `tauri-plugin-centrifugo/docs`: тот же стек и вид, короткий тон (раздел 8).

Ревизия 4:
- **wasm с первого дня**: крейт собирается и тестируется для `wasm32-unknown-unknown` (браузер, Rust-фронт в Tauri) и `wasm32-wasip1`; `ensure_async` для загрузки по сети; фича `tauri-guest` (разделы 2.1, 3.3, 7.6).
- **Тестирование** переписано по образцу Rust-пакетов владельца: сценарии через публичный вход, модели с фиксированным seed, истории, nextest с жёсткими таймаутами, мутации локально; compat-голдены в формате `call → result` (раздел 7).

Ревизия 3:
- **Глобальные и страничные переводы** стали основой с первого этапа. Root и page-чанки есть в загрузчике, API, передаче на фронт и интеграции с роутером (раздел 3).
- **ICU4X убран.** Плюралы и форматы считает собственный движок по данным CLDR, сгенерированным из официального `cldr-json` той же версии, что у Node (раздел 5). Runtime-зависимостей на i18n нет.
- MSRV 1.90 (Tauri 2.12), раздел 2.
- **Глобальные макросы** `t!("key")` без передачи `i18n`, константы `key!`, `scope` для локали и страницы на запрос или задачу (раздел 4.4). Фича `global` включена по умолчанию, Tauri-плагин ставит глобальный инстанс сам.

Ревизия 2 добавила форматы в v1, универсальный `Text` (аналог `<i18n-t>`) и `bind` для нативного UI Tauri.

## 1. Что изучено

### 1.1 nuxt-i18n-micro (`/Users/s00d/packeges/nuxt-i18n-next`)

Пакеты `@i18n-micro/*` 2.1.x.

| Тема | Как в JS (источник) | В Rust |
| --- | --- | --- |
| Формат | JSON-дерево любой глубины. Собственный плоский ключ `'a.b'` побеждает обход по точкам. `list.0` — элемент массива. `null` значит «нет значения», `__proto__` игнорируется (`core/src/tree.ts`) | так же |
| Раскладка | `locales/<locale>.json` — root-чанк. `locales/pages/<a>/<b>/<locale>.json` — page-чанк роута `a-b`. Прочие вложенные файлы игнорируются, BOM допустим, порядок файлов — по относительному пути (`utils/load-dir.ts`, `parse-path.ts`) | так же, раздел 3 |
| Слои | page роута, затем root локали, затем то же в fallback-локали. Root в page не копируется (`core/src/base.ts`, `resolveLookup`) | так же |
| Опции страниц | `disablePageLocales` (page-файлы сливаются в root), `routesLocaleLinks` (страница читает папку другой страницы) | `.pages(false)`, `.page_alias(from, to)` |
| Доставка страниц | Nuxt собирает payload на каждую пару страница × локаль (premerged) и подгружает его при навигации; старая страница отвечает до конца перехода (`docs/guide/translations.md`). `@i18n-micro/vue/router` сам чанки не грузит | Слои отдаются раздельно (root один раз, page маленькими чанками); подгрузку при навигации делает guest-js `/vue` (раздел 3.4) |
| Fallback | В core одна `fallbackLocale`. В Nuxt цепочка `de-at → de → en`, глобальный fallback — самый слабый слой | цепочка |
| Интерполяция | `{name}` по токену `\{([^{}\s]+)\}`, `{0}` из массива; без параметра плейсхолдер остаётся (`core/src/text.ts`) | так же, токены разбираются один раз при загрузке |
| Плюралы | `a \| b \| c`, индексное правило, случай «unmarked» по CLDR-категориям, маркер `=0 `. Теги из `iso`; неизвестный тег — индексное правило | дословный порт `pluralForm`, категории из сгенерированных правил CLDR |
| Linked | `@:key`, `@.upper:` / `@.lower:` / `@.capitalize:`, `@:{'key'}`, разрешаются при загрузке, opt-in | так же |
| Missing | возвращается `default \|\| key`, вызывается `missingHandler(locale, key, route)` | так же, плюс `try_t` |
| Форматы | `FormatService`: `Intl.*` с кешем; именованные форматы в духе Vue I18n с поиском по укороченным тегам; `timeZone`, `now`; `tdr` берёт самую крупную единицу, которая укладывается хотя бы раз; невалидная дата — `Invalid Date` | свой движок, тот же конфиг, раздел 5 |
| `<i18n-t>` | `resolveText` + `foldTemplate`: keypath, params, plural, number, date, relativeDate, defaultValue, html, hideIfEmpty, customPluralRule, слоты | `Text`, раздел 4 |
| API адаптеров | `I18n` (runtime) и `VueI18n`: `addTranslations`, `addRouteTranslations`, `setRoute`, сеттеры `locale` / `fallbackLocale`, `formatter.setNumberFormats` / `setDateTimeFormats` | guest-js стыкуется по этому интерфейсу |

### 1.2 Rust-крейты

| Крейт | Версия, загрузки | Формат | Сильное | Слабое для нас |
| --- | --- | --- | --- | --- |
| `rust-i18n` | 4.2.4, 4.9M | YAML / JSON / TOML, `%{name}` | `t!` без инстанса, codegen | глобальное состояние, нет плюралов и форматов, другой синтаксис |
| `fluent-bundle` | 0.16, 24M | FTL | мощный язык, CLDR-плюралы | другой формат, не `Sync` без `concurrent`, многословные аргументы |
| `fluent-templates` | 0.15, 1.4M | FTL | `static_loader!`, `ArcLoader` | то же |
| `i18n-embed` | 0.16, 9.8M | FTL / gettext | встраивание, `fl!`, язык из ОС | много трейтов |
| `gettext-rs` | 0.8, 5.2M | `.po` / `.mo` | стандарт | C-библиотека, глобальное состояние, mobile |
| `r_i18n` | 1.0.1, 2018 | JSON | — | заброшен |
| `es-fluent` | 0.21 | FTL | типобезопасные derive | молодой |
| `tauri-plugin-i18n` | 2.0.2, 4.3k | rust-i18n | — | IPC на каждый ключ |

Интернационализация: ICU4X 2.4 (`icu_plurals`, `icu_decimal`, `icu_datetime`) и `icu_experimental` 0.7 (relative, валюта, compact; нестабильный API, минор ломает). `intl_pluralrules` не обновлялся с 2022 года. `num-format` заброшен. Почему не берём ICU4X — раздел 5.1.

### 1.3 Копируем и избегаем

Копируем:
- встраивание через любой итератор `(путь, байты)`;
- короткий `t!` и опциональный global;
- согласование системной локали;
- из i18n-micro: неизменяемые снимки, root и page-чанки, linked при загрузке, одну точку разрешения (`resolveText`).

Избегаем:
- глобального состояния как единственного пути;
- IPC на ключ;
- своего формата файлов;
- разбора шаблона на каждый вызов;
- тяжёлых зависимостей, поведение которых мы не контролируем.

## 2. Основа

- Workspace: крейты `i18n-next` и `tauri-plugin-i18n-next`. `rust-version = "1.90"` (минимум Tauri 2.12), `edition = "2024"`, resolver 3, `clippy.toml` с `msrv = "1.90"`, CI-джоб на 1.90.
- Возможности 1.86–1.90, которые используем:
  - let chains — в разборе шаблонов, правил CLDR, паттернов дат и путей;
  - trait upcasting — `Box<dyn Relabel>` в `bind`;
  - `Vec::extract_if` / `HashMap::extract_if` — в `unbind` и снятии подписчиков;
  - `Cell::update`;
  - `cargo publish --workspace` — в релизе.

  Из базы 1.80–1.85: `LazyLock` / `OnceLock`, правила захвата edition 2024 и `+ use<..>`, `#[expect]`, `#[diagnostic::on_unimplemented]`.
- Зависимости — раздел 2.3: у крейта `i18n-next` их нет. Всё, что нужно задаче, пишется своим микрокодом с максимальными тестами. Интеграции (`serde`, `tracing`, `chrono`, `time`, `wasm-bindgen`) подключаются только фичами.
- Proc-macro-крейта нет: `t!` написан на `macro_rules!`. `i18n-next-macros` появится вместе с проверкой ключей при компиляции.

Фичи `i18n-next`:

| Фича | По умолчанию | Что |
| --- | --- | --- |
| `format` | да | `tn` / `td` / `tdr`, форматированные аргументы в `Text`, данные CLDR для форматов |
| `tz` | да | IANA-зоны и системная зона своим модулем `tz` (5.4); в браузере — через `Intl`; без неё только UTC и фиксированные смещения |
| `tzdb` | на Windows да, иначе нет | встроенная компактная tzdb из своего генератора (нужна там, где нет системных TZif: Windows) |
| `serde` | нет | `Serialize` для `Text` / `Key` (плагин включает) |
| `currency-names` | нет | имена валют для `currencyDisplay: name` (+~34 КБ на локаль), спайк S11 |
| `global` | да | глобальный инстанс, макросы `t!` / `tc!` / `tn!` / `td!` / `tdr!` / `key!` без передачи `i18n`, `scope` (4.4) |
| `chrono`, `time` | нет | `From<…>` для дат |
| `std` | да | без неё — `no_std` + `alloc`, если микроверсии это даст выигрыш в размере (решение по замеру в M1) |
| `watch` | нет | перезагрузка на лету в dev, native, раздел 8b.5 |
| `tracing` | нет | промахи и загрузки в `tracing`, раздел 8b.17 |
| `tauri-guest` | нет | Rust-фронтенд внутри Tauri (Leptos, Yew, Dioxus в wasm): `connect()` к плагину, раздел 2.1 |

Плюрал-правила CLDR есть всегда: это сгенерированный код в несколько КБ.

### 2.1 Платформы: native и wasm с первого дня

Крейт `i18n-next` с первого коммита собирается и тестируется на всех целях. Плагин — только native: Tauri в wasm не собирается.

| Цель | Где | Что меняется |
| --- | --- | --- |
| desktop (linux, macOS, windows), Android, iOS | native, Tauri | всё доступно |
| `wasm32-unknown-unknown` | браузер: Leptos, Yew, Dioxus, egui web, Rust-фронт внутри Tauri | нет файловой системы: `dir` / `lazy_dir` не существуют (`cfg`, а не ошибка в runtime); время через `Date.now`, зона через `js-sys` (`Date.getTimezoneOffset`, `Intl`), без встроенной tzdb (спайк S12) |
| `wasm32-wasip1` / `wasip2` | WASI-рантаймы, edge | файловая система есть, `dir` работает |

Что из этого следует для устройства:
- Нет потоков в браузере, но `Send + Sync`, `RwLock`, `OnceLock` и `thread_local!` компилируются и работают в однопоточном wasm. API одинаковый, `cfg` в пользовательском коде не нужен.
- `std::time::SystemTime::now()` в `wasm32-unknown-unknown` паникует, поэтому крейт его не вызывает. Часы по умолчанию: native — `SystemTime::now()`, браузер — `js_sys::Date::now()`. Без фичи `tz` на wasm часы задаются явно через `.clock()`; без них `tdr` вернёт ошибку при `build()`, а не панику.
- Выбор бэкенда времени — `cfg` по цели: `js-sys` только для `cfg(all(target_arch = "wasm32", target_os = "unknown"))`, иначе свой модуль `tz` (5.4). Пользователь ничего не настраивает.
- Размер важен: `I18N_NEXT_LOCALES` урезает данные CLDR в wasm так же, как в native. Таблица размеров `.wasm` (`opt-level = "z"`, `wasm-opt`) — в доках.
- Загрузка в браузере: встроенные файлы (`files(include_dir!(…))`) или подгрузка по сети через `ensure_async`, раздел 3.3. Крейт не тянет `web-sys` и HTTP-клиент: сеть даёт приложение (gloo-net, reqwest, `fetch`).
- `scope().wrap(fut)` работает с `wasm-bindgen-futures` так же, как с tokio.

**Rust-фронтенд в Tauri** (фича `tauri-guest`, зависимости `wasm-bindgen`, `js-sys`, `wasm-bindgen-futures`, только на wasm). Аналог guest-js для тех, у кого фронт написан на Rust:

```rust
let i18n = i18n_next::tauri_guest::connect().await?;   // invoke plugin:i18n-next|load → тот же payload
i18n.install()?;                                        // дальше t!("key") в компонентах Leptos/Yew
i18n_next::tauri_guest::set_page("settings").await?;   // load_page
i18n_next::tauri_guest::set_locale("de").await?;       // Rust-бэкенд и все окна
```

- Формат payload описан в крейте (`payload.rs`: `encode` для плагина, `apply` для гостя), поэтому обе стороны используют один код.
- Событие `changed` слушается через `__TAURI_INTERNALS__` по тому же протоколу, что у `@tauri-apps/api`.

### 2.2 Производительность: правила

Каждая функция работает настолько быстро, насколько возможно, и это измерено, а не предположено.

- `#![forbid(unsafe_code)]` в обоих крейтах (`[workspace.lints.rust] unsafe_code = "forbid"`). Скорость берём устройством данных, а не `unsafe`. Код может быть сложнее, если бенч это оправдывает; такое место получает комментарий с цифрой.
- Горячий путь (`t`, `Text`, `t!`, `tc`, `tn`, `td`, `tdr`, lookup по слоям):
  - **Без блокировок и счётчиков ссылок на чтении.** Глобальный `AtomicU64` хранит версию снимка, у потока в `thread_local!` лежит `(version, Arc<Snapshot>)`. Чтение — одна атомарная загрузка и сравнение, а `RwLock` трогается только после записи. Всё в safe Rust.
  - **Без аллокаций, где результат можно не создавать.**
    - `Display` пишет в форматтер напрямую, без промежуточной `String`.
    - `Text::write_to(&mut impl fmt::Write)` дописывает в чужой буфер.
    - Сообщение без параметров хранится как `Arc<str>`, и `get(key)` отдаёт его клоном без аллокации.
    - `t()` делает ровно одну аллокацию — результат.
  - **Всё разобрано при загрузке.** Дерево развёрнуто в плоскую карту, шаблон токенизирован, формы плюрала разделены, правило плюрала — указатель на сгенерированную функцию, паттерн формата разобран в части.
  - Параметры — встроенный массив на 4 элемента с линейным поиском, `Vec` только сверх: 0 аллокаций при сборке `Text` (спайки S3, S4).
  - Хешер карты ключей — свой, на 20 строк (2.3): вдвое быстрее std SipHash на коротких ключах (спайк S2: Fx-подобный 6.5 нс против 12.9).
  - Числа и даты пишутся вручную в `String` с заранее посчитанной ёмкостью. `f64` переводится в кратчайшее десятичное представление через `Display` из std (safe), дальше идёт десятичное округление строки, как в ICU.
- Загрузка: свой JSON-ридер (2.3) разбирает сразу в плоскую карту, без промежуточного дерева (спайк S6: такой путь в 1.7 раза быстрее дерева). Исходные байты файла сохраняются для payload и не сериализуются повторно; пересборка идёт, только если файл изменился (BOM, `__proto__`, linked).
- Payload для фронта — одна аллокация точного размера, склейка готовых байтов.
- wasm: тот же код; аргументы через `&dyn`, где мономорфизация раздувает `.wasm` без выигрыша в скорости. Размер в таблице рядом со скоростью.
- В доках рекомендации для приложений: `lto = "thin"`, `codegen-units = 1`, `I18N_NEXT_LOCALES`.

### 2.3 Политика зависимостей: свой микрокод

Крейт делает одну задачу, поэтому берёт ровно то, что ей нужно, и пишет это сам. Чужой крейт, из которого используются четыре функции, приносит годы обросшего кода, свой размер, своё поведение и свою производительность, которые никто не мерил на нашем сценарии. Чужие библиотеки служат справочником при реализации, а не зависимостью.

| Было в дизайне | Теперь | Почему можно | Как проверяется |
| --- | --- | --- | --- |
| `serde_json` (загрузка) | свой JSON-ридер: объект, массив, строка (escape, `\uXXXX`, суррогатные пары), число, `true` / `false` / `null`, BOM, лимит глубины. Читает сразу в плоскую карту и не строит дерево | формат входа узкий (файлы переводов и payload), нужен только разбор в плоскую карту | JSONTestSuite (все `y_` принимаются, все `n_` отвергаются, решения по `i_` записаны); дифференциальный тест против `serde_json` (только dev-dependency) на корпусе и случайных документах; fuzz-цель `load_json`; бенч против serde-visitor из спайка S6 |
| `serde` (сериализация payload) | свой писатель JSON-строк (escape) для слитых чанков; исходные байты файлов идут в payload как есть | писать нужно только объекты строк | голден payload, дифференциальный разбор браузерным `JSON.parse` в wasm-тестах |
| `thiserror` | ручной `Display` + `Error` | пять вариантов ошибки | компиляция и голдены текстов ошибок |
| `foldhash` | свой хешер на 20 строк (умножение и сдвиг, как Fx / fold) | ключи короткие и доверенные: из файлов и кода, имена с фронта проверяются по списку (8b.2) | бенч против `foldhash` и `rustc-hash` в `benches/compare` (спайк S2: Fx 6.5 нс, foldhash 6.0); выигрыш foldhash на 0.5 нс — не повод для зависимости |
| `jiff` (часовые пояса) | свой модуль `tz`, раздел 5.4 | нужно только: момент → дата и время в зоне | дифференциальный тест против `jiff` (dev-dependency) на всех зонах и миллионах моментов; фикстуры TZif |
| `sys-locale` (плагин) | локаль системы от WebView (`navigator.languages` при первом `load`) и переменные окружения (`LC_ALL`, `LC_MESSAGES`, `LANG`) вне Tauri | WebView уже знает языки пользователя на всех платформах, включая мобильные; FFI к Windows потребовал бы `unsafe` | story-тест: payload первого `load` содержит выбранную по `navigator.languages` локаль |
| `notify` (`watch`) | свой опрос `mtime` в отдельном потоке, только debug | dev-инструмент, задержка 200–300 мс приемлема | модель на синтетических событиях и виртуальных часах |

Что остаётся и почему:
- Плагин: `tauri`, `tauri-plugin`, `serde`, `serde_json` — их и так тянет Tauri, второй копии нет.
- Опциональные интеграции за фичами, выключены по умолчанию: `serde` (`Serialize` для `Text`), `tracing`, `chrono`, `time`.
- Браузер: `wasm-bindgen` + `js-sys` только для `wasm32-unknown-unknown` и только с фичами `tz` / `tauri-guest`. Это платформенная привязка к JS, а не библиотека логики.
- Dev-зависимости для проверок не попадают к пользователю: `proptest`, `criterion`, `gungraun`, `dhat`, а также эталоны для дифференциальных тестов — `serde_json`, `jiff`, `foldhash`.

Правило: новая runtime-зависимость в крейте `i18n-next` появляется только по решению владельца, с замером, который показывает, что свой код хуже.

### 2.4 Микроверсия и размер

Ориентир — реальный размер бинаря, а не gzip: `.wasm` после `wasm-opt -Oz` и прирост native-бинаря примера.

Замер спайка S16 (минимальное ядро, `opt-level = "z"`, LTO, `panic = "abort"`):

| Сборка | `.wasm` после `wasm-opt -Oz` |
| --- | --- |
| ядро: плоская карта, шаблоны, подстановка, свой хешер, свой вывод целых | **27.3 КБ** |
| то же с `std` SipHash вместо своего хешера | 28.8 КБ |
| + `core::fmt` для целых | 27.9 КБ |
| + `core::fmt` для `f64` (`Display`) | 44.2 КБ (+17 КБ) |
| для сравнения: + `jiff` с `js`, без tzdb (спайк S12) | 558 КБ |

Отсюда правила:
- Фичи режут крейт на слои. `default-features = false` даёт микроверсию: каталог, страницы, шаблоны, плюралы (правила только выбранных локалей), `t!` и `Text`. Без форматов, `tz`, `watch`, `tracing`.
- Целые числа выводятся своим кодом. `core::fmt` для `f64` (+17 КБ) подтягивается только фичей `format`: кратчайшее десятичное представление нужно для совместимости с `Intl`, и своя реализация Ryu тянула бы такие же таблицы.
- Никаких `format!` / `unwrap()` с сообщениями на горячем и библиотечном пути: каждый такой вызов тянет форматирование. Проверяет CI-джоб `size` через `twiggy`.
- Данные CLDR генерируются как Rust-статика (`&'static str`, массивы) только для локалей из `I18N_NEXT_LOCALES`. Нет декодера в рантайме, а неиспользуемое вырезает линкер.
- CI-джоб `size`: собирает микроверсию и полную версию пробы, печатает net-размер и рост к базе. Падает только при превышении бюджета (`tools/size-budget.env`: micro 110 КБ, full 160 КБ net); рост больше 2% — предупреждение для чтения в PR: новая фича законно добавляет код. Бюджет поднимается осознанно, отдельным коммитом.
- Ориентир размера: микроверсия до ~100 КБ `.wasm` после `wasm-opt -Oz` — это нормальный реальный размер, гнаться за меньшим ценой сложности не нужно (решение владельца). Контроль — относительный CI-гейт роста (2%) и разбор явного раздувания, а не абсолютная цель.

## 3. Глобальные и страничные переводы

### 3.1 Модель

| Чанк | Файл | Что в нём |
| --- | --- | --- |
| root (глобальный) | `locales/<locale>.json` | общее: меню, кнопки, ошибки, трей |
| page | `locales/pages/<page>/<locale>.json` | ключи одной страницы; `pages/user/profile/` даёт `user-profile`, как в Nuxt |

- Внутри чанк адресуется парой `(locale, page)`, у root `page = "index"`, как в JS (`translationCacheKey`).
- Поиск идёт по слоям, первое совпадение побеждает:
  1. page в активной локали;
  2. root в активной локали;
  3. page в fallback-локали;
  4. root в fallback-локали;
  5. и так далее по цепочке.

  Поэтому root активной локали сильнее page-чанка fallback-локали, ровно как у i18n-micro.
- Page-чанк хранит только свои ключи. Root в него не копируется: память не дублируется, и правка root видна всем страницам.
- Страница — просто имя. Оно может совпадать с роутом фронта (`settings`) или быть Rust-only областью (`tray`, `notifications`, `cli`). Rust-only страницы фронт никогда не грузит, пока не попросит.
- `.pages(false)` сливает page-файлы в root, как `disablePageLocales`. `.page_alias("about-us", "about")` повторяет `routesLocaleLinks`.

### 3.2 Rust API страниц

```rust
let settings = i18n.page("settings");          // Page: дешёвый view (Arc + имя), Clone + Send + Sync
settings.t("title");                           // page → root → fallback page → fallback root
settings.text("hint").arg("n", 3);             // Text в контексте страницы
i18n.text("hint").page("settings");            // то же из цепочки
i18n.with_locale("de").page("settings").t("title");

// Rust-only область для нативного UI
let tray = i18n.page("tray");
app.i18n().bind(&quit, tray.text("quit"));

i18n.pages();                                  // известные страницы (из файлов или lazy-списка)
i18n.add_messages("en", Some("settings"), json!({ … }))?;
```

### 3.3 Загрузка

- `dir` (eager): читаются все root и page-файлы.
- `lazy_dir` / `lazy_files`: при старте только список пар `(locale, page)` из имён файлов. Файл читается при первом обращении к своей паре. Страницы маленькие, поэтому ленивость получается по странице, а не по целой локали. Каждый файл загружается один раз.
- `files(iter)`: встроенные файлы (`include_dir`, `rust-embed`) с той же раскладкой путей. Это единственный надёжный путь на Android (ресурсы там не обычная файловая система) и самый простой в браузере.
- `remote(paths)` + `ensure_async`: список файлов известен, а байты даёт приложение. Это нужно для браузера, где синхронно читать нечего:

  ```rust
  let i18n = I18n::builder().remote(MANIFEST).locale("de").build()?;   // MANIFEST: ["de.json", "pages/settings/de.json", …]
  i18n.ensure_async("de", Some("settings"), |path| async move {
      gloo_net::http::Request::get(&format!("/locales/{path}")).send().await?.binary().await
  }).await?;                                                          // грузит только недостающие файлы цепочки
  ```

  `ensure_async` не зависит от рантайма, это просто замыкание, возвращающее `Future`. Тот же метод работает в native с `tokio::fs`. `i18n.missing_files(locale, page)` отдаёт список путей тем, кто хочет грузить сам, а `add_file(path, bytes)` добавляет результат.

### 3.4 Страницы на фронте

Фронту отдаются слои, а не premerged-словарь страницы. JS core сам разбирает слои (`resolveLookup`), поэтому root приходит один раз, а при навигации подгружается только маленький page-чанк.

- Старт: `load()` возвращает root активной локали и её цепочки и, если передан, page текущего роута.
- Навигация: `loadPage("settings")` возвращает page-чанки активной цепочки. JS вызывает `addRouteTranslations(locale, page, tree)` для каждого чанка, затем `setRoute(page)`.
- `@i18n-micro/vue`: `createTauriI18n({ router })` ставит `router.beforeResolve`. Переход завершается, когда page-чанк уже загружен, поэтому сырые ключи не мелькают. Ключи старой страницы остаются в хранилище, как в Nuxt. Ошибка загрузки оставляет текущий словарь и не роняет навигацию.
- Без Vue: `link.setPage(name)` вызывается вручную; `link.preload(name)` прогревает следующую страницу.
- Кеш на стороне JS: `(locale, page) → version`. Повторный заход на страницу не делает IPC, пока версия не изменилась.

## 4. Крейт `i18n-next`

### 4.1 Создание

```rust
let i18n = I18n::builder()
    .locale("de")
    .fallback("en")
    .fallback_for("de-at", "de")
    .iso("ua", "uk-UA")                     // тег для плюралов и форматов
    .linked_messages(true)
    .dir("locales")                         // | .lazy_dir(..) | .files(..) | .messages(locale, page, json)
    .pages(true).page_alias("about-us", "about")
    .formats(json!({ "numberFormats": { … }, "datetimeFormats": { … } }))
    .time_zone("Europe/Berlin")             // по умолчанию системная
    .clock(|| now)                          // для tdr и тестов
    .missing(|locale, key, page| …)
    .build()?;
```

`I18n` — это `Arc` внутри: `Clone`, `Send`, `Sync`, `'static`. Работает в потоках, async-задачах и Tauri state без обёрток.

### 4.2 Короткие вызовы (имена из JS)

```rust
i18n.t("nav.home"); i18n.try_t("nav.home"); i18n.has("nav.home");
i18n.tc("apples", 5);
i18n.tn(1234.5); i18n.tn_as(9.99, "currency");
i18n.td(when); i18n.td_as(when, "short"); i18n.tdr(when);
i18n.set_locale("fr")?; i18n.locale(); i18n.locales();
i18n.with_locale("de"); i18n.page("settings");
i18n.add_messages("en", None, json!({ … }))?; i18n.reload()?;
i18n.subscribe(|c: &Change| …);         // Change { locale, version }
```

### 4.3 `Text`: аналог `<i18n-t>`

В JS `<i18n-t>` закрывает всё в одной точке. В Rust ту же роль играет ленивое значение `Text`:
1. одна цепочка с опциями, повторяющими пропсы компонента;
2. `Display`: подходит везде, где Rust принимает текст (`format!`, `tracing`, `thiserror`, `clap`, egui, ratatui, askama, maud, GTK, Slint);
3. `Serialize` как готовая строка: Tauri-команда или axum-хендлер, вернувшие `Text`, отдают перевод;
4. части шаблона для rich-текста (слоты);
5. `'static + Send + Sync + Clone`, разрешается при каждом показе. Хранится в состоянии UI и после `set_locale` показывает новый язык.

```rust
let total = i18n.text("cart.total")      // keypath
    .arg("name", &user.name)              // params: строки, числа, char, Text, Arg; Display → Arg::display
    .count(items.len())                   // plural → {count}
    .number(sum).number_as(sum, "currency")   // number → {number}
    .date(at).date_as(at, "short")        // date → {date}
    .relative(at)                         // relativeDate → {relativeDate}
    .or("Total")                          // defaultValue
    .hide_if_empty()                      // hideIfEmpty
    .plural_rule(rule)                    // customPluralRule
    .page("checkout").locale("de");

println!("{total}");
total.to_string(); total.resolve();        // Option<String>: None при промахе или hide_if_empty
total.html();                              // параметры экранированы, перевод доверенный

t!(i18n => "cart.summary", name = user.name, count = n);  // явный инстанс; без `i18n =>` — глобальный (4.4)
t!(i18n => "pair", ["Ann", "Bob"]);
t!(i18n => "hello", who = i18n.text("user.guest"));       // вложенный перевод
t!(i18n => "due", when = Arg::date_as(d, "short"));

// слоты, аналог foldTemplate: "Я принимаю {link} и {policy}"
let pieces: Vec<Widget> = i18n.text("terms").fold(
    |text| Widget::label(text),
    &[("link", &|| Widget::link("условия", URL)), ("policy", &|| Widget::link("политику", URL2))],
);
for part in i18n.text("terms").parts() { match part { Part::Text(s) => …, Part::Slot(name) => … } }
```

- Короткие `t` / `tc` / `tn` идут тем же путём (`text(..).to_string()`), поэтому логика одна, как `resolveText` в JS.
- `Arg`: `Str`, `Int`, `Float`, `Bool`, `Number { value, format }`, `Date { at, format }`, `Relative(at)`, `Text`. Обычные числа подставляются как `String(value)` в JS. Локализуются только явные `number` / `date` / `relative`.
- Рецепты в доках: egui, ratatui, Leptos/Dioxus, askama, axum. От UI-фреймворков крейт не зависит.

### 4.4 Глобальный доступ и макросы

Цель — не таскать `i18n` по функциям. Создал один раз при старте, дальше пишешь `t!("key")` в любом месте: в обработчике, в `Display` ошибки, в Tauri-команде, в CLI. Явный инстанс остаётся для библиотек и тестов. Фича `global` включена по умолчанию, она стоит один `OnceLock` и без зависимостей.

Установка:

```rust
I18n::builder().dir("locales").locale("ru").fallback("en").install()?;   // build() + глобальный инстанс
// или: i18n_next::install(i18n.clone());  Tauri-плагин делает это сам (.global(false) — отключить)
```

`install` один на процесс (`OnceLock`); второй вызов возвращает `Error::AlreadyInstalled`. Менять глобальный инстанс не нужно: он сам изменяемый (`set_locale`, `reload`, `add_messages`).

Макросы. Все возвращают ленивый `Text` (кроме форматов), поэтому подходят для `format!`, `Display`, `Serialize`:

```rust
use i18n_next::{t, tc, tn, td, tdr, key};

t!("nav.home");
t!("welcome", name = user.name);
t!("cart.summary", name = user.name, count = n);          // count = плюрал, как в tc
t!("pair", ["Ann", "Bob"]);
t!("title", @page = "settings");                           // опции — с `@`, чтобы не путать с параметрами
t!("title", @locale = "de", @or = "Title");
t!("total", @number = sum, @date = at, @relative = at);    // {number} {date} {relativeDate}
t!("terms", @html);
tc!("apples", 5);                                          // алиас для привычки из JS
tn!(1234.5); tn!(9.99, "currency");
td!(at); td!(at, "short"); tdr!(at);

t!(i18n => "nav.home");                                    // явный инстанс: та же грамматика
```

Синтаксис без двусмысленности: `name = x` в Rust тоже выражение, поэтому инстанс отделяется `=>`. Опции получают префикс `@`, чтобы параметр `{page}` оставался параметром.

Функции для тех, кто не любит макросы: `i18n_next::t("key")`, `i18n_next::global()` (`&I18n`), `set_locale`, `locale()`, `page("settings")`.

`key!` — константа для статики и enum'ов ошибок:

```rust
const QUIT: Key = key!("menu.quit");            // const fn, ничего не аллоцирует
println!("{QUIT}");                             // Display: разрешается в текущей локали
QUIT.args().arg("n", 3);                        // → Text

#[derive(thiserror::Error, Debug)]
enum AppError {
    #[error("{}", key!("errors.not_found"))] NotFound,
    #[error("{}", t!("errors.io", path = Arg::display(.0.display())))] Io(std::path::PathBuf),
}
```

**`scope`: локаль и страница на запрос или задачу без передачи параметров.** Глобальный инстанс один, а у серверного запроса или окна своя локаль. Аналог `run(locale, fn)` из `@i18n-micro/node`: thread-local стек переопределений, который читают макросы.

```rust
// синхронно
i18n_next::scope().locale("de").page("checkout").run(|| render());      // внутри t!("x") — de + checkout

// async, без зависимости от рантайма: обёртка ставит scope на время каждого poll и снимает после
let fut = i18n_next::scope().locale(lang).wrap(async { handler(req).await });

// axum: middleware
async fn locale_mw(req: Request, next: Next) -> Response {
    let lang = negotiate(req.headers());                  // i18n_next::negotiate(accept_language, locales)
    i18n_next::scope().locale(lang).wrap(next.run(req)).await
}

// тесты: свой инстанс только в этом потоке или задаче, без глобального состояния
i18n_next::scope().instance(test_i18n).run(|| assert_eq!(t!("a").to_string(), "A"));
```

Порядок разрешения у макроса без `=>`:
1. ближайший `scope` (instance, locale, page);
2. глобальный инстанс;
3. если `install` не было — ключ как текст и одно предупреждение в stderr (в debug), без паники в проде.

Опции `@locale` / `@page` в самом вызове сильнее scope.

Почему так, а не `tokio::task_local!`: обёртка над `Future` работает с любым рантаймом (tokio, async-std, smol, Tauri async runtime) и не тянет зависимостей. Цена — запись в TLS на каждый poll, это наносекунды; бенч `scope_poll` это покажет.

В Tauri плагин вызывает `install` сам, поэтому в командах, меню и уведомлениях достаточно `t!("key")`.

### 4.5 Внутреннее устройство

```
I18n(Arc<Inner>)
Inner    { id, version: AtomicU64, state: RwLock<Arc<Snapshot>>, source, listeners, missing, clock, zone }
Snapshot { locale, chains, version: u64, chunks: HashMap<(locale, page), Arc<Chunk>>, formats }
Chunk    { flat: HashMap<Box<str>, Msg>, json: Arc<[u8]> /* для фронта, сериализуется один раз */ }
Msg      = Text(Template) | Plural { marked, forms: Vec<Template> }
```

- Чтение: атомарная загрузка `version` и сравнение с `thread_local!` кешем `(inner id, version, Arc<Snapshot>)` этого потока. Совпало — работа идёт со снимком без блокировок и без изменения счётчика ссылок. Не совпало — один раз read-lock и обновление кеша. Запись строит новый снимок, подменяет его под write-lock и увеличивает `version`.
- Дерево разворачивается в плоскую карту при загрузке. Lookup — один хеш-поиск на слой.
- `std::RwLock` только на записи и на промахе кеша. Хешер карты выбирает бенч (раздел 2.2).
- Глобальное состояние: `static GLOBAL: OnceLock<I18n>` и `thread_local!` стек `scope`. Обёртка `Scoped<F: Future>` кладёт кадр в стек на время `poll` и снимает его даже при панике (guard в `Drop`).
- Ошибки: `Io`, `Json { path }`, `NotObject { path }`, `UnknownLocale`, `UnsupportedFormat { locale, name, option }`, `AlreadyInstalled`. Битый файл — ошибка, в JS — warn и пропуск.

## 5. Плюралы и форматы без ICU4X

### 5.1 Почему не ICU4X

| | ICU4X 2.4 + `icu_experimental` 0.7 | Свой движок на данных CLDR |
| --- | --- | --- |
| Зависимости | `icu_*`, `zerovec`, `yoke`, `writeable`, `fixed_decimal`, провайдеры данных — десятки крейтов | ноль (данные — сгенерированные файлы в крейте) |
| Стабильность | relative, валюта, проценты, compact — только в `icu_experimental`, где минор ломает API | API наш |
| Совпадение с браузером | своя версия CLDR, свой алгоритм выбора паттерна; расхождения с `Intl` чинить нечем | данные той же версии CLDR, что у Node, и тот же алгоритм; golden-фикстуры из `Intl`, отличие — красный тест, и чинится у нас |
| Объём | мегабайты compiled data; урезание через `icu4x-datagen` и `ICU4X_DATA_DIR` — отдельный инструмент у пользователя | только нужные поля; урезание — переменной окружения в нашем `build.rs` |
| Производительность | неизвестна для нашего сценария | таблицы `&'static`, без аллокаций кроме результата; измеряется нашими бенчами, ICU4X — в сравнении (`benches/compare`) |

Кода действительно немного. Основной объём — данные: названия месяцев (в русском родительный падеж отличается от именительного), паттерны дат по длинам, разделители и группировка (индийская 3;2, `minimumGroupingDigits` 2 в польском и испанском: `1234` без пробела), символы валют, паттерны относительного времени по плюрал-формам. Данные генерируем, а не пишем руками.

### 5.2 Данные

- Источник: официальные пакеты Unicode `cldr-core`, `cldr-numbers-full`, `cldr-dates-full`. Версия закреплена и **равна CLDR в Node CI** (Node 22.23 = ICU 78.3 = CLDR 48.0; пакеты `48.0.0` есть). Тогда golden-фикстуры из `Intl` и наши данные из одного источника.
- `scripts/cldr.mjs` берёт только нужные поля и пишет `crates/i18n-next/data/cldr/<locale>.json` (минифицированно, в репозитории; перегенерация явная, CI проверяет актуальность):

  | Поле | Откуда |
  | --- | --- |
  | плюрал-правила (cardinal) | `cldr-core/supplemental/plurals.json` → компилируются в Rust-код `src/plural/rules.rs` (match по правилам с операндами n, i, v, w, f, t, e) |
  | символы, система цифр по умолчанию, `minimumGroupingDigits` | `numbers.json` + `numberingSystems.json` |
  | паттерны decimal / percent / currency, compact short | `numbers.json` |
  | символы и имена валют | `currencies.json` |
  | названия месяцев и дней (format и standalone; wide, abbreviated, narrow), dayPeriods, эры | `ca-gregorian.json` |
  | `dateFormats` / `timeFormats` / `dateTimeFormats` и `dateTimeFormats-atTime` по длинам | `ca-gregorian.json` |
  | паттерны всех поддерживаемых комбинаций компонентов (год, месяц, день, день недели, час, минута, секунда, hour12) | ICU в Node: `formatToParts` на пробных датах (однозначные поля для ширины, май для контекста месяца), U+202F → пробел (спайки S9, S10); интернируются между локалями |
  | относительное время по единицам и плюрал-формам, `-1` / `0` / `1` для `numeric: auto` | `dateFields.json` |
  | hour cycle по региону, parent locales | `timeData.json`, `parentLocales.json` |

- Набор локалей по умолчанию — уровень покрытия `modern` из `cldr-core/coverageLevels.json`.
- `build.rs` склеивает в `OUT_DIR` только выбранные локали. По умолчанию это все, а переменная `I18N_NEXT_LOCALES=en,de,ru` урезает набор. Цепочки parent-локалей добавляются автоматически.
- Данные генерируются как Rust-статика для выбранных локалей: декодера в рантайме нет (2.4). Размер замеряется на этапе M5, таблица — в доках.

### 5.3 Что поддерживаем (ровно то, что проверяют фикстуры)

| JS (`Intl` options) | v1 |
| --- | --- |
| плюралы: cardinal-категории для любого числа | да |
| number `decimal`, `minimum/maximumFractionDigits`, `minimumIntegerDigits`, `useGrouping`; округление half-expand по кратчайшему десятичному представлению `f64` (как ICU) | да |
| `percent`; `currency` + `currencyDisplay: symbol / narrowSymbol / code / name` | да |
| `notation: compact` (`compactDisplay: short`) | да |
| система цифр по умолчанию для локали (`ar-EG` → `١٬٢٣٤`) | да |
| `style: unit`, `signDisplay`, `roundingMode`, `compactDisplay: long`, scientific | нет, ошибка при `build()` |
| date `dateStyle` / `timeStyle` | да, прямо из `dateFormats` / `timeFormats` / `dateTimeFormats` |
| компоненты `year`, `month`, `day`, `weekday`, `hour`, `minute`, `second`, `hour12` / `hourCycle` | да; паттерн каждой комбинации заранее получен у ICU в Node (`formatToParts`) и лежит в данных (спайки S9, S10), рантайм только отрисовывает |
| `timeZone` | да (фича `tz`) |
| `timeZoneName`, `timeStyle: full / long` (нужны имена зон), `era`, не-григорианские календари | нет, ошибка при `build()` |
| relative `numeric: always / auto`, `style: long / short / narrow`; выбор единицы как в `FormatService` | да |

Невалидная дата даёт `"Invalid Date"`. Именованные форматы ищутся по укороченным тегам (`zh-Hant-TW → zh-Hant → zh`), неизвестная локаль идёт по CLDR parent-цепочке до `root`. Неподдерживаемая опция — ошибка при `build()`, а не тихое расхождение с фронтом.

### 5.4 Время: свой модуль `tz`

Нужно одно: момент → дата и время в зоне. Это небольшая календарная арифметика и чтение данных зон, поэтому пишется своим кодом.

| Часть | Как |
| --- | --- |
| Календарь | `days_from_civil` / `civil_from_days` (алгоритмы Говарда Хиннанта), день недели, без таблиц |
| UTC и фиксированные смещения | всегда, без фичи |
| Данные зон на unix, macOS, iOS, WASI | свой читатель TZif v1 / v2 / v3 (`/usr/share/zoneinfo`, `/var/db/timezone/zoneinfo`, `$TZDIR`) + вычисление POSIX-правила из хвоста файла для дат после последнего перехода |
| Android | контейнер tzdata (`/apex/com.android.tzdata/etc/tz/tzdata`, `/system/usr/share/zoneinfo/tzdata`): индекс + те же TZif внутри |
| Windows | системных TZif нет, а API требует `unsafe` FFI, поэтому фича `tzdb` (на Windows включена по умолчанию): компактная tzdb из своего генератора (`scripts/tzdb.mjs` из IANA tzdata), только переходы с 1970 года и POSIX-правило |
| Браузер | `js-sys`: локальная зона — `Date.getTimezoneOffset()`, именованная — `Intl.DateTimeFormat({ timeZone }).formatToParts` с кешем смещения по (зона, час) |
| Системная зона | `TZ`, затем цель симлинка `/etc/localtime`; в Tauri — зона WebView (`Intl.DateTimeFormat().resolvedOptions().timeZone`) из первого `load`; явная `.time_zone(...)` сильнее всего |
| Часы | native — `SystemTime` (вне wasm он не паникует); браузер — `Date.now()`; тесты — `.clock()` |

Проверка:
- дифференциальный тест против `jiff` (только dev-dependency): все зоны tzdb × моменты 1900–2100 (переходы, их окрестности, случайные) → одинаковые смещения;
- фикстуры TZif (v1, v2 с 64-битными временами, v3, пустые переходы, только POSIX-хвост);
- fuzz-цель `tzif`;
- в браузере сверка с `Intl` в headless Chrome.

Объём: оценка 400–600 строк вместе с POSIX-правилами. Это меньше, чем код, который пришлось бы проверять в чужом крейте.

### 5.5 Производительность форматов

Форматтер = индекс локали, `&'static` поля, запись в `String`. Кеша не нужно: строить нечего. Бенчи (раздел 7.7) меряют `tn` / `td` / `tdr`, а `benches/compare` — то же у ICU4X, как точку отсчёта.

## 6. Tauri-плагин `tauri-plugin-i18n-next`

Структура как у `tauri-plugin-centrifugo`. `core.rs` нет: состояние — сам `I18n`.

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_i18n_next::Builder::new(i18n.clone()).system_locale(true).build())
t!("menu.quit");                // плагин сам делает install; .global(false) — отключить
app.i18n().t("menu.quit");      // трейт-расширение Manager; app.state::<I18n>() тоже работает
```

### 6.1 Передача

Замер в настоящем WebView (спайк S15: macOS, WKWebView, Tauri 2.12, release; медиана, мс):

| Способ | 81 КБ (1k ключей) | 832 КБ (10k) | 8.5 МБ (100k) |
| --- | --- | --- | --- |
| invoke raw (`ipc::Response`) → `TextDecoder` → `JSON.parse` | 0.65 | 3.05 | 24.5 |
| invoke raw, только передача | 0.40 | 0.70 | 3.5 |
| invoke с `serde_json::Value` | 0.65 | 4.15 | 43.0 |
| **custom protocol: `fetch().json()`** | **0.35** | **2.45** | **21.0** |
| custom protocol, только передача (`arrayBuffer`) | 0.25 | 0.55 | 3.5 |
| `Channel` чанками по 256 КБ → разбор | 0.95 | 3.55 | 29.5 |
| строка уже в JS: `JSON.parse` (как встроенный в бандл JSON) | 0.10 | 1.25 | 17.0 |

Выводы:
- Сам транспорт быстрый: 8.5 МБ передаются за ~3.5 мс любым двоичным способом. 70–85% времени уходит на `JSON.parse`, и этого не избежать ни одним способом, даже встраиванием в бандл: там тот же разбор (17 мс). Значит, главный рычаг — **сколько** отправлять (только слои активной цепочки, страницы по требованию, вытеснение), а не **как**.
- `fetch().json()` по custom protocol быстрее всех: нет промежуточной JS-строки от `TextDecoder`. Плюс HTTP-механика бесплатно: `ETag` по версии чанка, `Cache-Control`, `<link rel="preload">`, параллельные запросы.
- `serde_json::Value` в 1.7–2 раза медленнее, отвергнут. `Channel` медленнее и сложнее, отвергнут.
- Замер сделан на macOS. WebView2 (Windows), WebKitGTK (Linux) и Android WebView проверяются той же страницей-бенчем в плейграунде `tauri-app` (inspector), цифры попадают в таблицу производительности. Если где-то custom protocol окажется медленнее invoke, guest-js переключится на raw invoke для этой платформы. Оба пути реализованы и покрыты тестами.

Решение:

| Канал | Для чего |
| --- | --- |
| **custom protocol `i18n-next://`** (`convertFileSrc(path, 'i18n-next')` даёт `i18n-next://localhost/…` на macOS / iOS / Linux и `http://i18n-next.localhost/…` на Windows / Android) | данные: `GET /v{version}/{locale}/{page}.json` (чанк как есть, сырые байты файла), `GET /v{version}/bundle?locale=…&page=…` (root цепочки + страница одним ответом). Ответ: `Content-Type: application/json`, `ETag`, `Cache-Control: max-age=31536000, immutable` (версия в пути) |
| команды IPC | управление и метаданные: `set_locale`, `get_locale`, `available_locales`, `pages`, `meta` (версия, цепочка, форматы, `locales` из `i18n.json`), `report_missing` (dev) |
| raw invoke `load` / `load_page` | запасной путь с тем же payload, когда протокол недоступен (переключается в guest-js) |
| событие `i18n-next://changed { locale, version }` | уведомление; новая версия меняет URL, поэтому кеш никогда не отдаёт старое |

Безопасность протокола: у custom protocol нет ACL, как у команд, поэтому обработчик:
- принимает только `GET`;
- принимает только имена из каталога (8b.2);
- сверяет `Origin` с origin приложения, чужой — 403.

Переводы не секрет, но это убирает доступ из встроенных сторонних фреймов. Обработчик протокола — чистая функция `Request → Response`, поэтому тестируется без WebView (story-тесты и `ipc_model`).

### 6.2 JS: `tauri-plugin-i18n-next-api`

Без runtime-зависимости от `@i18n-micro/*`. `/vue` использует опциональную peer-зависимость `@i18n-micro/vue`.

Данные guest-js берёт через `fetch(convertFileSrc(…, 'i18n-next')).json()`, с `<link rel="preload">` на root активной цепочки при старте. Управление идёт через команды. Если протокол недоступен, переключается на raw invoke `load` / `load_page` (6.1). Rust-гость (`tauri-guest`) делает то же через `fetch` из `js-sys`, без `web-sys`.

```ts
import { createI18n } from '@i18n-micro/runtime'
import { connect } from 'tauri-plugin-i18n-next-api'
const link = await connect(createI18n({ locale: 'en' }), { page: 'index' })
await link.setPage('settings')        // load_page + addRouteTranslations + setRoute
await link.setLocale('de')            // Rust и все окна
link.preload('about')

import { createTauriI18n } from 'tauri-plugin-i18n-next-api/vue'
app.use(await createTauriI18n({ router, /* опции createI18n */ }))   // beforeResolve грузит page-чанк
// <i18n-t keypath="cart.total" :number="sum" /> на сообщениях и форматах из Rust
```

Отвергнуто:
- симлинки: pnpm, публикация, Windows;
- обёртка над core: дубль `t`, двойной core в бандле;
- отдельный адаптер-пакет: лишний пакет на 40 строк.

### 6.3 `bind` для нативного UI

```rust
app.i18n().bind(&quit_item, app.i18n().page("tray").text("quit"));
app.i18n().bind(&tray, app.i18n().text("tray.tooltip"));
app.i18n().bind(&window, app.i18n().text("window.title"));
```

Трейт `Relabel` реализован для `MenuItem`, `CheckMenuItem`, `IconMenuItem`, `Submenu`, `TrayIcon` (tooltip) и `WebviewWindow` (title). Плагин подписан на `Change`. Есть `unbind`. Команды, возвращающие `Text`, сериализуются переведёнными.

## 7. Проверка

Подход тот же, что в `tauri-plugin-centrifugo`, `tauri-plugin-serialplugin`, `cargo-autoprune` и `oxlint-vue-rs`:
- тестируется не функция, а сценарий через публичный вход: крейт — через `I18n` и макросы, плагин — через IPC, guest-js — через `connect`;
- proptest-модели гоняют случайные последовательности действий пользователя и сверяют каждый ответ с маленькой эталонной моделью после каждого шага, а в конце проверяют инварианты всей сессии;
- story-тесты с Given/When/Then закрывают то, что модель выражает плохо: тайминги, гонки, порядок событий;
- мутации (cargo-mutants) показывают, чего сценарии не замечают. Выживший мутант — повод расширить модель или историю, а не конфиг.

### 7.1 Инфраструктура

- `tests/support/` — симулятор окружения, как `support/sim.rs` в cargo-autoprune:
  - `Locales`: дерево файлов локалей (root, `pages/…`, битые файлы, BOM, `__proto__`), отдаётся как `files()`, как временный каталог для `dir` / `lazy_dir` или как `remote`;
  - виртуальные часы (`clock`) и фиксированная зона;
  - `FakeFetch` для `ensure_async`: запросы копятся, тест отвечает на них в любом порядке, с ошибкой или без, как `fakeHost` в loader-model nuxt-i18n-micro;
  - ручной executor для `scope().wrap()`: поллит задачи в порядке, который задаёт тест, без рантайма и без sleep;
  - guard: каждый вызов `missing` и каждое предупреждение записываются, и тест падает на неожиданном, даже если assert'ы прошли. Ожидаемое разрешается явно (`guard.allow("nav.unknown")`).
- `proptest`: `TestRunner` с `Config { cases, failure_persistence: None, max_shrink_time }`, с фиксированным seed (`TestRng::from_seed`), поэтому падение воспроизводится. При расхождении proptest сжимает последовательность до самой короткой и печатает её.
- `cargo nextest` и `.config/nextest.toml`:
  - `default`: жёсткий таймаут на тест, зависание убивается за секунды;
  - `ci`: `fail-fast = false`, JUnit;
  - `mutants`: короткий таймаут, `fail-fast = true`, ограниченное число потоков.

  Retries нет нигде: тест, который падает раз в сто запусков, — баг.
- Реального времени в тестах нет: часы виртуальные, события собираются в mpsc, ожидание — `recv_timeout` только как предохранитель.

### 7.2 Совместимость с JS: golden-файлы

- `scripts/compat.mjs` (Node 22, full-icu, CLDR 48) прогоняет матрицу кейсов через настоящие `@i18n-micro/core` и `@i18n-micro/utils`:
  - загрузка каталога (`loadTranslationDir`), слои `BaseI18n` с роутами, fallback;
  - `interpolate`, `pluralForm`, `resolveLinkedMessages`, `resolveText`;
  - `Intl.PluralRules`, `NumberFormat`, `DateTimeFormat`, `RelativeTimeFormat` по опциям из 5.3 для `en`, `de`, `ru`, `uk`, `pl`, `fr`, `es`, `ar`, `ar-EG`, `hi`, `ja`, `zh-Hant`, `lv`, `he`.
- Формат как у characterization-голденов nuxt-i18n-micro: одна строка на вызов, `call → result`, в `fixtures/compat/__golden__/*.md`.
- Rust-тест `compat.rs` выполняет те же вызовы, печатает те же строки и сравнивает файл целиком. Diff — это падение, и он показывает ровно те строки, что разошлись.
- Компат-голдены пишет только JS: Rust их не перезаписывает.
- Свои characterization-голдены Rust (`tests/__golden__`, например схема payload) обновляются через `UPDATE_GOLDEN=1`. Каждый diff читается целиком: это либо задуманная правка, либо баг.
- CI проверяет, что голдены и `data/cldr` совпадают с перегенерированными.

### 7.3 Модели

| Файл | Сессия (действия) | Что предсказывает модель | Инварианты в конце |
| --- | --- | --- | --- |
| `catalog_model.rs` | добавить или изменить файл (root / page, битый), `reload`, `set_locale`, цепочки fallback, `pages(false)`, `page_alias`, linked, `t` / `page().t` / `with_locale().t` / `has` / `try_t`, `add_messages` | каждый ответ, каждый вызов `missing`, ошибки загрузки с путём | версия снимка только растёт; старый `Arc<Snapshot>` у читателя не меняется; после последнего `reload` ответы совпадают с моделью «с нуля» |
| `loading_model.rs` | `lazy_dir` / `remote` + `ensure_async` на разные `(locale, page)`, ответы `FakeFetch` в случайном порядке, ошибки сети | какие файлы запрошены, что видно после каждого ответа | каждый файл успешно загружен не более одного раза; ошибка не портит словарь; два `ensure` одного файла дают один запрос |
| `text_model.rs` | цепочки `Text` и формы макросов (`t!` с `=>` и без, `@page`, `@locale`, `@or`, `count`, `@number`, `@date`, `@html`, `hide_if_empty`, слоты `fold`) | строку и части (`parts`) как `resolveText` + `foldTemplate` | `Text` после `set_locale` показывает новую локаль; `Serialize` == `Display` |
| `scope_model.rs` | вложенные `scope`, `wrap` нескольких задач, поллинг вперемешку на одном потоке, паника внутри `run`, `install` | локаль и страницу, которые видит каждый `t!` | стек scope пуст после каждой задачи; паника его не оставляет |
| плагин, `ipc_model.rs` | два окна, `set_locale` из JS и из Rust, `load`, `load_page`, `add_messages`, `bind` / `unbind` пунктов меню, трея и заголовка | байты ответа (чанки, версия), события в каждом окне, текст каждого привязанного элемента | все окна видят одну версию; отвязанный элемент больше не меняется |
| guest-js, `connect.model.test.ts` (fast-check, фиксированный seed) | `setPage`, `setLocale`, `preload`, событие `changed` от Rust, ответы IPC в любом порядке, ошибки | `locale` / `route` инстанса, загруженные чанки, число вызовов IPC | поздний ответ для страницы A не перебивает уже открытую B; кеш по версии без лишних вызовов |

`global_scope` вынесен в отдельный test binary: глобальное состояние одно на процесс.

### 7.3a Свой код против эталонов

Каждая замена зависимости (2.3) проверяется дифференциально: эталон подключается только в dev-dependencies.

| Свой модуль | Эталон | Проверка |
| --- | --- | --- |
| `json` | `serde_json`, JSONTestSuite | корпус: все `y_` дают то же, что `serde_json`, все `n_` — ошибку, `i_` — записанное решение; proptest на случайных документах (с escape, суррогатами, BOM и глубиной) против `serde_json`; fuzz |
| `hash` | — | распределение по корзинам на реальных ключах; бенч против `foldhash` и `rustc-hash` |
| `tz` | `jiff`, `Intl` | все зоны × моменты 1900–2100 против `jiff`; в Chrome — против `Intl`; фикстуры TZif; fuzz `tzif` |
| числа и даты | `Intl` (Node) | compat-голдены (7.2) |
| плюралы | `Intl.PluralRules` | `plural_props` |

### 7.4 Истории

Короткие файлы на один поток событий. Doc-комментарий содержит Given/When/Then, в теле — `// step n: действие → проверка`.
- `stories.rs` (крейт):
  - Android-путь: `files()` с вложенными страницами;
  - битый файл → ошибка с путём;
  - неподдерживаемая опция Intl → ошибка при `build()`;
  - `key!` в `thiserror`-enum показывает текущую локаль;
  - axum-подобный обработчик: два запроса с разными `Accept-Language` вперемешку.
- `stories.rs` (плагин):
  - навигация → в ответе только чанки страницы, повторный заход без IPC;
  - смена локали из трея → событие в обоих окнах, перезагружены root и страница, переведены меню и заголовок;
  - `Text`, возвращённый командой, приходит переведённым.

### 7.5 Мутации

`.cargo/mutants.toml` в стиле остальных пакетов. Запуск только локально, по крейту:

```bash
cargo install --locked cargo-mutants
export TMPDIR="$PWD/target/mutants-tmp" && mkdir -p "$TMPDIR"
cargo mutants -p i18n-next
cargo mutants -p tauri-plugin-i18n-next
git diff origin/main.. > target/pr.diff && cargo mutants -p i18n-next --in-diff target/pr.diff
```

- `test_tool = "nextest"`, `additional_cargo_test_args = ["--profile", "mutants"]`, `[profile.mutants]` в `Cargo.toml` (dev без debug info, быстрее линковка).
- Таймаут засчитывается как убитый мутант: такой мутант зацикливает код.
- Исключения только трёх видов, у каждого причина в комментарии:
  1. код, не собираемый на хосте (`tauri_guest`, ветки `cfg(wasm)`): он проверяется wasm-тестами, раздел 7.6;
  2. эквивалентные мутанты: результат тот же при любом входе;
  3. `Debug` и текст логов.

  Всё остальное закрывается сценарием.
- Чтобы мутанты были видны в тестах, решения отделяются от обвязки, как в tauri-plugin-widgets: `Relabel` — тонкий вызов Tauri, а что и когда переименовать — чистая функция, которую видит модель.
- guest-js: Stryker локально (`pnpm test:mutation`), как в nuxt-i18n-micro.

### 7.6 wasm

- CI собирает `i18n-next` для `wasm32-unknown-unknown` (с фичами по умолчанию, с `tauri-guest` и без `tz`) и для `wasm32-wasip1`.
- `wasm-bindgen-test`:
  - те же compat-голдены и истории (кроме файловых) в Node и в headless Chrome;
  - браузерные проверки: модуль `tz` даёт ту же зону, что `Intl`, `scope().wrap()` с `wasm-bindgen-futures`, `ensure_async` с настоящим `fetch` к локальной статике теста;
  - `tauri_guest` против заглушки `__TAURI_INTERNALS__`, которая отдаёт payload, построенный `payload::encode`.
- `wasm32-wasip1`: `cargo test` с раннером wasmtime проверяет путь `dir` и `lazy_dir`.
- Proptest-модели запускаются только native: логика та же, а wasm-прогон проверяет платформенные места.
- Размер `.wasm` примера (полный CLDR, три локали, без `format`) записывается в таблицу производительности.

### 7.7 Производительность

Три уровня. Каждый ловит своё, и ни один не зависит от шума машины в CI.

**1. criterion — бенч на каждую публичную функцию** (`crates/*/benches/`).
- Правило: у каждого `pub fn` и публичного метода есть бенч с тем же именем (`group/function/case`). Тест `bench_inventory.rs` проверяет это: берёт публичные функции из снимка `api/<crate>.txt` (8b.9), имена `bench_function` из `benches/` и падает на функции без бенча, как `command-inventory` в serialplugin.
- Кейсы для каждой функции — реальные размеры и худшие случаи:
  - lookup: root hit, page hit, page miss → root, промах по всей цепочке из трёх локалей, ключ глубиной 6;
  - `Text`: 0, 1, 5 и 10 аргументов, плюрал (index, unmarked, `=0 `), слоты, `html`;
  - `t!` через global, через `scope` и через явный инстанс; `scope().wrap` на один poll;
  - `tn` / `td` / `tdr` по каждому стилю из 5.3, на `en`, `ru`, `ar-EG`;
  - загрузка 1k / 10k / 100k ключей, eager / lazy / `ensure_async`;
  - `payload::encode` / `apply`, `load` / `load_page` плагина;
  - конкуренция: 1, 4 и 16 потоков читают, пока один пишет.
- Локальное A/B: `cargo bench -- --save-baseline main` на main, затем `--baseline main` на ветке; отчёт criterion показывает изменение по каждой функции. Регрессия горячего пути — баг: чинится устройство, а не принимается цифра.

**2. Детерминированные проверки в CI** (без замеров времени, поэтому не флакают):
- `gungraun` (бывший iai-callgrind, Valgrind, только linux) считает инструкции и обращения к кешу на тех же сценариях. CI-джоб `perf` запускает его на базовом коммите и на PR в одном джобе с одним toolchain и падает при росте инструкций горячего пути больше порога (по умолчанию 3%). Число инструкций не зависит от загрузки машины.
- `tests/alloc.rs` (dhat в режиме testing) фиксирует число аллокаций:
  - `Display` у `Text` без параметров — 0;
  - `t()` — ровно 1;
  - `get(key)` — 0;
  - `write_to` в готовый буфер — 0;
  - `load_page` payload — 1.

  Лишняя аллокация — красный тест.

**3. Сравнение и итоговые таблицы.**
- `benches/compare` (`publish = false`): lookup, интерполяция и плюралы у `rust-i18n` и `fluent-bundle`; плюралы и форматы у ICU4X.
- JS-сторона: vitest-бенч `JSON.parse` payload и `connect` / `setPage` на тех же размерах.
- IPC: размер и время `get_ipc_response` для root 1k / 10k и типичной страницы, raw против `Value`.
- Размеры: native-бинарь и `.wasm` примера (весь CLDR, три локали, без `format`).
- `scripts/bench-table.mjs` собирает criterion JSON в `docs/content/5.performance.md`: ns/op по каждой функции, аллокации, сравнение, размеры. Цифры в доках — с машины и версии, указанных в шапке таблицы.

## 8. Документация

Сайт по образцу `tauri-plugin-centrifugo/docs`: тот же стек, тот же вид, тот же тон.

- **Стек:** Nuxt 4 + `@nuxt/ui` 4 + `@nuxt/content` 3, `sqliteConnector: 'native'`, статическая генерация на GitHub Pages (`baseURL` `/i18n-next-rs/`, `autoSubfolderIndex: false`, `failOnError`), иконки встроены в бандл (`clientBundle`).
- **Каркас:**
  - `app/app.vue`, `app/error.vue`, `app/layouts/docs.vue`, `app/pages/index.vue`, `app/pages/[...slug].vue`, `app/components/AppHeader.vue`, `app/components/prose/H1.vue`, `app/assets/css/main.css` — копия из centrifugo с заменой названий;
  - `app.config.ts`: `primary`, `neutral: 'zinc'`, prose-настройки таблиц и кода, иконки языков (`rust`, `toml`, `ts`, `json`, `bash`), `nav`;
  - `content.config.ts`: коллекции `landing` и `docs`, флаг `generated`.
- **Тон:** коротко и по делу. Страница начинается с одного-двух предложений, что это, затем код, затем таблица опций и короткие абзацы про поведение и крайние случаи. Без вступлений и маркетинга, без карточек. Ссылки между страницами — там, где нужна деталь. Язык сайта — английский, как в centrifugo.
- **Главная** (`index.md`): одна строка описания и один блок кода, который показывает всё сразу (Rust `install` + `t!` и фронт `createTauriI18n` + `<i18n-t>`).
- **`scripts/generate.mjs`** строит перед `dev` / `generate`:
  - `3.reference/` — JS API и типы из `guest-js` (TypeScript compiler API), команды из `commands.rs`, permissions из `permissions/`, схему payload из голдена;
  - `9.changelog.md` из `CHANGELOG.md`.

  Сгенерированное в `.gitignore`, правится только исходник.
- **Rust API** живёт на docs.rs; reference-страница даёт ссылки по разделам. README крейта — это и главная страница docs.rs (`#![doc = include_str!("../README.md")]`), доктесты компилируются.

```
docs/content/
  index.md                         одна строка + один пример
  1.getting-started/               .navigation.yml (icon: i-lucide-rocket)
    1.introduction.md              что это, чем отличается, когда брать
    2.installation.md              crate, плагин, npm, фичи, I18N_NEXT_LOCALES, i18n.json
    3.quick-start.md               Rust: install + t!; Tauri + Vue: createTauriI18n + <i18n-t>
  2.guides/                        .navigation.yml (icon: i-lucide-book-open)
    1.locale-files.md              формат, i18n.json, общие файлы с nuxt-i18n-micro
    2.pages.md                     root и page, слои, Rust-only страницы, alias, pages(false)
    3.parameters.md                {name}, {0}, Arg, вложенный Text
    4.plurals.md                   индексы, unmarked, =0, iso
    5.fallback.md                  цепочки, missing
    6.linked-messages.md
    7.loading.md                   dir / lazy_dir / files / remote + ensure_async / extend (библиотеки) / reload / watch
    8.text.md                      Text: Display, Serialize, resolve, html, хранение в состоянии
    9.rich-text.md                 fold / parts, рецепты egui, ratatui, Leptos, askama
    10.global-and-scope.md         install, макросы, key!, scope для запроса
    11.formats.md                  tn / td / tdr, именованные форматы, зоны, clock
    12.tauri.md                    передача, события, кеш, окна, bind, persist, отчёт о промахах, безопасность имён
    13.frontend.md                 connect, /vue, router guard, <html lang dir>, React / Preact / Solid, types-generator
    15.checks.md                   check(), типобезопасные ключи (кодоген), CLI
    14.testing-your-app.md         scope().instance, фиксированные clock / zone
  3.reference/                     генерируется (icon: i-lucide-file-code)
  4.platforms/                     .navigation.yml (icon: i-lucide-monitor-smartphone)
    1.overview.md                  таблица целей и что где доступно
    2.desktop.md
    3.android.md                   files() вместо dir, ресурсы
    4.ios.md
    5.browser-wasm.md              Leptos / Yew / Dioxus, ensure_async, размер .wasm
    6.tauri-guest.md               Rust-фронт в Tauri
  5.performance.md                 методика и таблицы (генерируется из criterion JSON)
  6.compatibility.md               матрица с nuxt-i18n-micro, опции Intl, данные CLDR и версии ICU у WebView
  7.contributing.md                сценарии, модели, истории, голдены, nextest, mutants, wasm-тесты, бенчи
  9.changelog.md                   генерируется
```

README: корень (обзор и ссылки), `crates/i18n-next` (быстрый старт, он же docs.rs), `crates/tauri-plugin-i18n-next` (установка, permissions, JS). Каждый README короткий и ведёт на сайт.

## 8a. Плейграунды

Три приложения, как `examples/tauri-app` в centrifugo. Все работают с одним каталогом `examples/locales/` (root + `pages/`) и одним `examples/i18n.json` (8b.1). Это и есть демонстрация главного обещания: одни файлы для Rust, Tauri и браузера. Каждый плейграунд — витрина фич и ручной стенд для проверки, а CI его собирает.

| Плейграунд | Что это | Что показывает | Запуск |
| --- | --- | --- | --- |
| `examples/rust` (в workspace, `publish = false`) | `main.rs` — консольный тур; `bin/server.rs` — маленький axum-сервер | тур: `install` + `t!`, страницы, Rust-only страница, fallback-цепочка, плюралы на `en` / `ru` / `ar`, `Text` в `format!` и в `thiserror`, `fold` для rich-текста, `tn` / `td` / `tdr`, `scope`, `with_locale`, `reload`, время каждой операции. Сервер: `scope().wrap` на запрос по `Accept-Language`, ответ — JSON из `Text` через `Serialize` | `cargo run -p playground-rust -- --locale ru`; `cargo run -p playground-rust --bin server` |
| `examples/tauri-app` (свой Cargo workspace и lock, как в centrifugo) | Vue 3 + vue-router + `@i18n-micro/vue` + плагин; desktop, Android, iOS (`gen/`) | страницы `index` / `settings` / `about` с подгрузкой page-чанков через router guard; `<i18n-t>` с `plural` / `number` / `date` / `relativeDate` / слотами; переключатель локали; второе окно (синхронизация через `changed`); нативное меню, трей и заголовок через `bind`; команда, которая возвращает `Text`; уведомление из Rust через `t!`; панель «inspector»: размер payload, время IPC и `JSON.parse`, загруженные чанки и версии | `cd examples/tauri-app && pnpm install && pnpm tauri dev` (`pnpm tauri android dev`) |
| `examples/leptos-app` (свой workspace; Leptos CSR + trunk) | один Rust-фронт в двух режимах: в браузере сам по себе и как фронт Tauri | браузер: `remote` + `ensure_async` (fetch `/locales/...`), `t!` в компонентах, переключатель, страницы через `leptos_router`, `tn` / `td` с зоной из `Intl`, размер `.wasm` на экране. Tauri: тот же код, при наличии `__TAURI_INTERNALS__` вызывается `tauri_guest::connect`, а сообщения приходят от плагина | браузер: `trunk serve`; Tauri: `cargo tauri dev` в той же папке |

- Корневые скрипты: `pnpm playground:tauri`, `pnpm playground:web`, `cargo run -p playground-rust`.
- CI собирает все три:
  - `playground-rust` запускается с `--check`: тур идёт по всем локалям и падает на любом `missing`;
  - `tauri-app` — `pnpm build` и `cargo build` для `src-tauri` на linux;
  - `leptos-app` — `trunk build --release`, размер `.wasm` уходит в таблицу производительности.
- Tauri-приложения вынесены из основного workspace: `generate_context!` требует собранный фронт, и `cargo test --workspace` не должен от него зависеть.
- Плейграунды используют только публичный API. Если что-то неудобно сделать в плейграунде, это сигнал поправить API, а не обходить его.

## 8b. Дополнительные механизмы

Всё, что всплыло при разборе упущенного. Колонка «Этап» ссылается на [PLAN.md](PLAN.md); M11–M13 идут после v1.

| # | Механизм | Этап |
| --- | --- | --- |
| 1 | `i18n.json`: единый конфиг локалей и форматов | M1, M7 |
| 2 | Проверка имён страниц и локалей с фронта | M1, M7 |
| 3 | Запоминание выбранной локали | M7 |
| 4 | Переводы из библиотечных крейтов (`extend`) | M3 |
| 5 | Перезагрузка на лету в dev (`watch`) | M3, M7 |
| 6 | Типобезопасные ключи: кодоген, макросы, TS-типы фронта | M8 (TS), M11 (кодоген), M13 (макросы) |
| 7 | Отчёт о недостающих ключах; источник для devtools | M7, M13 |
| 8 | Проверка каталога: `check()` в Rust и CLI без Nuxt | M3; CLI — предложение в nuxt-i18n-micro |
| 9 | Контроль публичного API | M0 |
| 10 | Аудит зависимостей | M0 |
| 11 | Fuzzing | M3, M5 |
| 12 | Синхронизация с `@i18n-micro/*` и Node | M0 |
| 13 | Релиз в связке | M0 |
| 14 | Названия языков из CLDR | M5 |
| 15 | `ListFormat`, ordinal, разбор чисел | M12 |
| 16 | Вытеснение страниц из памяти (Rust и фронт) | M3, M8 |
| 17 | `tracing` | M3 |
| 18 | Русская версия docs | M13 |
| 19 | Плейграунд «Nuxt-сайт + Tauri на одних файлах» | M10 |
| 20 | `connect` с `@i18n-micro/react` / `preact` / `solid` | M8 |

### 8b.1 `i18n.json`: один конфиг на обе стороны

Сейчас локали описываются вызовами builder'а, а `useI18n` из `@i18n-micro/vue` без `locales` и `defaultLocale` падает. Решение — один файл рядом с каталогом, **не внутри него**: загрузчик i18n-micro считает любой `*.json` в корне каталога локалью.

```jsonc
// i18n.json
{
  "defaultLocale": "en",
  "fallbackLocale": "en",
  "locales": [                                    // Locale[] из @i18n-micro/types
    { "code": "en", "iso": "en-US", "displayName": "English" },
    { "code": "de-at", "iso": "de-AT", "fallbackLocale": "de" },
    { "code": "ar", "iso": "ar", "dir": "rtl" },
    { "code": "es", "disabled": true }
  ],
  "numberFormats": { "en": { "currency": { "style": "currency", "currency": "USD" } } },
  "datetimeFormats": { "en": { "short": { "dateStyle": "short" } } }
}
```

- Rust: `.config("i18n.json")` или `.config_json(include_str!(…))`. Отдельные вызовы builder'а (`iso`, `fallback_for`, `formats`) работают и переопределяют файл.
- `disabled` скрывает локаль из `available_locales` и запрещает `set_locale`. `dir` уходит фронту. `displayName` по умолчанию берётся из CLDR (8b.14).
- Плагин кладёт `locales`, `defaultLocale` и форматы в payload `load`. `createTauriI18n` передаёт их в `createI18n` из `@i18n-micro/vue` (опции `locales`, `defaultLocale`) и сам ставит `<html lang dir>` при каждой смене локали. `connect` делает то же для runtime-инстанса.
- Тот же файл можно скормить Nuxt-модулю (`locales`, `defaultLocale`), поэтому сайт и приложение описывают локали один раз.

### 8b.2 Имена с фронта — только из известного списка

`load_page`, `load`, `set_locale`, `ensure_async` получают строки извне.
- Страница и локаль принимаются, только если есть в каталоге (из файлов, манифеста `remote` или `i18n.json`). Иначе — `Error::UnknownPage` / `UnknownLocale`, без обращения к файлам.
- Путь к файлу собирается только из известных имён, пользовательская строка никогда не склеивается в путь. Значит, `../`, абсолютные пути и `\0` невозможны по построению, а не по фильтру.
- `load` отдаёт не больше одной цепочки локалей за вызов.
- Default-permission плагина: `load`, `load_page`, `get_locale`, `available_locales`, `pages`, `set_locale`. `report_missing` (8b.7) только в dev-сборке.
- Проверка: story-тест с `../../etc/passwd`, `CON`, пустой строкой, юникодом и `__proto__`; fuzz-цель `page_names` (8b.11); действие `BadName` в `ipc_model`.

### 8b.3 Запоминание локали

- Плагин: `.persist(true)` (по умолчанию включено). Пишется `app_config_dir/i18n-next.json` `{ "locale": "de" }` при каждом `set_locale`, атомарно (временный файл + rename).
- Порядок выбора при старте:
  1. сохранённая, если она ещё есть и не `disabled`;
  2. системная (в Tauri — `navigator.languages` WebView, вне Tauri — `LC_ALL` / `LC_MESSAGES` / `LANG`) через `negotiate` (`de-AT` → `de-at` → `de`);
  3. `defaultLocale`.
- Браузер: рецепт с `localStorage` в `leptos-app`. В Tauri с Rust-фронтом хранит бэкенд, гостю ничего не нужно.
- Проверка: story «перезапуск»: два последовательных `App` на одном временном каталоге конфига.

### 8b.4 Переводы из библиотечных крейтов

Аналог `additionalTranslationDirs`: библиотека поставляет свои файлы, приложение их сливает.

```rust
// в библиотеке
pub static LOCALES: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/locales");
pub fn locales() -> impl Iterator<Item = (&'static str, &'static [u8])> { /* обход LOCALES */ }

// в приложении
I18n::builder().extend(my_lib::locales()).extend(other_lib::locales()).dir("locales").install()?;
```

- Порядок слоёв: `extend` в порядке вызовов, потом собственные файлы приложения. Позже добавленное побеждает, поэтому приложение может переопределить любую строку библиотеки. Слияние глубокое, на каждую пару `(locale, page)`.
- Рекомендация для библиотек — свой верхний ключ (`"my_lib": { … }`), чтобы не пересекаться.
- Слитый чанк сериализуется для фронта один раз при загрузке.
- Проверка: действие `Extend` в `catalog_model`, голдены порядка слоёв.

### 8b.5 Перезагрузка на лету в dev

- Фича `watch` (native, без зависимостей: опрос `mtime` в отдельном потоке). Работает только для источников `dir` / `lazy_dir` и только в `cfg(debug_assertions)`; в release вызов — no-op.
- `I18n::watch()` возвращает guard. Изменение файла запускает `reload` затронутых чанков, растёт `version`, и все подписчики узнают об этом.
- В Tauri плагин включает его сам в dev. Событие `changed` заставляет guest-js перезапросить root и текущую страницу, а `bind` перерисовывает меню и трей.
- Тестируется без реальной файловой системы и времени: источник событий абстрагирован каналом путей, тест кладёт синтетические события. Склейка повторных событий проверяется на виртуальных часах.

### 8b.6 Типобезопасные ключи

- **Фронт (v1):** твой `@i18n-micro/types-generator` уже работает как Vite-плагин (`translationDir`). В `tauri-app` он подключён, рецепт есть в доках. Свой код не пишем.
- **Rust-кодоген (M11):** `i18n_next::build::keys("locales", out_dir)` вызывается из `build.rs` приложения (крейт как build-dependency, отдельного крейта нет):

  ```rust
  // сгенерировано
  pub mod nav { pub const HOME: Key = Key::new("nav.home"); }      // doc: "Home" (en), параметры: —
  pub mod pages { pub mod settings { pub const TITLE: PageKey = PageKey::new("settings", "title"); } }
  ```

  Опечатка в ключе становится ошибкой компиляции. В doc-комментарии — текст дефолтной локали и список `{параметров}`.
- **Макросы с проверкой (M13):** `i18n-next-macros`, `t!` проверяет литеральный ключ по файлам при компиляции. Только если кодогена окажется мало.

### 8b.7 Отчёт о недостающих ключах

- dev-сборка: плагин собирает промахи Rust (`missing`) и JS. guest-js ставит `missingHandler` инстанса и пачкой отправляет промахи командой `report_missing`.
- Дедупликация по `(locale, page, key)` со счётчиком. Результат — файл `app_data_dir/i18n-missing.json` и событие `i18n-next://missing`. inspector-панель `tauri-app` показывает список.
- В release выключено: команда не регистрируется, handler не ставится.
- M13: отдавать эти данные (и каталог) в `@i18n-micro/devtools-ui` как источник.

### 8b.8 Проверка каталога

- **Rust, `I18n::check() -> CheckReport` (M3):**
  - ключи, которых нет в какой-то локали относительно `defaultLocale`;
  - расхождение `{параметров}` между локалями одного ключа;
  - число форм плюрала против CLDR-категорий локали;
  - битые linked;
  - страницы без файла в части локалей.

  Пользователь пишет в своих тестах `assert!(i18n.check().is_clean())`. Тур `playground-rust --check` использует то же самое.
- **CLI:** `@i18n-micro/cli` уже умеет `check`, `lint`, `translate`, `export`/`import` (PO, CSV), `clean`, `rename`, но запускается только внутри Nuxt-проекта. Предложение для nuxt-i18n-micro: режим без Nuxt (`--dir locales --config i18n.json`). Это правка в том репозитории, по решению владельца, отдельной задачей. До этого `check()` закрывает CI-гейт на стороне Rust.

### 8b.9 Публичный API

- `cargo public-api` снимает API обоих крейтов в `api/<crate>.txt` (голден, nightly только для этого джоба). Diff в PR показывает каждое изменение API.
- `cargo-semver-checks` сверяет с последней опубликованной версией и падает на ломающем изменении без major-бампа.
- `bench_inventory` берёт список функций из этого снимка, а не регэкспом по исходникам.
- guest-js: `attw` + `publint` (уже есть); снимок `.d.ts` в голдене.

### 8b.10 Зависимости

`cargo-deny` (`deny.toml`: лицензии MIT/Apache/BSD/Unicode, advisories, запрет дублей версий в основном крейте) и `cargo-machete` (неиспользуемые зависимости) в CI. `pnpm audit` для guest-js.

### 8b.11 Fuzzing

`fuzz/` (cargo-fuzz, nightly), локально, как mutants; прогон обязателен перед релизом. Цели:

| Цель | Что проверяет |
| --- | --- |
| `load_json` | любые байты в загрузчике: ошибка или каталог, без паники |
| `template` | токенизатор и подстановка против наивной модели |
| `plural_rules` | сгенерированные правила против табличного эталона из CLDR |
| `intl_options` | разбор опций форматов |
| `page_names` | имена с фронта (8b.2) |
| `payload_apply` | `payload::apply` на произвольных байтах |

Корпус собирается из `fixtures/` и голденов. Найденный крэш становится сценарием в модели.

### 8b.12 Синхронизация с JS-библиотекой и Node

- Renovate (`renovate.json`) следит за `@i18n-micro/*` и `cldr-*` в `scripts/package.json` и за версией Node в CI.
- В PR с обновлением compat-джоб перегенерирует голдены и падает, если поведение разошлось. Diff голденов показывает, что именно поменялось в JS. Владелец решает: догнать в Rust или зафиксировать отличие.
- Обновление Node меняет ICU/CLDR. Тот же путь, плюс перегенерация `data/cldr`.

### 8b.13 Релиз в связке

- Одна версия у `i18n-next`, `tauri-plugin-i18n-next` и `tauri-plugin-i18n-next-api`: `[workspace.package] version` и `package.json`.
- `.versionrc.json` с postbump, как в centrifugo; один `CHANGELOG.md`.
- Тест `version_sync` падает, если версии разошлись.
- Плагин зависит от `i18n-next = "=x.y.z"` той же версии.
- Порядок выпуска: зелёный CI → тег без `v` → `gh release create` → `cargo publish --workspace` → `pnpm publish` без `--otp`.

### 8b.14 Названия языков

- Из CLDR `localeDisplayNames` берётся автоним каждой локали («Deutsch», «Русский», «العربية»): несколько байт на локаль, в `data/cldr`.
- `i18n.language_name("de")` → «Deutsch». В `available_locales` поле `displayName` берётся из `i18n.json`, иначе автоним.
- Название на текущем языке («немецкий») — это N² данных, поэтому только в M12, по фиче.

### 8b.15 `ListFormat`, ordinal, разбор чисел (M12)

JS core этого не умеет, поэтому это Rust-only API. В доках помечено как «без пары на фронте» (фронт может использовать `Intl` напрямую):
- `tl(&["a", "b", "c"])` → «a, b и c» (CLDR `listPatterns`: and / or / unit);
- `tc_ordinal("place", 2)` → форма по ordinal-категориям (CLDR `ordinals.json`, тот же генератор правил);
- `parse_number("1 234,5", "ru")` → `1234.5` по символам локали.

### 8b.16 Вытеснение страниц (v1)

Долго работающее приложение с сотнями страниц не должно держать в памяти все загруженные page-чанки.

**Rust: `.evict_pages(n)`**
- По умолчанию выключено (`n` не ограничен). Значение по умолчанию выберет бенч памяти на 1000 страниц.
- Вытесняются только page-чанки, у которых есть источник для повторной загрузки:
  - `dir` / `lazy_dir` — повторное чтение файла;
  - `files()` — повторный разбор встроенных байтов, без IO;
  - `remote` — помечается «нужно загрузить», следующий `ensure_async` загрузит снова.
- Никогда не вытесняются:
  - root-чанки;
  - чанки из `messages()` / `add_messages()`, потому что их не из чего восстановить;
  - страницы, закреплённые через `.pin_page("tray")` (Rust-only страницы нативного UI).
- LRU без цены на горячем пути. У чанка есть `AtomicU64` с отметкой последнего обращения, обращение — одна relaxed-запись глобального счётчика, без блокировок. При загрузке новой страницы сверх `n` строится снимок без страницы с самой старой отметкой.
- Читатели со старым `Arc<Snapshot>` не замечают вытеснения: их снимок цел.
- Обращение к вытесненной странице из `dir` / `files` загружает её снова синхронно, ответ не меняется. Для `remote` до `ensure_async` отвечает root и fallback, как для ещё не загруженной страницы. Guest-фронт и так вызывает `ensure_async` при навигации.
- Событие `Change` не отправляется: тексты не изменились.

**Фронт: `connect(i18n, { maxPages })`**
- guest-js держит такой же LRU по `(locale, page)` и удаляет вытесненные чанки из хранилища инстанса (`storage.translations.delete('locale:page')`). `storage` открыт и у runtime `I18n`, и у `VueI18n`.
- Текущая страница и страницы из `preload` не вытесняются. Повторный заход — снова `load_page`; кеш версий это учитывает.

**Проверка**
- действие `Evict` / `maxPages` в `loading_model` с инвариантами:
  - загружено не больше `n` вытесняемых чанков;
  - любой ответ `t` совпадает с моделью без вытеснения, то есть вытеснение невидимо;
  - каждый повторный подъём учтён в счётчике загрузок;
- то же в `connect.model.test.ts`;
- бенчи: горячий путь с включённым и выключенным вытеснением (разница — одна relaxed-запись); память и время на 1000 страниц.

### 8b.17 `tracing`

Фича `tracing`:
- промахи → `warn!` с `locale`, `page`, `key`;
- загрузки и `reload` → `debug!`-span с временем и размером;
- `ensure_async` → span на файл.

Без фичи промахи идут в `missing` handler, а в debug — одно предупреждение в stderr.

### 8b.18 Русская версия docs (M13)

Сайт сначала на английском. В M13 — `content/ru/…` отдельной коллекцией `@nuxt/content` с переключателем языка в шапке. Структура страниц одинаковая, сгенерированные разделы делаются для обоих языков.

### 8b.19 Плейграунд «Nuxt-сайт + Tauri на одних файлах» (M10)

`examples/nuxt-site`: `nuxt-i18n-micro` читает `examples/locales` и `examples/i18n.json`. Те же страницы `index` / `settings` / `about`, что в `tauri-app`. CI делает `nuxi generate` и сверяет тексты нескольких страниц с выводом тура `playground-rust` — живая проверка совместимости сверх голденов.

### 8b.20 Другие фронт-фреймворки

`I18nProvider` из `@i18n-micro/react` строит runtime-инстанс, поэтому `connect` по утиному интерфейсу должен работать без изменений. В M8 добавляются vitest-тесты `connect` с настоящими `@i18n-micro/react`, `preact` и `solid` и рецепты в `13.frontend.md`. Если адаптер чего-то не даёт, выбор такой: подпуть в guest-js или предложение в nuxt-i18n-micro.

## 9. Структура репозитория

```
Cargo.toml                         workspace, rust-version = 1.90, общие версии
clippy.toml                        msrv = "1.90"
crates/
  i18n-next/
    Cargo.toml  README.md  build.rs (склейка data/cldr по I18N_NEXT_LOCALES)
    data/cldr/<locale>.json        сгенерировано scripts/cldr.mjs
    src/  lib.rs  builder.rs  catalog.rs (Snapshot, Chunk, слои)  page.rs  tree.rs  template.rs
          plural/{mod,rules}.rs (rules.rs сгенерирован)  linked.rs  loader.rs  text.rs  macros.rs
          format/{mod,data,number,date,relative,skeleton,options}.rs  global.rs (install, scope, Scoped, key!)  error.rs
          payload.rs (encode / apply)  tauri_guest.rs (wasm)  config.rs (i18n.json)  check.rs  watch.rs
          json.rs (свой ридер и писатель)  hash.rs (свой хешер)  tz/{mod,civil,tzif,posix,android,browser,tzdb}.rs
          build.rs-API: build/keys.rs (кодоген ключей, M11)  names.rs (автонимы)
    tests/  support/{mod,locales,clock,fetch,executor,guard}.rs  compat.rs  catalog_model.rs  loading_model.rs
            text_model.rs  scope_model.rs  global_scope.rs  plural_props.rs  concurrency.rs  stories.rs
            __golden__/  wasm/{compat,browser,tauri_guest}.rs (wasm-bindgen-test)
    benches/  lookup.rs  text.rs  global.rs  format.rs  load.rs  payload.rs  concurrency.rs  (criterion)
              instructions.rs (gungraun)
    tests/alloc.rs (dhat)  tests/bench_inventory.rs
  tauri-plugin-i18n-next/
    Cargo.toml  README.md  build.rs  permissions/  windows-test-manifest.xml
    src/  lib.rs (Builder, расширение Manager)  commands.rs  protocol.rs (i18n-next://, чистая функция Request → Response)  bind.rs  persist.rs  missing.rs (dev)  models.rs  error.rs
    guest-js/  index.ts  vue.ts
    tests/  common/mod.rs  ipc_model.rs  stories.rs  guest-js/{connect.model,vue-router}.test.ts
    benches/ ipc.rs
    package.json  vite.config.ts  tsconfig*.json  vitest.config.ts  scripts/check-dist.mjs
benches/compare/                   publish = false: rust-i18n, fluent-bundle, ICU4X
fixtures/compat/                   locales/ (с pages/), cases.json, __golden__/*.md (call → result, пишет только JS)
fixtures/json-test-suite/          JSONTestSuite (y_ / n_ / i_) для своего JSON-ридера
fixtures/tzif/                     TZif v1 / v2 / v3 и контейнер Android tzdata
scripts/  compat.mjs  cldr.mjs  tzdb.mjs (компактная tzdb для фичи tzdb)     package.json скриптов: @i18n-micro/core, @i18n-micro/utils, cldr-*@48.0.0
examples/
  locales/                         общий каталог для всех плейграундов (root, pages/)
  i18n.json                        локали и форматы (8b.1) для всех плейграундов и Nuxt-сайта
  rust/                            playground-rust: консольный тур + bin/server.rs (axum, scope на запрос)
  tauri-app/                       Vue + router + плагин, desktop / Android / iOS, свой workspace
  leptos-app/                      Leptos CSR: браузер (trunk) и фронт Tauri (tauri-guest), свой workspace
  nuxt-site/                       nuxt-i18n-micro на тех же файлах (8b.19)
api/                               cargo public-api снимки (голден)
fuzz/                              cargo-fuzz цели (8b.11), локально
deny.toml  renovate.json
.config/nextest.toml               профили default / ci / mutants, жёсткие таймауты, без retries
docs/                              Nuxt 4 + @nuxt/ui + @nuxt/content, каркас из tauri-plugin-centrifugo/docs
scripts/bench-table.mjs            criterion JSON → docs/content/5.performance.md
.github/workflows/{ci,docs}.yml  .cargo/mutants.toml  CHANGELOG.md  .versionrc.json  pnpm-workspace.yaml
```

## 10. Не в v1

- custom protocol, `js_init_script`;
- проверка ключей при компиляции (`i18n-next-macros`) — M13, если кодогена (M11) окажется мало;
- локаль на окно;
- опции Intl из списка «нет» в 5.3;
- адаптер-крейты под UI-фреймворки (только рецепты).

## 11. Открытые решения (в документе, по умолчанию принято так)

1. `format`, `tz` и `global` включены по умолчанию.
2. Битый файл локали — ошибка.
3. Локали CLDR по умолчанию — все с покрытием `modern`, урезание через `I18N_NEXT_LOCALES`.
4. Версия CLDR = версия в Node CI (сейчас 48). Обновляется вместе с Node, одним коммитом с перегенерацией `data/cldr` и compat-голденов.
5. wasm-проверки в CI: сборка `wasm32-unknown-unknown` и `wasm32-wasip1`, `wasm-bindgen-test` в Node и headless Chrome.

## 12. Спайки: проверка спорных решений (2026-11-15)

Перед стартом спорные решения проверены маленькими Rust-проектами и одним настоящим Tauri-приложением (S15) (и Node-скриптами там, где проверялись данные и алгоритм, а не Rust). Машина — Apple Silicon, 10 ядер, Rust 1.100, release + thin LTO, Node 22.23 (ICU 78.3, CLDR 48). Спайки удалены. Ниже — что измерено и что решено.

| # | Вопрос | Результат | Решение |
| --- | --- | --- | --- |
| S1 | Чтение снимка: `RwLock<Arc>`, `arc-swap` или `thread_local!` + версия | 1 поток: RwLock + клон Arc 13.9 нс, arc-swap 10.7, thread_local 7.5. 8 потоков: RwLock + клон **3054 нс**, RwLock без клона 1713, arc-swap 22.3, thread_local **21.4** | `thread_local!` + `AtomicU64`-версия, без зависимости. `RwLock` на чтении исключён. Уточнение: кеш потока — маленький массив (4 записи по id инстанса), чтобы чередование двух `I18n` в одном потоке не сбрасывало кеш на каждом вызове |
| S2 | Хешер карты ключей (1k и 10k ключей вида `nav.form.item_123`) | std SipHash 12.9 / 13.3 нс, `rustc-hash` 6.5 / 7.0, `foldhash` **6.0 / 6.6**; промах 2.4 нс | `foldhash` — единственная новая зависимость горячего пути |
| S3 | Поиск параметра: линейный или хеш | линейный 2.7 нс (1 арг), 4.0 (3), 9.1 (5), 12.7 (8); хеш ≈4.6 нс при любом N, но требует построения | линейный поиск по встроенному массиву; хеш не нужен для реальных 1–4 параметров |
| S4 | Аллокации `Text` (dhat) | `Display` в буфер — **0**, `to_string` — 1, сборка `Text` с одним аргументом на `Vec` — **2** | аргументы во встроенном массиве на 4 элемента (без зависимостей), `Vec` только сверх; цель в `alloc.rs`: сборка ≤ 4 аргументов — 0 |
| S5 | Грамматика макросов и scope | работают: `t!(i => "k", name = x)`, `t!("k")`, `@locale = …`, `["a", "b"]`, ключ-переменная, вложенный `Text`, `#[error("{}", t!("k", p = .0.clone()))]`, `const Q: Key`, `Serialize` через `collect_str`; паника внутри `run` оставляет стек пустым; две задачи `wrap` вперемешку на одном потоке видят свои локали | грамматика из 4.4 подтверждена без правок; `Pop`-guard в `Drop` обязателен |
| S6 | Загрузка JSON (1k / 10k / 100k листьев) | `Value` + разворот 256 / 2295 / 27871 мкс; visitor сразу в плоскую карту **149 / 1322 / 16374**; копия сырых байтов для payload 0.8 / 8.5 / 122; пересериализация `Value` 159 / 1694 / 20377 | visitor (в 1.7 раза быстрее); байты файла хранятся как есть, пересериализация только для изменённых чанков |
| S7 | Числа: свой движок на CLDR 48 против `Intl.NumberFormat` | **1000 / 1000** совпадений: 10 локалей × 4 набора опций × 25 значений (1.005, 2.675, 0.0005, 1e21, 5e-324, −0, NaN, ±∞), минимальная группировка (`pl`, `es`), индийская (`hi`), арабские цифры и bidi-минус (`ar-EG`, `ar`) | алгоритм подтверждён: `Display` std (кратчайшее представление) + десятичное half-expand округление строки |
| S8 | Плюралы: генератор из `plurals.json` против `Intl.PluralRules` | **227 460 / 227 460** совпадений: 224 локали × (0..1000 + 21 дробь и большие числа). 40 уникальных функций, 16 КБ кода, 2.5 нс на категорию | генератор правил подтверждён. Найден баг генератора (`!` без скобок) — значит, генератор тоже покрывается тестом сравнения с `Intl` в CI |
| S9 | Даты: точный поиск скелета в `availableFormats` | совпадение скелета только у **220 из 1670** комбинаций опций (13%). Значит, нужен алгоритм ICU DateTimePatternGenerator — это сотни строк и главный риск расхождений | **не портируем DTPG.** Паттерны берутся у ICU в Node на этапе генерации (`formatToParts` на пробных датах) и кладутся в данные; отрисовка своя. Результат: **32 370 / 32 370** совпадений (10 локалей × 1079 комбинаций дата + время × 3 даты) |
| S10 | Подводные камни дат | (1) V8 в `format()` меняет U+202F на пробел, но в `formatToParts` оставляет; (2) ширина поля определяется по однозначным значениям (час 4, день 5, месяц 1); (3) контекст месяца (format или stand-alone) различим только на мае (`мая` / `май`); (4) `timeStyle: full / long` требует имён часовых поясов | эталон совместимости — `format()` V8 (Node, WebView2, Android WebView): U+202F → пробел в выводе дат. WKWebView и WebKitGTK могут отдавать U+202F: это в таблице совместимости. `timeStyle: full / long` и `timeZoneName` в v1 — ошибка при `build()` |
| S11 | Объём данных CLDR (нужные поля, минифицированный JSON) | в среднем на локаль: ядро (числа, даты, relative, символы валют) **26 КБ**, имена валют ещё **34 КБ**. Все 430 `modern`: **10.9 МБ** (25 МБ с именами валют); 100 локалей 2.5 МБ; 10 локалей 0.25 МБ | имена валют — отдельная фича `currency-names` (нужна только `currencyDisplay: name`). Региональные варианты хранятся как разница с родителем, строки интернируются. Цель по умолчанию — ≤ 1.5 МБ; не уложимся — набор по умолчанию сужается до ~40 самых частых языков, а `I18N_NEXT_LOCALES=all` даёт всё. Замер — в M5 |
| S12 | wasm (`wasm32-unknown-unknown`, `opt-level = "z"`, LTO, `wasm-opt -Oz`) | ядро (Text, scope, макросы, fmt): **74 КБ** (32 КБ gzip). С `jiff` (`js`, без tzdb): 558 КБ (162 КБ gzip). С встроенной tzdb: ~950 КБ до `wasm-opt`. `wasm32-wasip1` локально не установлен | в браузере фича `tz` по умолчанию идёт через `js-sys`: локальная зона — `Date.getTimezoneOffset`, именованные — `Intl.DateTimeFormat` с кешем по (зона, час). `jiff` — фича `tz-jiff` для тех, кому нужен чистый Rust везде. Native остаётся на `jiff`. CI ставит `wasm32-wasip1` сам |
| S13 | Tauri IPC (`mock_builder` + `get_ipc_response`) | raw `ipc::Response`: **2.0 мкс** (1k ключей, 47 КБ) и **11.5 мкс** (10k, 490 КБ); `serde_json::Value`: 134.6 и 1124.5 мкс (в 67–100 раз медленнее). Mock отдаёт raw как `InvokeResponseBody::Raw` — тестируемо. Команды Tauri должны жить в модуле, не в корне крейта (иначе конфликт имён макроса) | raw-ответ подтверждён; `commands.rs` как в шаблоне |
| S14 | Разбор payload в JS | `TextDecoder` + `JSON.parse`: 196 мкс (47 КБ) и 2103 мкс (490 КБ); сам `TextDecoder` — около 5% | узкое место — `JSON.parse`, а не транспорт. Это подтверждает раздельные слои: root один раз, страницы маленькими чанками, вытеснение страниц |

| S15 | Транспорт в настоящем WebView (macOS, WKWebView, Tauri 2.12, release) | 832 КБ: invoke raw + `JSON.parse` 3.05 мс, custom protocol `fetch().json()` 2.45, invoke `Value` 4.15, `Channel` 3.55, только `JSON.parse` 1.25; 8.5 МБ: 24.5 / 21.0 / 43.0 / 29.5 / 17.0. Голая передача 8.5 МБ ~3.5 мс | транспорт не узкое место; данные по custom protocol с `ETag` и версией в URL, IPC для управления и как запасной путь (6.1) |
| S16 | Базовый размер wasm без зависимостей | ядро 27.3 КБ после `wasm-opt -Oz`; std SipHash +1.4 КБ; `core::fmt` целых +0.6 КБ; `f64` `Display` +17 КБ. `#![forbid(unsafe_code)]` запрещает `#[unsafe(no_mangle)]`, но не мешает `#[wasm_bindgen]` | микроверсия и размеры (2.4); `f64`-форматирование только с фичей `format` |

Что это меняет в дизайне:
- 2.2: чтение через `thread_local!` подтверждено, аргументы `Text` во встроенном массиве. Хешер — свой (ревизия 9, 2.3), `foldhash` только в сравнении.
- 2.1 / 5.4: в браузере часовые пояса через `js-sys`. В ревизии 9 `jiff` убран совсем: свой модуль `tz`.
- 5.2: паттерны дат генерируются из ICU Node (`formatToParts`) вместе с данными CLDR; `currency-names` — отдельная фича; хранение с дедупликацией по родителю.
- 5.3: `timeStyle: full / long` и `timeZoneName` — ошибка при `build()` в v1.
- 6.2 / таблица совместимости: эталон — вывод `format()` V8; отличия WebKit по U+202F описаны.
- Ревизия 9 (S15, S16): custom protocol для данных; без runtime-зависимостей (2.3); микроверсия и CI-гейт размера (2.4).
