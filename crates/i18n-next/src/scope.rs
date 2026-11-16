//! Global access and scope (DESIGN 4.4, 4.5): one installed instance (`OnceLock`, feature
//! `global`), a thread-local stack of scope frames that implicit `Text` / `t!` read, and
//! `negotiate` for `Accept-Language`.
//!
//! A frame holds what it sets (instance, locale, page); a lookup takes each from the
//! nearest frame that sets it. `run` pushes a frame for the closure, `wrap` for every poll
//! of a future: no runtime, no task-local, any executor. Guards pop in `Drop`, so a panic
//! leaves the stack as it was.

use crate::error::Error;
use crate::i18n::{I18n, View};
use crate::text::Text;
use std::borrow::Cow;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

#[cfg(feature = "global")]
static GLOBAL: std::sync::OnceLock<I18n> = std::sync::OnceLock::new();

/// Makes `i18n` the global instance of the process: `t!` without `=>`, [`t()`],
/// [`Key`](crate::Key) and the free functions read it when no scope sets an instance.
///
/// ```
/// let i18n = i18n_next::I18n::builder().messages("en", None, r#"{"hi": "Hi"}"#).build()?;
/// i18n_next::install(i18n)?;
/// assert_eq!(i18n_next::t!("hi"), "Hi");
/// assert!(matches!(i18n_next::install(i18n_next::I18n::builder().build()?), Err(i18n_next::Error::AlreadyInstalled)));
/// # Ok::<(), i18n_next::Error>(())
/// ```
///
/// # Errors
/// [`Error::AlreadyInstalled`] when an instance is installed already (once per process;
/// the installed one stays mutable: `set_locale`, `reload`, `add_messages`).
#[cfg(feature = "global")]
pub fn install(i18n: I18n) -> Result<(), Error> {
    GLOBAL.set(i18n).map_err(|_| Error::AlreadyInstalled)
}

/// The installed instance, if any.
#[cfg(feature = "global")]
#[must_use]
pub fn global() -> Option<&'static I18n> {
    GLOBAL.get()
}

pub(crate) fn global_instance() -> Option<&'static I18n> {
    #[cfg(feature = "global")]
    return GLOBAL.get();
    #[cfg(not(feature = "global"))]
    None
}

/// No instance anywhere: the key is shown, with one warning per process in debug builds.
#[cold]
pub(crate) fn uninstalled(key: &str) {
    #[cfg(debug_assertions)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "i18n-next: \"{key}\" shown before install() and outside a scope with an instance: the key stands in"
            );
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = key;
}

/// A [`Text`] of `key` on the implicit instance (nearest scope, else the global one),
/// what `t!("key")` builds.
#[must_use]
pub fn t(key: impl Into<Cow<'static, str>>) -> Text {
    Text::new(key.into(), None)
}

/// The instance an implicit `Text` uses now: the nearest scope's, else the global one.
fn implicit() -> Option<I18n> {
    current(true, false, false)
        .0
        .or_else(|| global_instance().cloned())
}

/// Switches the active locale of the implicit instance (see [`I18n::set_locale`]).
///
/// # Errors
/// [`Error::NotInstalled`] without an instance; else what [`I18n::set_locale`] returns.
pub fn set_locale(code: &str) -> Result<(), Error> {
    implicit().ok_or(Error::NotInstalled)?.set_locale(code)
}

/// The locale an implicit `Text` resolves in now: the nearest scope's, else the implicit
/// instance's active locale; `None` without an instance.
#[must_use]
pub fn locale() -> Option<Arc<str>> {
    let (i18n, locale, ..) = current(true, true, false);
    locale.map(Code::arc).or_else(|| {
        i18n.or_else(|| global_instance().cloned())
            .map(|i| i.locale())
    })
}

/// A view of page `name` on the implicit instance, in the scope's locale if one is set;
/// `None` without an instance.
#[must_use]
pub fn page(name: &str) -> Option<View> {
    let (i18n, locale, ..) = current(true, true, false);
    let i18n = i18n.or_else(|| global_instance().cloned())?;
    Some(View {
        i18n,
        locale: locale.map(Code::arc),
        page: Some(Arc::from(name)),
    })
}

