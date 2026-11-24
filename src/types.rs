//! Shared result/warning types. Port of the relevant parts of `upstream/src/types.ts`.

/// A warning produced while tidying. The `code` is one of `MISSING_KEY`,
/// `DUPLICATE_ENTRY`, or `UNSUPPORTED_ESCAPE`; the CLI prints `code: message`.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub code: &'static str,
    pub message: String,
}

impl Warning {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Warning {
            code,
            message: message.into(),
        }
    }
}

/// The result of tidying: the formatted BibTeX, any warnings, and the entry count.
#[derive(Debug, Clone)]
pub struct TidyResult {
    pub bibtex: String,
    pub warnings: Vec<Warning>,
    pub count: usize,
}

/// A single transform: consumes and rebuilds the AST, optionally emitting warnings.
pub struct TransformResult {
    pub ast: crate::ast::Root,
    pub warnings: Vec<Warning>,
}

pub type TransformFn = Box<dyn Fn(crate::ast::Root) -> TransformResult>;
