//! criterion benches of the plugin (DESIGN 7.7): every public function has a bench named
//! `group/function/case` (`tests/bench_inventory.rs` checks it). IPC goes through the
//! same mock path as the stories (`get_ipc_response`), the way guest-js calls it; the
//! protocol through its pure handler, as the WebView reaches it. `compare/*` measures the
//! rejected `serde_json::Value` answer of the same payload (S13).

#[path = "../tests/common/mod.rs"]
mod common;

use common::{App, json, origin, plugin};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use i18n_next::I18n;
use std::hint::black_box;
use std::sync::Arc;
use tauri::test::MockRuntime;
use tauri_plugin_i18n_next::{Builder, ManagerExt, Relabel};

/// A locale file with `keys` leaves, ten per section.
fn doc(keys: usize, word: &str) -> String {
    let mut s = String::from("{");
    for k in 0..keys {
        if k % 10 == 0 {
            if k > 0 {
                s.push_str("},");
            }
            s.push_str(&format!("\"s{}\":{{", k / 10));
        } else {
            s.push(',');
        }
        s.push_str(&format!("\"k{}\":\"{word} {k}\"", k % 10));
    }
    if keys > 0 {
        s.push('}');
    }
    s + "}"
}

/// `en` and `de` with `keys` root keys each and a 50-key `settings` page.
fn catalog(keys: usize) -> I18n {
    I18n::builder()
        .messages("en", None, &doc(keys, "en"))
        .messages("de", None, &doc(keys, "de"))
        .messages("en", Some("settings"), &doc(50, "Settings"))
        .messages("de", Some("settings"), &doc(50, "Einstellungen"))
        .build()
        .unwrap()
}

/// Counts labels without touching a window.
#[derive(Clone)]
struct Sink;

impl Relabel for Sink {
    fn relabel(&self, label: &str) -> tauri::Result<()> {
        black_box(label);
        Ok(())
    }
}

fn ipc(c: &mut Criterion) {
    let mut load = c.benchmark_group("ipc/load");
    let apps: Vec<(usize, App)> = [1_000, 10_000]
        .into_iter()
        .map(|keys| (keys, App::new(catalog(keys))))
        .collect();
    for (keys, app) in &apps {
        load.bench_function(format!("root_{keys}_meta"), |b| {
            b.iter(|| black_box(app.call_raw("load", json!({})).unwrap()))
        });
    }
    load.finish();
    for (keys, app) in &apps {
        let (_, bytes) = i18n_next::payload::encode(app.handle(), None, None, true);
        eprintln!("load payload, {keys} keys: {} bytes", bytes.len());
        c.bench_function(&format!("compare/value_json/root_{keys}_meta"), |b| {
            b.iter(|| black_box(app.call("load", json!({})).unwrap()))
        });
    }
    let app = App::new(catalog(1_000));
    c.bench_function("ipc/load_page/page_50", |b| {
        b.iter(|| {
            black_box(
                app.call_raw("load_page", json!({ "page": "settings" }))
                    .unwrap(),
            )
        })
    });
    let mut flip = false;
    c.bench_function("ipc/set_locale/switch", |b| {
        b.iter(|| {
            flip = !flip;
            let locale = if flip { "de" } else { "en" };
            black_box(app.call("set_locale", json!({ "locale": locale })).unwrap())
        })
    });
    c.bench_function("ipc/get_locale/mock_ipc", |b| {
        b.iter(|| black_box(app.call("get_locale", json!({})).unwrap()))
    });
    c.bench_function("ipc/available_locales/two", |b| {
        b.iter(|| black_box(app.call("available_locales", json!({})).unwrap()))
    });
    c.bench_function("ipc/pages/one", |b| {
        b.iter(|| black_box(app.call("pages", json!({})).unwrap()))
    });
    c.bench_function("ipc/meta/chain_page", |b| {
        b.iter(|| black_box(app.call_raw("meta", json!({ "page": "settings" })).unwrap()))
    });
    // The dev report records nothing in release builds (benches run in release): these two
    // measure on purpose what an app pays for leaving the handler and the command in a
    // release build, the IPC round trip and a no-op. The dev path is covered by
    // tests/dev_report.rs and the unit test of src/missing.rs (one write per batch).
    let keys = json!({ "keys": [{ "locale": "en", "page": "index", "key": "nope" }] });
    c.bench_function("ipc/report_missing/release_noop", |b| {
        b.iter(|| black_box(app.call("report_missing", keys.clone()).unwrap()))
    });
    c.bench_function("plugin/missing/release_noop", |b| {
        b.iter(|| tauri_plugin_i18n_next::missing(black_box("en"), "nope", "index"))
    });
}