/// A locale or page name: a literal, or shared (a `View`'s, an owned string). A literal
/// goes on the scope stack without allocating.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Code {
    Static(&'static str),
    Shared(Arc<str>),
}

impl Code {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Code::Static(s) => s,
            Code::Shared(s) => s,
        }
    }

    fn arc(self) -> Arc<str> {
        match self {
            Code::Static(s) => Arc::from(s),
            Code::Shared(s) => s,
        }
    }
}

impl From<Cow<'static, str>> for Code {
    fn from(s: Cow<'static, str>) -> Self {
        match s {
            Cow::Borrowed(s) => Code::Static(s),
            Cow::Owned(s) => Code::Shared(Arc::from(s)),
        }
    }
}

#[derive(Debug, Default)]
struct Frame {
    instance: Option<I18n>,
    locale: Option<Code>,
    page: Option<Code>,
    /// Pushed around a nested `Text` written for a sizing or blank check, or a comparison:
    /// its misses are not reported (the outer reports once, when it is shown).
    quiet: bool,
}

thread_local! {
    #[allow(
        clippy::missing_const_for_thread_local,
        reason = "already `const`: clippy misreads the expansion on targets without native TLS (Android)"
    )]
    static STACK: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// What the scope stack sets, each from its nearest frame; only the parts asked for are
/// cloned; last, whether a frame is quiet. An empty stack costs one thread-local read.
pub(crate) fn current(
    instance: bool,
    locale: bool,
    page: bool,
) -> (Option<I18n>, Option<Code>, Option<Code>, bool) {
    STACK
        .try_with(|cell| {
            let Ok(stack) = cell.try_borrow() else {
                return (None, None, None, false);
            };
            if stack.is_empty() {
                return (None, None, None, false);
            }
            let frames = || stack.iter().rev();
            (
                instance
                    .then(|| frames().find_map(|f| f.instance.clone()))
                    .flatten(),
                locale
                    .then(|| frames().find_map(|f| f.locale.clone()))
                    .flatten(),
                page.then(|| frames().find_map(|f| f.page.clone()))
                    .flatten(),
                frames().any(|f| f.quiet),
            )
        })
        .unwrap_or((None, None, None, false))
}

/// Whether a frame says a miss is not reported now (see `Frame::quiet`).
pub(crate) fn quiet() -> bool {
    current(false, false, false).3
}

/// Runs `f` with a frame of the outer `Text`'s instance, locale and page on top: a nested
/// `Text` without its own resolves in the outer one's (JS: the inner `<i18n-t>` inherits).
pub(crate) fn nest<R>(
    instance: Option<&I18n>,
    locale: Option<&Code>,
    page: Option<&Code>,
    quiet: bool,
    f: impl FnOnce() -> R,
) -> R {
    let frame = Frame {
        instance: instance.cloned(),
        locale: locale.cloned(),
        page: page.cloned(),
        quiet,
    };
    let _pushed = Pushed::new(frame, None);
    f()
}

/// Pushes a frame; dropping it truncates the stack back, handing the frame to `back`.
struct Pushed<'a> {
    depth: usize,
    back: Option<&'a mut Option<Frame>>,
}

impl<'a> Pushed<'a> {
    fn new(frame: Frame, back: Option<&'a mut Option<Frame>>) -> Self {
        let depth = STACK
            .try_with(|cell| {
                let mut stack = cell.borrow_mut();
                stack.push(frame);
                stack.len() - 1
            })
            .unwrap_or(usize::MAX);
        Pushed { depth, back }
    }
}

impl Drop for Pushed<'_> {
    fn drop(&mut self) {
        let _ = STACK.try_with(|cell| {
            let mut stack = cell.borrow_mut();
            if self.depth < stack.len() {
                let mut frames = stack.drain(self.depth..);
                if let Some(back) = self.back.take() {
                    *back = frames.next();
                }
            }
        });
    }
}

