//! Transform pipeline stages. Ports of the always-on and default-path transforms from
//! `upstream/src/transforms/*.ts`.
//!
//! Each transform consumes and rebuilds the [`Root`](crate::ast::Root) via the walker,
//! optionally emitting warnings. The full option-driven pipeline is assembled in
//! [`crate::pipeline`]; the transforms here cover the subset applied by the default
//! configuration (S2 skeleton) plus the option-parameterized formatting stages.

use crate::ast::{BibtexNode, Block, BlockInner, Entry, Field, Root, Value, Visitors, walk};
use crate::latex::{self, Block as LatexBlock, LatexNode};
use crate::types::{TransformFn, TransformResult, Warning};
use crate::unicode::{core_special_character, special_character};
use crate::utils::{unwrap_text, wrap_text};
use crate::value_nodes::{render_value_node, replace_value_node_text};

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
pub fn align_values(column: usize) -> TransformFn {
    Box::new(move |ast| {
        let visitors = Visitors {
            field: Some(Box::new(move |field, _ctx| {
                let pad = column.saturating_sub(field.name.chars().count()).max(1);
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
pub fn wrap_values(indent: &str, align: usize, wrap: Option<usize>) -> TransformFn {
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

fn wrap_value(
    node: &Value,
    indent: &str,
    align: usize,
    wrap: Option<usize>,
    single: bool,
) -> Value {
    let braced = match node {
        Value::Braced(b) => b,
        _ => return node.clone(),
    };
    let mut value = unwrap_text(&render_value_node(&Value::Braced(braced.clone())));
    if single {
        value = value.trim().to_string();
    }
    let line_length = format!("{indent}{align}{{{value}}}").chars().count();
    let multi_line = value.contains("\n\n");
    if wrap.map(|w| line_length > w).unwrap_or(false) || multi_line {
        let val_indent = indent.repeat(2);
        let paragraphs: Vec<String> = value
            .split("\n\n")
            .map(|p| {
                if let Some(wrap_col) = wrap {
                    wrap_text(p, wrap_col.saturating_sub(val_indent.chars().count()))
                        .join(&format!("\n{val_indent}"))
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
