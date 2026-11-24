//! Unicode-to-LaTeX escape lookups. Port of `unicode.ts` / `unicodeCore.ts` lookups.
//!
//! The tables live in `unicode_data.rs` (generated from the upstream TypeScript maps).

use crate::unicode_data::{CORE_SPECIAL_CHARACTERS, SPECIAL_CHARACTERS};

/// Look up the LaTeX escape for a codepoint in the legacy/full table.
pub fn special_character(code: u32) -> Option<&'static str> {
    lookup(SPECIAL_CHARACTERS, code)
}

/// Look up the LaTeX escape for a codepoint in the "new mode" (package-independent) table.
pub fn core_special_character(code: u32) -> Option<&'static str> {
    lookup(CORE_SPECIAL_CHARACTERS, code)
}

fn lookup(table: &[(u32, &'static str)], code: u32) -> Option<&'static str> {
    table
        .binary_search_by_key(&code, |(cp, _)| *cp)
        .ok()
        .map(|i| table[i].1)
}
