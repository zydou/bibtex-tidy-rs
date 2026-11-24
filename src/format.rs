//! Render an AST back to BibTeX source. Port of `upstream/src/format.ts`.

use crate::ast::{BibtexNode, BlockInner, Field, Root, Value};
use crate::value_nodes::{double_enclose_latex, render_value_node};

/// Render the AST to a BibTeX string (always ends with a trailing newline). Returns an
/// error for malformed blocks that have no inner content (the original throws "FATAL!").
pub fn format_bibtex(root: &Root) -> Result<String, String> {
    let mut out = String::new();
    for child in &root.children {
        out.push_str(&format_node(child)?);
    }
    let trimmed = out.trim_end();
    Ok(format!("{trimmed}\n"))
}

fn format_node(node: &BibtexNode) -> Result<String, String> {
    match node {
        BibtexNode::Text(t) => Ok(format!("{}{}", t.whitespace_prefix, t.text)),
        BibtexNode::Block(block) => {
            let inner = block.inner.as_ref().ok_or("FATAL!".to_string())?;
            match inner {
                BlockInner::Comment { raw }
                | BlockInner::Preamble { raw }
                | BlockInner::StringDef { raw } => {
                    Ok(format!("{}{}", block.whitespace_prefix, raw))
                }
                BlockInner::Entry(entry) => Ok(format!(
                    "{}{}",
                    block.whitespace_prefix,
                    format_entry(&block.command, entry)?
                )),
            }
        }
    }
}

fn format_entry(entry_type: &str, entry: &crate::ast::Entry) -> Result<String, String> {
    let mut out = format!("@{entry_type}{{");
    if let Some(key) = &entry.key {
        out.push_str(key);
        out.push(',');
    }
    for field in &entry.fields {
        out.push_str(&field.whitespace_prefix);
        out.push_str(&field.name);
        if let Some(value) = format_value(field)? {
            // `value_prefix` carries the concat node's whitespace prefix (alignment).
            out.push_str(&format!("{}= {value}", field.value_prefix));
        }
        if field.has_comma {
            out.push(',');
        }
    }
    out.push_str("\n}");
    Ok(out)
}

fn format_value(field: &Field) -> Result<Option<String>, String> {
    if field.values.is_empty() {
        return Ok(None);
    }
    let parts: Result<Vec<String>, String> = field
        .values
        .iter()
        .map(|v| match v {
            Value::Literal(s) => Ok(s.clone()),
            Value::Braced(b) => Ok(double_enclose_latex(b)),
            Value::Quoted(b) => Ok(format!(
                "\"{}\"",
                render_value_node(&Value::Quoted(b.clone()))
            )),
        })
        .collect();
    Ok(Some(parts?.join(" # ")))
}
