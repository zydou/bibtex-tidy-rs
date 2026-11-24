//! Command-line argument parsing and option normalization. Ports of
//! `upstream/src/parsers/argsParser.ts`, `upstream/src/cli/argsToOptions.ts`,
//! `upstream/src/optionDefinitions.ts`, and `upstream/src/optionUtils.ts::normalizeOptions`.

use std::collections::HashMap;

pub const DEFAULT_ALIGN: usize = 14;
pub const DEFAULT_SPACE: usize = 2;
pub const DEFAULT_WRAP: usize = 80;
pub const DEFAULT_SORT: [&str; 1] = ["key"];
pub const DEFAULT_MERGE_CHECK: [&str; 3] = ["doi", "citation", "abstract"];
pub const DEFAULT_FIELD_SORT: [&str; 24] = [
    "title",
    "shorttitle",
    "author",
    "year",
    "month",
    "day",
    "journal",
    "booktitle",
    "location",
    "on",
    "publisher",
    "address",
    "series",
    "volume",
    "number",
    "pages",
    "doi",
    "isbn",
    "issn",
    "url",
    "urldate",
    "copyright",
    "category",
    "note",
];
pub const DEFAULT_KEY_TEMPLATE: &str =
    "[auth:required:lower][year:required][veryshorttitle:lower][duplicateNumber]";

/// A loosely-typed option value, mirroring the original's dynamic `options` object.
#[derive(Clone, Debug, PartialEq)]
pub enum OptVal {
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<String>),
    /// An option that was provided but resolved to `undefined` (e.g. `-o` with no
    /// following value). It is a known option (not "unknown") but carries no value.
    Undef,
}

/// The escape mode: legacy (full table), "new" (package-independent), or off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escape {
    Legacy,
    New,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeStrategy {
    First,
    Last,
    Combine,
    Overwrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateRule {
    Doi,
    Key,
    Abstract,
    Citation,
}

/// Fully resolved options after applying CLI overrides, `convertBoolean`, and defaults.
#[derive(Clone, Debug)]
pub struct Options {
    pub help: bool,
    pub version: bool,
    pub quiet: bool,
    pub backup: bool,
    pub modify: bool,
    pub v2: bool,
    pub output_path: Option<String>,

    pub escape: Escape,
    pub unescape: bool,
    pub lowercase: bool,
    pub remove_duplicate_fields: bool,
    pub tidy_comments: bool,
    pub tab: bool,
    pub space: usize,
    pub align: usize,
    pub wrap: Option<usize>,
    pub curly: bool,
    pub numeric: bool,
    pub months: bool,
    pub blank_lines: bool,
    pub strip_comments: bool,
    pub trailing_commas: bool,
    pub encode_urls: bool,
    pub remove_empty_fields: bool,
    pub strip_enclosing_braces: bool,
    pub drop_all_caps: bool,
    pub omit: Vec<String>,
    pub sort: Option<Vec<String>>,
    pub sort_fields: Option<Vec<String>>,
    pub merge: Option<MergeStrategy>,
    pub duplicates: Option<Vec<DuplicateRule>>,
    pub generate_keys: Option<String>,
    pub max_authors: Option<usize>,
    pub enclosing_braces: Option<Vec<String>>,
    pub remove_braces: Option<Vec<String>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            help: false,
            version: false,
            quiet: false,
            backup: true,
            modify: true,
            v2: false,
            output_path: None,
            escape: Escape::Legacy,
            unescape: false,
            lowercase: true,
            remove_duplicate_fields: true,
            tidy_comments: true,
            tab: false,
            space: DEFAULT_SPACE,
            align: DEFAULT_ALIGN,
            wrap: None,
            curly: false,
            numeric: false,
            months: false,
            blank_lines: false,
            strip_comments: false,
            trailing_commas: false,
            encode_urls: false,
            remove_empty_fields: false,
            strip_enclosing_braces: false,
            drop_all_caps: false,
            omit: vec![],
            sort: None,
            sort_fields: None,
            merge: None,
            duplicates: None,
            generate_keys: None,
            max_authors: None,
            enclosing_braces: None,
            remove_braces: None,
        }
    }
}

/// The indentation string implied by `tab`/`space`.
pub fn indent_string(opts: &Options) -> String {
    if opts.tab {
        "\t".to_string()
    } else {
        " ".repeat(opts.space)
    }
}

