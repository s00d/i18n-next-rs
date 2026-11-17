#![doc = include_str!("../CRATE_README.md")]
#![forbid(unsafe_code)]

use i18n_next::{Change, I18n, Subscription, Text, WatchGuard};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Emitter, Manager, Runtime};

mod bind;
pub mod commands;
pub mod error;
mod missing;
pub mod models;
mod persist;
pub mod protocol;

pub use bind::{BindId, Relabel};
pub use error::{Error, Result};
pub use missing::missing;

/// The event every window gets after a change of what lookups answer (a locale switch,
/// `add_messages`, `reload`, a `watch` reload): `{ "locale": "de", "version": 7 }`, in
/// version order. A changed chunk gets a new protocol URL (its content tag), so nothing
/// stale is read from the cache; the front end asks `meta` for the new tags.
pub const EVENT_CHANGED: &str = "i18n-next://changed";

/// The dev event of the missing-key report (DESIGN 8b.7): one `{ locale, page, key, count }`
/// per key the first time it is missed.
pub const EVENT_MISSING: &str = "i18n-next://missing";

/// The custom protocol the front end reads data from (DESIGN 6.1).
pub const SCHEME: &str = "i18n-next";

/// The plugin's state: the catalog (it derefs to [`I18n`]) and the native labels bound to
/// it. [`ManagerExt::i18n`] returns it; `app.state::<I18n>()` works too.
pub struct Handle {
    i18n: I18n,
    binds: Mutex<bind::Bindings>,
    system_locale: bool,
    /// The start locale is decided: a saved one was applied, or the first `load` came.
    chosen: AtomicBool,
    persist: Option<PathBuf>,
    /// The locale in the persist file (or the one the system picked, which is not saved).
    saved: Mutex<Saved>,
    /// Origins the protocol answers (the app's own, the dev server's in debug, the
    /// builder's extra ones).
    origins: Vec<String>,
    /// The last change handled: persist, relabel and the event run in version order, under
    /// this lock, and a change older than it is not handled again.
    handled: Mutex<u64>,
    sub: Mutex<Option<Subscription>>,
    watch: Mutex<Option<WatchGuard>>,
}

#[derive(Default)]
struct Saved {
    file: Option<Arc<str>>,
    system_pick: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Deref for Handle {
    type Target = I18n;
    fn deref(&self) -> &I18n {
        &self.i18n
    }
}

impl Handle {
    /// Shows `text` on `target` now and again after every change (locale switch, reload),
    /// until [`Handle::unbind`] (DESIGN 6.3). A target that fails to relabel (a destroyed
    /// item or window) is unbound then; a window's bindings (and those of targets whose
    /// [`Relabel::window`] names it) go when the window is destroyed.
    ///
    /// ```rust,no_run
    /// # fn demo<R: tauri::Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    /// use tauri_plugin_i18n_next::ManagerExt;
    /// let quit = tauri::menu::MenuItem::new(app, "Quit", true, None::<&str>)?;
    /// let i18n = app.i18n();
    /// i18n.bind(&quit, i18n.page("tray").text("quit"));
    /// # Ok(()) }
    /// ```
    pub fn bind<T: Relabel + Clone + 'static>(&self, target: &T, text: Text) -> BindId {
        let (id, label) = lock(&self.binds).add(Arc::new(target.clone()), text);
        if target.relabel(&label).is_err() {
            self.unbind(id);
        }
        id
    }

    /// The origins the protocol serves: the app's own, the dev server's in debug builds and
    /// [`Builder::origins`]. What the plugin passes to [`protocol::handle`].
    #[must_use]
    pub fn origins(&self) -> &[String] {
        &self.origins
    }

    /// Stops updating a bound target; `false` when `id` is not bound (any more).
    pub fn unbind(&self, id: BindId) -> bool {
        lock(&self.binds).remove(id)
    }

    /// After a change, in version order: save the active locale (8b.3), relabel what shows
    /// something new, then `emit` the change. The catalog delivers changes in order; a
    /// change older than one handled already is skipped all the same. What is saved and
    /// shown is read from the catalog now, not from the change.
    fn on_change(&self, change: &Change, emit: impl FnOnce(models::Changed)) {
        let mut handled = lock(&self.handled);
        if change.version <= *handled {
            return;
        }
        *handled = change.version;
        if let Some(path) = &self.persist {
            let locale = self.i18n.locale();
            let mut saved = lock(&self.saved);
            let picked = saved.system_pick.take();
            if picked.as_deref() != Some(&*locale) && saved.file.as_deref() != Some(&*locale) {
                saved.file = Some(locale.clone());
                // ponytail: a failed write is dropped (read-only config dir); the locale
                // is still switched, only not remembered
                let _ = persist::write(path, &locale);
            }
        }
        let todo = lock(&self.binds).refresh();
        let failed: Vec<BindId> = todo
            .into_iter()
            .filter(|(_, target, label)| target.relabel(label).is_err())
            .map(|(id, _, _)| id)
            .collect();
        for id in failed {
            self.unbind(id);
        }
        emit(models::Changed {
            locale: change.locale.to_string(),
            version: change.version,
        });
    }

