//! Catalog sources shared by the benches (benches/compare.rs, benches/memory.rs): the same
//! `s{sec}.g{g}.k{k}` leaves as the main crate's lookup bench, as JSON, FTL and flat pairs.

/// The flat `(key, value)` pairs of a catalog with `keys` leaves `s{sec}.g{g}.k{k}`, the
/// shape of the main crate's lookup bench and of `locales/*.json`.
pub fn pairs(keys: usize, word: &str) -> Vec<(String, String)> {
    (0..keys)
        .map(|i| {
            let (s, g, k) = (i / 100, i / 10 % 10, i % 10);
            (
                format!("s{s}.g{g}.k{k}"),
                format!("{word} text {s}.{g}.{k}"),
            )
        })
        .collect()
}

/// The pairs as a nested JSON document for i18n-next.
pub fn json(pairs: &[(String, String)], extra: &str) -> String {
    let mut s = String::from("{");
    let mut open: [Option<String>; 2] = [None, None];
    for (key, value) in pairs {
        let mut parts = key.split('.');
        let (sec, grp, leaf) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        if open[0].as_deref() != Some(sec) {
            if open[0].is_some() {
                s.push_str("}},");
            }
            s.push_str(&format!("\"{sec}\":{{\"{grp}\":{{"));
            open = [Some(sec.into()), Some(grp.into())];
        } else if open[1].as_deref() != Some(grp) {
            s.push_str(&format!("}},\"{grp}\":{{"));
            open[1] = Some(grp.into());
        } else {
            s.push(',');
        }
        s.push_str(&format!("\"{leaf}\":\"{value}\""));
    }
    if open[0].is_some() {
        s.push_str("}}");
    }
    if !extra.is_empty() {
        if s.len() > 1 {
            s.push(',');
        }
        s.push_str(extra);
    }
    s.push('}');
    s
}

/// The pairs as FTL (`s3-g4-k5 = en text 3.4.5`).
pub fn ftl(pairs: &[(String, String)], extra: &str) -> String {
    let mut s = String::new();
    for (key, value) in pairs {
        s.push_str(&key.replace('.', "-"));
        s.push_str(" = ");
        s.push_str(value);
        s.push('\n');
    }
    s.push_str(extra);
    s
}
