//! Citation-key generation. Port of `upstream/src/generateKeys.ts` and
//! `upstream/src/parsers/entryKeyTemplateParser.ts`.
//!
//! The template uses JabRef citation-key patterns: `[marker]` special markers, `[FIELD]`
//! field lookups (all-uppercase), `:modifier` chains, and a trailing `[duplicateLetter]`
//! (appended if absent) to disambiguate collisions.

use crate::ast::Entry;
use crate::name::parse_name_list;
use crate::value_nodes::render_field_value;
use std::collections::HashMap;

/// A parsed template token: either literal text or a `[marker:param:mod…]` group.
enum KeyToken {
    Literal(String),
    Marker {
        marker: String,
        parameter: Option<usize>,
        modifiers: Vec<String>,
    },
}

/// Generate a new citation key for each entry (indexed by entry order). `None` means "keep
/// the existing key". Port of `generateKeys`.
pub fn generate_keys(entries: &[&Entry], entry_key_template: &str) -> Vec<Option<String>> {
    let mut template = entry_key_template.to_string();
    if !template.contains("[duplicateLetter]") && !template.contains("[duplicateNumber]") {
        template = format!("{template}[duplicateLetter]");
    }
    let parsed = parse_entry_key_template(&template);

    let values: Vec<HashMap<String, String>> = entries
        .iter()
        .map(|e| {
            e.fields
                .iter()
                .map(|f| (f.name.clone(), render_field_value(f)))
                .collect()
        })
        .collect();

    // Group entries by their base key, preserving file order within each group.
    let mut entries_by_key: HashMap<String, Vec<usize>> = HashMap::new();
    let mut base_keys: Vec<Option<String>> = Vec::with_capacity(entries.len());
    for (i, v) in values.iter().enumerate() {
        let key = generate_key(v, &parsed, None);
        base_keys.push(key.clone());
        if let Some(k) = key {
            entries_by_key.entry(k).or_default().push(i);
        }
    }

    let mut result: Vec<Option<String>> = vec![None; entries.len()];
    for idxs in entries_by_key.values() {
        let regenerate = idxs.len() > 1;
        for (pos, &i) in idxs.iter().enumerate() {
            result[i] = if regenerate {
                generate_key(&values[i], &parsed, Some(pos + 1))
            } else {
                base_keys[i].clone()
            };
        }
    }
    result
}

fn generate_key(
    v: &HashMap<String, String>,
    template: &[KeyToken],
    duplicate: Option<usize>,
) -> Option<String> {
    let mut new_key = String::new();
    for token in template {
        match token {
            KeyToken::Literal(s) => new_key.push_str(s),
            KeyToken::Marker {
                marker,
                parameter,
                modifiers,
            } => {
                let mut key: Vec<String> = match marker.as_str() {
                    "auth" => marker_auth(v),
                    "authEtAl" => marker_auth_et_al(v),
                    "authors" => marker_authors(v),
                    "authorsN" => marker_authors_n(v, parameter.unwrap_or(0)),
                    "veryshorttitle" => non_function_words(&title(v)).into_iter().take(1).collect(),
                    "shorttitle" => non_function_words(&title(v)).into_iter().take(3).collect(),
                    "title" => capitalize_words(&words(&title(v))),
                    "fulltitle" => words(&title(v)),
                    "year" => marker_year(v),
                    "duplicateLetter" => vec![duplicate.map(num_to_letter).unwrap_or_default()],
                    "duplicateNumber" => {
                        vec![duplicate.map(|d| d.to_string()).unwrap_or_default()]
                    }
                    _ => {
                        // An all-uppercase marker is a field lookup; anything else is
                        // invalid. (The original throws; we keep the existing key.)
                        if !marker.is_empty() && marker == &marker.to_uppercase() {
                            v.get(&marker.to_lowercase())
                                .map(|value| words(value))
                                .unwrap_or_default()
                        } else {
                            return None;
                        }
                    }
                };
                for modifier in modifiers {
                    match modifier.as_str() {
                        "required" => {
                            if key.is_empty() {
                                return None;
                            }
                        }
                        "lower" => key = key.iter().map(|w| w.to_lowercase()).collect(),
                        "upper" => key = key.iter().map(|w| w.to_uppercase()).collect(),
                        "capitalize" => key = capitalize_words(&key),
                        _ => return None,
                    }
                }
                new_key.push_str(&key.join(""));
            }
        }
    }
    let new_key = remove_unsafe_entry_key_chars(&new_key);
    if new_key.is_empty() {
        None
    } else {
        Some(new_key)
    }
}

fn marker_auth(v: &HashMap<String, String>) -> Vec<String> {
    let authors = parse_name_list(v.get("author").map(String::as_str).unwrap_or(""));
    match authors.first() {
        Some(a) if !a.last.is_empty() => vec![a.last.clone()],
        _ => vec![],
    }
}

fn marker_auth_et_al(v: &HashMap<String, String>) -> Vec<String> {
    let authors = parse_name_list(v.get("author").map(String::as_str).unwrap_or(""));
    let mut out: Vec<String> = authors.iter().take(2).map(|a| a.last.clone()).collect();
    if authors.len() > 2 {
        out.push("Et".to_string());
        out.push("Al".to_string());
    }
    out
}

fn marker_authors(v: &HashMap<String, String>) -> Vec<String> {
    parse_name_list(v.get("author").map(String::as_str).unwrap_or(""))
        .iter()
        .map(|a| a.last.clone())
        .collect()
}