/// Locale, page and instance for a closure or a future, without passing them down
/// (`run(locale, fn)` of `@i18n-micro/node`). Start with [`scope`].
///
/// ```
/// use i18n_next::{I18n, scope, t};
///
/// let i18n = I18n::builder()
///     .messages("en", None, r#"{"hi": "Hi"}"#)
///     .messages("de", None, r#"{"hi": "Hallo"}"#)
///     .build()?;
/// let out = scope().instance(i18n.clone()).locale("de").run(|| t!("hi").to_string());
/// assert_eq!(out, "Hallo");
/// assert_eq!(scope().locale("de").run(|| i18n.text("hi").to_string()), "Hallo");
/// assert_eq!(scope().locale("de").run(|| i18n.text("hi").locale("en").to_string()), "Hi");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[derive(Debug, Default)]
#[must_use]
pub struct Scope {
    frame: Frame,
}

/// A new scope frame; nothing is set until [`Scope::run`] or [`Scope::wrap`].
pub fn scope() -> Scope {
    Scope::default()
}

impl Scope {
    /// The locale implicit and explicit-instance `Text` resolve in (unless the `Text` sets
    /// its own).
    pub fn locale(mut self, code: impl AsRef<str>) -> Self {
        self.frame.locale = Some(Code::Shared(Arc::from(code.as_ref())));
        self
    }

    /// The page `Text` reads (unless it sets its own).
    pub fn page(mut self, name: impl AsRef<str>) -> Self {
        self.frame.page = Some(Code::Shared(Arc::from(name.as_ref())));
        self
    }

    /// The instance implicit `Text` (`t!` without `=>`, [`Key`](crate::Key)) uses, over the
    /// global one: a test or a library without global state.
    pub fn instance(mut self, i18n: I18n) -> Self {
        self.frame.instance = Some(i18n);
        self
    }

    /// Runs `f` with this frame on top of the thread's scope stack; popped after, also
    /// when `f` panics.
    pub fn run<R>(self, f: impl FnOnce() -> R) -> R {
        let _pushed = Pushed::new(self.frame, None);
        f()
    }

    /// `fut` with this frame on top of the stack during each of its polls, popped after
    /// each: works with any executor. The future is boxed once, here.
    pub fn wrap<F: Future>(self, fut: F) -> Scoped<F> {
        Scoped {
            frame: Some(self.frame),
            fut: Box::pin(fut),
        }
    }
}

/// A future run in a scope ([`Scope::wrap`]).
// ponytail: boxed for a safe pin projection (`forbid(unsafe_code)`); one allocation per wrap.
pub struct Scoped<F> {
    frame: Option<Frame>,
    fut: Pin<Box<F>>,
}

impl<F> std::fmt::Debug for Scoped<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scoped")
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}

impl<F: Future> Future for Scoped<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = self.get_mut();
        let frame = this.frame.take().unwrap_or_default();
        let _pushed = Pushed::new(frame, Some(&mut this.frame));
        this.fut.as_mut().poll(cx)
    }
}

