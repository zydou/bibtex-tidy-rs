//! Assemble the ordered transform pipeline from resolved options. Port of
//! `upstream/src/pipeline.ts::generateTransformPipeline`, including the dependency sort
//! (`sortPipeline`) that hoists transforms ahead of the stages that depend on them
//! (e.g. `prefer-curly` before `prefer-numeric`/`enclose-braces`).

use crate::options::{Escape, Options, indent_string};
use crate::transforms;
use crate::types::TransformFn;
use std::collections::HashSet;

/// A named transform with its dependencies, mirroring the original's `Transform` type.
struct Transform {
    name: &'static str,
    dependencies: &'static [&'static str],
    apply: TransformFn,
}

fn tr(name: &'static str, dependencies: &'static [&'static str], apply: TransformFn) -> Transform {
    Transform {
        name,
        dependencies,
        apply,
    }
}

/// Topologically sort transforms so each runs after the stages it depends on. Port of
/// `sortPipeline`.
fn sort_pipeline(pipeline: Vec<Transform>) -> Vec<Transform> {
    let mut slots: Vec<Option<Transform>> = pipeline.into_iter().map(Some).collect();
    let mut sorted: Vec<Transform> = Vec::with_capacity(slots.len());
    let mut visited: HashSet<&str> = HashSet::new();
    for i in 0..slots.len() {
        // A slot already taken (`None`) was visited as a dependency of an earlier stage.
        if slots[i].is_some() {
            visit(i, &mut slots, &mut sorted, &mut visited);
        }
    }
    sorted
}

fn visit(
    i: usize,
    pipeline: &mut Vec<Option<Transform>>,
    sorted: &mut Vec<Transform>,
    visited: &mut HashSet<&str>,
) {
    let name = pipeline[i].as_ref().expect("present").name;
    if !visited.insert(name) {
        return;
    }
    let dependencies = pipeline[i].as_ref().expect("present").dependencies;
    for dep in dependencies {
        if let Some(di) = pipeline
            .iter()
            .position(|t| t.as_ref().is_some_and(|t| t.name == *dep))
        {
            visit(di, pipeline, sorted, visited);
        }
    }
    sorted.push(pipeline[i].take().expect("present"));
}

/// Build the ordered list of transforms to apply for the given options. Port of
/// `generateTransformPipeline`. Transforms whose enabling option is off are omitted; the
/// result is dependency-sorted.
pub fn generate_pipeline(options: &Options) -> Vec<TransformFn> {
    let mut pipeline: Vec<Transform> = Vec::new();

    if options.months {
        pipeline.push(tr("months", &[], transforms::months()));
    }
    if options.drop_all_caps {
        pipeline.push(tr("drop-all-caps", &[], transforms::drop_all_caps()));
    }
    if options.encode_urls {
        pipeline.push(tr("encode-urls", &[], transforms::encode_urls()));
    }
    if options.unescape {
        pipeline.push(tr(
            "unescape-characters",
            &[],
            transforms::unescape_characters(),
        ));
    } else {
        match options.escape {
            Escape::Off => {}
            Escape::Legacy => pipeline.push(tr(
                "escape-characters",
                &[],
                transforms::escape_characters(false),
            )),
            Escape::New => pipeline.push(tr(
                "escape-characters",
                &[],
                transforms::escape_characters(true),
            )),
        }
    }
    pipeline.push(tr(
        "format-page-range",
        &[],
        transforms::format_page_range(),
    ));
    if let Some(template) = &options.generate_keys {
        pipeline.push(tr(
            "generate-keys",
            &[],
            transforms::generate_keys(template.clone()),
        ));
    }
    if let Some(max) = options.max_authors {
        pipeline.push(tr("limit-authors", &[], transforms::limit_authors(max)));
    }
    if options.lowercase {
        pipeline.push(tr(
            "lowercase-entry-type",
            &[],
            transforms::lowercase_entry_type(),
        ));
        pipeline.push(tr("lowercase-fields", &[], transforms::lowercase_fields()));
    }
    if options.merge.is_some() || options.duplicates.is_some() {
        pipeline.push(tr(
            "merge-entries",
            &["generate-keys", "sort-entries"],
            crate::duplicates::merge_entries(options.duplicates.clone(), options.merge),
        ));
    }
    if !options.omit.is_empty() {
        pipeline.push(tr(
            "remove-specified-fields",
            &[],
            transforms::remove_specified_fields(&options.omit),
        ));
    }
    if let Some(fields) = &options.enclosing_braces {
        pipeline.push(tr(
            "enclose-braces",
            &["prefer-curly"],
            transforms::enclose_braces(fields),
        ));
    }
    if options.curly {
        pipeline.push(tr("prefer-curly", &[], transforms::prefer_curly()));
    }
    if options.numeric {
        pipeline.push(tr(
            "prefer-numeric",
            &["prefer-curly"],
            transforms::prefer_numeric(),
        ));
    }
    if let Some(fields) = &options.remove_braces {
        pipeline.push(tr("remove-braces", &[], transforms::remove_braces(fields)));
    }
    if options.strip_comments {
        pipeline.push(tr("remove-comments", &[], transforms::remove_comments()));
    }
    if options.remove_duplicate_fields {
        pipeline.push(tr(
            "remove-duplicate-fields",
            &[],
            transforms::remove_duplicate_fields(),
        ));
    }
    if options.remove_empty_fields {
        pipeline.push(tr(
            "remove-empty-fields",
            &[],
            transforms::remove_empty_fields(),
        ));
    }
    if let Some(sort) = &options.sort {
        pipeline.push(tr(
            "sort-entries",
            &["generate-keys", "merge-entries", "prefer-numeric"],
            transforms::sort_entries(sort),
        ));
    }
    if let Some(fields) = &options.sort_fields {
        pipeline.push(tr("sort-fields", &[], transforms::sort_fields(fields)));
    }
    if options.strip_enclosing_braces {
        pipeline.push(tr(
            "strip-enclosing-braces",
            &[],
            transforms::remove_enclosing_braces(),
        ));
    }
    pipeline.push(tr(
        "reset-whitespace",
        &[],
        transforms::reset_whitespace(!options.tidy_comments),
    ));
    let indent = indent_string(options);
    pipeline.push(tr("indent-fields", &[], transforms::indent_fields(&indent)));
    if options.blank_lines {
        pipeline.push(tr("blank-lines", &[], transforms::blank_lines()));
    }
    pipeline.push(tr(
        "align-values",
        &[],
        transforms::align_values(options.align),
    ));
    pipeline.push(tr(
        "field-commas",
        &[],
        transforms::field_commas(options.trailing_commas),
    ));
    pipeline.push(tr(
        "wrap-values",
        &[],
        transforms::wrap_values(&indent, options.align, options.wrap),
    ));

    sort_pipeline(pipeline)
        .into_iter()
        .map(|t| t.apply)
        .collect()
}
