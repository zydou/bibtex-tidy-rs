//! The top-level `tidy` function. Port of `upstream/src/tidy.ts`.

use crate::ast::{BibtexNode, BlockInner, Root};
use crate::format::format_bibtex;
use crate::options::{OptVal, normalize};
use crate::parser;
use crate::pipeline::generate_pipeline;
use crate::types::{TidyResult, Warning};
use crate::utils::convert_crlf;
use std::collections::HashMap;

/// Parse, run the transform pipeline, and format. `raw` is the un-normalized option map
/// (defaults are applied here, matching the original). Port of `tidy`.
pub fn tidy(input: &str, raw: &HashMap<String, OptVal>) -> Result<TidyResult, String> {
    let options = normalize(raw);
    let input_fixed = convert_crlf(input);
    let ast = parser::parse(&input_fixed).map_err(|e| e.to_string())?;

    // MISSING_KEY warnings are computed from the original (pre-transform) AST and use the
    // original-case command, matching the original's ordering (these come first).
    let mut warnings = missing_key_warnings(&ast);

    let mut ast = ast;
    for transform in generate_pipeline(&options)? {
        let result = transform(ast);
        ast = result.ast;
        warnings.extend(result.warnings);
    }

    let bibtex = format_bibtex(&ast)?;
    let count = count_entries(&ast);
    Ok(TidyResult {
        bibtex,
        warnings,
        count,
    })
}

fn missing_key_warnings(root: &Root) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for child in &root.children {
        if let BibtexNode::Block(block) = child
            && let Some(BlockInner::Entry(entry)) = &block.inner
            && entry.key.is_none()
        {
            warnings.push(Warning::new(
                "MISSING_KEY",
                format!("{} entry does not have a citation key.", block.command),
            ));
        }
    }
    warnings
}

fn count_entries(root: &Root) -> usize {
    root.children
        .iter()
        .filter(
            |c| matches!(c, BibtexNode::Block(b) if matches!(b.inner, Some(BlockInner::Entry(_)))),
        )
        .count()
}
