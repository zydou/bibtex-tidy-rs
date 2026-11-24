//! BibTeX name-field parser. Port of `upstream/src/parsers/nameFieldParser.ts`.
//!
//! Name fields (`author`, `editor`, …) hold a list of names delimited by `and`. Each name
//! has up to four parts (Lamport 1994, p157): given names, prefixes (`von`, `de`), family
//! names, and suffixes (`Jr.`), separable by spaces and/or commas. See the upstream
//! docstring for the supported syntaxes.

/// A parsed name. `pre` is the prefix (von/de), `suf` the suffix (Jr.).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Name {
    pub last: String,
    pub first: String,
    pub pre: String,
    pub suf: String,
}

/// Parse a name list (names delimited by `and`, case-insensitive).
pub fn parse_name_list(value: &str) -> Vec<Name> {
    split_and(value).iter().map(|n| parse_name(n)).collect()
}

/// Split on `/\s+and\s+/i`: whitespace, the word `and` (any case), whitespace.
fn split_and(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    let n = chars.len();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < n {
        if chars[i].is_whitespace() {
            let content_end = i;
            while i < n && chars[i].is_whitespace() {
                i += 1;
            }
            if i + 3 <= n && is_and(&chars[i..i + 3]) && i + 3 < n && chars[i + 3].is_whitespace() {
                let mut j = i + 3;
                while j < n && chars[j].is_whitespace() {
                    j += 1;
                }
                parts.push(chars[start..content_end].iter().collect());
                start = j;
                i = j;
                continue;
            }
        } else {
            i += 1;
        }
    }
    parts.push(chars[start..n].iter().collect());
    parts
}

fn is_and(cs: &[char]) -> bool {
    cs.len() == 3
        && cs
            .iter()
            .zip(['a', 'n', 'd'])
            .all(|(a, b)| a.to_ascii_lowercase() == b)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Prefix(String),
    Comma,
}

/// A name's syntax, detected from its token sequence.
enum Syntax {
    Empty,
    Others,
    LastName,
    FirstNameLastNames,
    FirstNamesPrefixesLastNames,
    LastNamesFirstNamesPrefixes,
    LastNamesSuffixesFirstNamesPrefixes,
}

pub fn parse_name(name: &str) -> Name {
    let tokens = tokenise_name(name);
    match detect_syntax(&tokens) {
        Syntax::Empty => Name::default(),
        Syntax::Others => Name {
            last: "others".to_string(),
            ..Name::default()
        },
        Syntax::LastName => Name {
            last: name_str(&tokens),
            ..Name::default()
        },
        Syntax::FirstNameLastNames => {
            let p = partition(&tokens, &[Kind::Name, Kind::Name]);
            Name {
                first: name_str(&p[0]),
                last: name_str(&p[1]),
                ..Name::default()
            }
        }
        Syntax::FirstNamesPrefixesLastNames => {
            let p = partition(&tokens, &[Kind::Name, Kind::Prefix, Kind::Name]);
            Name {
                first: name_str(&p[0]),
                pre: name_str(&p[1]),
                last: name_str(&p[2]),
                suf: String::new(),
            }
        }
        Syntax::LastNamesFirstNamesPrefixes => {
            let p = partition(&tokens, &[Kind::Name, Kind::Comma, Kind::Prefix]);
            Name {
                last: name_str(&p[0]),
                first: name_str(&p[1]),
                pre: name_str(&p[2]),
                suf: String::new(),
            }
        }
        Syntax::LastNamesSuffixesFirstNamesPrefixes => {
            let p = partition(
                &tokens,
                &[Kind::Name, Kind::Comma, Kind::Comma, Kind::Prefix],
            );
            Name {
                last: name_str(&p[0]),
                suf: name_str(&p[1]),
                first: name_str(&p[2]),
                pre: name_str(&p[3]),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Name,
    Prefix,
    Comma,
}

fn kind_of(token: &Token) -> Kind {
    match token {
        Token::Name(_) => Kind::Name,
        Token::Prefix(_) => Kind::Prefix,
        Token::Comma => Kind::Comma,
    }
}

fn detect_syntax(tokens: &[Token]) -> Syntax {
    let names = tokens
        .iter()
        .filter(|t| matches!(t, Token::Name(_)))
        .count();
    let prefixes = tokens
        .iter()
        .filter(|t| matches!(t, Token::Prefix(_)))
        .count();
    let commas = tokens.iter().filter(|t| matches!(t, Token::Comma)).count();
    if tokens.is_empty() {
        return Syntax::Empty;
    }
    if tokens.len() == 1 && name_str(tokens) == "others" {
        return Syntax::Others;
    }
    if tokens.len() == names && tokens.len() == 1 {
        return Syntax::LastName;
    }
    if tokens.len() == names {
        return Syntax::FirstNameLastNames;
    }
    if prefixes > 0 && commas == 0 {
        return Syntax::FirstNamesPrefixesLastNames;
    }
    if commas == 1 {
        return Syntax::LastNamesFirstNamesPrefixes;
    }
    // commas >= 2 (the original throws on >2; we use the two-comma partition as a
    // non-panicking fallback, which the partition logic absorbs gracefully).
    Syntax::LastNamesSuffixesFirstNamesPrefixes
}

fn tokenise_name(name: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in name.chars() {
        if c == ',' {
            flush_token(&mut tokens, &mut current);
            tokens.push(Token::Comma);
        } else if c.is_whitespace() {
            flush_token(&mut tokens, &mut current);
        } else {
            current.push(c);
        }
    }
    flush_token(&mut tokens, &mut current);
    tokens
}

fn flush_token(tokens: &mut Vec<Token>, current: &mut String) {
    if current.is_empty() {
        return;
    }
    let token = if is_prefix_token(current) {
        Token::Prefix(current.clone())
    } else {
        Token::Name(current.clone())
    };
    tokens.push(token);
    current.clear();
}

fn name_str(tokens: &[Token]) -> String {
    tokens
        .iter()
        .filter_map(|t| match t {
            Token::Name(v) | Token::Prefix(v) => Some(v.as_str()),
            Token::Comma => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A token starting with a lowercase ASCII letter is a prefix (`von`, `de`).
fn is_prefix_token(token: &str) -> bool {
    token.chars().next().is_some_and(|c| c.is_ascii_lowercase())
}

/// Split tokens into partitions, starting a new partition before each kind in
/// `divide_before`. Port of the original's `partition`.
fn partition(tokens: &[Token], divide_before: &[Kind]) -> Vec<Vec<Token>> {
    let mut parts: Vec<Vec<Token>> = vec![Vec::new(); divide_before.len()];
    let mut curr: isize = -1;
    for token in tokens {
        let next = curr + 1;
        if next < divide_before.len() as isize && divide_before[next as usize] == kind_of(token) {
            curr = next;
        }
        if curr >= 0 {
            parts[curr as usize].push(token.clone());
        }
    }
    parts
}
