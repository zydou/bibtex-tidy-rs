//! Entry point for the `bibtex-tidy` port.
//!
//! Replace the placeholders in `Cargo.toml` and generate the lockfile before
//! the first build. Replace this stub during S2 (walking skeleton) with the
//! real CLI.

fn main() {
    println!("bibtex-tidy {}", env!("CARGO_PKG_VERSION"));
}
