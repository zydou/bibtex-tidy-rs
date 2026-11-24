//! BibTeX AST and the generic tree walker used by transforms.
//!
//! The AST is an owned, enum-based tree mirroring `upstream/src/parsers/bibtexParser.ts`.
//! Transforms rebuild the tree functionally through [`walk`] with per-node-type visitors.
//!
//! Walker semantics match the original `walkAst`: a visitor returns zero or more
//! replacements for its node (an empty return removes the node and its subtree); when a
//! node kind has no visitor, its children are walked by default.

use crate::latex::Block as LatexBlock;

/// A full BibTeX document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Root {
    pub children: Vec<BibtexNode>,
}

/// Top-level AST nodes.
#[derive(Debug, Clone, PartialEq)]
pub enum BibtexNode {
    Text(Text),
    Block(Block),
}

/// The `@command{...}` wrapper (or a text node in its place for malformed input).
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The command/entry-type name following `@` (e.g. `article`).
    pub command: String,
    pub whitespace_prefix: String,
    pub inner: Option<BlockInner>,
}

/// The inner content of a block.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockInner {
    Entry(Entry),
    Comment { raw: String },
    Preamble { raw: String },
    StringDef { raw: String },
}

/// A `@type{key, field = value, ...}` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: Option<String>,
    pub fields: Vec<Field>,
    /// The opening delimiter, preserved so formatting can round-trip.
    pub wrap_type: WrapType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapType {
    Curly,
    Paren,
}

/// A `name = value` pair within an entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub whitespace_prefix: String,
    /// Spacing between the field name and `=` (the concat node's whitespace prefix,
    /// which the align transform sets to pad values into a column).
    pub value_prefix: String,
    pub values: Vec<Value>,
    pub has_comma: bool,
}

/// A field value component; a field may concatenate several with `#`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Literal(String),
    Braced(LatexBlock),
    Quoted(LatexBlock),
}

/// The Text AST node (free text between/around entries).
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub text: String,
    pub whitespace_prefix: String,
}

impl Field {
    pub fn new(name: impl Into<String>, whitespace_prefix: impl Into<String>) -> Self {
        Field {
            name: name.into(),
            whitespace_prefix: whitespace_prefix.into(),
            value_prefix: String::new(),
            values: Vec::new(),
            has_comma: false,
        }
    }
}

impl Root {
    pub fn new() -> Self {
        Root {
            children: Vec::new(),
        }
    }
}

/// The context handed to a visitor. When visiting a value or concat node it carries the
/// field that owns the value.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    current_field: Option<&'a Field>,
}

impl<'a> Ctx<'a> {
    /// The field whose value (or concat) is being visited, if any.
    pub fn current_field(&self) -> Option<&Field> {
        self.current_field
    }
}

/// Visitor closure types. Each returns an `Option`: `None` means "no replacement" (keep
/// the node, walk its children by default); `Some` replaces the node's content (an empty
/// vec removes the node/subtree).
pub type RootVisitor<'a> = Box<dyn Fn(Root, &Ctx) -> Option<Root> + 'a>;
pub type TextVisitor<'a> = Box<dyn Fn(Text, &Ctx) -> Option<Vec<BibtexNode>> + 'a>;
pub type BlockVisitor<'a> = Box<dyn Fn(Block, &Ctx) -> Option<Vec<Block>> + 'a>;
pub type EntryVisitor<'a> = Box<dyn Fn(Entry, &Ctx) -> Option<Entry> + 'a>;
pub type FieldVisitor<'a> = Box<dyn Fn(Field, &Ctx) -> Option<Vec<Field>> + 'a>;
/// Visits a field's value list (the `concat` node).
pub type ConcatVisitor<'a> = Box<dyn Fn(&Field, &Ctx) -> Option<Vec<Value>> + 'a>;
/// Visits a braced/quoted value's LaTeX tree.
pub type LatexValueVisitor<'a> = Box<dyn Fn(LatexBlock, &Ctx) -> Option<Vec<Value>> + 'a>;

/// Per-node-type visitor hooks. A `None` hook means the node kind is walked by default.
#[derive(Default)]
pub struct Visitors<'a> {
    pub root: Option<RootVisitor<'a>>,
    pub text: Option<TextVisitor<'a>>,
    pub block: Option<BlockVisitor<'a>>,
    pub entry: Option<EntryVisitor<'a>>,
    pub field: Option<FieldVisitor<'a>>,
    pub concat: Option<ConcatVisitor<'a>>,
    pub braced: Option<LatexValueVisitor<'a>>,
    pub quoted: Option<LatexValueVisitor<'a>>,
}

