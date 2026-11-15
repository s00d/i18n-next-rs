//! `Text` (DESIGN 4.3): a lazy translation, resolved each time it is shown, the way
//! `resolveText` of `@i18n-micro/core` resolves the props of `<i18n-t>`; `parts` / `fold`
//! follow `foldTemplate`. `Key` (4.4) is a `const` key that resolves through scope and the
//! global instance.
//!
//! Resolution writes straight into the caller's writer: the template is read inside the
//! snapshot (no copy), parameters go through a stack buffer, so `Display` into a reused
//! buffer allocates nothing and `to_string` allocates once (a sizing pass first).

use crate::args::{Arg, Args, Count, Small, escape_html};
use crate::catalog::{Entry, Msg, hole_ranges, is_js_space};
use crate::i18n::{I18n, View};
use crate::scope;
use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

/// A locale or page name: a literal, or shared (a `View`'s, an owned string).
#[derive(Clone, Debug, PartialEq)]
enum Name {
    Static(&'static str),
    Shared(Arc<str>),
}

impl Name {
    fn as_str(&self) -> &str {
        match self {
            Name::Static(s) => s,
            Name::Shared(s) => s,
        }
    }
}

impl From<Cow<'static, str>> for Name {
    fn from(s: Cow<'static, str>) -> Self {
        match s {
            Cow::Borrowed(s) => Name::Static(s),
            Cow::Owned(s) => Name::Shared(Arc::from(s)),
        }
    }
}

/// The custom plural rule of [`Text::plural_rule`].
type Rule = Arc<dyn Fn(&PluralCtx<'_>) -> Option<String> + Send + Sync>;

/// A translation shown later: the Rust counterpart of `<i18n-t>`.
///
/// Built by [`I18n::text`], [`View::text`] or the [`t!`](crate::t!) macro, with options that
/// mirror the component's props. Nothing is looked up until it is shown: every `Display`
/// resolves it again, so a `Text` kept in UI state follows `set_locale` and the
/// [`scope`](crate::scope()) it is shown in. `'static + Send + Sync + Clone`.
///
/// ```
/// use i18n_next::I18n;
///
/// let i18n = I18n::builder()
///     .messages("en", None, r#"{"hi": "Hi, {name}!", "apples": "no apples | one apple | {count} apples"}"#)
///     .messages("de", None, r#"{"hi": "Hallo, {name}!"}"#)
///     .build()?;
/// let hi = i18n.text("hi").arg("name", "Ann");
/// assert_eq!(hi.to_string(), "Hi, Ann!");
/// i18n.set_locale("de")?;
/// assert_eq!(hi, "Hallo, Ann!");                       // the same value, the new locale
/// assert_eq!(i18n.text("apples").count(5).locale("en"), "5 apples");
/// assert_eq!(i18n.text("nope").or("Default").to_string(), "Default");
/// assert_eq!(i18n.text("nope").resolve(), None);       // a miss without a default
/// # Ok::<(), i18n_next::Error>(())
/// ```
///
/// Resolution, as `resolveText`:
/// - the key's text in the locale and page (explicit on the `Text`, else the nearest
///   scope's, else the active locale and the root), interpolated with the arguments;
/// - with [`count`](Text::count): the plural form `tc` picks, with `{count}`;
/// - a miss calls `missing` and shows the [`or`](Text::or) default, else the key (both
///   interpolated); a subtree shows the key;
/// - [`html`](Text::html): text arguments are escaped (numbers and booleans are not), the
///   translation is trusted, a key standing in for it is escaped;
/// - [`hide_if_empty`](Text::hide_if_empty): blank text resolves to `None` and displays
///   as the default (or nothing), as `<i18n-t>` renders it.
#[derive(Clone)]
pub struct Text {
    key: Cow<'static, str>,
    i18n: Option<I18n>,
    locale: Option<Name>,
    page: Option<Name>,
    args: Args<'static>,
    count: Option<Count>,
    default: Option<Cow<'static, str>>,
    rule: Option<Rule>,
    html: bool,
    hide: bool,
    /// [`Text::emit`], reached through this pointer from `Args` (a `Text` argument of
    /// `t_with`): an app that never builds a `Text` does not link its resolution (~8 KB of
    /// `.wasm`).
    shown: fn(&Text, &mut dyn fmt::Write) -> fmt::Result,
}

/// What a custom plural rule ([`Text::plural_rule`]) gets: JS `customPluralRule(key,
/// count, params, locale, getter)`.
#[derive(Debug)]
#[non_exhaustive]
pub struct PluralCtx<'a> {
    /// The key of the `Text`.
    pub key: &'a str,
    /// The count, as JS `Number(plural)` holds it.
    pub count: f64,
    /// The locale the `Text` resolves in.
    pub locale: &'a str,
    text: &'a Text,
    i18n: &'a I18n,
    view: Option<&'a str>,
    page: Option<&'a str>,
}

