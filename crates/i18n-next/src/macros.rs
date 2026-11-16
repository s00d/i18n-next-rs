//! `t!`, `tc!`, `key!` (DESIGN 4.4, spike S5): `macro_rules!` only, each builds a lazy
//! [`Text`](crate::Text) (or a [`Key`](crate::Key)) through public methods.

/// A [`Text`](crate::Text): `t!("key", name = value, …)`.
///
/// - `t!("key")`: the implicit instance (nearest [`scope`](crate::scope()), else the
///   global one; without either the key shows, with one debug warning);
/// - `t!(i18n => "key", …)`: an explicit [`I18n`](crate::I18n) or [`View`](crate::View);
/// - `name = value`: the argument `{name}` (`r#type = ..` fills `{type}`); `count = n`:
///   the plural form, as `tc`;
/// - `[a, b]`: positional `{0}`, `{1}`;
/// - options with `@`, so `{page}` stays a parameter: `@page = ..`, `@locale = ..`,
///   `@or = ..` (default), `@html`, `@number = n` (fills `{number}` with `n` formatted in
///   the text's locale, as `<i18n-t :number>`), `@date = t` (`{date}`, `<i18n-t :date>`),
///   `@relative = t` (`{relativeDate}`, `<i18n-t :relative-date>`); the last three need
///   the feature `format`.
///
/// ```
/// use i18n_next::{I18n, t};
///
/// let i18n = I18n::builder()
///     .messages("en", None, r#"{"hi": "Hi, {name}!", "pair": "{0} & {1}", "n": "none | one | {count} items"}"#)
///     .messages("de", Some("cart"), r#"{"n": "keine | eins | {count} Stück"}"#)
///     .build()?;
/// assert_eq!(t!(i18n => "hi", name = "Ann"), "Hi, Ann!");
/// assert_eq!(t!(i18n => "pair", ["A", "B"]), "A & B");
/// assert_eq!(t!(i18n => "n", count = 3), "3 items");
/// assert_eq!(t!(i18n => "n", count = 3, @locale = "de", @page = "cart"), "3 Stück");
/// assert_eq!(t!(i18n => "nope", @or = "Default"), "Default");
/// assert_eq!(t!(i18n => "hi", name = "<b>", @html), "Hi, &lt;b&gt;!");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[macro_export]
macro_rules! t {
    (@opts $t:expr ;) => { $t };
    (@opts $t:expr ; @page = $v:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.page($v) ; $($($rest)*)?)
    };
    (@opts $t:expr ; @locale = $v:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.locale($v) ; $($($rest)*)?)
    };
    (@opts $t:expr ; @or = $v:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.or($v) ; $($($rest)*)?)
    };
    (@opts $t:expr ; @html $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.html() ; $($($rest)*)?)
    };
    (@opts $t:expr ; @number = $v:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.number($v) ; $($($rest)*)?)
    };
    (@opts $t:expr ; @date = $v:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.date($v) ; $($($rest)*)?)
    };
    (@opts $t:expr ; @relative = $v:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.relative($v) ; $($($rest)*)?)
    };
    (@opts $t:expr ; @$other:ident $($rest:tt)*) => {
        ::core::compile_error!(::core::concat!(
            "t!: unknown option `@", ::core::stringify!($other),
            "` (known: @page, @locale, @or, @html, @number, @date, @relative)"
        ))
    };
    (@opts $t:expr ; count = $v:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.count($v) ; $($($rest)*)?)
    };
    (@opts $t:expr ; [$($item:expr),* $(,)?] $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.args({
            let list: [$crate::Arg<'static>; 0 $(+ { let _ = ::core::stringify!($item); 1 })*] =
                [$($crate::Arg::from($item)),*];
            list
        }) ; $($($rest)*)?)
    };
    (@opts $t:expr ; $name:ident = $v:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $t.arg(const { $crate::__ident(::core::stringify!($name)) }, $v) ; $($($rest)*)?)
    };
    ($i18n:expr => $key:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts ($i18n).text($key) ; $($($rest)*)?)
    };
    ($key:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $crate::t($key) ; $($($rest)*)?)
    };
}