/// Parse shell arguments of the form `<input files> <options>` (in any order). Options
/// with values may be `--thing=a,b` or `--thing a b c`; some values may start with `-`
/// (descending sort fields). Faithful port of `parseCLIArguments`.
pub fn parse_cli_arguments(
    argv: &[String],
    no_trailing_input_paths: bool,
) -> HashMap<String, Vec<String>> {
    let mut input_paths: Vec<String> = Vec::new();
    let mut key_values: HashMap<String, Vec<String>> = HashMap::new();
    let mut curr_key: Option<String> = None;
    let mut curr_values: Vec<String> = Vec::new();
    // `mode` is derived from whether a key is pending (the original tracks "root" vs
    // "values"; a pending key means values mode).
    let mut mode_values = false;

    for arg in argv {
        let matches_stdin_marker = arg == "-";
        let matches_long_arg = arg.starts_with("--");
        let matches_short_arg = arg.starts_with('-');
        let is_negated_value = matches_short_arg
            && curr_key
                .as_deref()
                .map(|k| matches!(k, "--sort" | "--sort-fields" | "--sort-properties"))
                .unwrap_or(false);

        if matches_long_arg {
            // flush
            if let Some(k) = curr_key.take() {
                key_values.insert(k, std::mem::take(&mut curr_values));
            }
            let (key, values) = parse_long_cli_option(arg);
            if !values.is_empty() {
                key_values.insert(key, values);
            } else {
                curr_key = Some(key);
                mode_values = true;
            }
        } else if matches_short_arg && !matches_stdin_marker && !is_negated_value {
            if let Some(k) = curr_key.take() {
                key_values.insert(k, std::mem::take(&mut curr_values));
            }
            for c in arg[1..].chars() {
                let key = format!("-{c}");
                key_values.insert(key.clone(), vec![]);
                curr_key = Some(key);
                mode_values = true;
            }
        } else if !mode_values {
            input_paths.push(arg.clone());
        } else {
            curr_values.push(arg.clone());
        }
    }

    if input_paths.is_empty() && !no_trailing_input_paths {
        input_paths = std::mem::take(&mut curr_values);
    }

    if let Some(k) = curr_key.take() {
        key_values.insert(k, curr_values);
    }

    key_values.insert(String::new(), input_paths);
    key_values
}

/// Parse a single long option `--key=v1,v2` into its key and value tokens (honoring
/// single/double quotes). Faithful port of `parseLongCLIOption`.
fn parse_long_cli_option(arg: &str) -> (String, Vec<String>) {
    #[derive(PartialEq)]
    enum Mode {
        Key,
        Values,
        DoubleQuoted,
        SingleQuoted,
    }
    let mut mode = Mode::Key;
    let mut curr_token = String::new();
    let mut key: Option<String> = None;
    let mut values: Vec<String> = Vec::new();

    let flush =
        |key: &mut Option<String>, values: &mut Vec<String>, token: &mut String, mode: &Mode| {
            if token.is_empty() {
                return;
            }
            match mode {
                Mode::Key => *key = Some(std::mem::take(token)),
                Mode::Values => values.push(std::mem::take(token)),
                _ => {}
            }
        };

    for c in arg.chars() {
        match mode {
            Mode::Key => {
                if c == '=' {
                    key = Some(std::mem::take(&mut curr_token));
                    mode = Mode::Values;
                } else {
                    curr_token.push(c);
                }
            }
            Mode::Values => {
                if c == '"' {
                    mode = Mode::DoubleQuoted;
                } else if c == '\'' {
                    mode = Mode::SingleQuoted;
                } else if c == ',' {
                    flush(&mut key, &mut values, &mut curr_token, &mode);
                } else {
                    curr_token.push(c);
                }
            }
            Mode::DoubleQuoted => {
                if c == '"' {
                    mode = Mode::Values;
                } else {
                    curr_token.push(c);
                }
            }
            Mode::SingleQuoted => {
                if c == '\'' {
                    mode = Mode::Values;
                } else {
                    curr_token.push(c);
                }
            }
        }
    }
    flush(&mut key, &mut values, &mut curr_token, &mode);

    (key.unwrap_or_default(), values)
}