impl PluralCtx<'_> {
    /// The raw text of `key` in the same locale and page (JS `getter(key)`): not
    /// interpolated; a miss calls `missing` and gives the `Text`'s default, else the key
    /// (escaped under `html`).
    #[must_use]
    pub fn get(&self, key: &str) -> String {
        if key.is_empty() {
            return String::new();
        }
        let found = self
            .i18n
            .inner
            .read(|s| match s.lookup(self.view, self.page, key) {
                Some(e) => Ok(e.text().map(|t| String::from(&**t))),
                None => Err(s.locale.clone()),
            });
        match found {
            Ok(Some(t)) => t,
            Ok(None) => key_text(key, self.text.html),
            Err(active) => {
                self.i18n
                    .report(Some(self.view.unwrap_or(&active)), self.page, key);
                match self.text.default_text() {
                    Some(d) => d.to_owned(),
                    None => key_text(key, self.text.html),
                }
            }
        }
    }
}

/// One piece of [`Text::parts`]: text (parameters filled in), or a placeholder no
/// argument fills, for the caller to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part<'a> {
    /// Text to show as is.
    Text(Cow<'a, str>),
    /// A `{name}` placeholder without an argument: a slot.
    Slot(Cow<'a, str>),
}

/// A `const` translation key ([`key!`](crate::key)): `Display` resolves it through the
/// nearest scope or the global instance, like `t!(key)`.
///
/// ```
/// use i18n_next::{I18n, Key, key, scope};
///
/// const QUIT: Key = key!("menu.quit");
/// let i18n = I18n::builder().messages("en", None, r#"{"menu": {"quit": "Quit {app}"}}"#).build()?;
/// scope().instance(i18n).run(|| {
///     assert_eq!(QUIT.args().arg("app", "Demo").to_string(), "Quit Demo");
///     assert_eq!(format!("{QUIT}"), "Quit {app}");
/// });
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key(&'static str);

impl Key {
    /// A key; `const`, allocates nothing.
    #[must_use]
    pub const fn new(key: &'static str) -> Key {
        Key(key)
    }

    /// A [`Text`] of this key, to add arguments and options to.
    #[must_use]
    pub fn args(self) -> Text {
        Text::new(Cow::Borrowed(self.0), None)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.args().emit(f)
    }
}

/// The text of a template, read inside the snapshot: placeholders found at load (a range
/// of the raw text and its holes), found now (a default, the key), or none.
#[derive(Clone, Copy)]
enum Tpl<'s> {
    Holes {
        raw: &'s str,
        start: usize,
        end: usize,
        holes: &'s [(u32, u32)],
    },
    Scan(&'s str),
    Literal(&'s str),
}

enum Piece<'s> {
    Lit(&'s str),
    Hole(&'s str),
}

impl<'s> Tpl<'s> {
    /// The literal runs and placeholders in order (a hole is `raw[s..e]` between braces).
    fn pieces(self, f: &mut dyn FnMut(Piece<'_>) -> fmt::Result) -> fmt::Result {
        match self {
            Tpl::Literal(s) => f(Piece::Lit(s)),
            Tpl::Scan(raw) => walk(raw, 0, raw.len(), hole_ranges(raw), f),
            Tpl::Holes {
                raw,
                start,
                end,
                holes,
            } => walk(
                raw,
                start,
                end,
                holes.iter().map(|&(s, e)| (s as usize, e as usize)),
                f,
            ),
        }
    }

    /// Writes the template with each placeholder an argument fills replaced (JS
    /// `interpolate`); the others stay as written.
    fn write(self, out: &mut dyn fmt::Write, p: Params<'_>) -> fmt::Result {
        self.pieces(&mut |piece| match piece {
            Piece::Lit(s) => out.write_str(s),
            Piece::Hole(name) => {
                if !p.put(name, out)? {
                    out.write_char('{')?;
                    out.write_str(name)?;
                    out.write_char('}')?;
                }
                Ok(())
            }
        })
    }

    /// Whether the interpolated text is blank (JS `!text.trim()`): stops at the first
    /// character that is not JS whitespace.
    fn blank(self, p: Params<'_>) -> Result<bool, fmt::Error> {
        let mut probe = NonBlank(false);
        match self.write(&mut probe, p) {
            Err(_) if probe.0 => Ok(false),
            Err(e) => Err(e),
            Ok(()) => Ok(true),
        }
    }

    /// Bytes [`Tpl::write`] produces, a nested `Text` counted as nothing.
    fn measure(self, p: Params<'_>) -> usize {
        let mut len = Len(0);
        let p = Params { measure: true, ..p };
        let _ = self.write(&mut len, p);
        len.0
    }
}

fn walk(
    raw: &str,
    start: usize,
    end: usize,
    holes: impl Iterator<Item = (usize, usize)>,
    f: &mut dyn FnMut(Piece<'_>) -> fmt::Result,
) -> fmt::Result {
    let mut at = start;
    for (s, e) in holes {
        // A hole of the whole string outside the form's trimmed range is not the form's.
        if s <= start || e >= end {
            continue;
        }
        let Some(name) = raw.get(s..e) else {
            continue;
        };
        f(Piece::Lit(raw.get(at..s - 1).unwrap_or("")))?;
        f(Piece::Hole(name))?;
        at = e + 1;
    }
    f(Piece::Lit(raw.get(at..end).unwrap_or("")))
}

/// Where placeholder values come from: the arguments (JS `params`, or `{ ...params }` when
/// spread: a list then has no `{length}`), `{count}` on top for a plural.
#[derive(Clone, Copy)]
struct Params<'x> {
    args: &'x Args<'static>,
    count: Option<Count>,
    spread: bool,
    html: bool,
    /// Sizing pass: a nested `Text` writes nothing.
    measure: bool,
}

impl Params<'_> {
    fn put(&self, name: &str, out: &mut dyn fmt::Write) -> Result<bool, fmt::Error> {
        if let Some(c) = self.count
            && name == "count"
        {
            let mut b = Small::new();
            c.write(&mut b);
            out.write_str(b.as_str())?;
            return Ok(true);
        }
        let value = if self.spread {
            self.args.spread(name)
        } else {
            self.args.find(name)
        };
        match value {
            Some(v) if self.measure && matches!(*v, Arg::Text(_)) => Ok(true),
            Some(v) => v.emit(out, self.html).map(|()| true),
            None => Ok(false),
        }
    }
}

/// Fails at the first character that is not JS whitespace.
struct NonBlank(bool);

impl fmt::Write for NonBlank {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if s.chars().all(is_js_space) {
            Ok(())
        } else {
            self.0 = true;
            Err(fmt::Error)
        }
    }
}

struct Len(usize);

impl fmt::Write for Len {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.len();
        Ok(())
    }
}

/// Compares what is written with a string, without building it.
struct Same<'a> {
    rest: &'a str,
    ok: bool,
}

impl fmt::Write for Same<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        match self.rest.strip_prefix(s) {
            Some(rest) => self.rest = rest,
            None => self.ok = false,
        }
        if self.ok { Ok(()) } else { Err(fmt::Error) }
    }
}

