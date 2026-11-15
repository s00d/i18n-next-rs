//! criterion benches of the own `json` reader and `hash` hasher against the dependencies
//! they replace (DESIGN 2.3, 7.7, spikes S2 and S6). Ids are `group/function/case`.
//!
//! - `json/reader/<n>`: the own reader straight into a flat map of dotted keys;
//! - `json/serde_value/<n>`: `serde_json::Value`, then flattened into the same map;
//! - `json/serde_visitor/<n>`: a serde visitor straight into the same map (spike S6);
//! - `json/write_str/locale_text`: the string writer of the payload;
//! - `hash/<hasher>/short_key`: one `hash_one` of a locale key;
//!   `hash/<hasher>/lookup_10k`: 100 hits in a map of 10k such keys.
//!
//! The modules are crate-private: they are included with `#[path]`, so the public API
//! stays as it is.

#[path = "../src/hash.rs"]
#[allow(dead_code)]
mod hash;
#[path = "../src/json.rs"]
#[allow(dead_code)]
mod json;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use hash::{FastHasher, FastMap};
use json::{Event, Reader};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;
use std::hash::{BuildHasher, BuildHasherDefault};
use std::hint::black_box;

type Flat = FastMap<String, String>;
type ToFlat = fn(&[u8]) -> Flat;

/// A locale file with about `leaves` strings: sections of 10 groups of 10 keys, texts
/// with a placeholder, every 16th with an escape (as real files have a few).
fn locale_doc(leaves: usize) -> Vec<u8> {
    let mut s = String::from("{\n");
    let sections = leaves.div_ceil(100);
    for sec in 0..sections {
        s.push_str(&format!("  \"section_{sec}\": {{\n"));
        for g in 0..10 {
            s.push_str(&format!("    \"group{g}\": {{\n"));
            for k in 0..10 {
                let n = sec * 100 + g * 10 + k;
                let text = if n % 16 == 0 {
                    format!("Line one\\nline \\\"{n}\\\" caf\\u00e9")
                } else {
                    format!("Welcome back, {{name}}! You have {n} new messages")
                };
                let comma = if k < 9 { "," } else { "" };
                s.push_str(&format!("      \"item_{k}_label\": \"{text}\"{comma}\n"));
            }
            s.push_str(if g < 9 { "    },\n" } else { "    }\n" });
        }
        s.push_str(if sec + 1 < sections {
            "  },\n"
        } else {
            "  }\n"
        });
    }
    s.push('}');
    s.into_bytes()
}

/// The own reader into a flat map: the path is one buffer, truncated on the way out.
fn reader_flat(input: &[u8]) -> Flat {
    let mut map = Flat::default();
    let mut path = String::new();
    let mut lens: Vec<usize> = Vec::new();
    let mut r = Reader::new(input);
    while let Some(ev) = r.next().unwrap() {
        match ev {
            Event::Key(k) => {
                lens.push(path.len());
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(&k);
            }
            Event::Str(s) => {
                map.insert(path.clone(), s.into_owned());
                pop(&mut path, &mut lens);
            }
            Event::ObjectEnd => pop(&mut path, &mut lens),
            Event::ObjectStart => {}
            _ => unreachable!("locale documents hold objects and strings"),
        }
    }
    map
}

fn pop(path: &mut String, lens: &mut Vec<usize>) {
    if let Some(l) = lens.pop() {
        path.truncate(l);
    }
}

fn flatten(v: &serde_json::Value, path: &mut String, map: &mut Flat) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                let l = path.len();
                if l > 0 {
                    path.push('.');
                }
                path.push_str(k);
                flatten(v, path, map);
                path.truncate(l);
            }
        }
        serde_json::Value::String(s) => {
            map.insert(path.clone(), s.clone());
        }
        _ => unreachable!(),
    }
}

fn serde_value_flat(input: &[u8]) -> Flat {
    let v: serde_json::Value = serde_json::from_slice(input).unwrap();
    let mut map = Flat::default();
    flatten(&v, &mut String::new(), &mut map);
    map
}

/// Spike S6: a visitor straight into the flat map.
struct FlatSeed<'m> {
    path: &'m mut String,
    map: &'m mut Flat,
}

