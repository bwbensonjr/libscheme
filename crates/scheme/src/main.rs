//! The `scheme` REPL binary — the Rust analogue of `src/main.c`.
//!
//! A thin driver around the library: collect the command-line files and hand
//! them to [`libscheme::repl::run`], which builds the standard environment,
//! loads the files, and enters the read-eval-print loop on a large-stack thread.

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    libscheme::repl::run(files);
}