/// The best of `available` for an `Accept-Language` header or a comma-separated list of
/// tags (`navigator.languages` joined): entries by `q` (ties keep their order), each tried
/// as is, then shortened (`de-AT` → `de`), then as any `available` of the same language
/// (`de` → `de-at`, one with the same script first: `zh-Hant-TW` → `zh-hant-hk` over
/// `zh-hans`); `*` takes the first available. A tag with `q=0` is refused: never chosen,
/// not by `*` and not by shortening or the same-language step. An entry with a malformed
/// weight (`q=2`, `q = 0.5`, any other parameter) is dropped (RFC 9110). Case-insensitive,
/// `_` equals `-`. Returns the code as `available` spells it. Only the first 32 entries
/// within the first 4 KiB count (cut at the last `,` in that span): the work stays bounded
/// for any header a client sends.
///
/// ```
/// use i18n_next::negotiate;
///
/// let available = ["en", "de", "pt-br"];
/// assert_eq!(negotiate("de-AT,de;q=0.9,en;q=0.5", available).as_deref(), Some("de"));
/// assert_eq!(negotiate("fr, pt;q=0.8", available).as_deref(), Some("pt-br"));
/// assert_eq!(negotiate("en;q=0.1, DE", available).as_deref(), Some("de"));
/// assert_eq!(negotiate("*, en;q=0", available).as_deref(), Some("de"));
/// assert_eq!(negotiate("fr", available), None);
/// ```
pub fn negotiate<I>(accept: &str, available: I) -> Option<String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    const MAX_ENTRIES: usize = 32;
    const MAX_BYTES: usize = 4096;
    let accept = match accept.as_bytes().get(..=MAX_BYTES) {
        // `,` is ASCII, so the cut is on a char boundary.
        Some(head) => accept
            .get(..head.iter().rposition(|&b| b == b',').unwrap_or(0))
            .unwrap_or(""),
        None => accept,
    };
    let mut refused: Vec<&str> = Vec::new();
    let mut entries: Vec<(&str, u16)> = accept
        .split(',')
        .take(MAX_ENTRIES)
        .filter_map(|entry| {
            let mut fields = entry.split(';');
            let tag = fields.next()?.trim();
            let mut q = 1000;
            for f in fields {
                let f = f.trim_matches([' ', '\t']);
                // `weight = OWS ";" OWS "q=" qvalue`: nothing else, no space around `=`.
                let v = f.strip_prefix("q=").or_else(|| f.strip_prefix("Q="))?;
                q = quality(v)?;
            }
            if tag.is_empty() {
                return None;
            }
            if q == 0 {
                refused.push(tag);
                return None;
            }
            Some((tag, q))
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.1));
    let ok = |a: &str| !refused.iter().any(|r| same_tag(a, r));
    // Refusals are checked once per available code, not on every lookup.
    let available: Vec<I::Item> = available.into_iter().filter(|a| ok(a.as_ref())).collect();
    let find = |hit: &dyn Fn(&str) -> bool| {
        available
            .iter()
            .map(AsRef::as_ref)
            .find(|a| hit(a))
            .map(str::to_owned)
    };
    for (tag, _) in entries {
        if tag == "*" {
            if let Some(a) = find(&|_| true) {
                return Some(a);
            }
            continue;
        }
        let mut t = tag;
        loop {
            if ok(t)
                && let Some(a) = find(&|a| same_tag(a, t))
            {
                return Some(a);
            }
            match t.rfind(['-', '_']) {
                Some(cut) => t = t.get(..cut).unwrap_or(""),
                None => break,
            }
        }
        let want = script(tag);
        let same_lang = |a: &str| same_tag(primary(a), t);
        if let Some(a) = want
            .and_then(|s| find(&|a| same_lang(a) && script(a).is_some_and(|x| same_tag(x, s))))
            .or_else(|| find(&same_lang))
        {
            return Some(a);
        }
    }
    None
}

/// The script subtag (`Hant` of `zh-Hant-TW`): four letters right after the language.
fn script(tag: &str) -> Option<&str> {
    tag.split(['-', '_'])
        .nth(1)
        .filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphabetic()))
}

/// `q` as thousandths: `0`–`1` with up to three decimals (RFC 9110); anything else is no
/// valid weight and drops the entry.
fn quality(v: &str) -> Option<u16> {
    let (int, frac) = v.split_once('.').unwrap_or((v, ""));
    if frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut n: u16 = match int {
        "0" => 0,
        "1" => 1000,
        _ => return None,
    };
    for (i, b) in frac.bytes().enumerate() {
        n += u16::from(b - b'0') * [100, 10, 1][i];
    }
    (n <= 1000).then_some(n)
}

fn primary(tag: &str) -> &str {
    tag.split(['-', '_']).next().unwrap_or(tag)
}

fn same_tag(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes().zip(b.bytes()).all(|(x, y)| {
            let norm = |c: u8| {
                if c == b'_' {
                    b'-'
                } else {
                    c.to_ascii_lowercase()
                }
            };
            norm(x) == norm(y)
        })
}