    /// The first `load`: the system locale from `navigator.languages` (8b.3 step 2), unless
    /// a saved one was applied or a `load` came before.
    fn first_load(&self, languages: Option<&[String]>) {
        if self.chosen.swap(true, Ordering::AcqRel) || !self.system_locale {
            return;
        }
        let Some(languages) = languages.filter(|l| !l.is_empty()) else {
            return;
        };
        // ponytail: the first 32 tags; nobody lists more languages than that
        let accept = languages
            .iter()
            .take(32)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(",");
        let codes = self.i18n.locales().into_iter().map(|l| l.code);
        if let Some(code) = i18n_next::negotiate(&accept, codes)
            && *self.i18n.locale() != *code
        {
            lock(&self.saved).system_pick = Some(code.clone());
            let _ = self.i18n.set_locale(&code);
        }
    }
}

/// `app.i18n()` on anything that is a [`Manager`]: the plugin's [`Handle`].
pub trait ManagerExt<R: Runtime> {
    /// The plugin's state. Panics when the plugin is not registered, like `app.state()`.
    fn i18n(&self) -> &Handle;
}

impl<R: Runtime, T: Manager<R>> ManagerExt<R> for T {
    fn i18n(&self) -> &Handle {
        self.state::<Handle>().inner()
    }
}

/// The plugin, registered as `i18n-next`.
///
/// ```rust,no_run
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let i18n = i18n_next::I18n::builder().dir("locales").config("i18n.json").build()?;
/// tauri::Builder::default()
///     .plugin(tauri_plugin_i18n_next::Builder::new(i18n).build())
/// #   ;
/// # Ok(()) }
/// ```
///
/// At start it installs the catalog as the global instance (`t!` everywhere; skipped when
/// one is installed already), applies the saved locale (8b.3), registers the `i18n-next://`
/// protocol and, in debug builds, watches `dir` / `lazy_dir` sources and collects missing
/// keys (8b.5, 8b.7).
pub struct Builder {
    i18n: I18n,
    system_locale: bool,
    persist: bool,
    global: bool,
    watch: bool,
    data_dir: Option<PathBuf>,
    origins: Vec<String>,
}

impl Builder {
    /// A plugin over `i18n`: system locale, persist, global install and dev watch on.
    #[must_use]
    pub fn new(i18n: I18n) -> Self {
        Self {
            i18n,
            system_locale: true,
            persist: true,
            global: true,
            watch: true,
            data_dir: None,
            origins: Vec::new(),
        }
    }

    /// More origins the protocol serves, besides the app's own (`tauri://localhost`,
    /// `http(s)://tauri.localhost`) and, in debug builds, the dev server's: pages loaded
    /// from elsewhere that read translations (`tauri-plugin-localhost`'s
    /// `http://localhost:<port>`, an external URL, an Android dev host). Each is an origin
    /// as the WebView sends it: scheme, host and port, no path.
    #[must_use]
    pub fn origins<I, S>(mut self, origins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.origins.extend(origins.into_iter().map(Into::into));
        self
    }

    /// Picks the start locale from the WebView's `navigator.languages` (sent with the first
    /// `load`) when no saved locale applies. On by default.
    #[must_use]
    pub fn system_locale(mut self, enabled: bool) -> Self {
        self.system_locale = enabled;
        self
    }

    /// Remembers the active locale in `i18n-next.json` of the app config directory and
    /// applies it at the next start, if it is still available. On by default.
    #[must_use]
    pub fn persist(mut self, enabled: bool) -> Self {
        self.persist = enabled;
        self
    }

    /// Installs the catalog as the global instance for `t!` / `Text` without an instance.
    /// On by default.
    #[must_use]
    pub fn global(mut self, enabled: bool) -> Self {
        self.global = enabled;
        self
    }

    /// Reloads the catalog when files of its `dir` / `lazy_dir` sources change, in debug
    /// builds (`I18n::watch`, DESIGN 8b.5); windows get [`EVENT_CHANGED`] and bound labels
    /// update. On by default; release builds never watch.
    #[must_use]
    pub fn watch(mut self, enabled: bool) -> Self {
        self.watch = enabled;
        self
    }

