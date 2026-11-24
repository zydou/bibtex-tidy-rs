//! Transform pipeline stages. Ports of the always-on and default-path transforms from
//! `upstream/src/transforms/*.ts`.
//!
//! Each transform consumes and rebuilds the [`Root`](crate::ast::Root) via the walker,
//! optionally emitting warnings. The full option-driven pipeline is assembled in
//! [`crate::pipeline`]; the transforms here cover the subset applied by the default
//! configuration (S2 skeleton) plus the option-parameterized formatting stages.

use crate::ast::{
    BibtexNode, Block, BlockInner, Ctx, Entry, Field, Root, Value, Visitors, walk, walk_entries,
};
use crate::latex::{self, Block as LatexBlock, LatexNode};
use crate::types::{TransformFn, TransformResult, Warning};
use crate::unicode::{core_special_character, special_character};
use crate::unicode_data::CORE_SPECIAL_CHARACTERS_REVERSIBLE;
use crate::utils::{unwrap_text, wrap_text};
use crate::value_nodes::{
    enclose_latex_in_curly, render_field_value, render_value_node, replace_value_node_text,
};
use std::collections::HashSet;

const MAX_ASCII: u32 = 0x7f;
const VERBATIM_FIELDS: [&str; 5] = ["url", "file", "pdf", "doi", "doiurl"];

