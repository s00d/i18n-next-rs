//! Scope model (DESIGN 7.3, 4.4): random programs of nested `scope().run`, texts shown at
//! every level (implicit `t!`, explicit instances, explicit `.locale` / `.page`), runs that
//! panic, and groups of `wrap`ped tasks polled in a random interleaving on one thread by
//! the manual executor. A reference stack predicts what every text shows: each of
//! instance, locale and page from the nearest frame that sets it, an explicit option on the
//! `Text` over any frame, and the key when no instance is anywhere (no `install` in this
//! binary). After a panic and after each task group the stack is as before: the next text
//! at that level sees the outer frames only.
#![cfg(not(target_arch = "wasm32"))]

mod support;

use i18n_next::{I18n, Text, scope, t};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use support::Executor;

#[derive(Debug, Clone, Default, PartialEq)]
struct Frame {
    instance: Option<usize>,
    locale: Option<&'static str>,
    page: Option<&'static str>,
}

/// A text shown: explicit parts, `None` = taken from the scope.
#[derive(Debug, Clone)]
struct Show {
    instance: Option<usize>,
    locale: Option<&'static str>,
    page: Option<&'static str>,
}

#[derive(Debug, Clone)]
enum Op {
    Show(Show),
    Run(Frame, Vec<Op>),
    /// A run that shows a text, then panics; caught outside it.
    Panic(Frame, Show),
    /// Wrapped tasks (frame, texts shown one per poll), polled in this order.
    Tasks(Vec<(Frame, Vec<Show>)>, Vec<usize>),
}

fn frame() -> impl Strategy<Value = Frame> {
    (
        prop::option::of(0..2usize),
        prop::option::of(prop::sample::select(vec!["en", "de", "xx"])),
        prop::option::of(prop::sample::select(vec!["p", "q", "index"])),
    )
        .prop_map(|(instance, locale, page)| Frame {
            instance,
            locale,
            page,
        })
}

fn show() -> impl Strategy<Value = Show> {
    (
        prop::option::weighted(0.3, 0..2usize),
        prop::option::weighted(0.3, prop::sample::select(vec!["en", "de"])),
        prop::option::weighted(0.3, prop::sample::select(vec!["p", "index"])),
    )
        .prop_map(|(instance, locale, page)| Show {
            instance,
            locale,
            page,
        })
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let leaf = prop_oneof![
        3 => show().prop_map(Op::Show),
        1 => (frame(), show()).prop_map(|(f, s)| Op::Panic(f, s)),
        1 => prop::collection::vec((frame(), prop::collection::vec(show(), 1..4)), 1..4)
            .prop_flat_map(|tasks| {
                let polls: usize = tasks.iter().map(|(_, s)| s.len() + 1).sum();
                let n = tasks.len();
                (Just(tasks), prop::collection::vec(0..n, polls..polls * 2))
            })
            .prop_map(|(tasks, order)| Op::Tasks(tasks, order)),
    ];
    let op = leaf.prop_recursive(3, 24, 4, |inner| {
        (frame(), prop::collection::vec(inner, 0..4)).prop_map(|(f, ops)| Op::Run(f, ops))
    });
    prop::collection::vec(op, 1..6)
}

