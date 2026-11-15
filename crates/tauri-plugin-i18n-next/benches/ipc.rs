//! criterion benches of the plugin (DESIGN 7.7): every public function has a bench named
//! `group/function/case` (`tests/bench_inventory.rs` checks it). IPC goes through the
//! same mock path as the stories (`get_ipc_response`), the way guest-js calls it.

#[path = "../tests/common/mod.rs"]
mod common;

use common::{App, json};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use tauri::test::MockRuntime;

fn ping(c: &mut Criterion) {
    let app = App::new();
    c.bench_function("ipc/ping/mock_ipc", |b| {
        b.iter(|| black_box(app.ok("ping", json!({}))))
    });
}

fn init(c: &mut Criterion) {
    c.bench_function("plugin/init/build", |b| {
        b.iter(|| black_box(tauri_plugin_i18n_next::init::<MockRuntime>()))
    });
}

criterion_group!(benches, ping, init);
criterion_main!(benches);
