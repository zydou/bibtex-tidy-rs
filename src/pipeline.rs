//! Assemble the ordered transform pipeline from resolved options. Port of
//! `upstream/src/pipeline.ts::generateTransformPipeline` (dependency sort omitted until a
//! transform with dependencies — `prefer-numeric`/`prefer-curly` — is ported in S3).

use crate::options::{Escape, Options, indent_string};
use crate::transforms;
use crate::types::TransformFn;

/// Build the list of transforms to apply, in order, for the given options. Only the
/// always-on and default-path transforms are wired so far; option-gated transforms that
/// are off by default are added in S3.
pub fn generate_pipeline(options: &Options) -> Vec<TransformFn> {
    let mut pipeline: Vec<TransformFn> = Vec::new();

    match options.escape {
        Escape::Off => {}
        Escape::Legacy => pipeline.push(transforms::escape_characters(false)),
        Escape::New => pipeline.push(transforms::escape_characters(true)),
    }
    pipeline.push(transforms::format_page_range());
    if options.lowercase {
        pipeline.push(transforms::lowercase_entry_type());
        pipeline.push(transforms::lowercase_fields());
    }
    if !options.omit.is_empty() {
        pipeline.push(transforms::remove_specified_fields(&options.omit));
    }
    if options.remove_duplicate_fields {
        pipeline.push(transforms::remove_duplicate_fields());
    }
    pipeline.push(transforms::reset_whitespace(!options.tidy_comments));
    let indent = indent_string(options);
    pipeline.push(transforms::indent_fields(&indent));
    pipeline.push(transforms::align_values(options.align));
    pipeline.push(transforms::field_commas(options.trailing_commas));
    pipeline.push(transforms::wrap_values(
        &indent,
        options.align,
        options.wrap,
    ));
    pipeline
}
