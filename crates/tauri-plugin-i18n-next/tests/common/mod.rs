//! Shared helpers of the plugin scenario tests: a mock app with the plugin set up, its
//! commands callable over IPC the way guest-js calls them, protocol requests through the
//! same pure function the WebView reaches, and the events each window gets.
#![allow(dead_code)] // each test binary uses a part

pub use serde_json::{Value, json};

use i18n_next::I18n;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use tauri::Listener;
use tauri::test::MockRuntime;
use tauri_plugin_i18n_next::{Builder, Handle, ManagerExt};

/// The app's own origin, as the WebView sends it.
pub fn origin() -> &'static str {
    if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
}

/// A mock app with the plugin and two windows (`main`, `second`).
pub struct App {
    pub w: tauri::WebviewWindow<MockRuntime>,
    pub second: tauri::WebviewWindow<MockRuntime>,
    pub app: tauri::App<MockRuntime>,
}

/// A fresh directory for the plugin's files under cargo's per-test temp dir.
pub fn data_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("plugin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every command of the plugin, as an app handler.
fn commands() -> impl Fn(tauri::ipc::Invoke<MockRuntime>) -> bool + Send + Sync + 'static {
    use tauri_plugin_i18n_next::commands::*;
    tauri::generate_handler![
        load,
        load_page,
        meta,
        set_locale,
        get_locale,
        available_locales,
        pages,
        report_missing
    ]
}

/// The plugin over `i18n` for a test: no global install (tests share a process), no
/// persist, no real watch thread, its files under cargo's temp dir.
pub fn plugin(i18n: I18n) -> Builder {
    Builder::new(i18n)
        .global(false)
        .persist(false)
        .watch(false)
        .data_dir(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("plugin-default"))
}

impl App {
    pub fn new(i18n: I18n) -> Self {
        Self::with(plugin(i18n))
    }

    pub fn with(plugin: Builder) -> Self {
        Self::with_handler(plugin, |_| false)
    }

    /// The plugin's commands registered as app commands (by path, no capability needed),
    /// plus `extra` for test commands.
    pub fn with_handler(
        plugin: Builder,
        extra: impl Fn(tauri::ipc::Invoke<MockRuntime>) -> bool + Send + Sync + 'static,
    ) -> Self {
        let own = commands();
        let app = tauri::test::mock_builder()
            .plugin(plugin.build())
            .invoke_handler(move |invoke| {
                let cmd = invoke.message.command().to_owned();
                if cmd.starts_with("x_") {
                    extra(invoke)
                } else {
                    own(invoke)
                }
            })
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let second = tauri::WebviewWindowBuilder::new(&app, "second", Default::default())
            .build()
            .unwrap();
        Self { w, second, app }
    }

    pub fn handle(&self) -> &Handle {
        self.app.i18n()
    }

    /// Invokes a command as the JS API does; `Err` is the serialized error.
    pub fn call(&self, cmd: &str, args: Value) -> Result<Value, String> {
        self.call_raw(cmd, args)
            .map(|b| serde_json::from_slice(&b).unwrap_or_else(|_| panic!("{cmd}: not JSON")))
    }

    /// The response bytes, as the WebView gets them (raw for `load` / `load_page`).
    pub fn call_raw(&self, cmd: &str, args: Value) -> Result<Vec<u8>, String> {
        let request = tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            // The app's own origin: tauri rejects app commands from others.
            url: origin().parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(&self.w, request)
            .map(|r| match r {
                tauri::ipc::InvokeResponseBody::Json(s) => s.into_bytes(),
                tauri::ipc::InvokeResponseBody::Raw(b) => b,
            })
            .map_err(|e| e.to_string())
    }

    pub fn ok(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args)
            .unwrap_or_else(|e| panic!("{cmd}: {e}"))
    }

    /// `GET path` on the protocol from the app's origin: `(status, body)`.
    pub fn get(&self, path: &str) -> (u16, Vec<u8>) {
        let r = self.request(path, Some(origin()));
        (r.status().as_u16(), r.into_body())
    }

    pub fn request(&self, path: &str, origin: Option<&str>) -> tauri::http::Response<Vec<u8>> {
        self.send(
            tauri::http::Request::get(format!("i18n-next://localhost{path}")),
            origin,
        )
    }

    /// Sends a request built by the test (any method, URI, headers) from `origin`.
    pub fn send(
        &self,
        mut req: tauri::http::request::Builder,
        origin: Option<&str>,
    ) -> tauri::http::Response<Vec<u8>> {
        if let Some(o) = origin {
            req = req.header("origin", o);
        }
        let origins = [self::origin().to_owned()];
        tauri_plugin_i18n_next::protocol::handle(self.handle(), &origins, &req.body(()).unwrap())
    }

    /// The `meta` command's answer (tags of the chain of `locale` with `page`).
    pub fn meta(&self, locale: Option<&str>, page: Option<&str>) -> Value {
        let mut args = json!({});
        if let Some(l) = locale {
            args["locale"] = json!(l);
        }
        if let Some(p) = page {
            args["page"] = json!(p);
        }
        let bytes = self.call_raw("meta", args).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The current URL path of one chunk, as the front end builds it from `meta`.
    pub fn chunk_path(&self, locale: &str, page: &str) -> String {
        let (tag, _) = i18n_next::payload::chunk(self.handle(), locale, Some(page));
        format!("/{tag:016x}/{locale}/{page}.json")
    }

    /// The current URL path of a bundle.
    pub fn bundle_path(&self, locale: &str, page: Option<&str>) -> String {
        let meta = self.meta(Some(locale), page);
        let mut path = format!(
            "/{}/bundle?locale={locale}",
            meta["bundle"].as_str().unwrap()
        );
        if let Some(p) = page {
            path += &format!("&page={p}");
        }
        path
    }

    /// Events of `name` that `window` gets, as JSON, in order.
    pub fn events(
        &self,
        window: &tauri::WebviewWindow<MockRuntime>,
        name: &str,
    ) -> Receiver<Value> {
        let (tx, rx) = channel();
        window.listen(name, move |e| {
            let _ = tx.send(serde_json::from_str(e.payload()).unwrap());
        });
        rx
    }
}

/// Everything that arrived so far (with no other thread delivering, events are sent by the
/// writing thread before its call returns, so after the call that caused them they are
/// all in).
pub fn drain(rx: &Receiver<Value>) -> Vec<Value> {
    rx.try_iter().collect()
}
