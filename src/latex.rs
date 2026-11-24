//! LaTeX AST, parser, and stringifier.
//!
//! Faithful port of `upstream/src/parsers/latexParser.ts`. The LaTeX AST represents the
//! contents of braced/quoted BibTeX values so that transforms can inspect and rewrite the
//! value while preserving LaTeX commands, math, and grouping.

/// A node in the LaTeX AST.
#[derive(Debug, Clone, PartialEq)]
pub enum LatexNode {
    /// A grouping block: `root` (the implicit top level), `curly` (`{...}`), or `square` (`[...]`).
    Block(Block),
    /// Plain text.
    Text(String),
    /// Inline math `$...$`; the stored text excludes the delimiters.
    Math(String),
    /// A LaTeX command `\name` with block arguments.
    Command(Command),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub children: Vec<LatexNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Root,
    Curly,
    Square,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub name: String,
    pub args: Vec<Block>,
}

impl LatexNode {
    /// Render the node as plain text. A text node drops double quotes (a deliberate hack
    /// in the original to work around `{"}` escapes).
    pub fn render_as_text(&self) -> String {
        match self {
            LatexNode::Block(b) => b.children.iter().map(LatexNode::render_as_text).collect(),
            LatexNode::Text(t) => t.replace('"', ""),
            LatexNode::Math(t) => t.clone(),
            LatexNode::Command(c) => c
                .args
                .iter()
                .map(|arg| {
                    arg.children
                        .iter()
                        .map(LatexNode::render_as_text)
                        .collect::<String>()
                })
                .collect(),
        }
    }
}

impl Block {
    pub fn new(kind: BlockKind) -> Self {
        Block {
            kind,
            children: Vec::new(),
        }
    }

    pub fn root() -> Self {
        Block::new(BlockKind::Root)
    }
}

/// A parser stack frame: the path from the root block to the current node.
enum Frame {
    Block(Block),
    Command {
        name: String,
        args: Vec<Block>,
        parent_kind: BlockKind,
    },
}

/// Parse a LaTeX string into a root block. Faithful port of `parseLaTeX`.
pub fn parse(input: &str) -> Block {
    // The parser is a state machine whose stack holds the path from the root block to the
    // current node. Children attach to their parent frame when they are popped (complete).
    // Text is emitted one character at a time as `LatexNode::Text` leaves; this is
    // output-equivalent to the original's accumulated text nodes.
    let root = Block::root();
    let mut stack: Vec<Frame> = vec![Frame::Block(root)];
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        match stack.last() {
            Some(Frame::Block(_)) => {
                if c == '\\' {
                    // A command begins; the parent kind is this block's kind.
                    let parent_kind = block_kind_of(&stack);
                    stack.push(Frame::Command {
                        name: String::new(),
                        args: Vec::new(),
                        parent_kind,
                    });
                } else if c == '$' {
                    if let Some(math) = parse_math(&chars, i) {
                        append_child(&mut stack, LatexNode::Math(math.text));
                        i = math.end;
                    } else {
                        append_child(&mut stack, LatexNode::Text(c.to_string()));
                    }
                } else if c == '{' {
                    stack.push(Frame::Block(Block::new(BlockKind::Curly)));
                } else if is_block_close(c, block_kind_of(&stack)) && stack.len() > 1 {
                    let frame = stack.pop().expect("non-empty");
                    attach_frame(&mut stack, frame);
                } else {
                    append_child(&mut stack, LatexNode::Text(c.to_string()));
                }
            }

            Some(Frame::Command { .. }) => {
                let (parent_kind, has_args) = match stack.last() {
                    Some(Frame::Command {
                        parent_kind, args, ..
                    }) => (*parent_kind, !args.is_empty()),
                    _ => unreachable!("command frame expected"),
                };
                if c == '{' {
                    stack.push(Frame::Block(Block::new(BlockKind::Curly)));
                } else if c == '[' {
                    stack.push(Frame::Block(Block::new(BlockKind::Square)));
                } else if is_block_close(c, parent_kind) || c.is_whitespace() || has_args {
                    // The command name (and any args) are complete; the char is reprocessed
                    // in the parent block state.
                    let frame = stack.pop().expect("non-empty");
                    attach_frame(&mut stack, frame);
                    i -= 1;
                } else {
                    if let Some(Frame::Command { name, .. }) = stack.last_mut() {
                        name.push(c);
                    }
                }
            }

            None => unreachable!("latex parser stack underflow"),
        }

        i += 1;
    }

    // Finalize: unwind any frames left open at end of input.
    while stack.len() > 1 {
        let frame = stack.pop().expect("non-empty");
        attach_frame(&mut stack, frame);
    }
    match stack.pop().expect("root frame") {
        Frame::Block(b) => b,
        _ => unreachable!("root frame must be a block"),
    }
}