/// JS `keyText`: the key standing in for a translation, escaped under `html`.
fn key_text(key: &str, html: bool) -> String {
    let mut out = String::new();
    if html {
        let _ = escape_html(&mut out, key);
    } else {
        out.push_str(key);
    }
    out
}

fn needs_escape(s: &str) -> bool {
    s.bytes()
        .any(|b| matches!(b, b'&' | b'<' | b'>' | b'"' | b'\''))
}

impl Text {
    pub(crate) fn new(key: Cow<'static, str>, i18n: Option<I18n>) -> Text {
        Text {
            key,
            i18n,
            locale: None,
            page: None,
            args: Args::new(),
            count: None,
            default: None,
            rule: None,
            html: false,
            hide: false,
            shown: Text::emit,
        }
    }

    /// The displayed text through [`Text::shown`] (see there).
    pub(crate) fn show(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        (self.shown)(self, out)
    }

    /// Adds the value of `{name}` (JS `params[name]`): text, a number, a boolean, another
    /// `Text`. Up to four arguments are stored inline, without allocating.
    #[must_use]
    pub fn arg(
        mut self,
        name: impl Into<Cow<'static, str>>,
        value: impl Into<Arg<'static>>,
    ) -> Self {
        self.args.push_named(name.into(), value.into());
        self
    }