/// Replace single dashes with double dashes in page ranges (only inside `pages` fields).
pub fn format_page_range() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, ctx| {
                page_range_visit(&Value::Braced(block), ctx.current_field())
            })),
            quoted: Some(Box::new(|block, ctx| {
                page_range_visit(&Value::Quoted(block), ctx.current_field())
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

fn page_range_visit(value: &Value, field: Option<&Field>) -> Option<Vec<Value>> {
    let field = field?;
    if field.name.to_lowercase() != "pages" {
        return None;
    }
    let rendered = render_value_node(value);
    let formatted = format_page_range_str(&rendered);
    Some(vec![replace_value_node_text(value, &formatted)])
}

fn format_page_range_str(s: &str) -> String {
    // Replace `(\d)\s*-\s*(\d)` with `$1--$2`, applied four times (mirrors the original).
    let mut result = s.to_string();
    for _ in 0..4 {
        result = regex_replace_digit_dash(&result);
    }
    result
}

/// One pass of the page-range regex: find a digit, optional whitespace, a dash, optional
/// whitespace, and a digit; join the digits with `--`.
fn regex_replace_digit_dash(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            // lookahead: \s*- ?- \s* \d
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '-' {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                if k < chars.len() && chars[k].is_ascii_digit() {
                    out.push(chars[i]);
                    out.push_str("--");
                    out.push(chars[k]);
                    i = k + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Lowercase the entry type (`@Article` -> `@article`).
pub fn lowercase_entry_type() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            block: Some(Box::new(|block, _ctx| {
                if matches!(block.inner, Some(BlockInner::Entry(_))) {
                    Some(vec![Block {
                        command: block.command.to_lowercase(),
                        ..block
                    }])
                } else {
                    Some(vec![block])
                }
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Lowercase all field names.
pub fn lowercase_fields() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            field: Some(Box::new(|field, _ctx| {
                Some(vec![Field {
                    name: field.name.to_lowercase(),
                    ..field
                }])
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Keep only the first occurrence of each (lowercased) field name within an entry.
pub fn remove_duplicate_fields() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            entry: Some(Box::new(|entry, _ctx| {
                let mut seen = std::collections::HashSet::new();
                let mut fields = Vec::with_capacity(entry.fields.len());
                for field in entry.fields {
                    let name = field.name.to_lowercase();
                    if seen.insert(name) {
                        fields.push(field);
                    }
                }
                Some(Entry { fields, ..entry })
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Remove the fields named in `omit` (case-insensitive). Port of
/// `createRemoveSpecifiedFieldsTransform`.
pub fn remove_specified_fields(omit: &[String]) -> TransformFn {
    let set: std::collections::HashSet<String> = omit.iter().map(|f| f.to_lowercase()).collect();
    Box::new(move |ast| {
        let visitors = Visitors {
            field: Some(Box::new(|field, _ctx| {
                if set.contains(&field.name.to_lowercase()) {
                    Some(vec![])
                } else {
                    Some(vec![field])
                }
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Reset all inter-node whitespace: entries separated by a blank line, fields trimmed.
/// `keep_comment_whitespace` preserves comment surroundings (the `--no-tidy-comments` mode).
pub fn reset_whitespace(keep_comment_whitespace: bool) -> TransformFn {
    Box::new(move |ast| {
        let mut children: Vec<BibtexNode> = Vec::new();
        for original in ast.children {
            let mut child = original;
            let has_prev = !children.is_empty();
            let prev_is_comment = children.last().is_some_and(is_comment);
            let preserve = is_comment(&child) && keep_comment_whitespace;
            let preserve_prev = prev_is_comment && keep_comment_whitespace;

            if keep_comment_whitespace
                && matches!(child, BibtexNode::Block(_))
                && let Some(BibtexNode::Text(prev_text)) = children.last_mut()
                && !prev_text.text.ends_with('\n')
            {
                // Ensure the previous text node ends in a newline before a block.
                prev_text.text = format!("{}\n", prev_text.text.trim_end());
            }

            if !preserve {
                let prefix = if has_prev && !preserve_prev { "\n" } else { "" };
                child = set_whitespace_prefix(child, prefix);
            }
            children.push(child);
        }
        TransformResult {
            ast: Root { children },
            warnings: vec![],
        }
    })
}

fn is_comment(node: &BibtexNode) -> bool {
    match node {
        BibtexNode::Text(_) => true,
        BibtexNode::Block(b) => matches!(b.inner, Some(BlockInner::Comment { .. })),
    }
}

fn set_whitespace_prefix(node: BibtexNode, prefix: &str) -> BibtexNode {
    match node {
        BibtexNode::Text(mut t) => {
            t.text = t.text.trim().to_string();
            t.whitespace_prefix = prefix.to_string();
            BibtexNode::Text(t)
        }
        BibtexNode::Block(mut b) => {
            b.whitespace_prefix = prefix.to_string();
            if let Some(inner) = &mut b.inner {
                match inner {
                    BlockInner::Entry(entry) => {
                        for field in &mut entry.fields {
                            field.whitespace_prefix = String::new();
                        }
                    }
                    BlockInner::Comment { raw } => {
                        *raw = raw.trim().to_string();
                    }
                    _ => {}
                }
            }
            BibtexNode::Block(b)
        }
    }
}

/// Prefix each field with a newline plus the indentation string.
pub fn indent_fields(indent: &str) -> TransformFn {
    let indent = indent.to_string();
    Box::new(move |ast| {
        let indent = indent.clone();
        let visitors = Visitors {
            field: Some(Box::new(move |field, _ctx| {
                Some(vec![Field {
                    whitespace_prefix: format!("\n{indent}"),
                    ..field
                }])
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Pad the space between each field name and `=` so values line up at `column`.
/// `column` is f64 to replicate JS number semantics: `Math.max(NaN - len, 1)` = NaN and
/// `" ".repeat(NaN)` = "" (no padding), whereas a real column pads normally.
pub fn align_values(column: f64) -> TransformFn {
    Box::new(move |ast| {
        let visitors = Visitors {
            field: Some(Box::new(move |field, _ctx| {
                // Upstream: " ".repeat(Math.max(column - fieldName.length, 1)).
                // Math.max(NaN, 1) = NaN; " ".repeat(NaN) = "".
                let pad = if column.is_nan() {
                    0
                } else {
                    let diff = column - field.name.chars().count() as f64;
                    diff.max(1.0) as usize
                };
                Some(vec![Field {
                    value_prefix: " ".repeat(pad),
                    ..field
                }])
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Set each field's trailing comma: present between fields, and after the last only if
/// `trailing` is set.
pub fn field_commas(trailing: bool) -> TransformFn {
    Box::new(move |ast| {
        let visitors = Visitors {
            entry: Some(Box::new(move |entry, _ctx| {
                let len = entry.fields.len();
                let fields = entry
                    .fields
                    .into_iter()
                    .enumerate()
                    .map(|(i, field)| Field {
                        has_comma: i < len.saturating_sub(1) || trailing,
                        ..field
                    })
                    .collect();
                Some(Entry { fields, ..entry })
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Re-wrap long braced values at `wrap` columns. `align` is the value column (for the
/// multi-line indent) and `indent` the field indent.
/// `align`/`wrap` are f64 to replicate JS number semantics. `wrapValues` is always
/// pushed; the on/off decision lives in the gate `if ((wrap && lineLength > wrap) ||
/// multiLine)`, where `wrap` of 0/NaN is falsy (JS truthiness) so no wrapping happens.
pub fn wrap_values(indent: &str, align: f64, wrap: Option<f64>) -> TransformFn {
    let indent = indent.to_string();
    Box::new(move |ast| {
        let indent = indent.clone();
        let visitors = Visitors {
            concat: Some(Box::new(move |field, _ctx| {
                let single = field.values.len() == 1;
                let values = field
                    .values
                    .iter()
                    .map(|node| wrap_value(node, &indent, align, wrap, single))
                    .collect();
                Some(values)
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

fn wrap_value(node: &Value, indent: &str, align: f64, wrap: Option<f64>, single: bool) -> Value {
    let braced = match node {
        Value::Braced(b) => b,
        _ => return node.clone(),
    };
    let mut value = unwrap_text(&render_value_node(&Value::Braced(braced.clone())));
    if single {
        value = value.trim().to_string();
    }
    // `{align}` in a JS template string renders NaN as "NaN"; Rust's f64 Display does
    // the same, so this matches the upstream lineLength computation.
    let line_length = format!("{indent}{align}{{{value}}}").chars().count();
    let multi_line = value.contains("\n\n");
    // JS: `wrap && lineLength > wrap`. wrap of 0/NaN is falsy → no wrap.
    let wrap_truthy = wrap.is_some_and(|w| w != 0.0 && !w.is_nan());
    let over = wrap_truthy && wrap.is_some_and(|w| line_length as f64 > w);
    if over || multi_line {
        let val_indent = indent.repeat(2);
        let paragraphs: Vec<String> = value
            .split("\n\n")
            .map(|p| {
                // JS: `if (wrap) { ...wrapText... } else { paragraph }`.
                if wrap_truthy {
                    let wrap_col = wrap.unwrap();
                    let col = (wrap_col - val_indent.chars().count() as f64).max(0.0) as usize;
                    wrap_text(p, col).join(&format!("\n{val_indent}"))
                } else {
                    p.to_string()
                }
            })
            .collect();
        value = format!(
            "\n{val_indent}{}\n{indent}",
            paragraphs.join(&format!("\n\n{val_indent}"))
        );
    }
    replace_value_node_text(node, &value)
}

/// Escape non-ASCII characters into LaTeX macros. Legacy mode uses the full character
/// table; "new" mode uses the package-independent core table and warns on characters
/// that cannot be escaped.
pub fn escape_characters(new_mode: bool) -> TransformFn {
    Box::new(move |ast| {
        let warnings = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let w_braced = warnings.clone();
        let w_quoted = warnings.clone();
        let visitors = Visitors {
            braced: Some(Box::new(move |block, ctx| {
                escape_visit(
                    Value::Braced(block),
                    ctx.current_field(),
                    new_mode,
                    &w_braced,
                )
            })),
            quoted: Some(Box::new(move |block, ctx| {
                escape_visit(
                    Value::Quoted(block),
                    ctx.current_field(),
                    new_mode,
                    &w_quoted,
                )
            })),
            ..Default::default()
        };
        let new_ast = walk(ast, &visitors);
        let collected = std::mem::take(&mut *warnings.borrow_mut());
        TransformResult {
            ast: new_ast,
            warnings: collected,
        }
    })
}

type SharedWarnings = std::rc::Rc<std::cell::RefCell<Vec<Warning>>>;

fn escape_visit(
    original: Value,
    field: Option<&Field>,
    new_mode: bool,
    warnings: &SharedWarnings,
) -> Option<Vec<Value>> {
    let block = match &original {
        Value::Braced(b) | Value::Quoted(b) => b.clone(),
        Value::Literal(_) => return None,
    };
    let is_quoted = matches!(original, Value::Quoted(_));
    let field_name = field.map(|f| f.name.as_str()).unwrap_or("");
    if VERBATIM_FIELDS.contains(&field_name.to_lowercase().as_str()) {
        return None;
    }
    let escaped = escape_block(&block, field_name, is_quoted, new_mode, warnings);
    Some(vec![match original {
        Value::Braced(_) => Value::Braced(escaped),
        Value::Quoted(_) => Value::Quoted(escaped),
        Value::Literal(_) => unreachable!(),
    }])
}

fn escape_block(
    block: &LatexBlock,
    field: &str,
    is_quoted: bool,
    new_mode: bool,
    warnings: &SharedWarnings,
) -> LatexBlock {
    let mut new_block = LatexBlock {
        kind: block.kind,
        children: Vec::new(),
    };
    for child in &block.children {
        match child {
            LatexNode::Text(text) => {
                new_block
                    .children
                    .extend(escape_text(text, field, is_quoted, new_mode, warnings));
            }
            LatexNode::Command(_) | LatexNode::Math(_) => new_block.children.push(child.clone()),
            LatexNode::Block(b) => new_block.children.push(LatexNode::Block(escape_block(
                b, field, is_quoted, new_mode, warnings,
            ))),
        }
    }
    new_block
}

fn escape_text(
    text: &str,
    field: &str,
    is_quoted: bool,
    new_mode: bool,
    warnings: &SharedWarnings,
) -> Vec<LatexNode> {
    let mut frags: Vec<LatexNode> = Vec::new();
    for ch in text.chars() {
        let code = ch as u32;
        let escaped = if new_mode {
            core_special_character(code)
        } else {
            special_character(code)
        };
        match escaped {
            Some(esc) => {
                let parsed = if is_quoted && esc.contains('"') {
                    latex::parse(&format!("{{{esc}}}"))
                } else {
                    latex::parse(esc)
                };
                for node in parsed.children {
                    match node {
                        LatexNode::Text(t) => push_str(&mut frags, &t),
                        other => frags.push(other),
                    }
                }
            }
            None => {
                if code > MAX_ASCII {
                    let codepoint = format!("{:04X}", code);
                    warnings.borrow_mut().push(Warning::new(
                        "UNSUPPORTED_ESCAPE",
                        format!("Cannot escape character {ch} (U+{codepoint}) in {field} without LaTeX packages or special fonts."),
                    ));
                }
                push_str(&mut frags, &ch.to_string());
            }
        }
    }
    frags
}

fn push_str(frags: &mut Vec<LatexNode>, s: &str) {
    if let Some(LatexNode::Text(last)) = frags.last_mut() {
        last.push_str(s);
        return;
    }
    frags.push(LatexNode::Text(s.to_string()));
}

// ---------------------------------------------------------------------------
// Option-gated value transforms (S3)
// ---------------------------------------------------------------------------

/// The 3-letter month abbreviations, in calendar order. Used by month sorting and
/// abbreviation. Port of the `monthAliases`/`MONTH_MACROS` tables.
pub const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// All values recognized as a given month (including the numeric BibTeX forms).
const MONTH_ALIASES: [(&str, &[&str]); 12] = [
    ("jan", &["1", "jan", "january"]),
    ("feb", &["2", "feb", "february"]),
    ("mar", &["3", "mar", "march"]),
    ("apr", &["4", "apr", "april"]),
    ("may", &["5", "may"]),
    ("jun", &["6", "jun", "june"]),
    ("jul", &["7", "jul", "july"]),
    ("aug", &["8", "aug", "august"]),
    ("sep", &["9", "sep", "september"]),
    ("oct", &["10", "oct", "october"]),
    ("nov", &["11", "nov", "november"]),
    ("dec", &["12", "dec", "december"]),
];

/// The month abbreviation for a recognized month value (matched case-insensitively).
fn month_abbr(value: &str) -> Option<&'static str> {
    let v = value.to_lowercase();
    MONTH_ALIASES
        .iter()
        .find(|(_, aliases)| aliases.contains(&v.as_str()))
        .map(|(abbr, _)| *abbr)
}

/// Convert every month value to its 3-letter abbreviation. Port of `createMonthsTransform`.
pub fn months() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            field: Some(Box::new(|field, _ctx| {
                if field.name.to_lowercase() != "month" {
                    return Some(vec![field]);
                }
                let new_values: Vec<Value> = field
                    .values
                    .iter()
                    .map(|v| match v {
                        Value::Literal(s) => match month_abbr(s) {
                            Some(abbr) => Value::Literal(abbr.to_string()),
                            None => Value::Literal(s.clone()),
                        },
                        Value::Braced(_) | Value::Quoted(_) => {
                            match month_abbr(&render_value_node(v)) {
                                Some(abbr) => Value::Literal(abbr.to_string()),
                                None => v.clone(),
                            }
                        }
                    })
                    .collect();
                Some(vec![Field {
                    values: new_values,
                    ..field
                }])
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Convert literal and quoted values to braced ones, leaving `month` values that are
/// already 3-letter abbreviations alone. Port of `createPreferCurlyTransform`.
pub fn prefer_curly() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            concat: Some(Box::new(|field, ctx| {
                let values = field.values.clone();
                let mut result = Vec::with_capacity(values.len());
                for value in values {
                    result.extend(prefer_curly_value(value, field, ctx));
                }
                Some(result)
            })),
            braced: Some(Box::new(|block, ctx| match ctx.current_field() {
                Some(field) if is_month_abbrev(field) => Some(vec![Value::Braced(block)]),
                _ => None,
            })),
            quoted: Some(Box::new(|block, ctx| match ctx.current_field() {
                Some(field) if is_month_abbrev(field) => Some(vec![Value::Quoted(block)]),
                _ => Some(vec![Value::Braced(block)]),
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

fn prefer_curly_value(value: Value, field: &Field, _ctx: &Ctx) -> Vec<Value> {
    match value {
        Value::Literal(text) => {
            if is_month_abbrev(field) {
                vec![Value::Literal(text)]
            } else {
                vec![Value::Braced(latex::parse(&text))]
            }
        }
        other => vec![other],
    }
}

/// A `month` field whose rendered value is already a 3-letter abbreviation.
fn is_month_abbrev(field: &Field) -> bool {
    field.name.to_lowercase() == "month" && MONTHS.contains(&render_field_value(field).as_str())
}

/// Convert integer values (`1`, `23`, …) to literal (unbraced) values. Depends on
/// `prefer-curly` so that quoted numbers are normalized first. Port of
/// `createPreferNumericTransform`.
pub fn prefer_numeric() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, _ctx| {
                prefer_numeric_visit(&Value::Braced(block))
            })),
            quoted: Some(Box::new(|block, _ctx| {
                prefer_numeric_visit(&Value::Quoted(block))
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

fn prefer_numeric_visit(value: &Value) -> Option<Vec<Value>> {
    let rendered = render_value_node(value);
    if is_positive_integer(&rendered) {
        Some(vec![Value::Literal(rendered)])
    } else {
        None
    }
}

/// Matches `/^[1-9][0-9]*$/`.
fn is_positive_integer(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if ('1'..='9').contains(&c) => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_digit())
}

/// Remove curly-brace grouping (unless part of a command) from the given fields. Port of
/// `createRemoveBracesTransform`.
pub fn remove_braces(fields: &[String]) -> TransformFn {
    let set: HashSet<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    Box::new(move |ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, ctx| match ctx.current_field() {
                Some(field) if set.contains(&field.name.to_lowercase()) => {
                    Some(vec![Value::Braced(latex::flatten(&block))])
                }
                _ => None,
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Enclose the given fields' values in an extra set of braces. Depends on `prefer-curly`.
/// Port of `createEncloseBracesTransform`.
pub fn enclose_braces(fields: &[String]) -> TransformFn {
    let set: HashSet<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    Box::new(move |ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, ctx| match ctx.current_field() {
                Some(field) if set.contains(&field.name.to_lowercase()) => {
                    Some(vec![Value::Braced(enclose_latex_in_curly(&block))])
                }
                _ => None,
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Remove one level of enclosing braces (`{{x}}` -> `{x}`) from every braced value. Port of
/// `createStripEnclosingBracesTransform`.
pub fn remove_enclosing_braces() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, _ctx| {
                let rendered = latex::stringify(&block);
                Some(
                    strip_enclosing_braces(&rendered)
                        .map(|inner| vec![Value::Braced(latex::parse(&inner))])
                        .unwrap_or_else(|| vec![Value::Braced(block)]),
                )
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Matches `/^\{([^{}]*)\}$/`: a single outer group with no nested braces inside.
fn strip_enclosing_braces(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 2 || chars[0] != '{' || chars[chars.len() - 1] != '}' {
        return None;
    }
    let inner = &chars[1..chars.len() - 1];
    if inner.iter().any(|c| *c == '{' || *c == '}') {
        None
    } else {
        Some(inner.iter().collect())
    }
}

/// Remove BibTeX comments and `@comment` blocks. Port of `createStripCommentsTransform`.
pub fn remove_comments() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            // Upstream removes EVERY text node (free-text comments included), plus
            // `@comment` blocks. Port of `text: () => []`.
            text: Some(Box::new(|_text, _ctx| Some(vec![]))),
            block: Some(Box::new(|block, _ctx| {
                if matches!(block.inner, Some(BlockInner::Comment { .. })) {
                    Some(vec![])
                } else {
                    Some(vec![block])
                }
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// A text node every one of whose lines starts with `%`.
fn is_bibtex_comment_text(text: &str) -> bool {
    text.trim()
        .lines()
        .all(|line| line.trim_start().starts_with('%'))
}

/// Remove fields whose rendered value is empty. Port of `createRemoveEmptyFieldsTransform`.
pub fn remove_empty_fields() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            field: Some(Box::new(|field, _ctx| {
                let is_empty = field
                    .values
                    .iter()
                    .all(|v| render_value_node(v).trim().is_empty());
                if is_empty {
                    Some(vec![])
                } else {
                    Some(vec![field])
                }
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Insert an empty line between each top-level item. Port of `createBlankLinesTransform`.
pub fn blank_lines() -> TransformFn {
    Box::new(|ast| {
        // The original inserts a blank line before every child whose predecessor is not a
        // "comment-like" node (a free-text node or a `@comment` block).
        let is_separator = |node: &BibtexNode| match node {
            BibtexNode::Text(_) => true,
            BibtexNode::Block(b) => matches!(b.inner, Some(BlockInner::Comment { .. })),
        };
        let children = ast
            .children
            .iter()
            .enumerate()
            .map(|(i, child)| {
                if i > 0 && !is_separator(&ast.children[i - 1]) {
                    set_top_level_prefix(child.clone(), "\n\n")
                } else {
                    child.clone()
                }
            })
            .collect();
        TransformResult {
            ast: Root { children },
            warnings: vec![],
        }
    })
}

/// Set only a top-level node's whitespace prefix, leaving its subtree (e.g. entry field
/// indents) untouched. The original's `child.with({ whitespacePrefix })` semantics.
fn set_top_level_prefix(node: BibtexNode, prefix: &str) -> BibtexNode {
    match node {
        BibtexNode::Text(mut t) => {
            t.whitespace_prefix = prefix.to_string();
            BibtexNode::Text(t)
        }
        BibtexNode::Block(mut b) => {
            b.whitespace_prefix = prefix.to_string();
            BibtexNode::Block(b)
        }
    }
}

/// Percent-encode literal underscores in `url` values (`_` and `\_` -> `%5F`). Port of
/// `createEncodeUrlsTransform`.
pub fn encode_urls() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, ctx| match ctx.current_field() {
                Some(field) if field.name.to_lowercase() == "url" => {
                    let rendered = latex::stringify(&block);
                    let encoded = encode_underscores(&rendered);
                    Some(vec![Value::Braced(latex::parse(&encoded))])
                }
                _ => None,
            })),
            quoted: Some(Box::new(|block, ctx| match ctx.current_field() {
                Some(field) if field.name.to_lowercase() == "url" => {
                    let rendered = latex::stringify(&block);
                    let encoded = encode_underscores(&rendered);
                    Some(vec![Value::Quoted(latex::parse(&encoded))])
                }
                _ => None,
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Replace every (optionally backslash-escaped) `_` with `\%5F`. Matches `/\\?_/g`.
fn encode_underscores(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '_' {
            out.push_str("\\%5F");
            i += 1;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '_' {
            out.push_str("\\%5F");
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Title-case the values of fields that contain no lowercase letters (all-caps titles).
/// Port of `createDropAllCapsTransform`.
pub fn drop_all_caps() -> TransformFn {
    Box::new(|ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, ctx| {
                drop_all_caps_visit(&Value::Braced(block), ctx.current_field())
            })),
            quoted: Some(Box::new(|block, ctx| {
                drop_all_caps_visit(&Value::Quoted(block), ctx.current_field())
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

fn drop_all_caps_visit(value: &Value, field: Option<&Field>) -> Option<Vec<Value>> {
    let field = field?;
    // Only rewrite when the whole field value has no lowercase letters.
    if render_field_value(field)
        .chars()
        .any(|c| c.is_ascii_lowercase())
    {
        return None;
    }
    let rendered = render_value_node(value);
    Some(vec![replace_value_node_text(value, &title_case(&rendered))])
}

/// Capitalize the first letter of each word, lowercasing the rest, but leave Roman
/// numerals untouched. Port of `titleCase`.
fn title_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if is_word_char(chars[i]) {
            // A "word" is a word character followed by maximal non-whitespace (`(\w)(\S*)`).
            let start = i;
            i += 1;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if is_roman_numeral(&word) {
                out.push_str(&word);
            } else {
                let mut wc = word.chars();
                let first = wc.next().unwrap_or_default();
                out.extend(first.to_uppercase());
                let rest: String = wc.collect();
                out.push_str(&rest.to_lowercase());
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// JS `\w`: ASCII letters, digits, and underscore.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Matches `/^M{0,4}(CM|CD|D?C{0,3})(XC|XL|L?X{0,3})(IX|IV|V?I{0,3})$/`.
fn is_roman_numeral(s: &str) -> bool {
    let b = s.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut m = 0;
    while m < 4 && i < n && b[i] == b'M' {
        i += 1;
        m += 1;
    }
    i = match_hundreds(b, i);
    i = match_tens(b, i);
    i = match_units(b, i);
    i == n
}

fn match_hundreds(b: &[u8], mut i: usize) -> usize {
    let n = b.len();
    if i + 1 < n && b[i] == b'C' && b[i + 1] == b'M' {
        return i + 2;
    }
    if i + 1 < n && b[i] == b'C' && b[i + 1] == b'D' {
        return i + 2;
    }
    if i < n && b[i] == b'D' {
        i += 1;
    }
    let mut c = 0;
    while c < 3 && i < n && b[i] == b'C' {
        i += 1;
        c += 1;
    }
    i
}

fn match_tens(b: &[u8], mut i: usize) -> usize {
    let n = b.len();
    if i + 1 < n && b[i] == b'X' && b[i + 1] == b'C' {
        return i + 2;
    }
    if i + 1 < n && b[i] == b'X' && b[i + 1] == b'L' {
        return i + 2;
    }
    if i < n && b[i] == b'L' {
        i += 1;
    }
    let mut x = 0;
    while x < 3 && i < n && b[i] == b'X' {
        i += 1;
        x += 1;
    }
    i
}

fn match_units(b: &[u8], mut i: usize) -> usize {
    let n = b.len();
    if i + 1 < n && b[i] == b'I' && b[i + 1] == b'X' {
        return i + 2;
    }
    if i + 1 < n && b[i] == b'I' && b[i + 1] == b'V' {
        return i + 2;
    }
    if i < n && b[i] == b'V' {
        i += 1;
    }
    let mut u = 0;
    while u < 3 && i < n && b[i] == b'I' {
        i += 1;
        u += 1;
    }
    i
}

/// Truncate the `author` field to `max` names, appending "others". Port of
/// `createLimitAuthorsTransform`.
pub fn limit_authors(max: usize) -> TransformFn {
    Box::new(move |ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, ctx| {
                limit_authors_visit(&Value::Braced(block), ctx.current_field(), max)
            })),
            quoted: Some(Box::new(|block, ctx| {
                limit_authors_visit(&Value::Quoted(block), ctx.current_field(), max)
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

fn limit_authors_visit(value: &Value, field: Option<&Field>, max: usize) -> Option<Vec<Value>> {
    let field = field?;
    if field.name.to_lowercase() != "author" {
        return None;
    }
    let rendered = render_value_node(value);
    let authors: Vec<&str> = rendered.split(" and ").collect();
    if authors.len() <= max {
        return None;
    }
    let mut kept: Vec<String> = authors[..max].iter().map(|s| s.to_string()).collect();
    kept.push("others".to_string());
    Some(vec![replace_value_node_text(value, &kept.join(" and "))])
}

/// Reverse the core Unicode->LaTeX escaping inside braced/quoted values. Port of
/// `createUnescapeCharactersTransform`.
pub fn unescape_characters() -> TransformFn {
    // escaped -> character, longest escaped first (mirrors the original's module-level sort).
    let mut table: Vec<(char, String)> = CORE_SPECIAL_CHARACTERS_REVERSIBLE
        .iter()
        .map(|(cp, escaped)| {
            (
                char::from_u32(*cp).expect("valid codepoint"),
                escaped.to_string(),
            )
        })
        .collect();
    table.sort_by_key(|(_, escaped)| std::cmp::Reverse(escaped.chars().count()));

    Box::new(move |ast| {
        let visitors = Visitors {
            braced: Some(Box::new(|block, _ctx| {
                let rendered = latex::stringify(&block);
                Some(vec![Value::Braced(latex::parse(&unescape(
                    &rendered, false, &table,
                )))])
            })),
            quoted: Some(Box::new(|block, _ctx| {
                let rendered = latex::stringify(&block);
                Some(vec![Value::Quoted(latex::parse(&unescape(
                    &rendered, true, &table,
                )))])
            })),
            ..Default::default()
        };
        TransformResult {
            ast: walk(ast, &visitors),
            warnings: vec![],
        }
    })
}

/// Replace every escaped core sequence with its Unicode character. When `quoted`, first
/// undo the `{"...}` forms (escaped sequences wrapped in braces). Port of
/// `unescapeCharacters`.
fn unescape(value: &str, quoted: bool, table: &[(char, String)]) -> String {
    let mut result = value.to_string();
    if quoted {
        for (character, escaped) in table {
            if escaped.contains('"') {
                let braced = format!("{{{escaped}}}");
                result = result.replace(&braced, &character.to_string());
            }
        }
    }
    for (character, escaped) in table {
        result = result.replace(escaped.as_str(), &character.to_string());
    }
    result
}

/// Replace every entry's citation key with one generated from a JabRef-style template.
/// Port of `createGenerateKeysTransform`. Entries whose generated key is empty keep their
/// existing key.
pub fn generate_keys(template: String) -> TransformFn {
    Box::new(move |ast| {
        let entries = crate::generate_keys::get_entries(&ast);
        let new_keys = crate::generate_keys::generate_keys(&entries, &template);
        let mut key_iter = new_keys.into_iter();
        let children = ast
            .children
            .into_iter()
            .map(|mut node| {
                if let BibtexNode::Block(ref mut block) = node
                    && let Some(BlockInner::Entry(ref mut entry)) = block.inner
                    && let Some(Some(new_key)) = key_iter.next()
                {
                    entry.key = Some(new_key);
                }
                node
            })
            .collect();
        TransformResult {
            ast: Root { children },
            warnings: vec![],
        }
    })
}

/// Sort the fields of each entry by a preferred field order (case-insensitive); fields not
/// in the order keep their relative positions after the ordered ones. Port of
/// `createSortFieldsTransform`.
pub fn sort_fields(field_order: &[String]) -> TransformFn {
    let order: Vec<String> = field_order.iter().map(|f| f.to_lowercase()).collect();
    Box::new(move |ast| TransformResult {
        ast: walk_entries(
            ast,
            Box::new(|entry, _ctx| {
                let mut fields = entry.fields.clone();
                fields.sort_by(|a, b| {
                    let ia = order.iter().position(|f| *f == a.name.to_lowercase());
                    let ib = order.iter().position(|f| *f == b.name.to_lowercase());
                    match (ia, ib) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (Some(x), Some(y)) => x.cmp(&y),
                    }
                });
                Some(Entry {
                    fields,
                    ..entry.clone()
                })
            }),
        ),
        warnings: vec![],
    })
}

/// Sort top-level entries by the given keys. Port of `createSortEntriesTransform`.
///
/// Each sort key may be prefixed with `-` for descending order. Keys `key`, `type`,
/// `month`, and `special` have dedicated semantics; any other key reads that field's
/// rendered value. Comments, `@string`/`@preamble` blocks, and BibLaTeX `@set`/`@xdata`
/// entries inherit the sort index of the entry they precede.
pub fn sort_entries(sort: &[String]) -> TransformFn {
    let sort: Vec<String> = sort.to_vec();
    Box::new(move |ast| {
        let sorts_special = sort.iter().any(|k| k == "special");
        // A sort index (key -> value) for every sortable top-level item; `None` for items
        // that stay put or inherit another entry's index.
        let mut indexes: Vec<Option<SortIndex>> = vec![None; ast.children.len()];
        let mut fixed_text: HashSet<usize> = HashSet::new();
        let mut preceding_meta: Vec<usize> = Vec::new();

        for (i, item) in ast.children.iter().enumerate() {
            match item {
                // BibTeX comments travel with the entry that follows them.
                BibtexNode::Text(t) if is_bibtex_comment_text(&t.text) => {
                    preceding_meta.push(i);
                }
                // Non-entry blocks travel with the following entry unless sorting specials.
                BibtexNode::Block(b)
                    if !matches!(b.inner, Some(BlockInner::Entry(_))) && !sorts_special =>
                {
                    preceding_meta.push(i);
                }
                // Other free text stays in place.
                BibtexNode::Text(_) => {
                    fixed_text.insert(i);
                }
                BibtexNode::Block(_) => {
                    let index = build_sort_index(item, &sort);
                    for meta in preceding_meta.drain(..) {
                        indexes[meta] = Some(index.clone());
                    }
                    indexes[i] = Some(index);
                }
            }
        }

        // Sortable items are every non-fixed-text child, in original order.
        let mut sortable: Vec<usize> = (0..ast.children.len())
            .filter(|i| !fixed_text.contains(i))
            .collect();

        // Apply keys from last to first; Rust's stable sort makes earlier keys dominant.
        for prefixed_key in sort.iter().rev() {
            let (desc, key) = match prefixed_key.strip_prefix('-') {
                Some(rest) => (true, rest.to_string()),
                None => (false, prefixed_key.clone()),
            };
            sortable.sort_by(|&a, &b| {
                let mut ia = sort_value_for(&indexes, a, &key);
                let mut ib = sort_value_for(&indexes, b, &key);
                if desc {
                    std::mem::swap(&mut ia, &mut ib);
                }
                // Proxy for the original's `localeCompare` over lowercased values.
                ia.cmp(&ib)
            });
        }

        // Rebuild the children: fixed text stays put, everything else takes sorted order.
        let mut sorted_iter = sortable.into_iter();
        let children: Vec<BibtexNode> = ast
            .children
            .iter()
            .enumerate()
            .map(|(i, item)| {
                if fixed_text.contains(&i) {
                    item.clone()
                } else {
                    let src = sorted_iter.next().unwrap_or(i);
                    ast.children[src].clone()
                }
            })
            .collect();
        TransformResult {
            ast: Root { children },
            warnings: vec![],
        }
    })
}

type SortIndex = Vec<(String, SortValue)>;

#[derive(Clone)]
enum SortValue {
    Str(String),
    Num(usize),
}

/// Sentinel used for items with no value for a key so they sort last.
const SORT_MISSING: &str = "\u{fff0}";

fn sort_value_for(indexes: &[Option<SortIndex>], i: usize, key: &str) -> String {
    let index = match &indexes[i] {
        Some(index) => index,
        None => return SORT_MISSING.to_string(),
    };
    match index.iter().find(|(k, _)| k == key) {
        Some((_, SortValue::Str(s))) => s.clone(),
        // Numbers sort as zero-padded strings (e.g. months) so they order numerically.
        Some((_, SortValue::Num(n))) => format!("{n:0>50}"),
        None => SORT_MISSING.to_string(),
    }
}

fn build_sort_index(item: &BibtexNode, sort: &[String]) -> SortIndex {
    let mut index = Vec::new();
    for raw_key in sort {
        let key = raw_key.strip_prefix('-').unwrap_or(raw_key);
        let val = match key {
            "key" => match entry_key(item) {
                Some(k) => SortValue::Str(k.to_lowercase()),
                None => continue,
            },
            "type" => SortValue::Str(block_command(item).to_lowercase()),
            "month" => {
                let v = entry_field(item, "month")
                    .map(render_field_value)
                    .unwrap_or_default();
                match MONTHS.iter().position(|m| *m == v) {
                    Some(i) => SortValue::Num(i),
                    None => SortValue::Str(String::new()),
                }
            }
            "special" => {
                let is_special = matches!(
                    item,
                    BibtexNode::Block(b) if matches!(
                        b.command.to_lowercase().as_str(),
                        "string" | "preamble" | "set" | "xdata"
                    )
                );
                SortValue::Num(if is_special { 0 } else { 1 })
            }
            _ => {
                let v = entry_field(item, key)
                    .map(render_field_value)
                    .unwrap_or_default();
                SortValue::Str(v.to_lowercase())
            }
        };
        index.push((key.to_string(), val));
    }
    index
}

fn entry_key(item: &BibtexNode) -> Option<&str> {
    match item {
        BibtexNode::Block(b) => match &b.inner {
            Some(BlockInner::Entry(e)) => e.key.as_deref(),
            _ => None,
        },
        _ => None,
    }
}

fn block_command(item: &BibtexNode) -> &str {
    match item {
        BibtexNode::Block(b) => &b.command,
        _ => "",
    }
}

fn entry_field<'a>(item: &'a BibtexNode, name: &str) -> Option<&'a Field> {
    match item {
        BibtexNode::Block(b) => match &b.inner {
            Some(BlockInner::Entry(e)) => e
                .fields
                .iter()
                .find(|f| f.name.to_lowercase() == name.to_lowercase()),
            _ => None,
        },
        _ => None,
    }
}