    /// Where the plugin keeps its files (`i18n-next.json`, the dev `i18n-missing.json`)
    /// instead of the app config and data directories.
    #[must_use]
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// The plugin.
    pub fn build<R: Runtime>(self) -> TauriPlugin<R> {
        use commands::{available_locales, get_locale, load, load_page, meta, pages, set_locale};
        // `report_missing` only in debug builds (8b.2, 8b.7)
        type Handler<R> = Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>;
        let handler: Handler<R> = if cfg!(debug_assertions) {
            Box::new(tauri::generate_handler![
                load,
                load_page,
                meta,
                set_locale,
                get_locale,
                available_locales,
                pages,
                commands::report_missing
            ])
        } else {
            Box::new(tauri::generate_handler![
                load,
                load_page,
                meta,
                set_locale,
                get_locale,
                available_locales,
                pages
            ])
        };
        tauri::plugin::Builder::new("i18n-next")
            .invoke_handler(handler)
            .register_asynchronous_uri_scheme_protocol(SCHEME, |ctx, request, responder| {
                let app = ctx.app_handle().clone();
                // the shared blocking pool: no thread start per request; the handle is
                // dropped, the task runs on (it answers through `responder`)
                drop(tauri::async_runtime::spawn_blocking(move || {
                    let response = match app.try_state::<Handle>() {
                        Some(h) => protocol::handle(&h.i18n, h.origins(), &request),
                        None => protocol::unavailable(),
                    };
                    responder.respond(response);
                }));
            })
            .setup(move |app, _api| {
                self.setup(app)?;
                Ok(())
            })
            .on_event(|app, event| {
                if let tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::Destroyed,
                    ..
                } = event
                    && let Some(h) = app.try_state::<Handle>()
                {
                    lock(&h.binds).drop_window(label);
                }
            })
            .on_drop(|app| {
                // breaks the cycle catalog → listener → app → state → catalog
                if let Some(h) = app.try_state::<Handle>() {
                    lock(&h.sub).take();
                    lock(&h.watch).take();
                }
            })
            .build()
    }

    fn setup<R: Runtime>(self, app: &AppHandle<R>) -> tauri::Result<()> {
        let dir = |default: tauri::Result<PathBuf>| match &self.data_dir {
            Some(d) => Ok(d.clone()),
            None => default,
        };
        let persist = if self.persist {
            Some(dir(app.path().app_config_dir())?.join("i18n-next.json"))
        } else {
            None
        };
        let mut origins: Vec<String> = ["tauri://localhost", "http://tauri.localhost"]
            .into_iter()
            .chain(["https://tauri.localhost"])
            .map(str::to_owned)
            .collect();
        if cfg!(debug_assertions)
            && let Some(url) = &app.config().build.dev_url
        {
            origins.push(url.origin().ascii_serialization());
        }
        origins.extend(self.origins);
        let saved = persist.as_deref().and_then(persist::read);
        let i18n = self.i18n;
        if self.global {
            // another instance installed first keeps the global slot
            let _ = i18n_next::install(i18n.clone());
        }
        app.manage(i18n.clone());
        app.manage(Handle {
            i18n: i18n.clone(),
            binds: Mutex::default(),
            system_locale: self.system_locale,
            chosen: AtomicBool::new(false),
            persist,
            saved: Mutex::new(Saved {
                file: saved.as_deref().map(Arc::from),
                system_pick: None,
            }),
            origins,
            handled: Mutex::new(i18n.version()),
            sub: Mutex::default(),
            watch: Mutex::default(),
        });
        let handle = app.state::<Handle>();
        let emitter = app.clone();
        let sub = i18n.subscribe(move |change| {
            if let Some(h) = emitter.try_state::<Handle>() {
                h.on_change(change, |payload| {
                    let _ = emitter.emit(EVENT_CHANGED, payload);
                });
            }
        });
        *lock(&handle.sub) = Some(sub);
        // step 1 of the start order (8b.3): a saved locale that is still available
        // (`set_locale` refuses an unknown or disabled one)
        if let Some(code) = saved
            && i18n.set_locale(&code).is_ok()
        {
            handle.chosen.store(true, Ordering::Release);
        }
        if cfg!(debug_assertions) {
            if self.watch {
                *lock(&handle.watch) = Some(i18n.watch());
            }
            let file = dir(app.path().app_data_dir())?.join("i18n-missing.json");
            missing::attach(app.clone(), file);
        }
        Ok(())
    }
}
