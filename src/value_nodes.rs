//! Helpers for rendering and rewriting field values. Port of `upstream/src/valueNodes.ts`.

use crate::ast::Value;
use crate::latex::{self, Block, BlockKind, LatexNode};

/// Render a single value node to its source form (used for quoted values).
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

fn is_double_enclosed(block: &Block) -> bool {
    block.children.len() == 1
        && match &block.children[0] {
            LatexNode::Block(inner) => {
                inner.kind == BlockKind::Curly
                    && inner.children.len() == 1
                    && match &inner.children[0] {
                        LatexNode::Block(deepest) => {
                            deepest.kind == BlockKind::Curly && deepest.children.len() == 1
                        }
                        _ => false,
                    }
            }
            _ => false,
        }
}

/// Render a braced value, ensuring the whole value is wrapped in one extra set of braces
/// unless it is already double-enclosed.
pub fn double_enclose_latex(block: &Block) -> String {
    let result = latex::stringify(block);
    if is_double_enclosed(block) {
        result
    } else {
        format!("{{{result}}}")
    }
}