    /// Adds positional values `{0}`, `{1}`… and `{length}` (a JS array as `params`).
    #[must_use]
    pub fn args<I>(mut self, items: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<Arg<'static>>,
    {
        self.args.extend_list(items.into_iter().map(Into::into));
        self
    }

    /// Picks the plural form for `n` and fills `{count}` (JS `plural`), as `tc` does.
    #[must_use]
    pub fn count(mut self, n: impl Into<Count>) -> Self {
        self.count = Some(n.into());
        self
    }

    /// The text shown on a miss (JS `defaultValue`), interpolated; empty counts as none.
    #[must_use]
    pub fn or(mut self, default: impl Into<Cow<'static, str>>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Blank text resolves to `None` and displays as the default, or nothing (JS
    /// `hideIfEmpty`).
    #[must_use]
    pub fn hide_if_empty(mut self) -> Self {
        self.hide = true;
        self
    }

    /// Replaces the plural rule (JS `customPluralRule`): with a [`count`](Text::count), the
    /// rule's answer is the text as is (not interpolated); `None` is empty.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder().messages("en", None, r#"{"cats": "cat|cats"}"#).build()?;
    /// let cats = i18n.text("cats").count(3).plural_rule(|c| {
    ///     let forms = c.get(c.key);
    ///     let mut it = forms.split('|');
    ///     let form = if c.count == 1.0 { it.next() } else { it.nth(1) };
    ///     form.map(|f| format!("{} {f}", c.count))
    /// });
    /// assert_eq!(cats.to_string(), "3 cats");
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    #[must_use]
    pub fn plural_rule(
        mut self,
        rule: impl Fn(&PluralCtx<'_>) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.rule = Some(Arc::new(rule));
        self
    }

    /// Reads this page (its layer, then the root), over any scope's page.
    #[must_use]
    pub fn page(mut self, name: impl Into<Cow<'static, str>>) -> Self {
        self.page = Some(Name::from(name.into()));
        self
    }

    /// Resolves in this locale (as [`I18n::with_locale`]), over any scope's locale.
    #[must_use]
    pub fn locale(mut self, code: impl Into<Cow<'static, str>>) -> Self {
        self.locale = Some(Name::from(code.into()));
        self
    }

    /// The result goes into markup (JS `html`): text arguments are escaped, the
    /// translation is not; a key standing in for a missing translation is.
    #[must_use]
    pub fn html(mut self) -> Self {
        self.html = true;
        self
    }

    /// The text, `None` when the key is missing (or a subtree) and there is no default,
    /// or when [`hide_if_empty`](Text::hide_if_empty) hides it. One allocation.
    #[must_use]
    pub fn resolve(&self) -> Option<String> {
        let (out, keyed, hidden) = self.sized();
        (!keyed && !hidden).then_some(out)
    }

    /// The displayed text in one allocation: what `Display` writes, sized first.
    #[allow(clippy::inherent_to_string_shadow_display)] // same text, exact capacity
    #[must_use]
    pub fn to_string(&self) -> String {
        let (out, _, hidden) = self.sized();
        if hidden {
            return self.default_text().map(str::to_owned).unwrap_or_default();
        }
        out
    }

    /// Appends the displayed text to `out` (what `Display` writes), allocating nothing.
    ///
    /// # Errors
    /// Only what `out` returns.
    pub fn write_to(&self, out: &mut impl fmt::Write) -> fmt::Result {
        self.emit(out)
    }

    /// The template in pieces for rich text: runs of text (arguments filled in) and
    /// [`Part::Slot`]s for placeholders no argument fills.
    ///
    /// ```
    /// use i18n_next::{I18n, Part};
    ///
    /// let i18n = I18n::builder().messages("en", None, r#"{"terms": "I accept the {link}, {who}"}"#).build()?;
    /// let parts: Vec<Part> = i18n.text("terms").arg("who", "Ann").parts().collect();
    /// assert_eq!(parts, [Part::Text("I accept the ".into()), Part::Slot("link".into()), Part::Text(", Ann".into())]);
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    pub fn parts(&self) -> impl Iterator<Item = Part<'static>> + use<> {
        self.split(None).into_iter()
    }

    /// Folds the template into your own nodes (JS `foldTemplate`): a placeholder with a
    /// slot becomes the slot's node; else its argument, else the placeholder as written,
    /// joins the surrounding text, which `text` turns into a node. Empty runs are skipped.
    ///
    /// ```
    /// use i18n_next::I18n;
    ///
    /// let i18n = I18n::builder().messages("en", None, r#"{"terms": "I accept the {link}."}"#).build()?;
    /// let nodes = i18n.text("terms").fold(|t| format!("<{t}>"), &[("link", &|| "[terms]".to_owned())]);
    /// assert_eq!(nodes, ["<I accept the >", "[terms]", "<.>"]);
    /// # Ok::<(), i18n_next::Error>(())
    /// ```
    pub fn fold<T>(
        &self,
        mut text: impl FnMut(&str) -> T,
        slots: &[(&str, &dyn Fn() -> T)],
    ) -> Vec<T> {
        let names: Vec<&str> = slots.iter().map(|(n, _)| *n).collect();
        self.split(Some(&names))
            .into_iter()
            .filter_map(|part| match part {
                Part::Text(s) => Some(text(&s)),
                Part::Slot(name) => slots.iter().find(|(n, _)| *n == name).map(|(_, f)| f()),
            })
            .collect()
    }

    /// Text runs and slots: with `slots`, a placeholder is a slot only when named there
    /// (and wins over an argument); with `None` (`parts`), when no argument fills it.
    fn split(&self, slots: Option<&[&str]>) -> Vec<Part<'static>> {
        let mut parts = Vec::new();
        let mut buf = String::new();
        let mut hidden = false;
        let _ = self.with_tpl(&mut |tpl, p| {
            if self.hide && tpl.blank(p)? {
                hidden = true;
                return Ok(());
            }
            tpl.pieces(&mut |piece| {
                match piece {
                    Piece::Lit(s) => buf.push_str(s),
                    Piece::Hole(name) => {
                        let slot = if let Some(slots) = slots
                            && slots.contains(&name)
                        {
                            true
                        } else if slots.is_none() {
                            !p.put(name, &mut buf)?
                        } else {
                            if !p.put(name, &mut buf)? {
                                buf.push('{');
                                buf.push_str(name);
                                buf.push('}');
                            }
                            false
                        };
                        if slot {
                            if !buf.is_empty() {
                                parts.push(Part::Text(Cow::Owned(std::mem::take(&mut buf))));
                            }
                            parts.push(Part::Slot(Cow::Owned(name.to_owned())));
                        }
                    }
                }
                Ok(())
            })
        });
        if hidden {
            parts.clear();
            buf = self.default_text().map(str::to_owned).unwrap_or_default();
        }
        if !buf.is_empty() {
            parts.push(Part::Text(Cow::Owned(buf)));
        }
        parts
    }

    /// The default when it counts (JS `defaultValue ||`).
    fn default_text(&self) -> Option<&str> {
        self.default.as_deref().filter(|d| !d.is_empty())
    }

    /// `Display`: the text, or the default when hidden.
    pub(crate) fn emit(&self, out: &mut dyn fmt::Write) -> fmt::Result {
        let mut hidden = false;
        self.with_tpl(&mut |tpl, p| {
            if self.hide && tpl.blank(p)? {
                hidden = true;
                return Ok(());
            }
            tpl.write(out, p)
        })?;
        match self.default_text() {
            Some(d) if hidden => out.write_str(d),
            _ => Ok(()),
        }
    }

    /// The text in one exactly sized `String` (a nested `Text` may grow it), whether the key
    /// stood in for it, whether it is hidden.
    fn sized(&self) -> (String, bool, bool) {
        let mut out = String::new();
        let mut hidden = false;
        let keyed = self.with_tpl(&mut |tpl, p| {
            if self.hide && tpl.blank(p)? {
                hidden = true;
                return Ok(());
            }
            out.reserve_exact(tpl.measure(p));
            tpl.write(&mut out, p)
        });
        (out, keyed.unwrap_or(false), hidden)
    }

    /// Structural equality (an [`Arg`] holding a `Text`).
    pub(crate) fn same(&self, other: &Text) -> bool {
        let rule = match (&self.rule, &other.rule) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (a, b) => a.is_none() && b.is_none(),
        };
        let i18n = match (&self.i18n, &other.i18n) {
            (Some(a), Some(b)) => Arc::ptr_eq(&a.inner, &b.inner),
            (a, b) => a.is_none() && b.is_none(),
        };
        self.key == other.key
            && i18n
            && rule
            && self.locale == other.locale
            && self.page == other.page
            && self.args.same(&other.args)
            && self.count == other.count
            && self.default == other.default
            && self.html == other.html
            && self.hide == other.hide
    }