impl<'de> DeserializeSeed<'de> for FlatSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for FlatSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object of strings")
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<(), E> {
        self.map.insert(self.path.clone(), v.to_owned());
        Ok(())
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<(), E> {
        self.map.insert(self.path.clone(), v);
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
        while let Some(k) = a.next_key::<std::borrow::Cow<'de, str>>()? {
            let l = self.path.len();
            if l > 0 {
                self.path.push('.');
            }
            self.path.push_str(&k);
            a.next_value_seed(FlatSeed {
                path: self.path,
                map: self.map,
            })?;
            self.path.truncate(l);
        }
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<(), A::Error> {
        unreachable!()
    }
}

fn serde_visitor_flat(input: &[u8]) -> Flat {
    let mut map = Flat::default();
    let mut path = String::new();
    let mut de = serde_json::Deserializer::from_slice(input);
    FlatSeed {
        path: &mut path,
        map: &mut map,
    }
    .deserialize(&mut de)
    .unwrap();
    de.end().unwrap();
    map
}

fn json(c: &mut Criterion) {
    let docs: Vec<(&str, Vec<u8>)> = vec![
        ("1k", locale_doc(1_000)),
        ("10k", locale_doc(10_000)),
        ("100k", locale_doc(100_000)),
    ];
    // The three paths build the same map.
    for (_, d) in &docs {
        let a = reader_flat(d);
        assert_eq!(a, serde_value_flat(d));
        assert_eq!(a, serde_visitor_flat(d));
    }
    let runs: [(&str, ToFlat); 3] = [
        ("json/reader", reader_flat),
        ("json/serde_value", serde_value_flat),
        ("json/serde_visitor", serde_visitor_flat),
    ];
    for (group, f) in runs {
        let mut g = c.benchmark_group(group);
        for (size, d) in &docs {
            g.throughput(Throughput::Bytes(d.len() as u64));
            g.bench_with_input(BenchmarkId::from_parameter(size), d, |b, d| {
                b.iter(|| f(black_box(d)))
            });
        }
        g.finish();
    }
    let text = "Welcome back, \"{name}\"!\nYou have {count} new messages \u{2014} caf\u{e9}";
    let mut out = String::with_capacity(128);
    c.bench_function("json/write_str/locale_text", |b| {
        b.iter(|| {
            out.clear();
            json::write_str(&mut out, black_box(text));
            black_box(out.len())
        })
    });
}

fn hashers(c: &mut Criterion) {
    let keys: Vec<String> = (0..10_000).map(|i| format!("nav.form.item_{i}")).collect();
    let probe = "nav.form.item_4321";
    fn bench<S: BuildHasher + Clone>(
        c: &mut Criterion,
        name: &str,
        s: S,
        keys: &[String],
        probe: &str,
    ) {
        let mut g = c.benchmark_group(format!("hash/{name}"));
        let h = s.clone();
        g.bench_function("short_key", |b| b.iter(|| h.hash_one(black_box(probe))));
        let mut map = std::collections::HashMap::with_hasher(s);
        for (i, k) in keys.iter().enumerate() {
            map.insert(k.as_str(), i);
        }
        // 100 different hits per iteration: one key's bucket neighbourhood does not
        // decide the number (time / 100 = ns per lookup).
        let probes: Vec<&str> = keys
            .iter()
            .step_by(97)
            .take(100)
            .map(String::as_str)
            .collect();
        g.throughput(Throughput::Elements(probes.len() as u64));
        g.bench_function("lookup_10k", |b| {
            b.iter(|| probes.iter().filter_map(|p| map.get(black_box(*p))).count())
        });
        g.finish();
    }
    bench(
        c,
        "fast_hasher",
        BuildHasherDefault::<FastHasher>::default(),
        &keys,
        probe,
    );
    bench(
        c,
        "std_sip",
        std::collections::hash_map::RandomState::new(),
        &keys,
        probe,
    );
    bench(
        c,
        "foldhash",
        foldhash::fast::FixedState::default(),
        &keys,
        probe,
    );
    bench(c, "rustc_hash", rustc_hash::FxBuildHasher, &keys, probe);
}

criterion_group!(benches, json, hashers);
criterion_main!(benches);
