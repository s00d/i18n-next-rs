//! Feature `tracing` (DESIGN 8b.17): a miss is a `warn!` with `locale`, `page` and `key`;
//! a load, `add_file` and `reload` are `debug!` events with the path and bytes; an
//! `ensure_async` fetch runs in a span. A minimal subscriber records what arrives.
#![cfg(all(feature = "tracing", not(target_arch = "wasm32")))]

use std::fmt::Write as _;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// Every event and span as `LEVEL name: field=value …`.
#[derive(Clone, Default)]
struct Log {
    lines: Arc<Mutex<Vec<String>>>,
    next: Arc<AtomicU64>,
}

struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, " {}={:?}", field.name(), value);
    }
}

impl Subscriber for Log {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut f = Fields(format!("SPAN {}:", span.metadata().name()));
        span.record(&mut f);
        self.lines.lock().unwrap().push(f.0);
        Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut f = Fields(format!("{}:", event.metadata().level()));
        event.record(&mut f);
        self.lines.lock().unwrap().push(f.0);
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

/// Given the `tracing` feature and a subscriber, when the app misses a key, loads a lazy
/// page, reloads and ensures a remote file, then each shows up with its fields.
#[test]
fn misses_loads_and_fetches_are_traced() {
    let log = Log::default();
    let lines = log.lines.clone();
    tracing::subscriber::with_default(log, || {
        let i18n = i18n_next::I18n::builder()
            .lazy_files([("pages/p/en.json", br#"{"t": "T"}"#.as_slice())])
            .remote(["de.json"])
            .build()
            .unwrap();
        // step 1: a miss → warn with locale, page, key
        assert_eq!(i18n.page("p").t("nope"), "nope");
        // step 2: a reload → debug
        i18n.reload().unwrap();
        // step 3: ensure_async → a span for the fetch, then add_file's debug
        let mut fut = Box::pin(i18n.ensure_async("de", None, |_| async {
            Ok::<_, String>(br#"{"a": "A"}"#.to_vec())
        }));
        let Poll::Ready(r) = fut.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
            panic!("ready at once")
        };
        r.unwrap();
    });
    let lines = lines.lock().unwrap().join("\n");
    for want in [
        "WARN: message=i18n-next: missing translation locale=\"en\" page=\"p\" key=\"nope\"",
        "DEBUG: message=i18n-next: load path=\"pages/p/en.json\" bytes=10",
        "DEBUG: message=i18n-next: reload",
        "SPAN i18n_next::ensure: path=de.json",
        "DEBUG: message=i18n-next: add_file path=\"de.json\" bytes=10",
    ] {
        assert!(lines.contains(want), "{want:?} not in\n{lines}");
    }
}

/// Given the `tracing` feature, when a named number or date format no locale of the chain
/// has is used, then a warn names its kind, name and locale (JS `warnMissingFormat`).
#[test]
fn missing_formats_are_traced() {
    let log = Log::default();
    let lines = log.lines.clone();
    tracing::subscriber::with_default(log, || {
        let i18n = i18n_next::I18n::builder().time_zone("UTC").build().unwrap();
        // step 1: unknown names → the default options
        assert_eq!(i18n.tn_as(1.5, "nope"), "1.5");
        assert_eq!(i18n.td_as(0i64, "never"), "1/1/1970");
    });
    let lines = lines.lock().unwrap().join("\n");
    for want in [
        "kind=\"number\" name=\"nope\" locale=\"en\"",
        "kind=\"datetime\" name=\"never\" locale=\"en\"",
    ] {
        assert!(lines.contains(want), "{want:?} not in\n{lines}");
    }
}
