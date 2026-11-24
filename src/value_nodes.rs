//! Helpers for rendering and rewriting field values. Port of `upstream/src/valueNodes.ts`
//! and `upstream/src/fieldValues.ts`.

use crate::ast::{Field, Value};
use crate::latex::{self, Block, BlockKind, LatexNode};

/// Render every value of a field as text, joined by ` # ` (used by transforms that read
/// the field's whole textual content, e.g. drop-all-caps and prefer-curly's month check).
pub fn render_field_value(field: &Field) -> String {
    field
        .values
        .iter()
        .map(|v| match v {
            Value::Literal(s) => s.clone(),
            Value::Braced(b) | Value::Quoted(b) => {
                b.children.iter().map(LatexNode::render_as_text).collect()
            }
        })
        .collect::<Vec<_>>()
        .join(" # ")
}

/// Render a single value node to its source form.
pub fn render_value_node(value: &Value) -> String {
    match value {
        Value::Literal(s) => s.clone(),
        Value::Braced(b) | Value::Quoted(b) => latex::stringify(b),
    }
}

/// Replace the textual content of a value node, re-parsing LaTeX for braced/quoted values.
pub fn replace_value_node_text(value: &Value, text: &str) -> Value {
    match value {
        Value::Literal(_) => Value::Literal(text.to_string()),
        Value::Braced(_) => Value::Braced(latex::parse(text)),
        Value::Quoted(_) => Value::Quoted(latex::parse(text)),
    }
}

/// Render a braced value, ensuring it is wrapped in double braces: `{{...}}`. A value
/// already of the form `{ { ... } }` is returned as-is (it renders to `{{...}}`); any
/// other braced value is wrapped once. Port of `doubleEncloseLatex` (the formatter's
/// braced-value renderer).
pub fn double_enclose_latex(block: &Block) -> String {
    let result = latex::stringify(block);
    if is_double_enclosed(block) {
        result
    } else {
        format!("{{{result}}}")
    }
}

/// Wrap a braced value's LaTeX in an extra curly group unless it is already enclosed in
/// double braces. Port of `encloseLatexInCurly` (the enclosing-braces transform).
pub fn enclose_latex_in_curly(block: &Block) -> Block {
    if is_double_enclosed(block) {
        return block.clone();
    }
    let mut root = Block::root();
    let wrapper = Block {
        kind: BlockKind::Curly,
        children: block.children.clone(),
    };
    root.children.push(LatexNode::Block(wrapper));
    root
}

/// Whether the block is already `{ { ... } }`: a single curly group whose sole child is
/// itself a curly group (of any contents). Shared guard of `doubleEncloseLatex` and
/// `encloseLatexInCurly`.
fn is_double_enclosed(block: &Block) -> bool {
    block.children.len() == 1
        && match &block.children[0] {
            LatexNode::Block(inner) if inner.kind == BlockKind::Curly => {
                inner.children.len() == 1
                    && matches!(&inner.children[0], LatexNode::Block(deepest) if deepest.kind == BlockKind::Curly)
            }
            _ => false,
        }
}
