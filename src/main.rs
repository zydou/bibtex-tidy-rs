//! Entry point for the `bibtex-tidy` port. Port of `upstream/src/cli/cli.ts`.

mod ast;
mod format;
mod latex;
mod options;
mod parser;
mod pipeline;
mod tidy;
mod transforms;
mod types;
mod unicode;
mod unicode_data;
mod utils;
mod value_nodes;

use std::collections::HashMap;
use std::io::{IsTerminal, Read};
use std::process::exit;

use options::{OptVal, args_to_options, get_bool, get_str};
use types::TidyResult;

fn main() {
    // process.stdin.isTTY is true only when no input is piped; the pre-commit hook sets
    // PRE_COMMIT=1 to force the "no stdin" path.
    let has_stdin =
        !std::io::stdin().is_terminal() && std::env::var("PRE_COMMIT").ok().as_deref() != Some("1");

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (input_files, raw, unknown_args) = args_to_options(&argv, has_stdin);

    if !unknown_args.is_empty() {
        let plural = if unknown_args.len() > 1 { "s" } else { "" };
        eprintln!("Unknown option{plural}: {}", unknown_args.join(", "));
        eprintln!("Try 'bibtex-tidy --help' for more information.");
        exit(1);
    }

    if get_bool(&raw, "version").unwrap_or(false) {
        println!("v{}", env!("CARGO_PKG_VERSION"));
        exit(0);
    }

    if get_bool(&raw, "help").unwrap_or(false) || (input_files.is_empty() && !has_stdin) {
        print!("{HELP}");
        exit(0);
    }

    let deprecated_stdin_flag = input_files.iter().any(|f| f == "-");

    let modify = get_bool(&raw, "modify").unwrap_or(false);
    let output_to_stdout = get_str(&raw, "output-path").is_none() && !modify;

    let quiet = get_bool(&raw, "quiet").unwrap_or(false);
    let silent = quiet || output_to_stdout;

    if input_files.is_empty() {
        tidy_stdin(&raw, silent);
    } else if deprecated_stdin_flag {
        eprintln!(
            "Interpreting \"-\" as stdin. NOTICE: as of v1.10.0 \"-\" can be omitted and will be invalid in v2. Stdin is read when no input file is specified."
        );
        if input_files.len() > 1 {
            eprintln!("Input files cannot be specified with \"-\"");
            exit(1);
        }
        tidy_stdin(&raw, silent);
    } else {
        tidy_input_files(&input_files, &raw, silent);
    }
}

fn tidy_stdin(raw: &HashMap<String, OptVal>, silent: bool) {
    if get_bool(raw, "modify").unwrap_or(false) {
        eprintln!("--modify/-m is only valid when specifying input files");
        exit(1);
    }
    if get_bool(raw, "backup").unwrap_or(false) {
        eprintln!("--backup is only valid when specifying input files");
        exit(1);
    }

    if !silent {
        println!("Tidying...");
    }
    let input = read_stdin();
    let result = match tidy::tidy(&input, raw) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            exit(1);
        }
    };

    if let Some(path) = get_str(raw, "output-path") {
        tidy_to_output_file(
            &result,
            &path,
            false,
            get_bool(raw, "quiet").unwrap_or(false),
        );
    } else {
        print!("{}", result.bibtex);
    }
}

fn tidy_input_files(input_files: &[String], raw: &HashMap<String, OptVal>, silent: bool) {
    let v2 = get_bool(raw, "v2").unwrap_or(false);
    let has_output = get_str(raw, "output-path").is_some();
    let mut modify = get_bool(raw, "modify").unwrap_or(false);
    let mut backup = get_bool(raw, "backup").unwrap_or(false);

    let using_deprecated_stdio = !v2 && !has_output && !modify;
    if using_deprecated_stdio {
        eprintln!("NOTICE: In v2 you will need to specify --modify/-m to modify the input file.");
        modify = true;
        backup = true;
    }

    // The deprecation notice forces logging back on.
    let silent = if using_deprecated_stdio {
        false
    } else {
        silent
    };

    if modify {
        if has_output {
            eprintln!("--modify/-m is not valid when specifying an output file");
            exit(1);
        }
        if !silent {
            println!("Tidying...");
        }
        for input_file in input_files {
            let content = match read_file(input_file) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    exit(1);
                }
            };
            let result = match tidy::tidy(&content, raw) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{e}");
                    exit(1);
                }
            };
            tidy_to_output_file(
                &result,
                input_file,
                backup,
                get_bool(raw, "quiet").unwrap_or(false),
            );
        }
    } else if input_files.len() > 1 {
        eprintln!("Only one input file permitted unless using --modify/-m");
        exit(1);
    } else {
        if backup {
            eprintln!("--backup is only permitted when --modify/-m is provided");
            exit(1);
        }
        if !silent {
            println!("Tidying...");
        }
        let content = match read_file(&input_files[0]) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{e}");
                exit(1);
            }
        };
        let result = match tidy::tidy(&content, raw) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                exit(1);
            }
        };
        if let Some(path) = get_str(raw, "output-path") {
            tidy_to_output_file(
                &result,
                &path,
                backup,
                get_bool(raw, "quiet").unwrap_or(false),
            );
        } else {
            print!("{}", result.bibtex);
        }
    }
}

