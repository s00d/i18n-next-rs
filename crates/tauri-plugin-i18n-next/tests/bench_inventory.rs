//! Rule (DESIGN 7.7, PLAN): every public function of the plugin has a criterion bench.
//! Shares its logic with the core crate's inventory through `#[path]`, so this file
//! reads outside the plugin directory and is left out of the published package
//! (`exclude` in Cargo.toml).

#[path = "../../i18n-next/tests/support/inventory.rs"]
mod inventory;

#[test]
fn every_public_function_has_a_bench() {
    inventory::check(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
}
