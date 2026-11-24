//! Small string helpers. Port of `upstream/src/utils.ts`.

/// Normalize newlines: convert CR and CRLF to LF.
pub fn convert_crlf(s: &str) -> String {
    // Replace CRLF, then lone CR, with LF (mirrors /\r\n?/g).
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Greedy word-wrap a single logical line into lines of at most `line_width` chars.
pub fn wrap_text(line: &str, line_width: usize) -> Vec<String> {
    let words: Vec<&str> = line.split(' ').collect();
    let mut lines: Vec<String> = Vec::new();
    let mut curr = String::new();
    for (i, word) in words.iter().enumerate() {
        if curr.chars().count() + word.chars().count() + 1 > line_width && i > 0 {
            lines.push(curr.trim().to_string());
            curr = String::new();
        }
        curr.push_str(word);
        curr.push(' ');
    }
    lines.push(curr.trim().to_string());
    lines
}

/// Remove line breaks used to wrap text, preserving paragraphs (blank lines).
///
/// Mirrors the original's three regex substitutions: collapse each run of whitespace
/// containing 2+ newlines into a paragraph marker, collapse each run with a single
/// newline into a space, then restore paragraph markers as blank lines.
pub fn unwrap_text(s: &str) -> String {
    const PARA: &str = "\u{1}PARA\u{1}";
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\n' {
            // Consume the maximal run of whitespace around consecutive newlines.
            let start = i;
            // Back up over whitespace already emitted so the run owns its surrounding ws.
            while !out.is_empty() && out.ends_with(char::is_whitespace) {
                out.pop();
            }
            // Re-scan from the first whitespace char of the run.
            let mut j = start;
            while j > 0 && chars[j - 1].is_whitespace() {
                j -= 1;
            }
            let mut newlines = 0;
            let mut k = j;
            while k < chars.len() {
                if chars[k] == '\n' {
                    newlines += 1;
                    k += 1;
                } else if chars[k].is_whitespace() {
                    k += 1;
                } else {
                    break;
                }
            }
            if newlines >= 2 {
                out.push_str(PARA);
            } else {
                out.push(' ');
            }
            i = k;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out.replace(PARA, "\n\n")
}

/// Strip every non-alphanumeric character and lowercase the rest. Port of `alphaNum`
/// (used to normalize DOIs, citation tokens, and abstracts for duplicate detection).
pub fn alpha_num(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
