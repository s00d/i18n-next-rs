//! Linked-message model (DESIGN 7.3, M3): random catalogs full of links, checked against a
//! naive reference of the Nuxt semantics. The reference premerges each `(locale, page)`
//! view the way `mergeSourceTranslations` does (the chain's root and page files, weakest
//! first) and resolves every message by plain recursion with a stack of keys being resolved
//! (no cache, no layers, no stored texts), with ASCII case rules plus the Turkish `i`.
//!
//! Keys are flat (no subtrees, no `null`), so the premerge and the crate's layer walk read
//! the same values: any difference is the resolution's. After `build`: every view × key
//! answers like the reference (`try_t`), `has` sees only the locale's own keys, the same
//! catalog without `linked_messages` reads the raw premerge, and every `BrokenLink` of
//! `check()` names a link its message still shows. Two counters prove the space reaches
//! what matters: answers where a view's result differs from the stored result of the layer
//! the key comes from, and cycles.
#![cfg(not(target_arch = "wasm32"))]

use i18n_next::{CheckItem, I18n};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

const KEYS: &[&str] = &["a", "b", "c", "d", "e"];
/// `None` is the root; `nopage` has no file anywhere and reads the root.
const PAGES: &[Option<&str>] = &[None, Some("p"), Some("q"), Some("nopage")];

static VIEW_DIFFERS: AtomicUsize = AtomicUsize::new(0);
static CYCLES: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone)]
enum Value {
    Text(String),
    Num(u32),
}

/// `(locale, page)` → key → value.
type Files = BTreeMap<(String, Option<String>), BTreeMap<String, Value>>;

#[derive(Debug, Clone)]
struct Case {
    files: Files,
    /// `tr` falls back to `de` before `en`.
    tr_via_de: bool,
}

fn segment() -> impl Strategy<Value = String> {
    let target = prop::sample::select(vec!["a", "b", "c", "d", "e", "zz"]);
    prop_oneof![
        4 => prop::sample::select(vec!["x", "i", "I", " ", ".", "!", "{n}", "{N}", "{ x}", "|", "@", "Hi "])
            .prop_map(str::to_owned),
        2 => target.clone().prop_map(|k| format!("@:{k}")),
        1 => target.clone().prop_map(|k| format!("@:{{'{k}'}}")),
        2 => (prop::sample::select(vec!["upper", "lower", "capitalize", "bad"]), target)
            .prop_map(|(m, k)| format!("@.{m}:{k}")),
    ]
}

fn value() -> impl Strategy<Value = Value> {
    prop_oneof![
        8 => prop::collection::vec(segment(), 0..5).prop_map(|s| Value::Text(s.concat())),
        1 => (0u32..100).prop_map(Value::Num),
    ]
}

fn chunk() -> impl Strategy<Value = BTreeMap<String, Value>> {
    prop::collection::btree_map(
        prop::sample::select(KEYS).prop_map(str::to_owned),
        value(),
        0..5,
    )
}

fn case() -> impl Strategy<Value = Case> {
    let slots: Vec<(String, Option<String>)> = ["en", "de", "tr"]
        .iter()
        .flat_map(|l| [None, Some("p"), Some("q")].map(|p| (l.to_string(), p.map(str::to_owned))))
        .collect();
    (
        prop::collection::vec(prop::option::weighted(0.6, chunk()), slots.len()),
        any::<bool>(),
    )
        .prop_map(move |(chunks, tr_via_de)| {
            let mut files: Files = slots
                .iter()
                .cloned()
                .zip(chunks)
                .filter_map(|(slot, c)| c.map(|c| (slot, c)))
                .collect();
            // every locale has at least a root file
            for l in ["en", "de", "tr"] {
                files.entry((l.to_owned(), None)).or_default();
            }
            Case { files, tr_via_de }
        })
}

fn chain(case: &Case, locale: &str) -> Vec<&'static str> {
    match locale {
        "en" => vec!["en"],
        "de" => vec!["de", "en"],
        _ if case.tr_via_de => vec!["tr", "de", "en"],
        _ => vec!["tr", "en"],
    }
}

