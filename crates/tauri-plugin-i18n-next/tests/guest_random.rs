//! Random catalogs against guest-js (DESIGN 6.2): a fixed-seed generator builds catalogs over
//! the chain `x → de-at → de → en` with pages, `null`s, leaf/object conflicts, own dotted
//! keys, arrays, linked messages and plurals, and records, per catalog, the chunk tags `meta`
//! sends for every view, the protocol chunk bodies and what Rust `try_t` answers for every
//! key in every view. `tests/guest-js/random.test.ts` replays each catalog through `connect`
//! over the protocol and over raw invoke and must answer the same.
//!
//! The rest of `meta` (locales, chains, formats) is the same for every catalog: checked here
//! against one `base`, and the `load` payload against the protocol chunks, so the fixture
//! holds neither. `UPDATE_GOLDEN=1` rewrites `tests/guest-js/fixtures/random.json`.

mod common;

use common::{App, Value, json};
use i18n_next::I18n;
use std::collections::BTreeMap;

const LOCALES: [&str; 4] = ["x", "de-at", "de", "en"];
const PAGES: [&str; 3] = ["index", "p1", "p2"];
/// Root keys of a chunk: plain ones and own dotted ones.
const TOP: [&str; 6] = ["a", "b", "c", "a.b", "b.c", "c.a"];
/// Keys inside an object.
const SUB: [&str; 4] = ["a", "b", "c", "k"];
const CATALOGS: usize = 200;

const CONFIG: &str = r#"{"defaultLocale": "en", "fallbackLocale": "en", "locales": [
    {"code": "x", "fallbackLocale": "de-at"}, {"code": "de-at", "fallbackLocale": "de"},
    {"code": "de"}, {"code": "en"}]}"#;

/// xorshift64*: the same catalogs on every platform and run.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n
    }

    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        from[self.below(from.len())]
    }
}

/// Every key the answers are recorded for: the root keys, two and three levels deep
/// (array indexes too) and a miss.
fn keys() -> Vec<String> {
    let mut out: Vec<String> = TOP.iter().map(|k| (*k).to_owned()).collect();
    for a in ["a", "b", "c"] {
        for b in ["a", "b", "c", "0", "1"] {
            out.push(format!("{a}.{b}"));
        }
    }
    for a in ["a", "b", "c"] {
        for b in ["a", "b", "1"] {
            for c in ["a", "b", "k"] {
                out.push(format!("{a}.{b}.{c}"));
            }
        }
    }
    out.push("nope".to_owned());
    out.sort();
    out.dedup();
    out
}

struct Gen<'k> {
    rng: Rng,
    n: usize,
    keys: &'k [String],
}

impl Gen<'_> {
    fn leaf(&mut self, tag: &str) -> Value {
        self.n += 1;
        let n = self.n;
        match self.rng.below(20) {
            0..=1 => {
                let to = &self.keys[self.rng.below(self.keys.len())];
                json!(format!("L{n} @:{to}"))
            }
            2 => {
                let to = self.rng.pick(&TOP);
                json!(format!("L{n} @:{to}"))
            }
            3 => {
                let to = self.rng.pick(&["a", "b", "c", "a.b"]);
                json!(format!("@.upper:{to}"))
            }
            4 => json!(format!("{{count}} o{n} | {{count}} m{n}")),
            5 => json!(1.5),
            6 => json!(true),
            _ => json!(format!("{tag}{n}")),
        }
    }

    fn value(&mut self, tag: &str, depth: usize) -> Value {
        match self.rng.below(100) {
            0..=14 => Value::Null,
            15..=54 => self.leaf(tag),
            55..=84 if depth < 2 => {
                let mut o = serde_json::Map::new();
                for _ in 0..1 + self.rng.below(3) {
                    let k = self.rng.pick(&SUB);
                    let v = self.value(tag, depth + 1);
                    o.insert(k.to_owned(), v);
                }
                Value::Object(o)
            }
            85..=92 => json!([self.leaf(tag), { "k": self.leaf(tag) }]),
            _ => self.leaf(tag),
        }
    }

    /// One `(locale, page)` file: `None` (no file) about a third of the time.
    fn chunk(&mut self, tag: &str) -> Option<String> {
        if self.rng.below(3) == 0 {
            return None;
        }
        let mut o = serde_json::Map::new();
        for _ in 0..2 + self.rng.below(5) {
            let k = self.rng.pick(&TOP);
            // an own dotted `null` under a weaker container: what a merge must see through
            let v = if k.contains('.') && self.rng.below(4) == 0 {
                Value::Null
            } else {
                self.value(tag, 0)
            };
            o.insert(k.to_owned(), v);
        }
        Some(Value::Object(o).to_string())
    }
}

fn catalog(g: &mut Gen) -> Option<I18n> {
    let mut b = I18n::builder().config_json(CONFIG).linked_messages(true);
    for l in LOCALES {
        for p in PAGES {
            // short and traceable: `a1.7` is the 7th value, of de-at / p1
            let tag = format!(
                "{}{}.",
                &l[l.len() - 1..],
                PAGES.iter().position(|x| *x == p).unwrap_or(0)
            );
            if let Some(text) = g.chunk(&tag) {
                b = b.messages(l, (p != "index").then_some(p), &text);
            }
        }
    }
    // a catalog Rust refuses (a linked cycle, say) is not one guest-js can get
    b.build().ok()
}

