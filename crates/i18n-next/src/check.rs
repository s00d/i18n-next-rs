//! `I18n::check()` (DESIGN 8b.8, M3): what a translator forgot, found without running the app.
//!
//! - a page that has a file in some locales but not in others;
//! - a key of the default locale missing in another locale, page by page (root and pages);
//! - `{placeholders}` of one key that differ from the default locale's;
//! - a plural message whose form count does not fit the locale's CLDR categories, or whose
//!   shape makes `tc` fall back to the index rule;
//! - links that left a linked message as written (with `linked_messages(true)`).
//!
//! Disabled locales are skipped. Texts `linked.rs` stored for a view are not the locale's own
//! keys and are never counted.

use crate::catalog::{Chunk, Entry, Msg, hole_ranges};
use crate::linked::Broken;
use crate::plural::Plural;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

/// Why a link left its message as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum LinkError {
    /// The key names no message (missing, a subtree or a boolean).
    Missing,
    /// The link leads back to a message being resolved.
    Cycle,
    /// The target is itself left as written.
    Broken,
    /// Not `upper`, `lower` or `capitalize`.
    UnknownModifier,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LinkError::Missing => "names no message",
            LinkError::Cycle => "is a cycle",
            LinkError::Broken => "links a message left as written",
            LinkError::UnknownModifier => "has an unknown modifier",
        })
    }
}

/// One finding of [`I18n::check`](crate::I18n::check). `page` is `"index"` for the root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CheckItem {
    /// `locale` has no file for `page`, other locales do.
    MissingPage { locale: String, page: String },
    /// `key` of the default locale is missing in `locale`.
    MissingKey {
        locale: String,
        page: String,
        key: String,
    },
    /// The placeholders of `key` in `locale` are not those of the default locale.
    Placeholders {
        locale: String,
        page: String,
        key: String,
        /// In the default locale, sorted.
        expected: Vec<String>,
        /// In `locale`, sorted.
        found: Vec<String>,
    },
    /// A plural message with a form count the locale does not read by category.
    PluralForms {
        locale: String,
        page: String,
        key: String,
        forms: usize,
        /// Counts that fit: a zero form plus one per category small counts reach; a `=0 `
        /// message also one per category of the locale.
        expected: Vec<usize>,
        /// The categories small counts reach (`one`, `few`, `many`).
        categories: Vec<String>,
    },
    /// The form count fits, but the zero form has a `{` or a form shared by several counts
    /// lacks `{count}`: `tc` reads it by index.
    PluralShape {
        locale: String,
        page: String,
        key: String,
        categories: Vec<String>,
    },
    /// A link left the linked message `key` as written.
    BrokenLink {
        locale: String,
        page: String,
        key: String,
        /// As written (`@.upper:brand`).
        link: String,
        reason: LinkError,
    },
}

impl fmt::Display for CheckItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckItem::MissingPage { locale, page } => {
                write!(
                    f,
                    "{locale}: no file for page \"{page}\" (other locales have one)"
                )
            }
            CheckItem::MissingKey { locale, page, key } => {
                write!(
                    f,
                    "{locale}/{page}: \"{key}\" is missing (the default locale has it)"
                )
            }
            CheckItem::Placeholders {
                locale,
                page,
                key,
                expected,
                found,
            } => write!(
                f,
                "{locale}/{page}: \"{key}\" has {{{}}}, the default locale {{{}}}",
                found.join(", "),
                expected.join(", ")
            ),
            CheckItem::PluralForms {
                locale,
                page,
                key,
                forms,
                expected,
                categories,
            } => {
                let expected: Vec<String> = expected.iter().map(usize::to_string).collect();
                write!(
                    f,
                    "{locale}/{page}: \"{key}\" has {forms} plural forms, expected {} (zero | {})",
                    expected.join(" or "),
                    categories.join(" | ")
                )
            }
            CheckItem::PluralShape {
                locale,
                page,
                key,
                categories,
            } => write!(
                f,
                "{locale}/{page}: \"{key}\" is read by index, not by category (zero | {}): \
                 the zero form has a `{{` or a form of several counts lacks {{count}}",
                categories.join(" | ")
            ),
            CheckItem::BrokenLink {
                locale,
                page,
                key,
                link,
                reason,
            } => write!(
                f,
                "{locale}/{page}: \"{key}\" left as written: {link} {reason}"
            ),
        }
    }
}

/// What [`I18n::check`](crate::I18n::check) found, sorted. Display lists it line by line.
///
/// ```
/// use i18n_next::{CheckItem, I18n};
///
/// let i18n = I18n::builder()
///     .locale("en")
///     .messages("en", None, r#"{"hi": "Hi {name}", "bye": "Bye"}"#)
///     .messages("de", None, r#"{"hi": "Hallo {nam}"}"#)
///     .build()?;
/// let report = i18n.check();
/// assert!(!report.is_clean());
/// assert!(report.items.contains(&CheckItem::MissingKey {
///     locale: "de".into(),
///     page: "index".into(),
///     key: "bye".into(),
/// }));
/// println!("{report}");
/// # Ok::<(), i18n_next::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CheckReport {
    /// Every finding, sorted (by kind, then locale, page, key).
    pub items: Vec<CheckItem>,
}

