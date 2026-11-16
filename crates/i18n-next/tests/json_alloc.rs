//! Allocations of the own JSON reader (DESIGN 7.7); the cases are in
//! `support/alloc_cases/json.rs`. No libtest harness (`support/harness.rs`): dhat counts
//! the whole process, so no other thread may allocate during a measurement.

#[cfg(not(target_arch = "wasm32"))]
#[path = "support/alloc_cases/json.rs"]
mod cases;
#[cfg(not(target_arch = "wasm32"))]
#[path = "support/harness.rs"]
mod harness;

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    harness::run("reader_allocations", cases::reader_allocations);
}