fn tidy_to_output_file(result: &TidyResult, path: &str, backup: bool, quiet: bool) {
    if backup {
        let _ = std::fs::copy(path, format!("{path}.original"));
    }

    if let Err(e) = std::fs::write(path, &result.bibtex) {
        eprintln!("{e}");
        exit(1);
    }

    if !quiet {
        for warning in &result.warnings {
            eprintln!("{}: {}", warning.code, warning.message);
        }
        println!("Done. Successfully tidied {} entries.", result.count);
    }
}

fn read_stdin() -> String {
    let mut buf = String::new();
    let _ = std::io::stdin().read_to_string(&mut buf);
    buf
}

fn read_file(path: &str) -> std::io::Result<String> {
    let mut buf = String::new();
    std::fs::File::open(path)?.read_to_string(&mut buf)?;
    Ok(buf)
}

const HELP: &str = r#"Usage: bibtex-tidy [infile] [-o outfile] [option...]

bibtex-tidy — clean and format BibTeX files

Reads BibTeX from a file (or stdin when no file is given) and writes tidied
BibTeX to an output file or, by default, to stdout. Input files are modified in
place unless an output file is provided or stdin is used.

Options:
  -h, --help                 Show this help message and exit.
  -v, --version              Show the version and exit.
  -q, --quiet                Suppress status messages on stdout.
  -m, --modify               Overwrite the input file(s) with the tidied result.
      --no-modify            Write to stdout instead of modifying input files.
  -o, --output <path>        Write the result to the given path.
      --backup / --no-backup Make a <file>.original backup (default: on).
      --v2                   Enable the planned v2 behavior (no implicit modify).

Formatting:
      --space=<n>            Indent fields with <n> spaces (default: 2).
      --tab                  Indent fields with a tab (overrides --space).
      --align=<n>            Align values at column <n> (default: 14).
      --no-align             Disable value alignment.
      --wrap=<n>             Wrap long values at column <n> (default: 80).
      --no-wrap              Disable value wrapping.
      --blank-lines          Insert a blank line between entries.
      --trailing-commas      End the last field of each entry with a comma.

Fields and values:
      --omit=<f1,f2>         Remove the given fields.
      --sort-fields=<f1,f2>  Order fields within each entry.
      --curly / --no-curly   Enclose all values in braces.
      --numeric / --no-numeric
                             Strip quotes/braces from numeric and month values.
      --strip-enclosing-braces
                             Remove redundant double braces around a value.
      --enclosing-braces[=<f>]  Double-brace values to preserve case.
      --remove-braces[=<f>]  Remove braces inside the given fields.
      --remove-empty-fields  Drop fields whose value is empty.
      --remove-dupe-fields / --no-remove-dupe-fields
                             Keep only the first of each field (default: on).
      --lowercase / --no-lowercase
                             Lowercase field names and entry types (default: on).

Entries:
      --sort[=<f1,f2>]       Sort entries by the given fields (default: key).
      --no-sort              Do not sort entries.
      --months               Abbreviate month names.
      --drop-all-caps        Convert ALL-CAPS values to title case.
      --max-authors=<n>      Truncate long author lists to "and others".

Duplicates:
      --duplicates[=<rules>] Check for duplicates (doi, key, abstract, citation).
      --merge[=<strategy>]   Merge duplicates (first, last, combine, overwrite).

