//! The tokenizer and interpolation against a naive model of `core/src/text.ts`: the input is
//! a template, a NUL, then parameters (`name=value` lines, names and values free). The
//! template is stored as a message and read with `t_with`; the model replaces each leftmost
//! `\{([^{}\s]+)\}` (JS `\s`) whose name has a parameter (the last one of that name), once,
//! left to right, and leaves the rest as written.
#![no_main]

use i18n_next::{Args, I18n};
use libfuzzer_sys::fuzz_target;

/// ECMAScript WhiteSpace and LineTerminator (`\s`).
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{b}'
            | '\u{c}'
            | '\r'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn model(template: &str, params: &[(&str, &str)]) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '{' && chars[j] != '}' && !js_space(chars[j]) {
                j += 1;
            }
            if j > i + 1 && j < chars.len() && chars[j] == '}' {
                let name: String = chars[i + 1..j].iter().collect();
                if let Some((_, v)) = params.iter().rev().find(|(n, _)| *n == name) {
                    out.push_str(v);
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (template, rest) = text.split_once('\0').unwrap_or((&text, ""));
    let params: Vec<(&str, &str)> = rest
        .split('\n')
        .filter_map(|line| line.split_once('='))
        .take(12)
        .collect();
    let json = serde_json::json!({ "k": template }).to_string();
    let i18n = I18n::builder().messages("en", None, &json).build().unwrap();
    let mut args = Args::new();
    for (n, v) in &params {
        args = args.str(n, v);
    }
    assert_eq!(i18n.t_with("k", &args), model(template, &params), "{template:?} {params:?}");
    // no parameters: the text as written
    assert_eq!(i18n.t("k"), template);
});