    /// Resolves the template and hands it to `f` once, inside the catalog snapshot; then
    /// reports a miss. `Ok(true)`: the key stands in for the text.
    fn with_tpl(
        &self,
        f: &mut dyn FnMut(Tpl<'_>, Params<'_>) -> fmt::Result,
    ) -> Result<bool, fmt::Error> {
        let (scoped, scope_locale, scope_page) = scope::current(
            self.i18n.is_none(),
            self.locale.is_none(),
            self.page.is_none(),
        );
        let locale = self
            .locale
            .as_ref()
            .map(Name::as_str)
            .or(scope_locale.as_deref());
        let page = self
            .page
            .as_ref()
            .map(Name::as_str)
            .or(scope_page.as_deref());
        let key = &*self.key;
        let plain = Params {
            args: &self.args,
            count: None,
            // `escapeParams` builds an object: a list loses `{length}`.
            spread: self.html,
            html: self.html,
            measure: false,
        };
        let params = match self.count {
            Some(c) => Params {
                count: Some(c),
                spread: true,
                ..plain
            },
            None => plain,
        };
        let global: Option<&I18n> = scope::global_instance();
        let Some(i18n) = self.i18n.as_ref().or(scoped.as_ref()).or(global) else {
            scope::uninstalled(key);
            return self
                .fallback(f, params)
                .map(|keyed| keyed && !key.is_empty());
        };
        if let (Some(count), Some(rule)) = (self.count, &self.rule) {
            let active = locale.is_none().then(|| i18n.locale());
            let ctx = PluralCtx {
                key,
                count: count.as_f64(),
                locale: locale.or(active.as_deref()).unwrap_or(""),
                text: self,
                i18n,
                view: locale,
                page,
            };
            let answer = rule(&ctx).unwrap_or_default();
            return f(Tpl::Literal(&answer), params).map(|()| false);
        }
        if key.is_empty() {
            // JS `t('')` is `''`, never a miss.
            return f(Tpl::Literal(""), params).map(|()| false);
        }
        let mut miss = None;
        let mut keyed = false;
        i18n.inner.read(|s| {
            let found = s.lookup(locale, page, key);
            if let Some(count) = self.count {
                let plural = match locale {
                    None => s.plural,
                    Some(code) => s.plural_of(code),
                };
                match found {
                    Some(Entry::Msg(Msg::Text(t))) => {
                        let (start, end) = t.single_range();
                        return f(
                            Tpl::Holes {
                                raw: &t.raw,
                                start,
                                end,
                                holes: t.hole_slice(),
                            },
                            params,
                        );
                    }
                    Some(Entry::Msg(Msg::Plural(p))) => {
                        return match p.form(count, plural) {
                            Some((start, end, holes)) => f(
                                Tpl::Holes {
                                    raw: &p.text.raw,
                                    start,
                                    end,
                                    holes,
                                },
                                params,
                            ),
                            None => f(Tpl::Literal(""), params),
                        };
                    }
                    // a number is its own one form; a boolean or a subtree has none
                    Some(Entry::Value(v)) if !matches!(&**v, "true" | "false") => {
                        return f(Tpl::Literal(v), params);
                    }
                    Some(_) => {}
                    None => miss = Some(s.locale.clone()),
                }
            } else {
                match found {
                    Some(Entry::Msg(Msg::Text(t))) => {
                        return f(
                            Tpl::Holes {
                                raw: &t.raw,
                                start: 0,
                                end: t.raw.len(),
                                holes: t.hole_slice(),
                            },
                            params,
                        );
                    }
                    Some(Entry::Msg(Msg::Plural(p))) => {
                        let t = &p.text;
                        return f(
                            Tpl::Holes {
                                raw: &t.raw,
                                start: 0,
                                end: t.raw.len(),
                                holes: t.hole_slice(),
                            },
                            params,
                        );
                    }
                    Some(Entry::Value(v)) => return f(Tpl::Literal(v), params),
                    // JS shows `String(subtree)`; Rust the key, as `t` does
                    Some(Entry::Node) => {
                        keyed = true;
                        return self.key_tpl(f, params);
                    }
                    None => miss = Some(s.locale.clone()),
                }
            }
            keyed = self.fallback(f, params)?;
            Ok(())
        })?;
        if let Some(active) = miss {
            i18n.report(Some(locale.unwrap_or(&active)), page, key);
        }
        Ok(keyed)
    }