/// Two instances: `who` is `<instance>:<locale>:<page>` in every chunk; active `en` / `de`.
fn instances() -> [I18n; 2] {
    let one = |name: &str, active: &str| {
        let mut b = I18n::builder().locale(active).fallback("en");
        for locale in ["en", "de"] {
            for page in [None, Some("p")] {
                let json = format!(r#"{{"who": "{name}:{locale}:{}"}}"#, page.unwrap_or("root"));
                b = b.messages(locale, page, &json);
            }
        }
        b.build().unwrap()
    };
    [one("A", "en"), one("B", "de")]
}

const ACTIVE: [&str; 2] = ["en", "de"];

/// What a text shows on top of `stack`.
fn expect(stack: &[Frame], s: &Show) -> String {
    let near = |f: fn(&Frame) -> Option<&'static str>| stack.iter().rev().find_map(f);
    let Some(i) = s
        .instance
        .or_else(|| stack.iter().rev().find_map(|f| f.instance))
    else {
        return "who".to_owned();
    };
    let locale = s.locale.or(near(|f| f.locale)).unwrap_or(ACTIVE[i]);
    // an unknown locale reads the global fallback
    let locale = if locale == "xx" { "en" } else { locale };
    let page = match s.page.or(near(|f| f.page)) {
        Some("p") => "p",
        _ => "root",
    };
    format!("{}:{locale}:{page}", ["A", "B"][i])
}

fn text(all: &[I18n; 2], s: &Show) -> Text {
    let mut t = match s.instance {
        Some(i) => all[i].text("who"),
        None => t!("who"),
    };
    if let Some(l) = s.locale {
        t = t.locale(l);
    }
    if let Some(p) = s.page {
        t = t.page(p);
    }
    t
}

fn enter(all: &[I18n; 2], f: &Frame) -> i18n_next::Scope {
    let mut sc = scope();
    if let Some(i) = f.instance {
        sc = sc.instance(all[i].clone());
    }
    if let Some(l) = f.locale {
        sc = sc.locale(l);
    }
    if let Some(p) = f.page {
        sc = sc.page(p);
    }
    sc
}

/// Pending once, waking itself: the next poll goes on.
struct Yield(bool);

impl Future for Yield {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

fn exec(all: &[I18n; 2], stack: &mut Vec<Frame>, ops: &[Op]) -> Result<(), TestCaseError> {
    for op in ops {
        match op {
            Op::Show(s) => {
                prop_assert_eq!(
                    text(all, s).to_string(),
                    expect(stack, s),
                    "{:?} on {:?}",
                    s,
                    stack
                );
            }
            Op::Run(f, inner) => {
                stack.push(f.clone());
                let r = enter(all, f).run(|| exec(all, stack, inner));
                stack.pop();
                r?;
            }
            Op::Panic(f, s) => {
                stack.push(f.clone());
                let shown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    enter(all, f).run(|| {
                        let shown = text(all, s).to_string();
                        std::panic::panic_any(shown)
                    })
                }));
                let shown = *shown.unwrap_err().downcast::<String>().unwrap();
                prop_assert_eq!(shown, expect(stack, s));
                stack.pop();
                // step: the stack is clean → the same text sees the outer frames only
                prop_assert_eq!(text(all, s).to_string(), expect(stack, s));
            }
            Op::Tasks(tasks, order) => {
                let logs: Vec<Rc<RefCell<Vec<String>>>> =
                    tasks.iter().map(|_| Rc::default()).collect();
                let mut ex = Executor::new();
                for ((f, shows), log) in tasks.iter().zip(&logs) {
                    let (log, shows) = (log.clone(), shows.clone());
                    let all = all.clone();
                    ex.spawn(enter(&all, f).wrap(async move {
                        for s in &shows {
                            log.borrow_mut().push(text(&all, s).to_string());
                            Yield(false).await;
                        }
                    }));
                }
                for &i in order {
                    ex.poll(i);
                    // between polls the stack is the outer one
                    let probe = Show {
                        instance: None,
                        locale: None,
                        page: None,
                    };
                    prop_assert_eq!(t!("who").to_string(), expect(stack, &probe));
                }
                ex.run_until_stalled();
                prop_assert_eq!(ex.unfinished(), 0);
                for ((f, shows), log) in tasks.iter().zip(&logs) {
                    stack.push(f.clone());
                    let want: Vec<String> = shows.iter().map(|s| expect(stack, s)).collect();
                    stack.pop();
                    prop_assert_eq!(&*log.borrow(), &want, "task {:?}", f);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn scope_matches_the_model() {
    let all = instances();
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let config = Config {
        cases: 384,
        failure_persistence: None,
        max_shrink_time: 30_000,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &[45; 32]));
    let result = runner.run(&ops(), |ops| exec(&all, &mut Vec::new(), &ops));
    std::panic::set_hook(quiet);
    result.unwrap();
    // nothing left on the stack
    assert_eq!(t!("who").to_string(), "who");
    assert_eq!(i18n_next::locale(), None);
}

/// Given an explicit instance, locale and page on a `Text`, when it is shown inside scopes
/// that set others, then the explicit ones win; a `View`'s locale and page are explicit.
#[test]
fn explicit_options_beat_the_scope() {
    let [a, b] = instances();
    scope().instance(b.clone()).locale("de").page("p").run(|| {
        // step 1: implicit → everything from the scope
        assert_eq!(t!("who"), "B:de:p");
        // step 2: explicit instance → its texts, the scope's locale and page
        assert_eq!(a.text("who"), "A:de:p");
        // step 3: explicit locale / page / both on the macro and the method
        assert_eq!(t!("who", @locale = "en"), "B:en:p");
        assert_eq!(t!(a => "who", @page = "index"), "A:de:root");
        assert_eq!(a.text("who").locale("en").page("index"), "A:en:root");
        // step 4: a view carries its own locale and page
        assert_eq!(a.with_locale("en").text("who"), "A:en:p");
        assert_eq!(a.with_locale("en").page("index").text("who"), "A:en:root");
        // step 5: the free functions read the same frame
        assert_eq!(i18n_next::locale().as_deref(), Some("de"));
        assert_eq!(i18n_next::page("index").unwrap().t("who"), "B:de:root");
        // step 6: an inner frame overrides one part, inherits the rest
        scope().locale("en").run(|| assert_eq!(t!("who"), "B:en:p"));
        assert_eq!(t!("who"), "B:de:p");
    });
    // step 7: outside every scope, without install: the key
    assert_eq!(t!("who"), "who");
    assert!(i18n_next::page("x").is_none());
    assert!(matches!(
        i18n_next::set_locale("de"),
        Err(i18n_next::Error::NotInstalled)
    ));
}
