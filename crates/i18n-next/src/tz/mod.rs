//! Time zones without dependencies (DESIGN 5.4): civil calendar arithmetic, TZif readers,
//! POSIX TZ rules, the Android tzdata container, a browser backend and a compact bundled tzdb.
//! The date formatter only needs `offset_at(unix seconds) -> seconds east of UTC`.
//! Filled in by stage M6.