/// The premerged view: weakest first, root then page, later wins.
fn premerge(case: &Case, locale: &str, page: Option<&str>) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for code in chain(case, locale).into_iter().rev() {
        for p in [None, page.filter(|p| *p != "nopage")] {
            if let Some(c) = case.files.get(&(code.to_owned(), p.map(str::to_owned))) {
                for (k, v) in c {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
    }
    out
}

fn upper(text: &str, turkish: bool) -> String {
    text.chars()
        .map(|c| match c {
            'i' if turkish => 'İ',
            'ı' => 'I',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

fn lower(text: &str, turkish: bool) -> String {
    text.chars()
        .flat_map(|c| match c {
            'I' if turkish => vec!['ı'],
            'İ' if turkish => vec!['i'],
            'İ' => vec!['i', '\u{307}'],
            c => vec![c.to_ascii_lowercase()],
        })
        .collect()
}

/// Splits `text` into (is_placeholder, part): `{` + no `{ } space` + `}`.
fn parts(text: &str) -> Vec<(bool, String)> {
    let mut out = vec![(false, String::new())];
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let mut j = i + 1;
            while j < chars.len() && !matches!(chars[j], '{' | '}' | ' ') {
                j += 1;
            }
            if j < chars.len() && chars[j] == '}' && j > i + 1 {
                out.push((true, chars[i..=j].iter().collect()));
                out.push((false, String::new()));
                i = j + 1;
                continue;
            }
        }
        out.last_mut().unwrap().1.push(chars[i]);
        i += 1;
    }
    out
}

fn modify(text: &str, m: &str, turkish: bool) -> String {
    parts(text)
        .into_iter()
        .enumerate()
        .map(|(i, (hole, p))| match (hole, m) {
            (true, _) => p,
            (false, "upper") => upper(&p, turkish),
            (false, "lower") => lower(&p, turkish),
            (false, _) if i == 0 => {
                let mut cs = p.chars();
                match cs.next() {
                    Some(c) => upper(&c.to_string(), turkish) + cs.as_str(),
                    None => p,
                }
            }
            (false, _) => p,
        })
        .collect()
}

/// A link at the start of `s`: (length in chars, modifier, key).
fn link_at(s: &[char]) -> Option<(usize, Option<String>, String)> {
    let mut i = 1;
    let mut modifier = None;
    if s.get(i) == Some(&'.') {
        let start = i + 1;
        let mut j = start;
        while s.get(j).is_some_and(char::is_ascii_alphabetic) {
            j += 1;
        }
        if j == start || s.get(j) != Some(&':') {
            return None;
        }
        modifier = Some(s[start..j].iter().collect());
        i = j + 1;
    } else if s.get(i) == Some(&':') {
        i += 1;
    } else {
        return None;
    }
    if s.get(i) == Some(&'{') && s.get(i + 1) == Some(&'\'') {
        let start = i + 2;
        let end = (start..s.len()).find(|&k| s[k] == '\'')?;
        if s.get(end + 1) != Some(&'}') {
            return None;
        }
        return Some((end + 2, modifier, s[start..end].iter().collect()));
    }
    let start = i;
    let mut j = start;
    while s
        .get(j)
        .is_some_and(|c| !c.is_whitespace() && !"{}@%|()".contains(*c))
    {
        j += 1;
    }
    (j > start).then(|| (j, modifier, s[start..j].iter().collect()))
}

/// The naive resolution: `Some(text)` or `None` (left as written).
fn resolve(
    view: &BTreeMap<String, Value>,
    key: &str,
    stack: &mut Vec<String>,
    turkish: bool,
) -> Option<String> {
    let text = match view.get(key)? {
        Value::Num(n) => return Some(n.to_string()),
        Value::Text(t) => t.clone(),
    };
    stack.push(key.to_owned());
    let chars: Vec<char> = text.chars().collect();
    let (mut out, mut ok, mut i) = (String::new(), true, 0);
    while i < chars.len() {
        if chars[i] == '@'
            && let Some((len, m, target)) = link_at(&chars[i..])
        {
            let written: String = chars[i..i + len].iter().collect();
            i += len;
            let known = m
                .as_deref()
                .is_none_or(|m| ["upper", "lower", "capitalize"].contains(&m));
            let got = if !known {
                None
            } else if stack.contains(&target) {
                CYCLES.fetch_add(1, Ordering::Relaxed);
                None
            } else {
                resolve(view, &target, stack, turkish)
            };
            match got {
                Some(t) => out.push_str(&m.map_or(t.clone(), |m| modify(&t, &m, turkish))),
                None => {
                    ok = false;
                    out.push_str(&written);
                }
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    stack.pop();
    ok.then_some(out)
}

/// What `t` reads in a view: resolved, or the message as written.
fn answer(view: &BTreeMap<String, Value>, key: &str, turkish: bool) -> Option<String> {
    let raw = match view.get(key)? {
        Value::Num(n) => return Some(n.to_string()),
        Value::Text(t) => t.clone(),
    };
    Some(resolve(view, key, &mut Vec::new(), turkish).unwrap_or(raw))
}

fn json(chunk: &BTreeMap<String, Value>) -> String {
    let map: serde_json::Map<String, serde_json::Value> = chunk
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Value::Text(t) => serde_json::Value::from(t.as_str()),
                Value::Num(n) => serde_json::Value::from(*n),
            };
            (k.clone(), v)
        })
        .collect();
    serde_json::Value::Object(map).to_string()
}

fn build(case: &Case, linked: bool) -> I18n {
    let mut b = I18n::builder()
        .locale("en")
        .fallback("en")
        .fallback_for("de", "en")
        .iso("tr", "tr-TR")
        .linked_messages(linked);
    if case.tr_via_de {
        b = b.fallback_for("tr", "de");
    }
    for ((l, p), c) in &case.files {
        b = b.messages(l, p.as_deref(), &json(c));
    }
    b.build().unwrap()
}

fn run(case: &Case) -> Result<(), TestCaseError> {
    let on = build(case, true);
    let off = build(case, false);
    for locale in ["en", "de", "tr"] {
        for page in PAGES {
            let merged = premerge(case, locale, *page);
            let v_on = on.with_locale(locale).page(page.unwrap_or(""));
            let v_off = off.with_locale(locale).page(page.unwrap_or(""));
            for key in KEYS.iter().chain(&["zz"]) {
                let want = answer(&merged, key, locale == "tr");
                prop_assert_eq!(
                    v_on.get(key).map(str::to_owned),
                    want.clone(),
                    "{} {:?} {}",
                    locale,
                    page,
                    key
                );
                let raw = merged.get(*key).map(|v| match v {
                    Value::Text(t) => t.clone(),
                    Value::Num(n) => n.to_string(),
                });
                prop_assert_eq!(v_off.get(key).map(str::to_owned), raw.clone());
                // a view's answer that the layer's own stored result could not give
                let own_layer = chain(case, locale).into_iter().find_map(|code| {
                    [page.filter(|p| *p != "nopage"), None]
                        .into_iter()
                        .find_map(|p| {
                            case.files
                                .get(&(code.to_owned(), p.map(str::to_owned)))
                                .filter(|c| c.contains_key(*key))
                                .map(|_| (code, p))
                        })
                });
                if let Some((code, p)) = own_layer
                    && answer(&premerge(case, code, p), key, code == "tr") != want
                {
                    VIEW_DIFFERS.fetch_add(1, Ordering::Relaxed);
                }
                // `has`: the locale's own page and root files only
                let own = [page.filter(|p| *p != "nopage"), None]
                    .into_iter()
                    .any(|p| {
                        case.files
                            .get(&(locale.to_owned(), p.map(str::to_owned)))
                            .is_some_and(|c| c.contains_key(*key))
                    });
                prop_assert_eq!(v_on.has(key), own, "has {} {:?} {}", locale, page, key);
            }
        }
    }
    for item in on.check().items {
        if let CheckItem::BrokenLink {
            locale,
            page,
            key,
            link,
            ..
        } = item
        {
            let text = on
                .with_locale(&locale)
                .page(&page)
                .get(&key)
                .map(str::to_owned)
                .unwrap_or_default();
            prop_assert!(
                text.contains(&link),
                "{} {} {}: {} lacks {}",
                locale,
                page,
                key,
                text,
                link
            );
        }
    }
    Ok(())
}

#[test]
fn linked_messages_match_the_model() {
    let config = Config {
        cases: 384,
        failure_persistence: None,
        max_shrink_time: 30_000,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &[23; 32]));
    runner.run(&case(), |c| run(&c)).unwrap();
    let differs = VIEW_DIFFERS.load(Ordering::Relaxed);
    let cycles = CYCLES.load(Ordering::Relaxed);
    eprintln!("answers a view resolves apart from the layer's own: {differs}, cycles: {cycles}");
    assert!(differs > 0, "no view needed its own resolution");
    assert!(cycles > 0, "no cycle was drawn");
}