Comments and escaping:
      --tidy-comments / --no-tidy-comments
                             Trim whitespace around comments (default: on).
      --strip-comments       Remove comments entirely.
      --escape[=new]         Escape non-ASCII characters (default: legacy).
      --no-escape            Do not escape characters.
      --unescape             Convert LaTeX escapes to Unicode characters.
      --encode-urls          Percent-encode invalid URL characters.
      --generate-keys[=tpl]  Regenerate citation keys (experimental).
"#;

#[cfg(test)]
mod tests {
    use crate::format::format_bibtex;
    use crate::parser::parse;
    use crate::utils::{convert_crlf, unwrap_text, wrap_text};

    /// Round-trip a raw parse+format (no transforms) and compare to the original's
    /// baseline output captured via the pinned upstream source.
    fn roundtrip(input: &str) -> String {
        let fixed = convert_crlf(input);
        let ast = parse(&fixed).expect("parse");
        format_bibtex(&ast).expect("format")
    }

    #[test]
    fn roundtrip_basic_entry() {
        assert_eq!(
            roundtrip("@article{key, title={T}}"),
            "@article{key, title= {T}\n}\n"
        );
    }

    #[test]
    fn roundtrip_whitespace_before_at() {
        assert_eq!(
            roundtrip("some text before\n@article{key, title={T}}"),
            "some text before\n@article{key, title= {T}\n}\n"
        );
    }

    #[test]
    fn roundtrip_comment_string_preamble() {
        assert_eq!(
            roundtrip(
                "@comment{hi {x}}\n@string{foo = \"bar\"}\n@preamble{\"\\sl\"}\n@article{key, title={T}}"
            ),
            "@comment{hi {x}}\n@string{foo = \"bar\"}\n@preamble{\"\\sl\"}\n@article{key, title= {T}\n}\n"
        );
    }

    #[test]
    fn roundtrip_concat_and_quoted() {
        assert_eq!(
            roundtrip("@article{key, title={A} # \"B\" # foo}"),
            "@article{key, title= {A} # \"B\" # foo\n}\n"
        );
        assert_eq!(
            roundtrip("@article{key, title=\"Hello {\"} World\"}"),
            "@article{key, title= \"Hello {\"} World\"\n}\n"
        );
    }

    #[test]
    fn roundtrip_parens_entry_normalized_to_braces() {
        assert_eq!(
            roundtrip("@article(key, title={T})"),
            "@article{key, title= {T}\n}\n"
        );
    }

    #[test]
    fn roundtrip_unterminated_entry_autocloses() {
        assert_eq!(
            roundtrip("@article{key, author={smith}"),
            "@article{key, author= {smith}\n}\n"
        );
        assert_eq!(
            roundtrip("@article{key, author={smith"),
            "@article{key, author= {}\n}\n"
        );
        assert_eq!(
            roundtrip("@article{key, author"),
            "@article{key, author\n}\n"
        );
    }

    #[test]
    fn roundtrip_trailing_comma_drops_empty_field() {
        assert_eq!(
            roundtrip("@article{key, author={X},}"),
            "@article{key, author= {X}\n}\n"
        );
    }

    #[test]
    fn roundtrip_nokey_entry() {
        assert_eq!(
            roundtrip("@article{, title={T}}"),
            "@article{ title= {T}\n}\n"
        );
        assert_eq!(
            roundtrip("@article{title={T}, author={A}}"),
            "@article{title= {T} author= {A}\n}\n"
        );
    }

    #[test]
    fn roundtrip_leading_bom() {
        assert_eq!(
            roundtrip("\u{FEFF}@article{key, title={T}}"),
            "\u{FEFF}@article{key, title= {T}\n}\n"
        );
    }

    #[test]
    fn syntax_error_on_unclosed_quote() {
        assert!(parse("@article{key, title=\"unclosed}").is_err());
    }

    #[test]
    fn fatal_on_bare_at() {
        // A bare `@` produces a block with no inner content -> format error (original
        // throws "FATAL!").
        let ast = parse("@").expect("parse");
        assert!(format_bibtex(&ast).is_err());
    }

    #[test]
    fn wrap_and_unwrap_text() {
        assert_eq!(wrap_text("a bb ccc dddd", 6), vec!["a bb", "ccc", "dddd"]);
        assert_eq!(unwrap_text("a\nb\n\nc"), "a b\n\nc");
    }
}
