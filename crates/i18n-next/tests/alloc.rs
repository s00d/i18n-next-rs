//! Allocation counts on the hot path (DESIGN 7.7); the cases are in
//! `support/alloc_cases/hot_path.rs`. No libtest harness (`support/harness.rs`): dhat counts
//! the whole process, so no other thread may allocate during a measurement.

#[cfg(not(target_arch = "wasm32"))]
#[path = "support/alloc_cases/hot_path.rs"]
mod cases;
#[cfg(not(target_arch = "wasm32"))]
#[path = "support/harness.rs"]
mod harness;

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    harness::run("hot_path_allocations", cases::hot_path_allocations);
}
