//! CLDR number data (DESIGN 2.4, 5.2): reads `src/format/data/numbers.txt` (and, with the
//! `currency-names` feature, `currency-names.txt`), written by `scripts/cldr.mjs`, keeps the
//! locales of `I18N_NEXT_LOCALES` with their CLDR parent chains, and writes them to
//! `$OUT_DIR/numbers.rs` as Rust statics for `src/format/data.rs`: no decoder at run time.
//!
//! `I18N_NEXT_LOCALES`: unset, empty or `modern`, the CLDR `modern` locales; `all`, every locale of the
//! file; else a comma list (`en,de,ru`, `zh-TW`): each tag is matched like the run-time
//! resolution (exact, script from the region, then shorter), an unknown one fails the build.
//! `und` (the base of every chain) and `en` (where an unknown tag goes, like `FormatService`)
//! are always in.
//!
//! Patterns are parsed here: affixes come out with the locale's minus and percent signs in
//! place (`¤` stays, for the currency), grouping sizes and zeros as numbers. Strings are
//! interned; regional variants keep only the currency symbols (and names) that differ from
//! their parent, the formatter walks the chain for those.
//!
//! Dates (M6): `src/format/data/dates.txt` the same way, resolved through the parents: per
//! locale a row of patterns (every option combination of `format/date.rs`), a row of names,
//! a row of relative-time strings and the overrides, each row interned. Patterns come
//! compiled: a field is `\x01`, its CLDR letter and its width as a digit (`\x01M4`), `{0}` /
//! `{1}` of a glue is `\x01{0` / `\x01{1`, the rest is literal text.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

const CATS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

#[derive(Default, Clone)]
struct Raw {
    parent: Option<String>,
    fields: BTreeMap<String, String>,
    compact: Option<Vec<(u8, u8, String)>>,
    units: Option<Vec<(u8, String)>>,
    /// code, symbol, narrow, pattern, decimal, group
    currencies: Vec<[String; 6]>,
    names: Vec<(String, u8, String)>,
}

pub(crate) struct File {
    modern: Vec<String>,
    edge: String,
    fractions: Vec<(String, u8)>,
    /// key (`iw`, `hy-arevmda`), replacement, applies to the bare lowercase key too
    aliases: Vec<(String, String, bool)>,
    /// default numbering systems other than `latn`: name, digits
    numbering: Vec<(String, String)>,
    regions: Vec<(String, String)>,
    scripts: Vec<(String, String)>,
    locales: BTreeMap<String, Raw>,
}

fn cat(s: &str) -> u8 {
    CATS.iter()
        .position(|c| *c == s)
        .unwrap_or_else(|| panic!("plural category {s}")) as u8
}

pub(crate) fn parse(text: &str, names: Option<&str>) -> File {
    let mut f = File {
        modern: Vec::new(),
        edge: String::new(),
        fractions: Vec::new(),
        aliases: Vec::new(),
        numbering: Vec::new(),
        regions: Vec::new(),
        scripts: Vec::new(),
        locales: BTreeMap::new(),
    };
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((kind, rest)) = line.split_once(' ')
            && !kind.contains('\t')
        {
            let words: Vec<&str> = rest.split(' ').collect();
            match kind {
                "modern" => f.modern = words.iter().map(|s| s.to_string()).collect(),
                "edge" => f.edge = rest.to_owned(),
                "fraction" => f
                    .fractions
                    .push((words[0].into(), words[1].parse().unwrap())),
                "alias" => f.aliases.push((
                    words[0].into(),
                    words[1].into(),
                    words.get(2) == Some(&"lone"),
                )),
                "numbering" => f.numbering.push((words[0].into(), words[1].into())),
                "region" => f.regions.push((words[0].into(), words[1].into())),
                "script" => f.scripts.push((words[0].into(), words[1].into())),
                "locale" => {
                    let raw = Raw {
                        parent: (words[1] != "-").then(|| words[1].to_owned()),
                        ..Raw::default()
                    };
                    f.locales.insert(words[0].to_owned(), raw);
                    current = Some(words[0].to_owned());
                }
                _ => panic!("numbers.txt: unknown line {line}"),
            }
            continue;
        }
        let raw = f
            .locales
            .get_mut(current.as_deref().expect("a field before any locale"))
            .unwrap();
        let cols: Vec<&str> = line.split('\t').collect();
        match cols[0] {
            "compact" => {
                let list = raw.compact.get_or_insert_with(Vec::new);
                if cols.len() == 4 {
                    list.push((cols[1].parse().unwrap(), cat(cols[2]), cols[3].to_owned()));
                }
            }
            "unitPattern" => {
                let list = raw.units.get_or_insert_with(Vec::new);
                if cols.len() == 3 {
                    list.push((cat(cols[1]), cols[2].to_owned()));
                }
            }
            "currency" => raw.currencies.push(std::array::from_fn(|i| {
                cols.get(i + 1).copied().unwrap_or("").to_owned()
            })),
            key => {
                raw.fields.insert(key.to_owned(), cols[1].to_owned());
            }
        }
    }
    if let Some(names) = names {
        let mut current: Option<String> = None;
        for line in names.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("locale ") {
                current = Some(rest.split(' ').next().unwrap().to_owned());
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            let raw = f.locales.get_mut(current.as_deref().unwrap()).unwrap();
            raw.names
                .push((cols[0].into(), cat(cols[1]), cols[2].to_owned()));
        }
    }
    f
}