/// The argument name of an identifier: `r#type` is `type`.
#[doc(hidden)]
#[must_use]
pub const fn __ident(name: &'static str) -> &'static str {
    match name.as_bytes() {
        [b'r', b'#', rest @ ..] => match core::str::from_utf8(rest) {
            Ok(s) => s,
            Err(_) => name,
        },
        _ => name,
    }
}

/// `t!` with the count second, the JS habit: `tc!("apples", n, name = ..)` is
/// `t!("apples", count = n, name = ..)`; `tc!(i18n => "apples", n)` too.
///
/// ```
/// use i18n_next::{I18n, tc};
///
/// let i18n = I18n::builder().messages("en", None, r#"{"apples": "no apples | one apple | {count} apples"}"#).build()?;
/// assert_eq!(tc!(i18n => "apples", 2), "2 apples");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[macro_export]
macro_rules! tc {
    ($i18n:expr => $key:expr, $n:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts ($i18n).text($key).count($n) ; $($($rest)*)?)
    };
    ($key:expr, $n:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $crate::t($key).count($n) ; $($($rest)*)?)
    };
}

/// A number formatted now (JS `tn`), a `String`: `tn!(n)` and `tn!(n, "name")` (a named
/// format) on the implicit instance, in the scope's locale; `tn!(i18n => n)` and
/// `tn!(i18n => n, "name")` on an explicit [`I18n`](crate::I18n) or [`View`](crate::View).
/// Feature `format`.
///
/// ```
/// use i18n_next::{I18n, tn};
///
/// let i18n = I18n::builder()
///     .formats(r#"{"en": {"eur": {"style": "currency", "currency": "EUR"}}}"#)
///     .build()?;
/// assert_eq!(tn!(i18n => 1234.5), "1,234.5");
/// assert_eq!(tn!(i18n => 1234.5, "eur"), "€1,234.50");
/// assert_eq!(tn!(i18n.with_locale("ru") => 1234.5), "1\u{a0}234,5");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[cfg(feature = "format")]
#[macro_export]
macro_rules! tn {
    ($i18n:expr => $n:expr, $name:expr $(,)?) => {
        ($i18n).tn_as($n, $name)
    };
    ($i18n:expr => $n:expr $(,)?) => {
        ($i18n).tn($n)
    };
    ($n:expr, $name:expr $(,)?) => {
        $crate::tn_as($n, $name)
    };
    ($n:expr $(,)?) => {
        $crate::tn($n)
    };
}

/// A date formatted now (JS `td`), a `String`: `td!(t)` and `td!(t, "name")` (a named
/// format) on the implicit instance, in the scope's locale; `td!(i18n => t)` and
/// `td!(i18n => t, "name")` on an explicit [`I18n`](crate::I18n) or [`View`](crate::View).
/// Feature `format`.
///
/// ```
/// use i18n_next::{I18n, td};
///
/// let i18n = I18n::builder()
///     .time_zone("UTC")
///     .datetime_formats(r#"{"en": {"day": {"weekday": "long"}}}"#)
///     .build()?;
/// assert_eq!(td!(i18n => 1_704_428_640_000i64), "1/5/2024");
/// assert_eq!(td!(i18n => 1_704_428_640_000i64, "day"), "Friday");
/// assert_eq!(td!(i18n.with_locale("ja") => 1_704_428_640_000i64), "2024/1/5");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[cfg(feature = "format")]
#[macro_export]
macro_rules! td {
    ($i18n:expr => $t:expr, $name:expr $(,)?) => {
        ($i18n).td_as($t, $name)
    };
    ($i18n:expr => $t:expr $(,)?) => {
        ($i18n).td($t)
    };
    ($t:expr, $name:expr $(,)?) => {
        $crate::td_as($t, $name)
    };
    ($t:expr $(,)?) => {
        $crate::td($t)
    };
}

/// A date relative to now (JS `tdr`), a `String`: `tdr!(t)` on the implicit instance,
/// `tdr!(i18n => t)` on an explicit [`I18n`](crate::I18n) or [`View`](crate::View).
/// Feature `format`.
///
/// ```
/// use i18n_next::{I18n, tdr};
///
/// let i18n = I18n::builder().clock(|| 1_704_428_640_000).build()?;
/// assert_eq!(tdr!(i18n => 1_704_428_640_000i64 - 3_600_000), "1 hour ago");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[cfg(feature = "format")]
#[macro_export]
macro_rules! tdr {
    ($i18n:expr => $t:expr $(,)?) => {
        ($i18n).tdr($t)
    };
    ($t:expr $(,)?) => {
        $crate::tdr($t)
    };
}

/// A `const` [`Key`](crate::Key): `const QUIT: Key = key!("menu.quit");`.
///
/// ```
/// use i18n_next::{Key, key};
///
/// const QUIT: Key = key!("menu.quit");
/// assert_eq!(QUIT, Key::new("menu.quit"));
/// ```
#[macro_export]
macro_rules! key {
    ($key:expr) => {
        $crate::Key::new($key)
    };
}