fn marker_authors_n(v: &HashMap<String, String>, n: usize) -> Vec<String> {
    let authors = parse_name_list(v.get("author").map(String::as_str).unwrap_or(""));
    let mut out: Vec<String> = authors.iter().take(n).map(|a| a.last.clone()).collect();
    if authors.len() > n {
        out.push("Et".to_string());
        out.push("Al".to_string());
    }
    out
}

fn marker_year(v: &HashMap<String, String>) -> Vec<String> {
    match v.get("year") {
        Some(year) => {
            let digits: String = year.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                vec![]
            } else {
                vec![digits]
            }
        }
        None => vec![],
    }
}

fn title(v: &HashMap<String, String>) -> String {
    v.get("title")
        .or_else(|| v.get("booktitle"))
        .cloned()
        .unwrap_or_default()
}

fn words(value: &str) -> Vec<String> {
    value
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | ',' | ':' | ';'))
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

fn capitalize_words(words: &[String]) -> Vec<String> {
    words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().unwrap_or_default();
            let rest: String = chars.collect();
            format!("{}{}", first.to_uppercase(), rest.to_lowercase())
        })
        .collect()
}

fn non_function_words(value: &str) -> Vec<String> {
    words(value)
        .into_iter()
        .filter(|w| !FUNCTION_WORDS.iter().any(|fw| *fw == w.to_lowercase()))
        .collect()
}

fn num_to_letter(n: usize) -> String {
    // FIXME(upstream): breaks after 26 duplicates.
    char::from_u32(96 + n as u32)
        .map(String::from)
        .unwrap_or_default()
}

/// Remove characters that are unsafe (or merely untidy) in entry keys. Port of
/// `removeUnsafeEntryKeyChars`.
fn remove_unsafe_entry_key_chars(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(
                c,
                '{' | '}'
                    | ','
                    | '\\'
                    | '#'
                    | '%'
                    | '~'
                    | '('
                    | ')'
                    | '"'
                    | '\''
                    | '='
                    | '.'
                    | ':'
                    | ';'
                    | '['
                    | ']'
                    | '_'
            ) && !c.is_whitespace()
        })
        .collect()
}

/// Parse a `[marker:mod1:mod2]` template into tokens. Port of `parseEntryKeyTemplate`.
fn parse_entry_key_template(template: &str) -> Vec<KeyToken> {
    let chars: Vec<char> = template.chars().collect();
    let n = chars.len();
    let mut tokens = Vec::new();
    let mut literal_start = 0;
    let mut i = 0;
    while i < n {
        if chars[i] == '['
            && let Some(close) = (i + 1..n).find(|&j| chars[j] == ']')
        {
            let content: String = chars[i + 1..close].iter().collect();
            if let Some((marker, parameter, modifiers)) = parse_marker_content(&content) {
                if i > literal_start {
                    tokens.push(KeyToken::Literal(chars[literal_start..i].iter().collect()));
                }
                tokens.push(KeyToken::Marker {
                    marker,
                    parameter,
                    modifiers,
                });
                i = close + 1;
                literal_start = i;
                continue;
            }
        }
        i += 1;
    }
    if literal_start < n {
        tokens.push(KeyToken::Literal(chars[literal_start..n].iter().collect()));
    }
    tokens
}

/// Parse the inside of a `[…]` group: `marker[:mod…]`. A digit run in the marker becomes a
/// numeric parameter and the marker's digits are replaced with `N` (e.g. `authors2`).
fn parse_marker_content(content: &str) -> Option<(String, Option<usize>, Vec<String>)> {
    let segments: Vec<&str> = content.split(':').collect();
    let token_key_n = segments[0];
    if token_key_n.is_empty() {
        return None;
    }
    let mut modifiers = Vec::new();
    for segment in &segments[1..] {
        if segment.is_empty() {
            return None;
        }
        modifiers.push(segment.to_string());
    }
    let mut parameter = None;
    let mut marker = String::new();
    let mut digits = String::new();
    for c in token_key_n.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            if !digits.is_empty() {
                parameter = digits.parse().ok();
                marker.push('N');
                digits.clear();
            }
            marker.push(c);
        }
    }
    if !digits.is_empty() {
        parameter = digits.parse().ok();
        marker.push('N');
    }
    Some((marker, parameter, modifiers))
}

/// Words ignored by `veryshorttitle`/`shorttitle`. Port of `functionWords`.
static FUNCTION_WORDS: &[&str] = &[
    "a", "about", "above", "across", "against", "along", "among", "an", "and", "around", "at",
    "before", "behind", "below", "beneath", "beside", "between", "beyond", "but", "by", "down",
    "during", "except", "for", "from", "in", "inside", "into", "like", "near", "nor", "of", "off",
    "on", "onto", "or", "since", "so", "the", "through", "to", "toward", "under", "until", "up",
    "upon", "with", "within", "without", "yet",
];

/// Collect the top-level entries of a document (used by the generate-keys transform).
pub fn get_entries(root: &crate::ast::Root) -> Vec<&Entry> {
    root.children
        .iter()
        .filter_map(|child| match child {
            crate::ast::BibtexNode::Block(block) => match &block.inner {
                Some(crate::ast::BlockInner::Entry(entry)) => Some(entry),
                _ => None,
            },
            _ => None,
        })
        .collect()
}