/// Interned strings: index 0 is `""`.
struct Strings {
    text: String,
    ends: Vec<u32>,
    index: HashMap<String, u32>,
}

impl Strings {
    fn new() -> Strings {
        let mut s = Strings {
            text: String::new(),
            ends: Vec::new(),
            index: HashMap::new(),
        };
        s.add("");
        s
    }

    fn add(&mut self, v: &str) -> u32 {
        if let Some(&i) = self.index.get(v) {
            return i;
        }
        self.text.push_str(v);
        let i = self.ends.len() as u32;
        self.ends.push(self.text.len() as u32);
        self.index.insert(v.to_owned(), i);
        i
    }
}

/// A pattern parsed: affixes (`¤` kept), grouping sizes, minimum integer digits / zeros.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Pat {
    pp: String,
    ps: String,
    np: String,
    ns: String,
    g1: u8,
    g2: u8,
    zeros: u8,
}

/// `pos;neg` split at an unquoted `;`.
fn split_unquoted(p: &str, sep: char) -> (&str, Option<&str>) {
    let mut quoted = false;
    for (i, c) in p.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            c if c == sep && !quoted => return (&p[..i], Some(&p[i + 1..])),
            _ => {}
        }
    }
    (p, None)
}

/// prefix, number part, suffix of one subpattern.
fn sub(p: &str) -> (String, String, String) {
    let mut quoted = false;
    let (mut start, mut end) = (None, 0);
    for (i, c) in p.char_indices() {
        if c == '\'' {
            quoted = !quoted;
        } else if !quoted && "#0,.@".contains(c) {
            start.get_or_insert(i);
            end = i + 1;
        } else if start.is_some() && !quoted {
            break;
        }
    }
    let start = start.unwrap_or(p.len());
    let end = end.max(start);
    (p[..start].into(), p[start..end].into(), p[end..].into())
}

/// An affix with quotes removed and the signs of the locale in place.
fn affix(raw: &str, minus: &str, percent: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if chars.peek() == Some(&'\'') => {
                chars.next();
                out.push('\'');
            }
            '\'' => quoted = !quoted,
            '-' if !quoted => out.push_str(minus),
            '%' if !quoted => out.push_str(percent),
            c => out.push(c),
        }
    }
    out
}

fn pattern(p: &str, minus: &str, percent: &str) -> Pat {
    let (pos, neg) = split_unquoted(p, ';');
    let (pp, num, ps) = sub(pos);
    let int = num.split('.').next().unwrap_or("");
    let groups: Vec<&str> = int.split(',').collect();
    let digits = |s: &str| s.chars().filter(|c| matches!(c, '#' | '0')).count() as u8;
    let (g1, g2) = match groups.len() {
        0 | 1 => (0, 0),
        n => {
            let g1 = digits(groups[n - 1]);
            let g2 = if n > 2 { digits(groups[n - 2]) } else { g1 };
            (g1, g2)
        }
    };
    let zeros = int.chars().filter(|&c| c == '0').count() as u8;
    let pp = affix(&pp, minus, percent);
    let ps = affix(&ps, minus, percent);
    let (np, ns) = match neg {
        Some(neg) => {
            let (np, _, ns) = sub(neg);
            (affix(&np, minus, percent), affix(&ns, minus, percent))
        }
        None => (format!("{minus}{pp}"), ps.clone()),
    };
    Pat {
        pp,
        ps,
        np,
        ns,
        g1,
        g2,
        zeros,
    }
}

/// A locale with every field resolved through its parents.
#[derive(Clone, Default)]
struct Resolved {
    fields: BTreeMap<String, String>,
    compact: Vec<(u8, u8, String)>,
    units: Vec<(u8, String)>,
}

fn resolve(f: &File, name: &str, memo: &mut HashMap<String, Resolved>) -> Resolved {
    if let Some(r) = memo.get(name) {
        return r.clone();
    }
    let raw = &f.locales[name];
    let mut r = match &raw.parent {
        Some(p) => resolve(f, p, memo),
        None => Resolved::default(),
    };
    for (k, v) in &raw.fields {
        if v.is_empty() {
            r.fields.remove(k);
        } else {
            r.fields.insert(k.clone(), v.clone());
        }
    }
    if let Some(c) = &raw.compact {
        r.compact = c.clone();
    }
    if let Some(u) = &raw.units {
        r.units = u.clone();
    }
    memo.insert(name.to_owned(), r.clone());
    r
}