    /// The default, else the key (escaped under `html`), as the template; `Ok(true)` when
    /// it is the key.
    fn fallback(
        &self,
        f: &mut dyn FnMut(Tpl<'_>, Params<'_>) -> fmt::Result,
        params: Params<'_>,
    ) -> Result<bool, fmt::Error> {
        if let Some(d) = self.default_text() {
            return f(Tpl::Scan(d), params).map(|()| false);
        }
        self.key_tpl(f, params).map(|()| true)
    }

    /// The key (escaped under `html`) as the template (JS `keyText`).
    fn key_tpl(
        &self,
        f: &mut dyn FnMut(Tpl<'_>, Params<'_>) -> fmt::Result,
        params: Params<'_>,
    ) -> fmt::Result {
        let key = &*self.key;
        if self.html && needs_escape(key) {
            f(Tpl::Scan(&key_text(key, true)), params)
        } else {
            f(Tpl::Scan(key), params)
        }
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.emit(f)
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Text");
        d.field("key", &self.key);
        if self.i18n.is_some() {
            d.field("instance", &"explicit");
        }
        if let Some(l) = &self.locale {
            d.field("locale", &l.as_str());
        }
        if let Some(p) = &self.page {
            d.field("page", &p.as_str());
        }
        if !self.args.is_empty() {
            d.field("args", &self.args);
        }
        if let Some(c) = &self.count {
            d.field("count", c);
        }
        if let Some(o) = &self.default {
            d.field("or", o);
        }
        if self.rule.is_some() {
            d.field("plural_rule", &"custom");
        }
        if self.html {
            d.field("html", &true);
        }
        if self.hide {
            d.field("hide_if_empty", &true);
        }
        d.finish()
    }
}

