//! The `posix_scheme` REPL — the Rust analogue of `src/posix/main.c`.
//!
//! Identical in shape to the plain `scheme` driver, except it installs the
//! POSIX extensions into the interpreter before loading files and entering the
//! loop — exactly as `posix/main.c` calls `init_posix_file`/`init_posix_proc`
//! after `scheme_basic_env`. This is the "library, not application" principle in
//! action: the extension is a separate crate composed onto the core at startup.

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    libscheme::repl::run_with(files, |it| {
        posix::init_posix_file(it);
        posix::init_posix_proc(it);
    });
}
