//! The BibTeX parser: a faithful port of `upstream/src/parsers/bibtexParser.ts`.
//!
//! The original is a character-by-character state machine that tracks the current node
//! and its parent via a map. Here the current path is an explicit stack of builder
//! frames; children attach to their parent frame when they are popped (complete). This
//! preserves the exact states and transitions while fitting Rust ownership.

use crate::ast::{BibtexNode, Block, BlockInner, Entry, Field, Root, Text, Value, WrapType};
use crate::latex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub node_kind: &'static str,
    pub hint: Option<String>,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Line {}:{}: Syntax Error in {}",
            self.line, self.column, self.node_kind
        )?;
        if let Some(hint) = &self.hint {
            write!(f, " ({hint})")?;
        }
        Ok(())
    }
}

impl std::error::Error for SyntaxError {}

enum FieldMode {
    /// Accumulating the field name (before `=`).
    Name,
    /// Accumulating the value list; `can_consume` mirrors the concat node's flag.
    Value { can_consume: bool },
}

enum Frame {
    Root(Root),
    Text {
        text: String,
        ws: String,
    },
    Block {
        command: String,
        ws: String,
        inner: Option<BlockInner>,
    },
    Comment {
        raw: String,
        braces: i32,
        parens: i32,
    },
    Preamble {
        raw: String,
        braces: i32,
        parens: i32,
    },
    StringDef {
        raw: String,
        braces: i32,
        parens: i32,
    },
    Entry {
        key: Option<String>,
        key_ended: bool,
        wrap_type: WrapType,
        fields: Vec<Field>,
    },
    Field {
        field: Field,
        mode: FieldMode,
    },
    Literal {
        value: String,
    },
    Braced {
        latex: String,
        depth: i32,
    },
    Quoted {
        latex: String,
        depth: i32,
    },
}