fn protocol(c: &mut Criterion) {
    let app = App::new(catalog(1_000));
    let origins = [origin().to_owned()];
    let req = |path: &str| {
        tauri::http::Request::get(format!("i18n-next://localhost{path}"))
            .header("origin", origin())
            .body(())
            .unwrap()
    };
    let h = app.handle();
    let root = app.chunk_path("en", "index");
    let page = app.chunk_path("de", "settings");
    let bundle = app.bundle_path("de", Some("settings"));
    // what `convertFileSrc` sends: the whole path percent-encoded
    let encoded = root.replace('/', "%2F").replacen("%2F", "/", 1);
    let mut g = c.benchmark_group("protocol/handle");
    for (case, path) in [
        ("chunk_root_1000", root.clone()),
        ("chunk_root_encoded", encoded),
        ("chunk_page_50", page),
        ("bundle_1000x2", bundle),
        ("stale_tag_410", format!("/0000000000000000{}", &root[17..])),
        ("unknown_locale", format!("{}/xx/index.json", &root[..17])),
    ] {
        let r = req(&path);
        g.bench_function(case, |b| {
            b.iter(|| black_box(tauri_plugin_i18n_next::protocol::handle(h, &origins, &r)))
        });
    }
    g.finish();
}

fn plugin_api(c: &mut Criterion) {
    let i18n = catalog(10);
    c.bench_function("builder/new/catalog", |b| {
        b.iter(|| black_box(Builder::new(i18n.clone())))
    });
    c.bench_function("builder/system_locale/off", |b| {
        b.iter(|| black_box(Builder::new(i18n.clone()).system_locale(false)))
    });
    c.bench_function("builder/persist/off", |b| {
        b.iter(|| black_box(Builder::new(i18n.clone()).persist(false)))
    });
    c.bench_function("builder/global/off", |b| {
        b.iter(|| black_box(Builder::new(i18n.clone()).global(false)))
    });
    c.bench_function("builder/watch/off", |b| {
        b.iter(|| black_box(Builder::new(i18n.clone()).watch(false)))
    });
    c.bench_function("builder/data_dir/path", |b| {
        b.iter(|| black_box(Builder::new(i18n.clone()).data_dir("/tmp/x")))
    });
    c.bench_function("builder/origins/one", |b| {
        b.iter(|| black_box(Builder::new(i18n.clone()).origins(["http://localhost:1430"])))
    });
    c.bench_function("builder/build/mock", |b| {
        b.iter(|| black_box(plugin(i18n.clone()).build::<MockRuntime>()))
    });
    let app = App::new(catalog(10));
    c.bench_function("manager_ext/i18n/app", |b| {
        b.iter(|| black_box(app.app.i18n().version()))
    });
    let h = app.handle();
    c.bench_function("handle/origins/default", |b| {
        b.iter(|| black_box(black_box(h).origins().len()))
    });
    // each bound label is unbound outside the timing: the list stays at one entry
    c.bench_function("handle/bind/label", |b| {
        b.iter_custom(|iters| {
            let mut total = std::time::Duration::ZERO;
            for _ in 0..iters {
                let text = h.text("s0.k1");
                let start = std::time::Instant::now();
                let id = black_box(h.bind(&Sink, text));
                total += start.elapsed();
                h.unbind(id);
            }
            total
        })
    });
    c.bench_function("handle/unbind/bound", |b| {
        b.iter_batched(
            || h.bind(&Sink, h.text("s0.k1")),
            |id| black_box(h.unbind(id)),
            BatchSize::SmallInput,
        )
    });
    let window: Arc<dyn Relabel> = Arc::new(app.w.clone());
    c.bench_function("relabel/relabel/webview_window", |b| {
        b.iter(|| black_box(window.relabel(black_box("Title"))))
    });
    c.bench_function("relabel/window/webview_window", |b| {
        b.iter(|| black_box(window.window()))
    });
}

criterion_group!(benches, ipc, protocol, plugin_api);
criterion_main!(benches);