/// The data locale a requested tag names, as `format/locale.rs` resolves it at run time:
/// language aliases (`iw` → `he`, `hy-arevmda` → `hyw`), region aliases, the script the region
/// implies (`-`: the root), then exact, `lang-Script`, `lang-REGION`, `lang`.
pub(crate) fn find(f: &File, tag: &str) -> Option<String> {
    let tag = tag.trim().replace('_', "-");
    let mut parts = tag.split('-');
    let mut lang = parts.next()?.to_ascii_lowercase();
    let (mut script, mut region) = (None, None);
    let mut variants = Vec::new();
    for p in parts {
        if p.len() == 1 {
            break;
        }
        if p.len() == 4
            && p.bytes().all(|b| b.is_ascii_alphabetic())
            && script.is_none()
            && region.is_none()
            && variants.is_empty()
        {
            script = Some(format!(
                "{}{}",
                p[..1].to_ascii_uppercase(),
                p[1..].to_ascii_lowercase()
            ));
        } else if matches!(p.len(), 2 | 3) && region.is_none() && variants.is_empty() {
            region = Some(p.to_ascii_uppercase());
        } else {
            variants.push(p.to_ascii_lowercase());
        }
    }
    let alias = variants
        .iter()
        .find_map(|v| {
            let key = format!("{lang}-{v}");
            f.aliases.iter().find(|(k, ..)| *k == key)
        })
        .or_else(|| {
            f.aliases
                .iter()
                .find(|(k, _, lone)| *k == lang && (*lone || tag != lang))
        });
    if let Some((_, to, _)) = alias {
        let mut to = to.split('-');
        lang = to.next().unwrap_or("").to_owned();
        for p in to {
            if p.len() == 4 && script.is_none() {
                script = Some(p.to_owned());
            } else if region.is_none() {
                region = Some(p.to_owned());
            }
        }
    }
    if let Some(r) = &region
        && let Some((_, to)) = f.regions.iter().find(|(k, _)| k == r)
    {
        region = Some(to.clone());
    }
    if script.is_none()
        && let Some(r) = &region
    {
        let key = format!("{lang}-{r}");
        script = f
            .scripts
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, s)| s.clone());
    }
    if script.as_deref() == Some("-") {
        return Some("und".into());
    }
    let mut cands = Vec::new();
    if let (Some(s), Some(r)) = (&script, &region) {
        cands.push(format!("{lang}-{s}-{r}"));
    }
    if let Some(s) = &script {
        cands.push(format!("{lang}-{s}"));
    }
    if let Some(r) = &region {
        cands.push(format!("{lang}-{r}"));
    }
    cands.push(lang);
    cands.into_iter().find(|c| f.locales.contains_key(c))
}

