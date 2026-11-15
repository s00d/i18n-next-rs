//! wasm-bindgen-test (DESIGN 7.6): runs in Node and headless Chrome via
//! `wasm-bindgen-test-runner` (`cargo test --target wasm32-unknown-unknown`). The compat
//! goldens run here on compiled-in fixtures through `files()`, as an app in the browser
//! would load them, and so do the platform stories shared with `tests/wasi.rs`
//! (`support/platform.rs`, every one but the `dir` story). Native builds compile nothing.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

#[path = "support/compat.rs"]
mod compat;
#[path = "support/platform.rs"]
mod platform;

use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn version_in_wasm() {
    assert_eq!(i18n_next::version(), env!("CARGO_PKG_VERSION"));
}

/// Given the compat fixtures embedded in the binary, when the goldens are replayed in
/// wasm (the plural ones too: `tc` with the generated CLDR rules), then every line
/// matches, as on native.
#[wasm_bindgen_test]
fn compat_goldens_in_wasm() {
    if let Err(diff) = compat::run() {
        panic!("compat goldens differ:\n{diff}");
    }
}

#[wasm_bindgen_test]
fn files_and_switching() {
    platform::files_and_switching();
}

#[wasm_bindgen_test]
fn files_and_messages() {
    platform::files_and_messages();
}

#[wasm_bindgen_test]
fn chains_aliases_and_folding() {
    platform::chains_aliases_and_folding();
}

#[wasm_bindgen_test]
fn many_holes_beside_a_huge_argument() {
    platform::many_holes_beside_a_huge_argument();
}

const MANIFEST: [&str; 3] = ["en.json", "pages/cart/en.json", "pages/cart/de.json"];

fn body(path: &str) -> &'static str {
    match path {
        "en.json" => r#"{"hi": "Hi"}"#,
        "pages/cart/en.json" => r#"{"title": "Cart"}"#,
        _ => r#"{"title": "Warenkorb"}"#,
    }
}

/// Given a remote manifest, when `ensure_async` runs on the browser's executor
/// (`wasm-bindgen-futures`) with an in-memory fetch, then the chain's files are in.
#[wasm_bindgen_test]
async fn ensure_async_in_memory() {
    let i18n = i18n_next::I18n::builder()
        .locale("de")
        .fallback("en")
        .remote(MANIFEST)
        .build()
        .unwrap();
    let fetch = |path: String| async move { Ok::<_, String>(body(&path).as_bytes().to_vec()) };
    i18n.ensure_async("de", Some("cart"), fetch).await.unwrap();
    assert_eq!(i18n.page("cart").t("title"), "Warenkorb");
    assert_eq!(i18n.t("hi"), "Hi");
    assert!(i18n.missing_files("de", Some("cart")).is_empty());
}

/// The platform's own `fetch` (the global, no web-sys) of a `data:` URL: the real fetch
/// path in Node and Chrome without a server, since `wasm-bindgen-test-runner` serves no
/// fixture files.
async fn real_fetch(path: String) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    async fn go(path: &str) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
        let url = format!(
            "data:application/json,{}",
            String::from(js_sys::encode_uri_component(body(path)))
        );
        let global = js_sys::global();
        let fetch: js_sys::Function = js_sys::Reflect::get(&global, &"fetch".into())?.dyn_into()?;
        let response =
            JsFuture::from(js_sys::Promise::from(fetch.call1(&global, &url.into())?)).await?;
        let read: js_sys::Function =
            js_sys::Reflect::get(&response, &"arrayBuffer".into())?.dyn_into()?;
        let buffer = JsFuture::from(js_sys::Promise::from(read.call0(&response)?)).await?;
        Ok(js_sys::Uint8Array::new(&buffer).to_vec())
    }
    go(&path).await.map_err(|e| format!("{e:?}"))
}

/// Given a remote manifest, when `ensure_async` fetches with the real `fetch`, then the
/// files are in; a second call fetches nothing.
#[wasm_bindgen_test]
async fn ensure_async_with_a_real_fetch() {
    let i18n = i18n_next::I18n::builder()
        .locale("de")
        .fallback("en")
        .remote(MANIFEST)
        .build()
        .unwrap();
    i18n.ensure_async("de", Some("cart"), real_fetch)
        .await
        .unwrap();
    assert_eq!(i18n.page("cart").t("title"), "Warenkorb");
    assert_eq!(i18n.t("hi"), "Hi");
    let version = i18n.version();
    i18n.ensure_async("de", Some("cart"), real_fetch)
        .await
        .unwrap();
    assert_eq!(i18n.version(), version);
}

/// `lazy_files`, `extend`, `subscribe` and `reload` in the browser: the same code path.
#[wasm_bindgen_test]
fn lazy_extend_subscribe_reload() {
    use std::sync::{Arc, Mutex};
    let i18n = i18n_next::I18n::builder()
        .extend([("en.json", br#"{"lib": "L", "a": "lib"}"#.as_slice())])
        .lazy_files([("en.json", br#"{"a": "app"}"#.as_slice())])
        .build()
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let _sub = i18n.subscribe(move |c| log.lock().unwrap().push(c.version));
    assert_eq!((i18n.t("a"), i18n.t("lib")), ("app".into(), "L".into()));
    i18n.reload().unwrap();
    assert_eq!(*seen.lock().unwrap(), [1]);
}

/// One turn of the JS event loop (a resolved promise): the browser executor runs others.
async fn tick() {
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL))
        .await
        .unwrap();
}

/// Given two requests in different locales wrapped in scopes, when `wasm-bindgen-futures`
/// runs them interleaved (each awaits the event loop between its texts), then each sees its
/// own locale across its awaits, and nothing is left in scope between polls.
#[wasm_bindgen_test]
async fn scope_wrap_on_the_browser_executor() {
    use i18n_next::{scope, t};
    use std::cell::RefCell;
    use std::rc::Rc;
    let i18n = i18n_next::I18n::builder()
        .messages("en", None, r#"{"hi": "Hi", "bye": "Bye"}"#)
        .messages("de", None, r#"{"hi": "Hallo", "bye": "Tschüss"}"#)
        .build()
        .unwrap();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let request = |lang: &str| {
        let log = log.clone();
        scope()
            .instance(i18n.clone())
            .locale(lang)
            .wrap(async move {
                log.borrow_mut().push(t!("hi").to_string());
                tick().await;
                log.borrow_mut().push(t!("bye").to_string());
            })
    };
    let done = Rc::new(RefCell::new(false));
    let flag = done.clone();
    let b = request("de");
    wasm_bindgen_futures::spawn_local(async move {
        b.await;
        *flag.borrow_mut() = true;
    });
    request("en").await;
    for _ in 0..8 {
        if *done.borrow() {
            break;
        }
        tick().await;
    }
    assert!(*done.borrow());
    let mut seen = log.borrow().clone();
    seen.sort();
    assert_eq!(seen, ["Bye", "Hallo", "Hi", "Tschüss"]);
    assert_eq!(t!("hi"), "hi");
}