fn view(i: &I18n, l: &str, p: &str) -> i18n_next::View {
    if p == "index" {
        i.with_locale(l)
    } else {
        i.with_locale(l).page(p)
    }
}

#[test]
fn guest_random_fixture_is_up_to_date() {
    let keys = keys();
    let mut g = Gen {
        rng: Rng(0x18e8_5eed_0000_0001),
        n: 0,
        keys: &keys,
    };
    let mut base: Option<Value> = None;
    let mut catalogs = Vec::new();
    while catalogs.len() < CATALOGS {
        let Some(i18n) = catalog(&mut g) else {
            continue;
        };
        let app = App::new(i18n.clone());
        // every chunk with its tag, by `locale/page`: one body per pair, whatever the view
        let mut chunks = BTreeMap::<String, Value>::new();
        let mut answers = BTreeMap::new();
        let mut views = Vec::new();
        for l in LOCALES {
            app.ok("set_locale", json!({ "locale": l }));
            for p in PAGES {
                let id = format!("{l}|{p}");
                let args = if p == "index" {
                    json!({})
                } else {
                    json!({ "page": p })
                };
                let v = view(&i18n, l, p);
                // what Rust answers; a key left out is a miss
                let a: BTreeMap<&str, String> = keys
                    .iter()
                    .filter_map(|k| Some((k.as_str(), v.get(k).map(str::to_owned)?)))
                    .collect();
                answers.insert(id.clone(), a);
                // a page no locale has: `meta` refuses it, guest-js reads the root
                let Ok(raw) = app.call_raw("meta", args.clone()) else {
                    views.push((l, p, None));
                    continue;
                };
                let mut meta: Value = serde_json::from_slice(&raw).unwrap();
                let tags = meta["tags"].take();
                for (key, tag) in tags.as_object().unwrap() {
                    let path = format!("{}/{key}.json", tag.as_str().unwrap());
                    let (status, body) = app.get(&format!("/{}", path.replace('/', "%2F")));
                    assert_eq!(status, 200, "{path}");
                    let mut body: Value = serde_json::from_slice(&body).unwrap();
                    body["tag"] = tag.clone();
                    let was = chunks.insert(key.clone(), body.clone());
                    assert!(was.is_none_or(|w| w == body), "{key} differs between views");
                }
                // the raw `load` fallback carries the same chunks
                let load: Value =
                    serde_json::from_slice(&app.call_raw("load", args).unwrap()).unwrap();
                assert_eq!(load["locale"], json!(l));
                for c in load["chunks"].as_array().unwrap() {
                    let key = format!(
                        "{}/{}",
                        c["locale"].as_str().unwrap(),
                        c["page"].as_str().unwrap()
                    );
                    let body = &chunks[&key];
                    assert_eq!(
                        (&c["data"], c.get("linked")),
                        (&body["data"], body.get("linked")),
                        "{id} {key}"
                    );
                }
                assert_eq!(
                    load["chunks"].as_array().unwrap().len(),
                    tags.as_object().unwrap().len()
                );
                views.push((l, p, Some(tags)));
                // the rest of meta is the same for every view of every catalog
                assert_eq!(meta["locale"], json!(l));
                for k in ["version", "locale", "bundle"] {
                    meta.as_object_mut().unwrap().remove(k);
                }
                assert_eq!(base.get_or_insert_with(|| meta.clone()), &meta, "{id}");
            }
        }
        // the tags of a view are the chunks with data along its chain (root, page), and a
        // page is unknown when no locale has it: guest-js rebuilds `meta` from that
        let chains = &base.as_ref().unwrap()["chains"];
        for (l, p, tags) in views {
            let known = p == "index" || chunks.keys().any(|k| k.ends_with(&format!("/{p}")));
            let want = known.then(|| {
                let mut t = serde_json::Map::new();
                for c in chains[l].as_array().unwrap() {
                    for page in if p == "index" {
                        vec!["index"]
                    } else {
                        vec!["index", p]
                    } {
                        let key = format!("{}/{page}", c.as_str().unwrap());
                        if let Some(body) = chunks.get(&key) {
                            t.insert(key, body["tag"].clone());
                        }
                    }
                }
                Value::Object(t)
            });
            assert_eq!(tags, want, "{l}|{p}");
        }
        catalogs.push(json!({ "chunks": chunks, "answers": answers }));
    }
    // one catalog per line: a change shows as the catalogs it touched
    let mut text = format!(
        "{{\"keys\":{},\"base\":{},\"catalogs\":[\n",
        json!(keys),
        base.unwrap()
    );
    for (i, c) in catalogs.iter().enumerate() {
        text += &c.to_string();
        text += if i + 1 < catalogs.len() { ",\n" } else { "\n" };
    }
    text += "]}\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/guest-js/fixtures/random.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        on_disk == text,
        "guest-js random fixture differs (UPDATE_GOLDEN=1 rewrites it)"
    );
}
