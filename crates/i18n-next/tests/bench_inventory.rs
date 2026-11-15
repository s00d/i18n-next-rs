//! Rule (DESIGN 7.7, PLAN): every public function of the crate has a criterion bench.
//! The logic lives in `support/inventory.rs`, shared with the plugin's inventory.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/inventory.rs"]
mod inventory;

#[test]
fn every_public_function_has_a_bench() {
    inventory::check(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        env!("CARGO_PKG_NAME"),
    );
}