fn block_kind_of(stack: &[Frame]) -> BlockKind {
    match stack.last() {
        Some(Frame::Block(b)) => b.kind,
        _ => BlockKind::Root,
    }
}

fn is_block_close(c: char, kind: BlockKind) -> bool {
    matches!(
        (c, kind),
        ('}', BlockKind::Curly) | (']', BlockKind::Square)
    )
}

struct Math {
    text: String,
    end: usize,
}

/// If a `$`-delimited math span starts at `start`, return its inner text and the index of
/// the closing `$`. Escaped `\$` does not close the math.
fn parse_math(chars: &[char], start: usize) -> Option<Math> {
    let mut escaped = false;
    let mut i = start + 1;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '$' {
            let text: String = chars[start + 1..i].iter().collect();
            return Some(Math { text, end: i });
        }
        i += 1;
    }
    None
}

/// Append a leaf child (text or math) to the current block frame.
fn append_child(stack: &mut [Frame], node: LatexNode) {
    match stack.last_mut() {
        Some(Frame::Block(b)) => b.children.push(node),
        // Text/math are always children of a block (never of a command).
        Some(Frame::Command { .. }) => unreachable!("cannot append leaf child to command"),
        None => unreachable!("empty latex stack"),
    }
}

/// Attach a completed frame to its parent (the new top of the stack).
fn attach_frame(stack: &mut [Frame], frame: Frame) {
    match frame {
        Frame::Block(b) => {
            let node = LatexNode::Block(b);
            match stack.last_mut() {
                Some(Frame::Block(parent)) => parent.children.push(node),
                Some(Frame::Command { args, .. }) => {
                    if let LatexNode::Block(b) = node {
                        args.push(b);
                    }
                }
                _ => unreachable!("block parent must be block or command"),
            }
        }
        Frame::Command { name, args, .. } => {
            let node = LatexNode::Command(Command { name, args });
            match stack.last_mut() {
                Some(Frame::Block(parent)) => parent.children.push(node),
                _ => unreachable!("command parent must be a block"),
            }
        }
    }
}

/// Render a LaTeX AST back to its source form.
pub fn stringify(block: &Block) -> String {
    let content: String = block
        .children
        .iter()
        .map(|node| match node {
            LatexNode::Block(b) => stringify(b),
            LatexNode::Command(c) => {
                let args: String = c.args.iter().map(stringify).collect();
                format!("\\{}{}", c.name, args)
            }
            LatexNode::Math(t) => format!("${t}$"),
            LatexNode::Text(t) => t.clone(),
        })
        .collect();
    match block.kind {
        BlockKind::Root => content,
        BlockKind::Curly => format!("{{{content}}}"),
        BlockKind::Square => format!("[{content}]"),
    }
}

/// Remove curly-brace grouping unless a group contains (or is) a command. Port of
/// `flattenLaTeX` (used by the `remove-braces` transform).
pub fn flatten(block: &Block) -> Block {
    let mut new_block = Block::new(block.kind);
    for child in &block.children {
        if let LatexNode::Block(b) = child
            && b.kind == BlockKind::Curly
            && b.children
                .iter()
                .all(|c| !matches!(c, LatexNode::Command(_)))
        {
            let flattened = flatten(b);
            new_block.children.extend(flattened.children);
            continue;
        }
        new_block.children.push(child.clone());
    }
    new_block
}