impl CheckReport {
    /// No findings.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.items.is_empty()
    }
}

impl fmt::Display for CheckReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.items.is_empty() {
            return f.write_str("catalog check: clean");
        }
        write!(f, "catalog check: {} problem(s)", self.items.len())?;
        for item in &self.items {
            write!(f, "\n  - {item}")?;
        }
        Ok(())
    }
}

/// What `build` leaves for `check` (`i18n.rs` `Meta`).
#[derive(Debug, Default)]
pub(crate) struct Facts {
    /// The default locale: keys and placeholders are compared with it.
    pub(crate) reference: Option<Arc<str>>,
    /// Links left as written at `build` (`linked_messages(true)`).
    pub(crate) broken: Vec<Broken>,
}

fn page_name(page: Option<&Arc<str>>) -> String {
    page.map_or_else(|| "index".to_owned(), |p| String::from(&**p))
}

/// The placeholder names of a message; without `count` when `plural` (`tc` fills it, and a
/// form may show the count in one language and not in another).
fn holes(e: &Entry, plural: bool) -> Option<BTreeSet<&str>> {
    let raw = match e {
        Entry::Msg(_) => e.text()?,
        _ => return None,
    };
    Some(
        hole_ranges(raw)
            .filter_map(|(s, e)| raw.get(s..e))
            .filter(|name| !(plural && *name == "count"))
            .collect(),
    )
}

fn sorted(set: &BTreeSet<&str>) -> Vec<String> {
    set.iter().map(|s| (*s).to_owned()).collect()
}

/// The report over `chunks` (`(locale, page, chunk)`, disabled locales left out).
pub(crate) fn run<'a>(
    chunks: impl Iterator<Item = (&'a Arc<str>, Option<&'a Arc<str>>, &'a Chunk)>,
    facts: &Facts,
    plural: impl Fn(&str) -> Option<&'static Plural>,
) -> CheckReport {
    let chunks: BTreeMap<(&Arc<str>, Option<&Arc<str>>), &Chunk> = chunks
        .filter(|(_, _, c)| !c.only_linked())
        .map(|(l, p, c)| ((l, p), c))
        .collect();
    let locales: BTreeSet<&Arc<str>> = chunks.keys().map(|(l, _)| *l).collect();
    let pages: BTreeSet<Option<&Arc<str>>> = chunks.keys().map(|(_, p)| *p).collect();
    let mut items = Vec::new();

    for page in &pages {
        for locale in &locales {
            if !chunks.contains_key(&(*locale, *page)) {
                items.push(CheckItem::MissingPage {
                    locale: String::from(&***locale),
                    page: page_name(*page),
                });
            }
        }
    }

    if let Some(reference) = facts.reference.as_ref() {
        for ((_, page), base) in chunks.iter().filter(|((l, _), _)| *l == reference) {
            for locale in locales.iter().filter(|l| **l != reference) {
                let Some(other) = chunks.get(&(*locale, *page)) else {
                    continue;
                };
                for (key, e) in base.leaves() {
                    let mine = match other.get(key) {
                        Some(m) if other.has(key) => m,
                        _ => {
                            items.push(CheckItem::MissingKey {
                                locale: String::from(&***locale),
                                page: page_name(*page),
                                key: key.to_owned(),
                            });
                            continue;
                        }
                    };
                    let plural = [e, mine]
                        .iter()
                        .any(|x| matches!(x, Entry::Msg(Msg::Plural(_))));
                    if let (Some(want), Some(got)) = (holes(e, plural), holes(mine, plural))
                        && want != got
                    {
                        items.push(CheckItem::Placeholders {
                            locale: String::from(&***locale),
                            page: page_name(*page),
                            key: key.to_owned(),
                            expected: sorted(&want),
                            found: sorted(&got),
                        });
                    }
                }
            }
        }
    }

    for ((locale, page), chunk) in &chunks {
        let rules = plural(locale);
        for (key, e) in chunk.leaves() {
            let Entry::Msg(Msg::Plural(p)) = e else {
                continue;
            };
            let (forms, marked, shape) = p.layout();
            let Some((expected, shape_only)) = shape.check(marked, rules) else {
                continue;
            };
            let categories = rules
                .map(|r| r.named_names().into_iter().map(str::to_owned).collect())
                .unwrap_or_default();
            let (locale, page, key) = (String::from(&***locale), page_name(*page), key.to_owned());
            items.push(if shape_only {
                CheckItem::PluralShape {
                    locale,
                    page,
                    key,
                    categories,
                }
            } else {
                CheckItem::PluralForms {
                    locale,
                    page,
                    key,
                    forms,
                    expected,
                    categories,
                }
            });
        }
    }

    for b in &facts.broken {
        if locales.contains(&b.locale) {
            items.push(CheckItem::BrokenLink {
                locale: String::from(&*b.locale),
                page: page_name(b.page.as_ref()),
                key: String::from(&*b.key),
                link: String::from(&*b.link),
                reason: b.reason,
            });
        }
    }

    items.sort();
    items.dedup();
    CheckReport { items }
}
