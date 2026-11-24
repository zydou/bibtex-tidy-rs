//! Duplicate detection and merging. Port of `upstream/src/duplicates.ts` and the
//! `mergeEntries` transform (`upstream/src/transforms/mergeEntries.ts`).
//!
//! Entries are compared by `key` (always), plus optionally `doi`, `citation`
//! (author+title+number), and `abstract`. When a merge strategy is configured, the later
//! duplicate is removed and folded into the first-seen entry.

use crate::ast::{BibtexNode, BlockInner, Entry, Field};
use crate::name::parse_name_list;
use crate::options::{DuplicateRule, MergeStrategy};
use crate::types::Warning;
use crate::utils::alpha_num;
use crate::value_nodes::render_field_value;
use std::collections::{HashMap, HashSet};

/// A field lookup (case-insensitive). Port of `getField`.
fn get_field<'a>(entry: &'a Entry, name: &str) -> Option<&'a Field> {
    let name_lc = name.to_lowercase();
    entry
        .fields
        .iter()
        .find(|field| field.name.to_lowercase() == name_lc)
}

/// Render a field's value, or `""` when absent.
fn field_text(entry: &Entry, name: &str) -> String {
    get_field(entry, name)
        .map(render_field_value)
        .unwrap_or_default()
}

/// The entry's citation key as the original renders it in warnings (`undefined` when absent).
fn key_str(entry: &Entry) -> String {
    entry.key.clone().unwrap_or_else(|| "undefined".to_string())
}

/// Find duplicates and apply the merge strategy. `entries` is mutated in place (survivors
/// absorb merged fields). Returns the positions to remove and the warnings. Port of
/// `checkForDuplicates`.
pub fn check_for_duplicates(
    entries: &mut [Entry],
    duplicate_rules: Option<&[DuplicateRule]>,
    merge: Option<MergeStrategy>,
) -> (HashSet<usize>, Vec<Warning>) {
    // Each rule maps to "do we merge" (true iff a merge strategy is configured). The `key`
    // rule is always checked, but only warns (it never merges unless explicitly listed).
    let mut rules: Vec<(DuplicateRule, bool)> = Vec::new();
    if let Some(dr) = duplicate_rules {
        for rule in dr {
            if !rules.iter().any(|(r, _)| r == rule) {
                rules.push((*rule, merge.is_some()));
            }
        }
    }
    if !rules.iter().any(|(r, _)| *r == DuplicateRule::Key) {
        rules.push((DuplicateRule::Key, false));
    }

    let mut duplicate_entries: HashSet<usize> = HashSet::new();
    let mut warnings: Vec<Warning> = Vec::new();
    let mut keys: HashMap<String, usize> = HashMap::new();
    let mut dois: HashMap<String, usize> = HashMap::new();
    let mut citations: HashMap<String, usize> = HashMap::new();
    let mut abstracts: HashMap<String, usize> = HashMap::new();

    for i in 0..entries.len() {
        for (rule, do_merge) in &rules {
            let mut duplicate_of: Option<usize> = None;
            let mut warning: Option<String> = None;

            match rule {
                DuplicateRule::Key => {
                    // BibTeX keys are case-insensitive.
                    let key = match &entries[i].key {
                        Some(k) => k.clone(),
                        None => continue,
                    };
                    let key_lc = key.to_lowercase();
                    match keys.get(&key_lc) {
                        None => {
                            keys.insert(key_lc, i);
                        }
                        Some(&pos) => {
                            duplicate_of = Some(pos);
                            warning =
                                Some(format!("The citation key {key} has already been used."));
                        }
                    }
                }
                DuplicateRule::Doi => {
                    let doi = alpha_num(&field_text(&entries[i], "doi"));
                    if doi.is_empty() {
                        continue;
                    }
                    match dois.get(&doi) {
                        None => {
                            dois.insert(doi, i);
                        }
                        Some(&pos) => {
                            duplicate_of = Some(pos);
                            warning = Some(format!(
                                "Entry {} has an identical DOI to entry {}.",
                                key_str(&entries[i]),
                                key_str(&entries[pos])
                            ));
                        }
                    }
                }
                DuplicateRule::Citation => {
                    let ttl = field_text(&entries[i], "title");
                    let aut = field_text(&entries[i], "author");
                    // Author/title can be identical for numbered reports, so the number is
                    // part of the token.
                    let num = field_text(&entries[i], "number");
                    if ttl.is_empty() || aut.is_empty() {
                        continue;
                    }
                    let first_last = parse_name_list(&aut)
                        .first()
                        .map(|n| n.last.clone())
                        .unwrap_or_else(|| aut.clone());
                    let cit = format!(
                        "{}:{}:{}",
                        alpha_num(&first_last),
                        alpha_num(&ttl),
                        alpha_num(&num)
                    );
                    match citations.get(&cit) {
                        None => {
                            citations.insert(cit, i);
                        }
                        Some(&pos) => {
                            duplicate_of = Some(pos);
                            warning = Some(format!(
                                "Entry {} has similar content to entry {}.",
                                key_str(&entries[i]),
                                key_str(&entries[pos])
                            ));
                        }
                    }
                }
                DuplicateRule::Abstract => {
                    let abstract_text = alpha_num(&field_text(&entries[i], "abstract"));
                    let abs: String = abstract_text.chars().take(100).collect();
                    if abs.is_empty() {
                        continue;
                    }
                    match abstracts.get(&abs) {
                        None => {
                            abstracts.insert(abs, i);
                        }
                        Some(&pos) => {
                            duplicate_of = Some(pos);
                            warning = Some(format!(
                                "Entry {} has a similar abstract to entry {}.",
                                key_str(&entries[i]),
                                key_str(&entries[pos])
                            ));
                        }
                    }
                }
            }

            if let Some(pos) = duplicate_of {
                if *do_merge {
                    duplicate_entries.insert(i);
                    // Clone the source (the later duplicate) so we can mutably borrow the
                    // first-seen entry without aliasing.
                    let source = entries[i].clone();
                    merge_into(merge, &mut entries[pos], &source);
                }
                if let Some(w) = warning {
                    warnings.push(Warning::new(
                        "DUPLICATE_ENTRY",
                        format!(
                            "Duplicate {}. {w}",
                            if *do_merge { "removed" } else { "detected" }
                        ),
                    ));
                }
            }
        }
    }

    (duplicate_entries, warnings)
}