/// Map a parsed CLI key + values to its option key and value. Returns `None` for unknown
/// options (which become unknown args). Port of the `cliOptions` mapping in `argsToOptions`.
///
/// The returned key uses the option's dotted snake_case name (e.g. `output-path`).
fn resolve_cli_arg(key: &str, values: &[String]) -> Option<(String, OptVal)> {
    let b = |opt: &str, v: bool| Some((opt.to_string(), OptVal::Bool(v)));
    match key {
        "--help" | "-h" => b("help", true),
        "--version" | "-v" => b("version", true),
        "--quiet" => b("quiet", true),
        "--v2" => b("v2", true),
        "--modify" | "-m" => b("modify", true),
        "--no-modify" => b("modify", false),
        "--curly" => b("curly", true),
        "--no-curly" => b("curly", false),
        "--numeric" => b("numeric", true),
        "--no-numeric" => b("numeric", false),
        "--months" => b("months", true),
        "--tab" => b("tab", true),
        "--no-tab" => b("tab", false),
        "--blank-lines" => b("blank-lines", true),
        "--no-blank-lines" => b("blank-lines", false),
        "--strip-enclosing-braces" => b("strip-enclosing-braces", true),
        "--drop-all-caps" => b("drop-all-caps", true),
        "--unescape" => b("unescape", true),
        "--strip-comments" => b("strip-comments", true),
        "--no-strip-comments" => b("strip-comments", false),
        // NOTE: upstream maps both `--trailing-commas` and `--no-trailing-commas` to
        // `true`; preserved here for parity.
        "--trailing-commas" | "--no-trailing-commas" => b("trailing-commas", true),
        // NOTE: upstream maps both `--encode-urls` and `--no-encode-urls` to `true`.
        "--encode-urls" | "--no-encode-urls" => b("encode-urls", true),
        "--tidy-comments" => b("tidy-comments", true),
        "--no-tidy-comments" => b("tidy-comments", false),
        "--remove-empty-fields" => b("remove-empty-fields", true),
        "--no-remove-empty-fields" => b("remove-empty-fields", false),
        "--remove-dupe-fields" => b("remove-duplicate-fields", true),
        "--no-remove-dupe-fields" => b("remove-duplicate-fields", false),
        "--no-lowercase" => b("lowercase", false),
        "--backup" => b("backup", true),
        "--no-backup" => b("backup", false),
        "--no-align" => b("align", false),
        "--no-sort" => b("sort", false),
        "--no-merge" => b("merge", false),
        "--no-escape" => b("escape", false),
        "--no-wrap" => b("wrap", false),
        "--output" | "-o" => Some((
            "output-path".to_string(),
            match values.first() {
                Some(v) => OptVal::Str(v.clone()),
                None => OptVal::Undef,
            },
        )),
        "--omit" => {
            if values.is_empty() {
                eprintln!("Expected a omit list");
                std::process::exit(1);
            }
            Some(("omit".to_string(), OptVal::List(values.to_vec())))
        }
        "--space" => {
            if !values.is_empty() {
                Some(("space".to_string(), OptVal::Num(js_number(&values[0]))))
            } else {
                Some(("space".to_string(), OptVal::Bool(true)))
            }
        }
        "--align" => Some((
            "align".to_string(),
            OptVal::Num(js_number(values.first().map(|s| s.as_str()).unwrap_or(""))),
        )),
        "--sort" => {
            if !values.is_empty() {
                Some(("sort".to_string(), OptVal::List(values.to_vec())))
            } else {
                Some(("sort".to_string(), OptVal::Bool(true)))
            }
        }
        "--sort-fields" | "--sort-properties" => {
            if !values.is_empty() {
                Some(("sort-fields".to_string(), OptVal::List(values.to_vec())))
            } else {
                Some(("sort-fields".to_string(), OptVal::Bool(true)))
            }
        }
        "--duplicates" => {
            if values.is_empty() {
                return Some(("duplicates".to_string(), OptVal::Bool(true)));
            }
            for v in values {
                if !matches!(v.as_str(), "doi" | "key" | "abstract" | "citation") {
                    eprintln!("Invalid key for merge option: \"{v}\"");
                    std::process::exit(1);
                }
            }
            Some(("duplicates".to_string(), OptVal::List(values.to_vec())))
        }
        "--merge" => {
            if values.is_empty() {
                return Some(("merge".to_string(), OptVal::Bool(true)));
            }
            if !matches!(
                values[0].as_str(),
                "first" | "last" | "combine" | "overwrite"
            ) {
                eprintln!("Invalid merge strategy: \"{}\"", values[0]);
                std::process::exit(1);
            }
            Some(("merge".to_string(), OptVal::Str(values[0].clone())))
        }
        "--escape" => {
            if values.first().map(|s| s.as_str()) == Some("new") {
                Some(("escape".to_string(), OptVal::Str("new".to_string())))
            } else {
                Some(("escape".to_string(), OptVal::Bool(true)))
            }
        }
        "--generate-keys" => {
            if !values.is_empty() {
                Some(("generate-keys".to_string(), OptVal::List(values.to_vec())))
            } else {
                Some(("generate-keys".to_string(), OptVal::Bool(true)))
            }
        }
        "--max-authors" => Some((
            "max-authors".to_string(),
            OptVal::Num(js_number(values.first().map(|s| s.as_str()).unwrap_or(""))),
        )),
        "--enclosing-braces" => {
            if !values.is_empty() {
                Some((
                    "enclosing-braces".to_string(),
                    OptVal::List(values.to_vec()),
                ))
            } else {
                Some(("enclosing-braces".to_string(), OptVal::Bool(true)))
            }
        }
        "--remove-braces" => {
            if !values.is_empty() {
                Some(("remove-braces".to_string(), OptVal::List(values.to_vec())))
            } else {
                Some(("remove-braces".to_string(), OptVal::Bool(true)))
            }
        }
        "--wrap" => {
            if !values.is_empty() {
                Some(("wrap".to_string(), OptVal::Num(js_number(&values[0]))))
            } else {
                Some(("wrap".to_string(), OptVal::Bool(true)))
            }
        }
        _ => None,
    }
}