fn root_ctx() -> Ctx<'static> {
    Ctx {
        current_field: None,
    }
}

/// Walk the tree, applying visitors and rebuilding changed branches.
pub fn walk(root: Root, visitors: &Visitors) -> Root {
    if let Some(v) = &visitors.root
        && let Some(new_root) = v(root.clone(), &root_ctx())
    {
        return new_root;
    }
    walk_root(root, visitors)
}

fn walk_root(root: Root, visitors: &Visitors) -> Root {
    let mut children = Vec::with_capacity(root.children.len());
    for child in root.children {
        children.extend(walk_top(child, visitors));
    }
    Root { children }
}

fn walk_top(node: BibtexNode, visitors: &Visitors) -> Vec<BibtexNode> {
    match node {
        BibtexNode::Text(t) => walk_text(t, visitors),
        BibtexNode::Block(b) => walk_block(b, visitors),
    }
}

fn walk_text(text: Text, visitors: &Visitors) -> Vec<BibtexNode> {
    match &visitors.text {
        Some(v) => v(text.clone(), &root_ctx()).unwrap_or_else(|| vec![BibtexNode::Text(text)]),
        None => vec![BibtexNode::Text(text)],
    }
}

fn walk_block(block: Block, visitors: &Visitors) -> Vec<BibtexNode> {
    let blocks = match &visitors.block {
        Some(v) => v(block.clone(), &root_ctx()).unwrap_or_else(|| vec![block]),
        None => vec![block],
    };
    let mut out = Vec::with_capacity(blocks.len());
    for b in blocks {
        let inner = walk_inner(b.inner, visitors);
        out.push(BibtexNode::Block(Block { inner, ..b }));
    }
    out
}

fn walk_inner(inner: Option<BlockInner>, visitors: &Visitors) -> Option<BlockInner> {
    match inner? {
        BlockInner::Entry(e) => Some(BlockInner::Entry(walk_entry(e, visitors))),
        other => Some(other),
    }
}

fn walk_entry(entry: Entry, visitors: &Visitors) -> Entry {
    let mut fields = Vec::with_capacity(entry.fields.len());
    for field in &entry.fields {
        fields.extend(walk_field(field.clone(), visitors));
    }
    let entry = Entry { fields, ..entry };
    match &visitors.entry {
        Some(v) => v(entry.clone(), &root_ctx()).unwrap_or(entry),
        None => entry,
    }
}

fn walk_field(field: Field, visitors: &Visitors) -> Vec<Field> {
    // The field visitor runs on the node first; its result's value list (concat) is then
    // walked, mirroring the original's "visit node, then walk children" order.
    let results = match &visitors.field {
        Some(v) => v(field.clone(), &root_ctx()).unwrap_or_else(|| vec![field]),
        None => vec![field],
    };
    let mut out = Vec::with_capacity(results.len());
    for f in results {
        let values = walk_concat(&f, visitors);
        out.push(Field { values, ..f });
    }
    out
}

fn walk_concat(field: &Field, visitors: &Visitors) -> Vec<Value> {
    let ctx = Ctx {
        current_field: Some(field),
    };
    if let Some(v) = &visitors.concat
        && let Some(values) = v(field, &ctx)
    {
        return walk_values(values, visitors, Some(field));
    }
    walk_values(field.values.clone(), visitors, Some(field))
}

fn walk_values(values: Vec<Value>, visitors: &Visitors, field: Option<&Field>) -> Vec<Value> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.extend(walk_value(value, visitors, field));
    }
    out
}

fn walk_value(value: Value, visitors: &Visitors, field: Option<&Field>) -> Vec<Value> {
    match value {
        Value::Literal(_) => vec![value],
        Value::Braced(lb) => walk_latex_value(Value::Braced(lb), visitors.braced.as_ref(), field),
        Value::Quoted(lb) => walk_latex_value(Value::Quoted(lb), visitors.quoted.as_ref(), field),
    }
}

fn walk_latex_value(
    original: Value,
    visitor: Option<&LatexValueVisitor>,
    field: Option<&Field>,
) -> Vec<Value> {
    match visitor {
        Some(v) => {
            let ctx = Ctx {
                current_field: field,
            };
            let lb = match &original {
                Value::Braced(b) | Value::Quoted(b) => b.clone(),
                Value::Literal(_) => unreachable!("latex value visitor on literal"),
            };
            match v(lb, &ctx) {
                // A braced/quoted visitor fully processes its value; its replacements are
                // final and must NOT be re-walked with the same visitor (that would recurse
                // forever for visitors that always return `Some`).
                Some(values) => values,
                None => vec![original],
            }
        }
        None => vec![original],
    }
}