/// Compares the displayed text, without building it.
impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        let mut same = Same {
            rest: other,
            ok: true,
        };
        let _ = self.emit(&mut same);
        same.ok && same.rest.is_empty()
    }
}

impl PartialEq<str> for Text {
    fn eq(&self, other: &str) -> bool {
        *self == other
    }
}

impl PartialEq<Text> for &str {
    fn eq(&self, other: &Text) -> bool {
        other == self
    }
}

/// The displayed text as a string (`collect_str`): a command or handler returning a
/// `Text` sends the translation.
#[cfg(feature = "serde")]
impl serde::Serialize for Text {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl I18n {
    /// A [`Text`] of `key` on this instance (the active locale, the root), resolved when
    /// shown; a scope's locale and page apply unless the `Text` sets its own.
    #[must_use]
    pub fn text(&self, key: impl Into<Cow<'static, str>>) -> Text {
        Text::new(key.into(), Some(self.clone()))
    }
}

impl View {
    /// A [`Text`] of `key` in this view's locale and page.
    #[must_use]
    pub fn text(&self, key: impl Into<Cow<'static, str>>) -> Text {
        let mut t = Text::new(key.into(), Some(self.i18n.clone()));
        t.locale = self.locale.clone().map(Name::Shared);
        t.page = self.page.clone().map(Name::Shared);
        t
    }
}