/// `(key, row)` pairs sorted by key (which is byte order of the names): the length, the keys
/// and the rows, as two arrays (a `(u128, u16)` pair would pad to 32 bytes).
fn keyed(list: Vec<(&str, String)>) -> (usize, String, String) {
    let mut sorted: Vec<(u128, String)> = list.into_iter().map(|(k, v)| (key(k), v)).collect();
    sorted.sort();
    let keys = sorted
        .iter()
        .map(|(k, _)| format!("{k:#x}"))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = sorted
        .iter()
        .map(|(_, v)| v.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    (sorted.len(), keys, rows)
}

/// A name of at most 16 bytes as a number that sorts like its bytes (big-endian, zero-padded):
/// `format/locale.rs` binary-searches these.
fn key(name: &str) -> u128 {
    let b = name.as_bytes();
    assert!(b.len() <= 16, "{name}: over 16 bytes, no key");
    let mut a = [0u8; 16];
    a[..b.len()].copy_from_slice(b);
    u128::from_be_bytes(a)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/format/data/numbers.txt");
    println!("cargo:rerun-if-changed=src/format/data/currency-names.txt");
    println!("cargo:rerun-if-changed=src/format/data/dates.txt");
    println!("cargo:rerun-if-env-changed=I18N_NEXT_LOCALES");
    if std::env::var_os("CARGO_FEATURE_FORMAT").is_none() {
        return;
    }
    let names = std::env::var_os("CARGO_FEATURE_CURRENCY_NAMES").is_some();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/format/data");
    let text = std::fs::read_to_string(dir.join("numbers.txt")).expect("numbers.txt");
    let names_text = names.then(|| {
        std::fs::read_to_string(dir.join("currency-names.txt")).expect("currency-names.txt")
    });
    let f = parse(&text, names_text.as_deref());
    let date_text = std::fs::read_to_string(dir.join("dates.txt")).expect("dates.txt");
    let date_file = parse_dates(&date_text);

    // ---- selection ----
    let wanted = std::env::var("I18N_NEXT_LOCALES").unwrap_or_default();
    let mut selected: BTreeSet<String> = BTreeSet::new();
    match wanted.trim() {
        "" | "modern" => selected.extend(f.modern.iter().cloned()),
        "all" => selected.extend(f.locales.keys().cloned()),
        list => {
            for tag in list.split(',').filter(|t| !t.trim().is_empty()) {
                match find(&f, tag) {
                    // `und` only where a tag's region leads ICU to the root (`ku-IQ`)
                    Some(l) if l != "und" || !tag.trim().eq_ignore_ascii_case("und") => {
                        selected.insert(l);
                    }
                    _ => panic!(
                        "I18N_NEXT_LOCALES: no CLDR number data for \"{}\"",
                        tag.trim()
                    ),
                }
            }
        }
    }
    selected.insert("en".into());
    let mut closure: BTreeSet<String> = BTreeSet::new();
    for l in &selected {
        let mut at = Some(l.clone());
        while let Some(l) = at {
            at = f.locales[&l].parent.clone();
            closure.insert(l);
        }
    }
    // Sorted by bytes (BTreeSet of String): binary search at run time.
    let order: Vec<&String> = closure.iter().collect();
    let pos: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, l)| (l.as_str(), i))
        .collect();

    // ---- tables ----
    let mut s = Strings::new();
    let mut ns = Strings::new();
    let mut pats: Vec<Pat> = Vec::new();
    let mut pat_index: HashMap<Pat, usize> = HashMap::new();
    let mut pat_id = |p: Pat| -> usize {
        *pat_index.entry(p.clone()).or_insert_with(|| {
            pats.push(p);
            pats.len() - 1
        })
    };
    let mut compact: Vec<String> = Vec::new();
    let mut compact_lists: HashMap<Vec<String>, usize> = HashMap::new();
    let mut currencies: Vec<String> = Vec::new();
    let mut names_out: Vec<String> = Vec::new();
    let mut memo = HashMap::new();
    let mut locs = String::new();
    let mut dates = Dates::new();
    let mut date_memo = HashMap::new();
    for l in &order {
        let date_fields = dates.locale(&resolve_dates(&date_file, l, &mut date_memo));
        let r = resolve(&f, l, &mut memo);
        let raw = &f.locales[*l];
        let get = |k: &str| r.fields.get(k).map(String::as_str).unwrap_or("");
        let minus = get("minus");
        let percent = get("percent");
        let decimal_pat = pat_id(pattern(get("decimalPattern"), minus, percent));
        let percent_pat = pat_id(pattern(get("percentPattern"), minus, percent));
        let currency_pat = pat_id(pattern(get("currencyPattern"), minus, percent));
        let mut entries = Vec::new();
        for (mag, c, p) in &r.compact {
            let pat = pattern(p, minus, percent);
            // `0`: no compact form at this magnitude (ICU's USE_FALLBACK)
            let zeros = if p == "0" { 0 } else { pat.zeros };
            entries.push(format!(
                "Compact {{ mag: {mag}, cat: {c}, pp: {}, ps: {}, np: {}, ns: {}, zeros: {zeros} }}",
                s.add(&pat.pp),
                s.add(&pat.ps),
                s.add(&pat.np),
                s.add(&pat.ns)
            ));
        }
        let compact_at = match compact_lists.get(&entries) {
            Some(&at) => at,
            None => {
                let at = compact.len();
                compact.extend(entries.iter().cloned());
                compact_lists.insert(entries.clone(), at);
                at
            }
        };
        let cur_at = currencies.len();
        let mut own = raw.currencies.clone();
        own.sort();
        for [code, sym, narrow, pat, decimal, group] in &own {
            let pattern = if pat.is_empty() {
                u16::MAX as usize
            } else {
                pat_id(pattern(pat, minus, percent))
            };
            currencies.push(format!(
                "Currency {{ code: *b\"{code}\", symbol: {}, narrow: {}, pattern: {pattern}, decimal: {}, group: {} }}",
                s.add(sym),
                s.add(narrow),
                // a currency's own format: `.` and `,` unless it says otherwise (as ICU)
                s.add(if decimal.is_empty() && !pat.is_empty() { "." } else { decimal }),
                s.add(if group.is_empty() && !pat.is_empty() { "," } else { group }),
            ));
        }
        let names_at = names_out.len();
        if names {
            let mut own = raw.names.clone();
            own.sort();
            for (code, c, name) in &own {
                names_out.push(format!(
                    "Name {{ code: *b\"{code}\", cat: {c}, name: {} }}",
                    ns.add(name)
                ));
            }
        }
        let mut units = [0u32; 6];
        for (c, p) in &r.units {
            units[*c as usize] = s.add(p);
        }
        let parent = raw.parent.as_deref().map_or(u16::MAX as usize, |p| pos[p]);
        let min_grouping: u8 = get("minGrouping").parse().unwrap_or(1);
        let units_field = if names {
            format!(
                ", units: {units:?}, names: {names_at}, names_len: {}, name_cur: {}",
                names_out.len() - names_at,
                get("nameCurrency") == "1"
            )
        } else {
            String::new()
        };
        writeln!(
            locs,
            "    Loc {{ parent: {parent}, digits: {}, decimal: {}, group: {}, cur_decimal: {}, cur_group: {}, nan: {}, infinity: {}, \
             decimal_pat: {decimal_pat}, percent_pat: {percent_pat}, currency_pat: {currency_pat}, \
             min_grouping: {min_grouping}, compact: {compact_at}, compact_len: {}, currencies: {cur_at}, \
             currencies_len: {}, autonym: {}, plural: {}{units_field}, {date_fields} }},",
            s.add(get("digits")),
            s.add(get("decimal")),
            s.add(get("group")),
            s.add(match get("currencyDecimal") { "" => get("decimal"), d => d }),
            s.add(match get("currencyGroup") { "" => get("group"), g => g }),
            s.add(get("nan")),
            s.add(get("infinity")),
            entries.len(),
            currencies.len() - cur_at,
            s.add(get("name")),
            get("plural").parse::<u8>().expect("plural rule of a locale"),
        )
        .unwrap();
    }
    let mut pat_lines = String::new();
    for p in &pats {
        writeln!(
            pat_lines,
            "    Pat {{ pp: {}, ps: {}, np: {}, ns: {}, g1: {}, g2: {} }},",
            s.add(&p.pp),
            s.add(&p.ps),
            s.add(&p.np),
            s.add(&p.ns),
            p.g1,
            p.g2
        )
        .unwrap();
    }
    // Tables searched by `key`, replacement strings interned.
    // Only rows that can change which selected locale a tag gets: an alias from or to a
    // selected language, a script of one, a region alias onto a region they use.
    let lang_of = |t: &str| t.split('-').next().unwrap_or("").to_owned();
    let langs: BTreeSet<String> = order.iter().map(|l| lang_of(l)).collect();
    let alias_rows: Vec<(&str, String)> = f
        .aliases
        .iter()
        .filter(|(k, v, _)| langs.contains(&lang_of(k)) || langs.contains(&lang_of(v)))
        .map(|(k, v, lone)| (k.as_str(), format!("({}, {lone})", s.add(v))))
        .collect();
    let (aliases_len, alias_keys, aliases) = keyed(alias_rows);
    let script_rows: Vec<(&str, String)> = f
        .scripts
        .iter()
        .filter(|(k, _)| langs.contains(&lang_of(k)))
        .map(|(k, v)| (k.as_str(), s.add(v).to_string()))
        .collect();
    let used_regions: BTreeSet<&str> = order
        .iter()
        .flat_map(|l| l.split('-').skip(1))
        .chain(script_rows.iter().filter_map(|(k, _)| k.split('-').nth(1)))
        .collect();
    let region_rows: Vec<(&str, String)> = f
        .regions
        .iter()
        .filter(|(_, v)| used_regions.contains(v.as_str()))
        .map(|(k, v)| (k.as_str(), s.add(v).to_string()))
        .collect();
    let (regions_len, region_keys, regions) = keyed(region_rows);
    let (scripts_len, script_keys, scripts) = keyed(script_rows);
    // the numbering systems of the selected locales' digits (`numberingSystem` at its default)
    let mut numbering: Vec<(u32, u32)> = Vec::new();
    for (name, digits) in &f.numbering {
        if let Some(&d) = s.index.get(digits) {
            numbering.push((d, s.add(name)));
        }
    }
    numbering.sort();
    assert!(s.ends.len() <= u16::MAX as usize, "more than 65535 strings");
    assert!(order.len() < u16::MAX as usize, "too many locales");

    let mut out = String::new();
    out.push_str("// generated by build.rs from src/format/data/numbers.txt — do not edit\n");
    let _ = writeln!(out, "pub(crate) static STR: &str = {:?};", s.text);
    let _ = writeln!(
        out,
        "pub(crate) static ENDS: [u32; {}] = {:?};",
        s.ends.len(),
        s.ends
    );
    let _ = writeln!(
        out,
        "pub(crate) static LOCALES: [Loc; {}] = [\n{locs}];",
        order.len()
    );
    let _ = writeln!(
        out,
        "pub(crate) static PATTERNS: [Pat; {}] = [\n{pat_lines}];",
        pats.len()
    );
    let _ = writeln!(
        out,
        "pub(crate) static COMPACT: [Compact; {}] = [{}];",
        compact.len(),
        compact.join(", ")
    );
    let _ = writeln!(
        out,
        "pub(crate) static CURRENCIES: [Currency; {}] = [{}];",
        currencies.len(),
        currencies.join(", ")
    );
    let mut fractions = f.fractions.clone();
    fractions.sort();
    let _ = writeln!(
        out,
        "pub(crate) static FRACTIONS: [([u8; 3], u8); {}] = [{}];",
        fractions.len(),
        fractions
            .iter()
            .map(|(c, d)| format!("(*b\"{c}\", {d})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let _ = writeln!(
        out,
        "pub(crate) static ALIASES_KEYS: [u128; {aliases_len}] = [{alias_keys}];\npub(crate) static ALIASES: [(u16, bool); {aliases_len}] = [{aliases}];"
    );
    let _ = writeln!(
        out,
        "pub(crate) static REGIONS_KEYS: [u128; {regions_len}] = [{region_keys}];\npub(crate) static REGIONS: [u16; {regions_len}] = [{regions}];"
    );
    let _ = writeln!(
        out,
        "pub(crate) static SCRIPTS_KEYS: [u128; {scripts_len}] = [{script_keys}];\npub(crate) static SCRIPTS: [u16; {scripts_len}] = [{scripts}];"
    );
    let _ = writeln!(out, "pub(crate) static EDGE: &str = {:?};", f.edge);
    let _ = writeln!(out, "pub(crate) const EN: usize = {};", pos["en"]);
    let _ = writeln!(out, "pub(crate) const UND: usize = {};", pos["und"]);
    let _ = writeln!(
        out,
        "pub(crate) static LOCALE_KEYS: [u128; {}] = [{}];",
        order.len(),
        order
            .iter()
            .map(|l| format!("{:#x}", key(l)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let _ = writeln!(
        out,
        "pub(crate) static NUMBERING: [(u16, u16); {}] = {numbering:?};",
        numbering.len()
    );
    if names {
        let _ = writeln!(out, "pub(crate) static NAME_STR: &str = {:?};", ns.text);
        let _ = writeln!(
            out,
            "pub(crate) static NAME_ENDS: [u32; {}] = {:?};",
            ns.ends.len(),
            ns.ends
        );
        let _ = writeln!(
            out,
            "pub(crate) static NAMES: [Name; {}] = [{}];",
            names_out.len(),
            names_out.join(", ")
        );
    }
    out.push_str(&dates.finish());
    let path = Path::new(&std::env::var("OUT_DIR").unwrap()).join("numbers.rs");
    std::fs::write(path, out).unwrap();
}

// ---- dates (M6) ----

/// `dates.txt`: per locale its parent and the fields that differ from it (`-`: removed).
fn parse_dates(text: &str) -> HashMap<String, (Option<String>, BTreeMap<String, String>)> {
    let mut out: HashMap<String, (Option<String>, BTreeMap<String, String>)> = HashMap::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("locale ") {
            let mut w = rest.split(' ');
            current = w.next().unwrap().to_owned();
            let parent = w.next().filter(|p| *p != "-").map(str::to_owned);
            out.insert(current.clone(), (parent, BTreeMap::new()));
            continue;
        }
        let (k, v) = line.split_once('\t').expect("dates.txt: key<tab>value");
        out.get_mut(&current)
            .unwrap()
            .1
            .insert(k.to_owned(), v.to_owned());
    }
    out
}

fn resolve_dates(
    f: &HashMap<String, (Option<String>, BTreeMap<String, String>)>,
    name: &str,
    memo: &mut HashMap<String, BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    if let Some(r) = memo.get(name) {
        return r.clone();
    }
    let (parent, own) = f
        .get(name)
        .unwrap_or_else(|| panic!("dates.txt: no locale {name}"));
    let mut r = parent
        .as_deref()
        .map(|p| resolve_dates(f, p, memo))
        .unwrap_or_default();
    for (k, v) in own {
        if v == "-" {
            r.remove(k);
        } else {
            r.insert(k.clone(), v.clone());
        }
    }
    memo.insert(name.to_owned(), r.clone());
    r
}

/// A CLDR pattern compiled for `format/date.rs` (see the module docs).
fn compile(p: &str) -> String {
    let c: Vec<char> = p.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\'' if c.get(i + 1) == Some(&'\'') => {
                out.push('\'');
                i += 2;
            }
            '\'' => {
                i += 1;
                while i < c.len() {
                    if c[i] == '\'' {
                        if c.get(i + 1) == Some(&'\'') {
                            out.push('\'');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    out.push(c[i]);
                    i += 1;
                }
            }
            '{' if matches!(c.get(i + 1), Some('0' | '1')) && c.get(i + 2) == Some(&'}') => {
                out.push('\u{1}');
                out.push('{');
                out.push(c[i + 1]);
                i += 3;
            }
            ch if ch.is_ascii_alphabetic() => {
                let mut j = i;
                while j < c.len() && c[j] == ch {
                    j += 1;
                }
                assert!(j - i <= 9, "{p}: field too long");
                out.push('\u{1}');
                out.push(ch);
                out.push(char::from(b'0' + (j - i) as u8));
                i = j;
            }
            '\u{1}' => panic!("{p}: control character"),
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// Slots of the two pattern rows, as `format/date.rs` reads them. The date row: date
/// combinations, the date styles, the glues of components and of styles. The time row: time
/// combinations without an hour, the five hour tables (`j` as ICU picks, `K` `h` `H` `k`
/// forced), the time styles per table. Split, so that locales sharing their times (most
/// regional variants) share that row.
const MODES: [&str; 5] = ["j", "K", "h", "H", "k"];
const DATE_ROW: usize = 228;
const TIME_ROW: usize = 109;
fn pattern_slots() -> (Vec<String>, Vec<String>) {
    let mut date = vec![String::new(); DATE_ROW];
    for (i, k) in date.iter_mut().enumerate().take(216).skip(1) {
        *k = format!("d{i}");
    }
    for w in 0..4 {
        date[216 + w] = format!("ds{w}");
        date[220 + w] = format!("g{w}");
        date[224 + w] = format!("gs{w}");
    }
    let mut time = vec![String::new(); TIME_ROW];
    for (i, k) in time.iter_mut().enumerate().take(9).skip(1) {
        *k = format!("t{i}");
    }
    for (m, mode) in MODES.iter().enumerate() {
        for i in 0..18 {
            time[9 + m * 18 + i] = format!("{mode}{i}");
        }
        for w in 0..2 {
            time[99 + m * 2 + w] = format!("ts{mode}{}", w + 2);
        }
    }
    (date, time)
}
/// Name lists of a names row, in order: months (12 each), weekdays (7), AM / PM, eras (BC, AD).
const NAME_SETS: [(&str, usize); 20] = [
    ("M.fa", 12),
    ("M.fw", 12),
    ("M.fn", 12),
    ("M.sa", 12),
    ("M.sw", 12),
    ("M.sn", 12),
    ("E.fa", 7),
    ("E.fw", 7),
    ("E.fn", 7),
    ("E.fs", 7),
    ("E.sa", 7),
    ("E.sw", 7),
    ("E.sn", 7),
    ("E.ss", 7),
    ("a.a", 2),
    ("a.w", 2),
    ("a.n", 2),
    ("G.a", 2),
    ("G.w", 2),
    ("G.n", 2),
];
const REL_UNITS: [&str; 6] = ["year", "month", "day", "hour", "minute", "second"];
const REL_STYLES: [&str; 3] = ["long", "short", "narrow"];

struct Dates {
    pats: Strings,
    names: Strings,
    rows: Vec<Vec<u16>>,
    row_index: HashMap<(u8, Vec<u16>), usize>,
    pat_rows: Vec<usize>,
    time_rows: Vec<usize>,
    rel_parts: Vec<usize>,
    name_rows: Vec<usize>,
    rel_rows: Vec<usize>,
    b_rows: Vec<usize>,
    over: Vec<(u32, u32)>,
    over_index: HashMap<Vec<(u32, u32)>, usize>,
    slots: (Vec<String>, Vec<String>),
}

impl Dates {
    fn new() -> Dates {
        Dates {
            pats: Strings::new(),
            names: Strings::new(),
            rows: Vec::new(),
            row_index: HashMap::new(),
            pat_rows: Vec::new(),
            time_rows: Vec::new(),
            rel_parts: Vec::new(),
            name_rows: Vec::new(),
            rel_rows: Vec::new(),
            b_rows: Vec::new(),
            over: Vec::new(),
            over_index: HashMap::new(),
            slots: pattern_slots(),
        }
    }

    /// The index of `row` among the rows of its kind (interned).
    fn row(&mut self, kind: u8, row: Vec<u16>) -> usize {
        let list = match kind {
            0 => &mut self.pat_rows,
            1 => &mut self.name_rows,
            2 => &mut self.rel_rows,
            3 => &mut self.b_rows,
            4 => &mut self.time_rows,
            _ => &mut self.rel_parts,
        };
        *self
            .row_index
            .entry((kind, row.clone()))
            .or_insert_with(|| {
                self.rows.push(row);
                list.push(self.rows.len() - 1);
                list.len() - 1
            })
    }

    fn pat(&mut self, p: &str) -> u16 {
        let i = self.pats.add(&compile(p));
        u16::try_from(i).expect("more than 65535 date patterns")
    }

    fn name(&mut self, s: &str) -> u16 {
        u16::try_from(self.names.add(s)).expect("more than 65535 date strings")
    }

    /// The date fields of one `Loc`.
    fn locale(&mut self, r: &BTreeMap<String, String>) -> String {
        let slots = std::mem::take(&mut self.slots);
        let pats: Vec<u16> = slots
            .0
            .iter()
            .map(|k| r.get(k).map_or(0, |p| self.pat(p)))
            .collect();
        let times: Vec<u16> = slots
            .1
            .iter()
            .map(|k| r.get(k).map_or(0, |p| self.pat(p)))
            .collect();
        self.slots = slots;
        let mut names = Vec::new();
        for (k, n) in NAME_SETS {
            let list: Vec<&str> = r.get(k).map_or_else(Vec::new, |v| v.split('\t').collect());
            assert!(
                list.is_empty() || list.len() == n,
                "{k}: {} names",
                list.len()
            );
            for i in 0..n {
                names.push(self.name(list.get(i).copied().unwrap_or("")));
            }
        }
        let mut rel = Vec::new();
        for u in REL_UNITS {
            for st in REL_STYLES {
                let v = r
                    .get(&format!("r.{u}.{st}"))
                    .unwrap_or_else(|| panic!("r.{u}.{st}"));
                let list: Vec<&str> = v.split('\t').collect();
                assert_eq!(list.len(), 17, "r.{u}.{st}");
                let part: Vec<u16> = list.into_iter().map(|s| self.name(s)).collect();
                // one unit and style: shared between locales (and units) that say it alike
                let part = self.row(5, part);
                rel.push(u16::try_from(part).expect("relative parts"));
            }
        }
        let b = if r.contains_key("B.a") {
            let mut row = Vec::new();
            for w in ["B.a", "B.w", "B.n"] {
                let list: Vec<&str> = r[w].split('\t').collect();
                assert_eq!(list.len(), 25, "{w}");
                for s in list {
                    row.push(self.name(s));
                }
            }
            self.row(3, row)
        } else {
            u16::MAX as usize
        };
        // overrides: `x<date>.<time>.<mode>` (kind 0) and `y<dateStyle>.<timeStyle>.<mode>` (1)
        let mut own: Vec<(u32, u32)> = Vec::new();
        for (k, v) in r {
            let kind = match k.as_bytes()[0] {
                b'x' => 0u32,
                b'y' => 1,
                _ => continue,
            };
            let mut parts = k[1..].split('.');
            let a: u32 = parts.next().unwrap().parse().unwrap();
            let t: u32 = parts.next().unwrap().parse().unwrap();
            let m = parts.next().unwrap();
            let mode = MODES.iter().position(|x| *x == m).expect("override mode") as u32;
            own.push((
                (kind << 20) | (a << 12) | (t << 4) | mode,
                u32::from(self.pat(v)),
            ));
        }
        own.sort();
        // a regional variant mostly has its parent's overrides: one copy of each list
        let over = match self.over_index.get(&own) {
            Some(&at) => at,
            None => {
                let at = self.over.len();
                self.over.extend(own.iter().copied());
                self.over_index.insert(own.clone(), at);
                at
            }
        };
        let hc = r.get("hc").expect("hc").as_bytes();
        let pats = self.row(0, pats);
        let times = self.row(4, times);
        let names = self.row(1, names);
        let rel = self.row(2, rel);
        format!(
            "dpats: {pats}, dtimes: {times}, dnames: {names}, drel: {rel}, db: {b}, dover: {over}, dover_len: {}, hc12: b'{}', hc24: b'{}'",
            own.len(),
            char::from(hc[0]),
            char::from(hc[1])
        )
    }

    fn finish(&self) -> String {
        let mut out = String::new();
        // ends as `u16` while the text fits (a few locales), else `u32`
        let table = |out: &mut String, name: &str, s: &Strings| {
            let ty = if s.text.len() <= usize::from(u16::MAX) {
                "u16"
            } else {
                "u32"
            };
            let _ = writeln!(out, "pub(crate) static {name}_STR: &str = {:?};", s.text);
            let _ = writeln!(
                out,
                "pub(crate) static {name}_ENDS: [{ty}; {}] = {:?};",
                s.ends.len(),
                s.ends
            );
        };
        table(&mut out, "DPAT", &self.pats);
        table(&mut out, "DNAME", &self.names);
        let rows = |out: &mut String, name: &str, list: &[usize], len: usize| {
            let _ = writeln!(
                out,
                "pub(crate) static {name}: [[u16; {len}]; {}] = [",
                list.len()
            );
            for &i in list {
                let _ = writeln!(out, "    {:?},", self.rows[i]);
            }
            out.push_str("];\n");
        };
        rows(&mut out, "DATE_PATS", &self.pat_rows, DATE_ROW);
        rows(&mut out, "DATE_TIMES", &self.time_rows, TIME_ROW);
        rows(
            &mut out,
            "DATE_NAMES",
            &self.name_rows,
            NAME_SETS.iter().map(|(_, n)| n).sum(),
        );
        rows(
            &mut out,
            "DATE_REL",
            &self.rel_rows,
            REL_UNITS.len() * REL_STYLES.len(),
        );
        rows(&mut out, "DATE_REL_PARTS", &self.rel_parts, 17);
        rows(&mut out, "DATE_B", &self.b_rows, 75);
        let _ = writeln!(
            out,
            "pub(crate) static DATE_OVER_KEYS: [u32; {}] = {:?};\npub(crate) static DATE_OVER: [u16; {}] = {:?};",
            self.over.len(),
            self.over.iter().map(|o| o.0).collect::<Vec<_>>(),
            self.over.len(),
            self.over.iter().map(|o| o.1).collect::<Vec<_>>()
        );
        out
    }
}