/// Fold `entry` (the later duplicate) into `duplicate_of` (the first-seen entry). Port of
/// the internal `mergeEntries`.
fn merge_into(merge: Option<MergeStrategy>, duplicate_of: &mut Entry, entry: &Entry) {
    let merge = match merge {
        Some(m) => m,
        None => return,
    };
    match merge {
        MergeStrategy::Last => {
            duplicate_of.key = entry.key.clone();
            duplicate_of.fields = entry.fields.clone();
        }
        MergeStrategy::Combine | MergeStrategy::Overwrite => {
            for field in &entry.fields {
                let name_lc = field.name.to_lowercase();
                let existing = duplicate_of
                    .fields
                    .iter_mut()
                    .find(|f| f.name.to_lowercase() == name_lc);
                match existing {
                    None => duplicate_of.fields.push(field.clone()),
                    Some(existing) => {
                        if merge == MergeStrategy::Overwrite {
                            // Replaces the whole value (concat node): value nodes and the
                            // spacing between `=` and the value.
                            existing.values = field.values.clone();
                            existing.value_prefix = field.value_prefix.clone();
                        }
                    }
                }
            }
        }
        MergeStrategy::First => {}
    }
}

/// The `merge-entries` transform: detect duplicates among top-level entries, merge them into
/// the first-seen entry, and drop the later duplicates. Port of
/// `createMergeEntriesTransform`.
pub fn merge_entries(
    duplicates: Option<Vec<DuplicateRule>>,
    merge: Option<MergeStrategy>,
) -> crate::types::TransformFn {
    Box::new(move |mut ast| {
        // Collect top-level entries (cloned) alongside their child index.
        let mut entry_positions: Vec<usize> = Vec::new();
        let mut entries: Vec<Entry> = Vec::new();
        for (idx, child) in ast.children.iter().enumerate() {
            if let BibtexNode::Block(block) = child
                && let Some(BlockInner::Entry(entry)) = &block.inner
            {
                entry_positions.push(idx);
                entries.push(entry.clone());
            }
        }

        let (remove_positions, warnings) =
            check_for_duplicates(&mut entries, duplicates.as_deref(), merge);

        // Write back surviving (possibly merged) entries; drop removed duplicates.
        let mut children_out = Vec::with_capacity(ast.children.len());
        let mut cursor = 0usize;
        for (child_idx, mut child) in ast.children.into_iter().enumerate() {
            if cursor < entry_positions.len() && entry_positions[cursor] == child_idx {
                if !remove_positions.contains(&cursor) {
                    if let BibtexNode::Block(ref mut block) = child
                        && let Some(BlockInner::Entry(ref mut entry)) = block.inner
                    {
                        *entry = entries[cursor].clone();
                    }
                    children_out.push(child);
                }
                cursor += 1;
            } else {
                children_out.push(child);
            }
        }
        ast.children = children_out;
        crate::types::TransformResult { ast, warnings }
    })
}
