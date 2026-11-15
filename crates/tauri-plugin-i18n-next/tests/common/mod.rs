//! Shared helpers of the plugin scenario tests: a mock app that calls the plugin's
//! commands over IPC, the way guest-js does.
#![allow(dead_code)] // each test binary uses a part

pub use serde_json::{Value, json};

use tauri::test::MockRuntime;

/// A mock app with every plugin command and one webview.
pub struct App {
    pub w: tauri::WebviewWindow<MockRuntime>,
    pub _app: tauri::App<MockRuntime>,
}

impl App {
    pub fn new() -> Self {
        use tauri_plugin_i18n_next::commands::*;
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![ping])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        Self { w, _app: app }
    }

    /// Invokes a command as the JS API does; `Err` is the serialized error.
    pub fn call(&self, cmd: &str, args: Value) -> Result<Value, String> {
        let request = tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            // The app's own origin: tauri rejects app commands from others.
            url: if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(&self.w, request)
            .map(|r| r.deserialize::<Value>().unwrap())
            .map_err(|e| e.to_string())
    }

    pub fn ok(&self, cmd: &str, args: Value) -> Value {
        self.call(cmd, args)
            .unwrap_or_else(|e| panic!("{cmd}: {e}"))
    }
}
