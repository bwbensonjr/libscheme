# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`libscheme` is a Scheme interpreter packaged as a C library. The "library, not application" framing is the central design point: the same C-level extension API used to add new primitives is also how the interpreter implements its own built-ins (lists, numbers, ports, etc.). The bundled `scheme` binary in `src/main.c` is just a thin REPL driver around the library; `src/posix/` is a second driver that demonstrates extending the library with POSIX bindings.

For background, see `src/doc/libscheme.md` (the 1994 USENIX VHLL paper).

## Design principles 

- Focus on simplicity and ease of understanding 
- Allow easy integration with existing libraries and APIs 

## Build & run

The library and REPL live in `src/`:

```sh
cd src
make            # builds libscheme.dylib and the `scheme` REPL
./scheme        # REPL
./scheme test.scm   # load a file then drop into REPL
make clean
```

The Makefile depends on the **Boehm-Demers-Weiser garbage collector** via Homebrew (`BDW_PATH=/opt/homebrew/opt/bdw-gc`). Adjust `BDW_PATH` for non-Homebrew or non-Apple-Silicon installs. There is no `make depend` or `make test` target despite what the legacy `INSTALL` file says — `test.scm` is loaded manually at the REPL.

The POSIX-extended REPL builds against the already-built `libscheme.dylib`:

```sh
cd src/posix
make            # builds posixscheme, links -lscheme from ../
```

## Architecture

### Object representation (`src/scheme.h`)

Every Scheme value is a `Scheme_Object` — a tagged union (`u`) plus a `type` pointer to another `Scheme_Object` representing the type. Type checks compare `SCHEME_TYPE(obj)` against the well-known type singletons declared in `scheme.h` (e.g. `scheme_pair_type`, `scheme_integer_type`). All field access goes through `SCHEME_*` macros — never reach into the union directly.

`scheme.h` is the public API surface. Anything a C extension needs is declared there: constructors (`scheme_make_*`), accessors, the eval/read/write entry points, error handling (`scheme_signal_error` + `SCHEME_CATCH_ERROR` setjmp/longjmp), the hash table, and per-subsystem `scheme_init_*` hooks.

### Subsystem layout (`src/scheme_*.c`)

Each Scheme type or feature lives in its own file and exposes a `scheme_init_<name>(Scheme_Env *)` function that registers the relevant primitives into the global environment via `scheme_add_global`. To add a new built-in, write a `Scheme_Prim` (`Scheme_Object *fn(int argc, Scheme_Object *argv[])`), wrap it with `scheme_make_prim`, and call `scheme_add_global("name", prim, env)` — same pattern user extensions use (see `src/posix/posix_file.c` for an example).

### Bootstrap order matters

`scheme_basic_env()` in `src/scheme_env.c` calls the `scheme_init_*` functions in a deliberate order — types must exist before primitives can reference them, etc. The header comment explicitly says: **add new init calls to the end of the list, not the beginning**. Drivers (`main.c`, `posix/main.c`) call `scheme_basic_env()` first, then their own init functions.

### Memory and GC

`src/scheme_alloc.c` has a `#define NO_GC 1` switch at the top that routes `scheme_malloc` to libc `malloc` instead of `GC_malloc`. The Makefile still links `-lgc` regardless. If you flip this, also recheck `scheme_calloc`'s memset (it's only zeroed under `NO_GC` because BDW-GC zeroes by default).

### REPL loop

Both `main.c` files use the same shape: build env → load command-line files → enter `read → SCHEME_CATCH_ERROR(eval) → write` loop. `SCHEME_CATCH_ERROR` is a `setjmp` macro — primitives that detect bad input call `scheme_signal_error(...)` which `longjmp`s back to it.

## Rust port

A from-scratch re-implementation in idiomatic, safe Rust lives in `crates/` (a cargo workspace), preserving the same design principles and a module-per-`scheme_*.c` structure. The C sources under `src/` are the reference; they are untouched by the port.

```sh
cargo test                              # unit + integration + the R4RS golden suite
cargo run -p scheme                     # the REPL (port of src/main.c)
cargo run -p posix --bin posix_scheme   # REPL + POSIX extensions (port of src/posix/)
```

- **Crates:** `crates/libscheme` (the interpreter as a library), `crates/scheme` (REPL binary), `crates/posix` (a *separate* extension crate — `libc` is confined here so the core stays `unsafe`-free).
- **C → Rust mapping:** `Scheme_Object` union → `enum Value` (the discriminant *is* the type); runtime nominal types (`define-struct`, posix `<stat>`) carry a `Gc<TypeObject>` compared by pointer identity. `setjmp`/`longjmp` errors and escape `call/cc` → a `Result<Value, SchemeError>` channel (`SchemeError::ContinuationInvoked` is caught by the matching `call/cc` frame). Boehm GC → the `gc` crate (`Gc<GcCell<T>>`); the evaluator **trampolines** tail calls (the C tree-walker has no TCO).
- **Load-bearing invariants (easy to break unknowingly):** each `scheme_*.c` maps to a module of the same name, and `Interp::basic_env` calls each module's `init` in the *same bootstrap order* as `scheme_basic_env` — append new inits to the END. Symbol interning is **case-sensitive**; case folding happens in the *reader* (so `string->symbol` preserves case, per R4RS §6.4). `Interp::register` / `register_value` / `register_syntax` + `make_type` are the public extension API — built-ins and the posix crate use the identical path.
- **Acceptance test:** `crates/libscheme/tests/r4rs_suite.rs` drives the original `src/test.scm` (copied to `crates/libscheme/tests/fixtures/`). The core suite plus `(test-sc4)` and `(test-inexact)` must pass; `(test-cont)` is *expected* to fail gracefully (escape-only continuations, like the C version — surfaced as an error, never UB). CI gates `cargo test`, `clippy -D warnings`, and `cargo fmt --check`.
- For the full design rationale and phase-by-phase mapping, see `RUST_PORT_FEASIBILITY.md`.

## Code style

- C source is formatted with **clang-format using the WebKit style** (per the most recent commit). Run `clang-format -i -style=WebKit src/*.c src/*.h` before committing C changes.
- Rust source is formatted with **rustfmt** and must be clippy-clean under `-D warnings` — CI enforces both. Run `cargo fmt --all` and `cargo clippy --workspace --all-targets` before committing Rust changes. Note CI tracks `stable`, which may be a newer clippy than a local toolchain, so new lints can surface only in CI.
- The codebase predates C99 — function definitions still use the K&R-ish split-line return-type-on-its-own-line form in many places. Match the surrounding style of the file you're editing rather than modernizing.
- No test framework — correctness is checked by loading `src/test.scm` (Aubrey Jaffer's R4RS test suite) at the REPL and running `(test-sc4)` / `(test-inexact)`. `(test-cont)` is documented as always failing because libscheme does not support upward continuations.