/// Parse a numeric CLI value the way JavaScript's `Number()` does: empty/whitespace -> 0,
/// invalid -> NaN.
fn js_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// Convert CLI argument vector into input files, the raw (un-normalized) option map, and
/// unknown args. The CLI's control flow reads the *raw* map (where unset options are
/// absent, not defaulted); defaults are applied later by [`normalize`] inside `tidy`.
/// `skip_input_args` should be true when reading from stdin (so trailing filenames are
/// treated as option values, not inputs). Port of `argsToOptions`.
pub fn args_to_options(
    argv: &[String],
    skip_input_args: bool,
) -> (Vec<String>, HashMap<String, OptVal>, Vec<String>) {
    let kv = parse_cli_arguments(argv, skip_input_args);
    let input_files = kv.get("").cloned().unwrap_or_default();

    let mut raw: HashMap<String, OptVal> = HashMap::new();
    let mut unknown_args = Vec::new();
    for (key, values) in &kv {
        if key.is_empty() {
            continue;
        }
        match resolve_cli_arg(key, values) {
            Some((opt_key, val)) => {
                raw.insert(opt_key, val);
            }
            None => unknown_args.push(key.clone()),
        }
    }

    (input_files, raw, unknown_args)
}

/// Read a boolean option from the raw map (absent -> `None`).
pub fn get_bool(raw: &HashMap<String, OptVal>, key: &str) -> Option<bool> {
    match raw.get(key) {
        Some(OptVal::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// Read a string option from the raw map (absent -> `None`).
pub fn get_str(raw: &HashMap<String, OptVal>, key: &str) -> Option<String> {
    match raw.get(key) {
        Some(OptVal::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Apply `convertBoolean` + defaults to produce the resolved [`Options`]. Port of
/// `normalizeOptions`. Defaults that depend on other options (e.g. `duplicates` on
/// `merge`) read the raw values, matching the original.
pub fn normalize(raw: &HashMap<String, OptVal>) -> Options {
    let mut o = Options::default();

    // convertBoolean / defaults (raw merge read for the duplicates default).
    let raw_merge = raw.get("merge").cloned();
    let merge_set = raw_merge.is_some();

    // escape: true | false | "new"; default true (legacy).
    o.escape = match raw.get("escape") {
        Some(OptVal::Str(s)) if s == "new" => Escape::New,
        Some(OptVal::Bool(false)) => Escape::Off,
        _ => Escape::Legacy,
    };

    macro_rules! bool_field {
        ($field:ident, $key:expr, $default:expr) => {
            o.$field = get_bool(raw, $key).unwrap_or($default);
        };
    }
    bool_field!(help, "help", false);
    bool_field!(version, "version", false);
    bool_field!(quiet, "quiet", false);
    bool_field!(unescape, "unescape", false);
    bool_field!(lowercase, "lowercase", true);
    bool_field!(remove_duplicate_fields, "remove-duplicate-fields", true);
    bool_field!(tidy_comments, "tidy-comments", true);
    bool_field!(tab, "tab", false);
    bool_field!(curly, "curly", false);
    bool_field!(numeric, "numeric", false);
    bool_field!(months, "months", false);
    bool_field!(strip_comments, "strip-comments", false);
    bool_field!(encode_urls, "encode-urls", false);
    bool_field!(remove_empty_fields, "remove-empty-fields", false);
    bool_field!(strip_enclosing_braces, "strip-enclosing-braces", false);
    bool_field!(drop_all_caps, "drop-all-caps", false);
    bool_field!(v2, "v2", false);

    // backup: default true; `--no-backup` -> false.
    o.backup = get_bool(raw, "backup").unwrap_or(true);
    // modify: default true.
    o.modify = get_bool(raw, "modify").unwrap_or(true);
    // trailing-commas: default false (upstream maps the flags to true).
    o.trailing_commas = get_bool(raw, "trailing-commas").unwrap_or(false);
    // blank-lines: default false.
    o.blank_lines = get_bool(raw, "blank-lines").unwrap_or(false);

    // space: convertBoolean {true: 2, false: undefined}, default 2.
    o.space = match raw.get("space") {
        Some(OptVal::Num(n)) => *n as usize,
        Some(OptVal::Bool(true)) => DEFAULT_SPACE,
        _ => DEFAULT_SPACE,
    };

    // align: convertBoolean {true: 14, false: 1}, default 14.
    o.align = match raw.get("align") {
        Some(OptVal::Num(n)) => *n as usize,
        Some(OptVal::Bool(false)) => 1,
        _ => DEFAULT_ALIGN,
    };

    // wrap: convertBoolean {true: 80, false: undefined}; default undefined.
    o.wrap = match raw.get("wrap") {
        Some(OptVal::Num(n)) => Some(*n as usize),
        Some(OptVal::Bool(true)) => Some(DEFAULT_WRAP),
        _ => None,
    };

    o.output_path = match raw.get("output-path") {
        Some(OptVal::Str(s)) => Some(s.clone()),
        _ => None,
    };

    o.omit = match raw.get("omit") {
        Some(OptVal::List(l)) => l.clone(),
        _ => vec![],
    };

    o.sort = match raw.get("sort") {
        Some(OptVal::List(l)) => Some(l.clone()),
        Some(OptVal::Bool(true)) => Some(DEFAULT_SORT.iter().map(|s| s.to_string()).collect()),
        _ => None,
    };

    o.sort_fields = match raw.get("sort-fields") {
        Some(OptVal::List(l)) => Some(l.clone()),
        Some(OptVal::Bool(true)) => {
            Some(DEFAULT_FIELD_SORT.iter().map(|s| s.to_string()).collect())
        }
        _ => None,
    };

    o.merge = match raw_merge {
        Some(OptVal::Str(s)) => Some(match s.as_str() {
            "first" => MergeStrategy::First,
            "last" => MergeStrategy::Last,
            "overwrite" => MergeStrategy::Overwrite,
            _ => MergeStrategy::Combine,
        }),
        Some(OptVal::Bool(true)) => Some(MergeStrategy::Combine),
        _ => None,
    };

    let merge_check: Vec<DuplicateRule> = DEFAULT_MERGE_CHECK
        .iter()
        .map(|s| match *s {
            "doi" => DuplicateRule::Doi,
            "key" => DuplicateRule::Key,
            "abstract" => DuplicateRule::Abstract,
            _ => DuplicateRule::Citation,
        })
        .collect();
    o.duplicates = match raw.get("duplicates") {
        Some(OptVal::List(l)) => Some(
            l.iter()
                .map(|s| match s.as_str() {
                    "doi" => DuplicateRule::Doi,
                    "key" => DuplicateRule::Key,
                    "abstract" => DuplicateRule::Abstract,
                    _ => DuplicateRule::Citation,
                })
                .collect(),
        ),
        Some(OptVal::Bool(true)) => Some(merge_check),
        _ => {
            if merge_set {
                Some(merge_check)
            } else {
                None
            }
        }
    };

    o.generate_keys = match raw.get("generate-keys") {
        Some(OptVal::List(l)) => Some(l.join(",")),
        Some(OptVal::Bool(true)) => Some(DEFAULT_KEY_TEMPLATE.to_string()),
        _ => None,
    };

    o.max_authors = match raw.get("max-authors") {
        Some(OptVal::Num(n)) if !n.is_nan() => Some(*n as usize),
        _ => None,
    };

    o.enclosing_braces = match raw.get("enclosing-braces") {
        Some(OptVal::List(l)) => Some(l.clone()),
        Some(OptVal::Bool(true)) => Some(vec!["title".to_string()]),
        _ => None,
    };

    o.remove_braces = match raw.get("remove-braces") {
        Some(OptVal::List(l)) => Some(l.clone()),
        Some(OptVal::Bool(true)) => Some(vec!["title".to_string()]),
        _ => None,
    };

    o
}
