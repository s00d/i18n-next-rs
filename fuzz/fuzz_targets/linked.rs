//! Random catalogs full of links (`@:k`, modifiers, literal keys, cycles, case-sensitive
//! text in a Turkish and a Greek locale, plurals, nested keys): `build` with
//! `linked_messages(true)` never panics and ends (libFuzzer's `-timeout` fails a hang), and
//! every view answers. Each input byte pair is one message: which chunk and key, then a
//! value made of the following bytes up to a `;`.
#![no_main]

use i18n_next::I18n;
use libfuzzer_sys::fuzz_target;
use serde_json::{Map, Value};

const LOCALES: [&str; 4] = ["en", "de", "tr", "el"];
const PAGES: [Option<&str>; 3] = [None, Some("p"), Some("q")];
const KEYS: [&str; 8] = ["a", "b", "c", "n.x", "n.y", "list.0", "", "a b"];
const PIECES: [&str; 24] = [
    "@:a", "@:b", "@:c", "@:n.x", "@:{'n.y'}", "@:{'a b'}", "@:{''}", "@:n", "@:list.0",
    "@.upper:a", "@.lower:b", "@.capitalize:c", "@.bad:a", "@:a.", "@@:b", "{name}", "{count}",
    " | ", "istanbul", "IŞIK", "άυλος ή", "ΣΟΦΟΣ", "@", "x",
];

fuzz_target!(|data: &[u8]| {
    let mut chunks: Vec<Map<String, Value>> = vec![Map::new(); LOCALES.len() * PAGES.len()];
    for record in data.split(|&b| b == b';').take(64) {
        let [slot, key, rest @ ..] = record else {
            continue;
        };
        let text: String = rest
            .iter()
            .map(|&b| PIECES[usize::from(b) % PIECES.len()])
            .collect();
        let value = if rest.first() == Some(&255) {
            Value::from(u64::from(*key))
        } else {
            Value::from(text)
        };
        let at = usize::from(*slot) % chunks.len();
        let chunk = &mut chunks[at];
        let key = KEYS[usize::from(*key) % KEYS.len()];
        match key.split_once('.') {
            // a nested key as JSON nests it, so the walk and own keys both occur
            Some((head, tail)) if slot & 0x80 != 0 => {
                let inner = chunk
                    .entry(head.to_owned())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Value::Object(m) = inner {
                    m.insert(tail.to_owned(), value);
                }
            }
            _ => {
                chunk.insert(key.to_owned(), value);
            }
        }
    }
    let mut b = I18n::builder()
        .locale("en")
        .fallback("en")
        .fallback_for("de", "en")
        .fallback_for("tr", "de")
        .iso("tr", "tr-TR")
        .linked_messages(true);
    for (i, chunk) in chunks.iter().enumerate() {
        if !chunk.is_empty() {
            let (locale, page) = (LOCALES[i / PAGES.len()], PAGES[i % PAGES.len()]);
            b = b.messages(locale, page, &Value::Object(chunk.clone()).to_string());
        }
    }
    let Ok(i18n) = b.build() else {
        return;
    };
    for locale in LOCALES {
        for page in ["", "p", "q", "none"] {
            let view = i18n.with_locale(locale).page(page);
            for key in KEYS {
                let _ = view.t(key, str::to_owned);
                let _ = view.tc(key, 2);
            }
        }
    }
    let _ = i18n.check().to_string();
});
