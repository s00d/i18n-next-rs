//! Change notifications (DESIGN 4.5 `listeners`, 8b.5): `subscribe` and [`Change`].
//!
//! A change is any write that can change what a lookup answers: `set_locale` to another
//! locale, `add_messages`, `reload`, and `add_file` when it completes a pair. A lazy load
//! and an eviction answer exactly what was there before, so they send nothing (eviction
//! still bumps the version: thread caches must drop the old snapshot to free it).

use crate::i18n::{I18n, Inner};
use crate::lazy::lock;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// A listener and whether its subscription is still alive: `send` checks it before each
/// call, so a listener dropped while others run is not called afterwards.
struct Entry {
    live: AtomicBool,
    f: Box<dyn Fn(&Change) + Send + Sync>,
}

type Listener = Arc<Entry>;

/// What changed: the active locale after the write and the new snapshot version.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Change {
    /// The active locale after the change.
    pub locale: Arc<str>,
    /// The snapshot version the change produced ([`I18n::version`]).
    pub version: u64,
}

/// The listeners of one instance.
#[derive(Default)]
pub(crate) struct Listeners {
    list: Mutex<(u64, Vec<(u64, Listener)>)>,
}

impl Listeners {
    /// Calls every listener with `change`, over a copy of the list (a listener may
    /// subscribe or unsubscribe); a panicking listener does not stop the others.
    pub(crate) fn send(&self, change: &Change) {
        let listeners: Vec<Listener> = {
            let list = lock(&self.list);
            if list.1.is_empty() {
                return;
            }
            list.1.iter().map(|(_, f)| f.clone()).collect()
        };
        for e in listeners {
            // Acquire: pairs with the release in `Subscription::drop`.
            if e.live.load(Ordering::Acquire) {
                let _ = catch_unwind(AssertUnwindSafe(|| (e.f)(change)));
            }
        }
    }
}

/// A subscription made by [`I18n::subscribe`]: dropping it unsubscribes. It does not keep
/// the catalog alive.
#[must_use = "dropping a Subscription unsubscribes at once"]
pub struct Subscription {
    inner: Weak<Inner>,
    id: u64,
}

impl fmt::Debug for Subscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            // Dropped after the lock: a listener may own subscriptions of its own.
            let gone: Vec<_> = lock(&inner.listeners.list)
                .1
                .extract_if(.., |(id, _)| *id == self.id)
                .collect();
            for (_, e) in &gone {
                e.live.store(false, Ordering::Release);
            }
            drop(gone);
        }
    }
}

impl I18n {
    /// Calls `f` after every change of what lookups can answer: `set_locale` to another
    /// locale, `add_messages`, `reload`, and `add_file` when it completes a pair. A lazy
    /// load and a page eviction answer the same as before and call nothing.
    ///
    /// Listeners run on the writing thread after the new snapshot is in, in subscription
    /// order, over a copy of the list; a listener that panics is skipped, the others
    /// still run. Two writes from two threads may notify out of order: compare
    /// [`Change::version`].
    ///
    /// Once its [`Subscription`] is dropped a listener is not called again, also not by a
    /// notification already running on the same thread (another listener dropped it).
    /// Dropping does not wait for other threads: a notification running on another thread
    /// that already passed this listener's check may still call it once, during or right
    /// after the drop.
    ///
    /// ```
    /// use i18n_next::I18n;
    /// use std::sync::{Arc, Mutex};
    ///
    /// let i18n = I18n::builder().messages("de", None, "{}").build()?;
    /// let seen = Arc::new(Mutex::new(Vec::new()));
    /// let log = seen.clone();
    /// let sub = i18n.subscribe(move |c| log.lock().unwrap().push((c.locale.to_string(), c.version)));
    /// i18n.set_locale("de")?;
    /// drop(sub);
    /// i18n.set_locale("en")?;
    /// assert_eq!(*seen.lock().unwrap(), [("de".to_owned(), 1)]);
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    pub fn subscribe(&self, f: impl Fn(&Change) + Send + Sync + 'static) -> Subscription {
        let mut list = lock(&self.inner.listeners.list);
        list.0 += 1;
        let id = list.0;
        list.1.push((
            id,
            Arc::new(Entry {
                live: AtomicBool::new(true),
                f: Box::new(f),
            }),
        ));
        Subscription {
            inner: Arc::downgrade(&self.inner),
            id,
        }
    }
}