/// Parse a BibTeX document into an AST. Syntax errors are returned, not thrown.
pub fn parse(input: &str) -> Result<Root, SyntaxError> {
    let mut stack: Vec<Frame> = vec![Frame::Root(Root::new())];
    let mut whitespace = String::new();
    let mut prev: Option<char> = None;
    let mut line: usize = 1;
    let mut column: usize = 0;

    let mut err: Option<SyntaxError> = None;

    for c in input.chars() {
        if c == '\n' {
            line += 1;
            column = 0;
        }
        column += 1;

        // Dispatch on the current frame kind. Each arm may push/pop frames.
        let kind = frame_kind(&stack);
        match kind {
            FrameKind::Root => {
                if c == '@' {
                    let ws = take(&mut whitespace);
                    stack.push(Frame::Block {
                        command: String::new(),
                        ws,
                        inner: None,
                    });
                } else if is_ws(c) {
                    whitespace.push(c);
                } else {
                    let ws = take(&mut whitespace);
                    stack.push(Frame::Text {
                        text: c.to_string(),
                        ws,
                    });
                }
            }

            FrameKind::Text => {
                let prev_is_boundary = matches!(prev, Some(p) if is_ws(p) || p == '}');
                if c == '@' && prev_is_boundary {
                    // The text ends; a new block begins (the `@` starts a command).
                    let frame = stack.pop().expect("non-empty");
                    attach(&mut stack, frame);
                    stack.push(Frame::Block {
                        command: String::new(),
                        ws: String::new(),
                        inner: None,
                    });
                } else {
                    if let Some(Frame::Text { text, .. }) = stack.last_mut() {
                        text.push(c);
                    }
                }
            }

            FrameKind::Block => {
                if c == '@' {
                    // The accumulated `@command` was actually text preceding this `@`.
                    let command = match stack.last() {
                        Some(Frame::Block { command, .. }) => command.clone(),
                        _ => unreachable!(),
                    };
                    let ws = match stack.last() {
                        Some(Frame::Block { ws, .. }) => ws.clone(),
                        _ => unreachable!(),
                    };
                    match root_last_mut(&mut stack) {
                        Some(BibtexNode::Text(t)) => {
                            t.text.push_str(&format!("@{command}"));
                        }
                        _ => {
                            // Insert a text node before the in-progress block.
                            if let Some(Frame::Root(root)) = stack.first_mut() {
                                root.children.push(BibtexNode::Text(Text {
                                    text: format!("@{command}"),
                                    whitespace_prefix: ws,
                                }));
                            }
                        }
                    }
                    if let Some(Frame::Block { command, .. }) = stack.last_mut() {
                        *command = String::new();
                    }
                } else if c == '{' || c == '(' {
                    let command = match stack.last() {
                        Some(Frame::Block { command, .. }) => command.clone(),
                        _ => unreachable!(),
                    };
                    let command_trimmed = command.trim().to_string();
                    if command_trimmed.is_empty() || command_trimmed.chars().any(is_ws) {
                        // A block without a command is invalid; treat as literal text.
                        let ws = match stack.last() {
                            Some(Frame::Block { ws, .. }) => ws.clone(),
                            _ => unreachable!(),
                        };
                        stack.pop();
                        stack.push(Frame::Text {
                            text: format!("@{command}{c}"),
                            ws,
                        });
                    } else {
                        if let Some(Frame::Block { command, .. }) = stack.last_mut() {
                            *command = command_trimmed;
                        }
                        let command_lc = match stack.last() {
                            Some(Frame::Block { command, .. }) => command.to_lowercase(),
                            _ => unreachable!(),
                        };
                        // The raw source for comment/string/preamble starts with `@cmd{`
                        // (or `@cmd(`), and the opening delimiter counts toward balance.
                        let raw = format!("@{}{}", command_lc, c);
                        let (braces, parens) = if c == '{' { (1, 0) } else { (0, 1) };
                        match &command_lc[..] {
                            "string" => stack.push(Frame::StringDef {
                                raw,
                                braces,
                                parens,
                            }),
                            "preamble" => stack.push(Frame::Preamble {
                                raw,
                                braces,
                                parens,
                            }),
                            "comment" => stack.push(Frame::Comment {
                                raw,
                                braces,
                                parens,
                            }),
                            _ => {
                                // A real entry: record it on the block and push its frame.
                                if let Some(Frame::Block { inner: slot, .. }) = stack.last_mut() {
                                    *slot = Some(BlockInner::Entry(Entry {
                                        key: None,
                                        fields: Vec::new(),
                                        wrap_type: if c == '{' {
                                            WrapType::Curly
                                        } else {
                                            WrapType::Paren
                                        },
                                    }));
                                }
                                stack.push(Frame::Entry {
                                    key: None,
                                    key_ended: false,
                                    wrap_type: if c == '{' {
                                        WrapType::Curly
                                    } else {
                                        WrapType::Paren
                                    },
                                    fields: Vec::new(),
                                });
                            }
                        }
                    }
                } else if matches!(c, '=' | '#' | ',' | '}' | ')' | ']' | '[') {
                    // Replace the block with literal text.
                    let command = match stack.last() {
                        Some(Frame::Block { command, .. }) => command.clone(),
                        _ => unreachable!(),
                    };
                    let ws = take(&mut whitespace);
                    stack.pop();
                    stack.push(Frame::Text {
                        text: format!("@{command}{c}"),
                        ws,
                    });
                } else {
                    if let Some(Frame::Block { command, .. }) = stack.last_mut() {
                        command.push(c);
                    }
                }
            }

            FrameKind::Comment | FrameKind::Preamble | FrameKind::StringDef => {
                // Count delimiters and accumulate the raw source.
                {
                    let frame = stack.last_mut().expect("non-empty");
                    let (raw, braces, parens) = match frame {
                        Frame::Comment {
                            raw,
                            braces,
                            parens,
                        } => (raw, braces, parens),
                        Frame::Preamble {
                            raw,
                            braces,
                            parens,
                        } => (raw, braces, parens),
                        Frame::StringDef {
                            raw,
                            braces,
                            parens,
                        } => (raw, braces, parens),
                        _ => unreachable!("inner block frame expected"),
                    };
                    match c {
                        '{' => *braces += 1,
                        '}' => *braces -= 1,
                        '(' => *parens += 1,
                        ')' => *parens -= 1,
                        _ => {}
                    }
                    raw.push(c);
                    let balanced = *braces == 0 && *parens == 0;
                    if balanced {
                        let frame = stack.pop().expect("non-empty");
                        attach(&mut stack, frame);
                        // Now the block is on top; pop it onto the root.
                        let block = stack.pop().expect("non-empty");
                        attach(&mut stack, block);
                    }
                }
            }

            FrameKind::Entry => {
                let wrap_type = match stack.last() {
                    Some(Frame::Entry { wrap_type, .. }) => *wrap_type,
                    _ => unreachable!(),
                };
                let closing = matches!(
                    (c, wrap_type),
                    ('}', WrapType::Curly) | (')', WrapType::Paren)
                );

                if is_ws(c) {
                    if let Some(Frame::Entry { key, key_ended, .. }) = stack.last_mut() {
                        *key_ended |= key.is_some();
                    }
                } else if c == ',' {
                    // Begin a new field; the entry stays on the stack to receive it.
                    stack.push(Frame::Field {
                        field: Field::new("", ""),
                        mode: FieldMode::Name,
                    });
                } else if closing {
                    unwind_to_root(&mut stack);
                } else if c == '=' {
                    // A no-key entry: the accumulated `key` is actually a field name.
                    let name = match stack.last() {
                        Some(Frame::Entry { key, .. }) => key.clone().unwrap_or_default(),
                        _ => unreachable!(),
                    };
                    if is_valid_field_name_str(&name) {
                        if let Some(Frame::Entry { key, .. }) = stack.last_mut() {
                            *key = None;
                        }
                        stack.push(Frame::Field {
                            field: Field::new(name, ""),
                            mode: FieldMode::Value { can_consume: true },
                        });
                    } else {
                        err = Some(syntax_error(line, column, "entry", None));
                    }
                } else {
                    let key_ended = match stack.last() {
                        Some(Frame::Entry { key_ended, .. }) => *key_ended,
                        _ => unreachable!(),
                    };
                    if key_ended {
                        err = Some(syntax_error(
                            line,
                            column,
                            "entry",
                            Some("The entry key cannot contain whitespace".into()),
                        ));
                    } else if !is_valid_key_character(c) {
                        err = Some(syntax_error(
                            line,
                            column,
                            "entry",
                            Some(format!("The entry key cannot contain the character ({c})")),
                        ));
                    } else {
                        if let Some(Frame::Entry { key, .. }) = stack.last_mut() {
                            let k = key.get_or_insert_with(String::new);
                            k.push(c);
                        }
                    }
                }
            }

            FrameKind::Field => {
                let mode = match stack.last() {
                    Some(Frame::Field { mode, .. }) => match mode {
                        FieldMode::Name => FieldMode::Name,
                        FieldMode::Value { can_consume } => FieldMode::Value {
                            can_consume: *can_consume,
                        },
                    },
                    _ => unreachable!(),
                };

                match mode {
                    FieldMode::Name => {
                        let wrap_type = entry_wrap_type(&stack);
                        let closing = matches!(
                            (c, wrap_type),
                            ('}', WrapType::Curly) | (')', WrapType::Paren)
                        );
                        if closing {
                            finish_field_name(&mut stack);
                            unwind_to_root(&mut stack);
                        } else if c == '=' {
                            finish_field_name(&mut stack);
                            if let Some(Frame::Field { mode, .. }) = stack.last_mut() {
                                *mode = FieldMode::Value { can_consume: true };
                            }
                        } else if c == ',' {
                            finish_field_name(&mut stack);
                            // Pop this field (attach drops it if nameless), begin a new one.
                            let frame = stack.pop().expect("non-empty");
                            attach(&mut stack, frame);
                            stack.push(Frame::Field {
                                field: Field::new("", ""),
                                mode: FieldMode::Name,
                            });
                        } else if !is_valid_field_name_char(c) {
                            err = Some(syntax_error(line, column, "field", None));
                        } else if field_name_is_empty(&stack) {
                            if !is_ws(c) {
                                if let Some(Frame::Field { field, .. }) = stack.last_mut() {
                                    field.name.push(c);
                                }
                            } else {
                                if let Some(Frame::Field { field, .. }) = stack.last_mut() {
                                    field.whitespace_prefix.push(c);
                                }
                            }
                        } else {
                            if let Some(Frame::Field { field, .. }) = stack.last_mut() {
                                field.name.push(c);
                            }
                        }
                    }

                    FieldMode::Value { can_consume } => {
                        if is_ws(c) {
                            // no-op
                        } else if can_consume {
                            if matches!(c, '#' | '=' | ',' | '}' | '(' | ')' | '[' | ']') {
                                err = Some(syntax_error(line, column, "concat", None));
                            } else {
                                set_can_consume(&mut stack, false);
                                match c {
                                    '{' => stack.push(Frame::Braced {
                                        latex: String::new(),
                                        depth: 0,
                                    }),
                                    '"' => stack.push(Frame::Quoted {
                                        latex: String::new(),
                                        depth: 0,
                                    }),
                                    _ => stack.push(Frame::Literal {
                                        value: c.to_string(),
                                    }),
                                }
                            }
                        } else {
                            let wrap_type = entry_wrap_type(&stack);
                            let closing = matches!(
                                (c, wrap_type),
                                ('}', WrapType::Curly) | (')', WrapType::Paren)
                            );
                            if c == ',' {
                                // Finish this field and begin a new one.
                                let frame = stack.pop().expect("non-empty");
                                attach(&mut stack, frame);
                                stack.push(Frame::Field {
                                    field: Field::new("", ""),
                                    mode: FieldMode::Name,
                                });
                            } else if closing {
                                unwind_to_root(&mut stack);
                            } else if c == '#' {
                                set_can_consume(&mut stack, true);
                            } else {
                                err = Some(syntax_error(line, column, "concat", None));
                            }
                        }
                    }
                }
            }

            FrameKind::Literal => {
                let wrap_type = entry_wrap_type(&stack);
                let closing = matches!(
                    (c, wrap_type),
                    ('}', WrapType::Curly) | (')', WrapType::Paren)
                );
                if is_ws(c) {
                    let frame = stack.pop().expect("non-empty");
                    attach(&mut stack, frame);
                } else if c == ',' {
                    let frame = stack.pop().expect("non-empty");
                    attach(&mut stack, frame);
                    stack.push(Frame::Field {
                        field: Field::new("", ""),
                        mode: FieldMode::Name,
                    });
                } else if closing {
                    unwind_to_root(&mut stack);
                } else if c == '#' {
                    let frame = stack.pop().expect("non-empty");
                    attach(&mut stack, frame);
                    set_can_consume(&mut stack, true);
                } else {
                    if let Some(Frame::Literal { value, .. }) = stack.last_mut() {
                        value.push(c);
                    }
                }
            }

            FrameKind::Braced | FrameKind::Quoted => {
                let is_braced = matches!(kind, FrameKind::Braced);
                let depth = match stack.last() {
                    Some(Frame::Braced { depth, .. }) | Some(Frame::Quoted { depth, .. }) => *depth,
                    _ => unreachable!(),
                };
                if (is_braced && c == '}') || (!is_braced && c == '"' && depth == 0) {
                    // Value complete: parse the accumulated LaTeX and attach.
                    let latex_src = match stack.last_mut() {
                        Some(Frame::Braced { latex, .. }) | Some(Frame::Quoted { latex, .. }) => {
                            std::mem::take(latex)
                        }
                        _ => unreachable!(),
                    };
                    let block = latex::parse(&latex_src);
                    let frame = stack.pop().expect("non-empty");
                    let value = match frame {
                        Frame::Braced { .. } => Value::Braced(block),
                        Frame::Quoted { .. } => Value::Quoted(block),
                        _ => unreachable!(),
                    };
                    attach_value(&mut stack, value);
                } else {
                    match stack.last_mut() {
                        Some(Frame::Braced { latex, depth, .. })
                        | Some(Frame::Quoted { latex, depth, .. }) => {
                            match c {
                                '{' => *depth += 1,
                                '}' => {
                                    *depth -= 1;
                                    if !is_braced && *depth < 0 {
                                        err = Some(syntax_error(line, column, "quoted", None));
                                    }
                                }
                                _ => {}
                            }
                            latex.push(c);
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }

        if err.is_some() {
            break;
        }
        prev = Some(c);
    }

    if let Some(e) = err {
        return Err(e);
    }

    // Finalize: unwind any frames left open at end of input.
    while stack.len() > 1 {
        let frame = stack.pop().expect("non-empty");
        attach(&mut stack, frame);
    }
    match stack.pop().expect("root frame") {
        Frame::Root(root) => Ok(root),
        _ => unreachable!("root frame must be Root"),
    }
}

#[derive(Clone, Copy)]
enum FrameKind {
    Root,
    Text,
    Block,
    Comment,
    Preamble,
    StringDef,
    Entry,
    Field,
    Literal,
    Braced,
    Quoted,
}

fn frame_kind(stack: &[Frame]) -> FrameKind {
    match stack.last() {
        Some(Frame::Root(_)) => FrameKind::Root,
        Some(Frame::Text { .. }) => FrameKind::Text,
        Some(Frame::Block { .. }) => FrameKind::Block,
        Some(Frame::Comment { .. }) => FrameKind::Comment,
        Some(Frame::Preamble { .. }) => FrameKind::Preamble,
        Some(Frame::StringDef { .. }) => FrameKind::StringDef,
        Some(Frame::Entry { .. }) => FrameKind::Entry,
        Some(Frame::Field { .. }) => FrameKind::Field,
        Some(Frame::Literal { .. }) => FrameKind::Literal,
        Some(Frame::Braced { .. }) => FrameKind::Braced,
        Some(Frame::Quoted { .. }) => FrameKind::Quoted,
        None => unreachable!("empty parser stack"),
    }
}

fn take(s: &mut String) -> String {
    std::mem::take(s)
}

/// JavaScript's `\s` (used throughout the original) includes the byte-order mark
/// `\u{FEFF}`, which Rust's `char::is_whitespace` does not. Match JS semantics exactly.
fn is_ws(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn syntax_error(
    line: usize,
    column: usize,
    node_kind: &'static str,
    hint: Option<String>,
) -> SyntaxError {
    SyntaxError {
        line,
        column,
        node_kind,
        hint,
    }
}

fn is_valid_key_character(c: char) -> bool {
    !matches!(c, '#' | '%' | '{' | '}' | '~' | '$' | ',')
}

fn is_valid_field_name_char(c: char) -> bool {
    !matches!(c, '=' | ',' | '{' | '}' | '(' | ')' | '[' | ']')
}

fn is_valid_field_name_str(s: &str) -> bool {
    s.chars().all(is_valid_field_name_char)
}

/// The wrap type of the entry enclosing the current frame.
fn entry_wrap_type(stack: &[Frame]) -> WrapType {
    for frame in stack.iter().rev() {
        if let Frame::Entry { wrap_type, .. } = frame {
            return *wrap_type;
        }
    }
    WrapType::Curly
}

/// Whether the current field frame's name is still empty.
fn field_name_is_empty(stack: &[Frame]) -> bool {
    matches!(stack.last(), Some(Frame::Field { field, .. }) if field.name.is_empty())
}

fn finish_field_name(stack: &mut [Frame]) {
    if let Some(Frame::Field { field, .. }) = stack.last_mut() {
        field.name = field.name.trim().to_string();
    }
}

fn set_can_consume(stack: &mut [Frame], value: bool) {
    if let Some(Frame::Field {
        mode: FieldMode::Value { can_consume },
        ..
    }) = stack.last_mut()
    {
        *can_consume = value;
    }
}

/// Pop and attach frames until only the root frame remains. Used when an entry closes:
/// the field, entry, and block frames unwind onto the root in order.
fn unwind_to_root(stack: &mut Vec<Frame>) {
    while stack.len() > 1 {
        let frame = stack.pop().expect("non-empty");
        attach(stack, frame);
    }
}

fn root_last_mut(stack: &mut [Frame]) -> Option<&mut BibtexNode> {
    match stack.first_mut() {
        Some(Frame::Root(root)) => root.children.last_mut(),
        _ => None,
    }
}

/// Attach a completed value to the current field frame.
fn attach_value(stack: &mut [Frame], value: Value) {
    if let Some(Frame::Field { field, .. }) = stack.last_mut() {
        field.values.push(value);
    }
}

/// Pop-independently attach a completed frame to its parent (the new top).
fn attach(stack: &mut [Frame], frame: Frame) {
    match frame {
        Frame::Text { text, ws } => {
            let node = BibtexNode::Text(Text {
                text,
                whitespace_prefix: ws,
            });
            if let Some(Frame::Root(root)) = stack.last_mut() {
                root.children.push(node);
            }
        }
        Frame::Block { command, ws, inner } => {
            let node = BibtexNode::Block(Block {
                command,
                whitespace_prefix: ws,
                inner,
            });
            if let Some(Frame::Root(root)) = stack.last_mut() {
                root.children.push(node);
            }
        }
        Frame::Comment { raw, .. } => {
            if let Some(Frame::Block { inner, .. }) = stack.last_mut() {
                *inner = Some(BlockInner::Comment { raw });
            }
        }
        Frame::Preamble { raw, .. } => {
            if let Some(Frame::Block { inner, .. }) = stack.last_mut() {
                *inner = Some(BlockInner::Preamble { raw });
            }
        }
        Frame::StringDef { raw, .. } => {
            if let Some(Frame::Block { inner, .. }) = stack.last_mut() {
                *inner = Some(BlockInner::StringDef { raw });
            }
        }
        Frame::Entry {
            key,
            fields,
            wrap_type,
            ..
        } => {
            if let Some(Frame::Block { inner, .. }) = stack.last_mut() {
                *inner = Some(BlockInner::Entry(Entry {
                    key,
                    fields,
                    wrap_type,
                }));
            }
        }
        Frame::Field { field, .. } => {
            // Fields with a name are added to the entry; nameless fields (e.g. from a
            // trailing comma) are dropped, matching the original.
            if !field.name.is_empty()
                && let Some(Frame::Entry { fields, .. }) = stack.last_mut()
            {
                fields.push(field);
            }
        }
        Frame::Literal { value } => {
            let v = Value::Literal(value);
            if let Some(Frame::Field { field, .. }) = stack.last_mut() {
                field.values.push(v);
            }
        }
        // Braced/Quoted frames only reach attach() at end-of-input (an unterminated
        // value). The original never assigns the accumulated latex to the node in that
        // case, so the value becomes an empty group — discard the partial latex here.
        Frame::Braced { .. } => {
            let block = latex::parse("");
            if let Some(Frame::Field { field, .. }) = stack.last_mut() {
                field.values.push(Value::Braced(block));
            }
        }
        Frame::Quoted { .. } => {
            let block = latex::parse("");
            if let Some(Frame::Field { field, .. }) = stack.last_mut() {
                field.values.push(Value::Quoted(block));
            }
        }
        Frame::Root(_) => unreachable!("cannot attach a root frame"),
    }
}
