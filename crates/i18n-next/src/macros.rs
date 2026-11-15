//! `t!`, `tc!`, `key!` (DESIGN 4.4, spike S5): `macro_rules!` only, each builds a lazy
//! [`Text`](crate::Text) (or a [`Key`](crate::Key)) through public methods.

/// A [`Text`](crate::Text): `t!("key", name = value, …)`.
///
/// - `t!("key")`: the implicit instance (nearest [`scope`](crate::scope()), else the
///   global one; without either the key shows, with one debug warning);
/// - `t!(i18n => "key", …)`: an explicit [`I18n`](crate::I18n) or [`View`](crate::View);
/// - `name = value`: the argument `{name}`; `count = n`: the plural form, as `tc`;
/// - `[a, b]`: positional `{0}`, `{1}`;
/// - options with `@`, so `{page}` stays a parameter: `@page = ..`, `@locale = ..`,
///   `@or = ..` (default), `@html`. `@number` / `@date` / `@relative` are reserved for the
///   formatting stages and do not compile yet.
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
    (@opts $t:expr ; @number $($rest:tt)*) => {
        ::core::compile_error!("t!: `@number` is reserved: number formatting is not there yet (M5)")
    };
    (@opts $t:expr ; @date $($rest:tt)*) => {
        ::core::compile_error!("t!: `@date` is reserved: date formatting is not there yet (M6)")
    };
    (@opts $t:expr ; @relative $($rest:tt)*) => {
        ::core::compile_error!("t!: `@relative` is reserved: relative dates are not there yet (M6)")
    };
    (@opts $t:expr ; @$other:ident $($rest:tt)*) => {
        ::core::compile_error!(::core::concat!(
            "t!: unknown option `@", ::core::stringify!($other),
            "` (known: @page, @locale, @or, @html)"
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
        $crate::t!(@opts $t.arg(::core::stringify!($name), $v) ; $($($rest)*)?)
    };
    ($i18n:expr => $key:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts ($i18n).text($key) ; $($($rest)*)?)
    };
    ($key:expr $(, $($rest:tt)*)?) => {
        $crate::t!(@opts $crate::t($key) ; $($($rest)*)?)
    };
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
