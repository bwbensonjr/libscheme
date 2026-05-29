# Feasibility: Re-implementing libscheme in Rust

## Context

`libscheme` is a ~6,800-line C Scheme interpreter packaged as an *extensible library*
(20 `scheme_*.c` subsystems + a thin REPL `main.c`, plus a `posix/` driver that
demonstrates extending the library from outside). The user wants to assess whether it
can be re-implemented in Rust while keeping the same design principles (library-first,
extensible primitives, per-subsystem init order) and structure, with enough tests to
validate behavioral compatibility.

**Verdict: feasible and well-suited to Rust.** The codebase is small, single-threaded,
and cleanly factored into subsystems, each of which maps to a Rust module. The four
things that look hard in C — pointer-identity type tags, `setjmp`/`longjmp` errors and
escape continuations, Boehm GC, and the C function-pointer extension API — all have
clean, safe-Rust equivalents (enum discriminants, the `Result` channel, the `gc` crate,
and `Rc<dyn Fn>` primitives). The R4RS `test.scm` suite already exists and serves as a
ready-made acceptance oracle. Estimated effort: **~30–35 engineering-days** for full
parity.

**Decisions locked with the user:** idiomatic *safe* Rust (enum `Value`, `Result`-based
errors, minimal `unsafe`); **full parity** (all types, special forms, `defmacro`,
structs, promises, ports, escape-only `call/cc`, plus the POSIX extension demo); and
validation by **porting `test.scm` as a golden acceptance test *and* per-subsystem Rust
`#[test]` units**, both gating CI.

## Why it's feasible — the four "hard" things mapped

| C mechanism | Rust equivalent |
|---|---|
| `Scheme_Object` tagged union + `type` pointer; pointer-identity type checks (`scheme.h:60`) | `enum Value` — discriminant *is* the type. Dynamic nominal types (structs, posix `<stat>`/`<dir>`) carry a `Gc<TypeObject>` compared by `Gc::ptr_eq` to preserve identity predicates. |
| Interned symbols, `eq?`/`null?`/`boolean?` by pointer (`scheme_symbol.c`) | `Interner` in `Interp` returning `Copy Symbol(u32)`; `eq?` = u32/variant compare. |
| `scheme_signal_error` → `longjmp` to REPL `setjmp` (`scheme_error.c:49`) | `Result<Value, SchemeError>` threaded through eval; `?` unwinds the native stack. |
| escape `call/cc` via captured `jmp_buf` (`scheme_fun.c:363`) | `Err(SchemeError::ContinuationInvoked{id,value})`; the matching `call/cc` frame catches by `id`, others re-propagate. Upward continuations surface as a *safe* error (matching C's `(test-cont)` failure, without UB). |
| Boehm `GC_malloc`; cyclic graphs via `set-car!`/`vector-set!` (`scheme.h:177`) | `gc` + `gc_derive` crate: `Gc<GcCell<T>>` for mutable/cyclic payloads (pairs, vectors, strings, struct instances). |
| `Scheme_Prim fn(argc, argv[])` + `scheme_make_prim` + `scheme_add_global` + `scheme_init_*` ordered by `scheme_basic_env` (`scheme_env.c:37`) | `Rc<dyn Fn(&mut Interp,&[Value])->Result<Value,SchemeError>>`; `Interp::register(name, arity, f)`; per-module `init(&mut Interp)` called in the same deliberate order. |

A structural win discovered during design: C copies the shared `globals` pointer into
**every** `Scheme_Env` (`scheme_env.c:112`), so essentially every recursive global
closure forms a cycle (closure → env → globals → closure). In the Rust port, globals
live once in `Interp` and closures capture only their *lexical* chain (bottoming out at
`None`), so that dominant cycle disappears — the GC only handles user-built (`set-cdr!`)
cycles. This makes even an `Rc`-only fallback far less leaky than a naive translation.

## Value representation

`enum Value` (`#[derive(Trace, Finalize, Clone)]`), cheap to clone (heap variants are
`Gc` handles):

- Inline scalars: `Null`, `Bool`, `Int(i64)`, `Double(f64)`, `Char(char)`, `Eof`, `Symbol(Symbol)`.
- `Gc<GcCell<..>>` (mutable / cyclic): `Str`, `Pair{car,cdr}`, `Vector(Vec<Value>)`,
  `Promise`, `InputPort`, `OutputPort`, `Struct{ty,fields}`.
- `Gc<..>` (immutable handle): `Closure{env,params,body}`, `Prim`, `Continuation{id}`,
  `Syntax`, `Macro`, `TypeObject{name,id}`, `StructProc`.
- `Foreign(Gc<GcCell<dyn ForeignData>>)` for extension payloads (posix `DIR*`/`stat`),
  downcast via `Any`; the C `void* port_data`/`close_fun` pattern becomes `Drop`.

`eq?`/`eqv?`/`equal?` become `Value` methods mirroring C identity (`scheme.h:285`).
Start with Rust `char` + `String`; only revisit byte semantics if a `test-sc4` char case fails.

## Errors & escape continuations

```
enum SchemeError { User(UserError), ContinuationInvoked{ id: ContId, value: Value } }
```
`call/cc` mints a monotonic `ContId`, applies the proc to a `Continuation` value; invoking
that value returns `Err(ContinuationInvoked{..})`; the originating `call/cc` frame matches
its own `id` and converts to `Ok(value)`, re-propagating foreign tokens and `User` errors.
The REPL boundary is the `SCHEME_CATCH_ERROR` analogue (`main.c:61`). `(error ...)`
returns `Err(User{message,irritants})` instead of stderr-print + longjmp.

## Environment

`Gc<Env { vars: Vec<Symbol>, vals: GcCell<Vec<Value>>, parent: Option<Gc<Env>> }>`;
`globals: HashMap<Symbol,Value>` owned by `Interp` (not duplicated per frame). Lookup
walks frames by `Symbol`, then globals — faithful to `scheme_env.c:185`. **Preserve
exactly:** top-level `define` writes to globals (`scheme_syntax.c:132`), while *internal*
defines are rewritten to a `letrec`-style frame by the closure/let path
(`scheme_fun.c:139-183`) — port this almost line-for-line; do not "modernize" it.

## Eval loop & TCO

Tree-walker mirroring `scheme_eval.c`/`scheme_fun.c`, with two deliberate,
behavior-preserving safety improvements over the C (which has no TCO and overflows):
1. **Tail-position loop** in `eval`/`apply_closure`: in tail position, reassign
   `expr`/`env` and `continue` instead of recursing — proper tail calls for the common
   forms, no semantic change.
2. **Run eval on a worker thread with a large stack** (`thread::Builder::stack_size`),
   because a Rust stack overflow *aborts* (uncatchable) whereas the heavier safe-Rust
   frames overflow sooner than C.

Do **not** attempt a full CPS/trampoline rewrite — unnecessary for the suite and it would
obscure the C correspondence. The printer matches C (no cycle detection).

## Crate / module layout (mirrors the C files 1:1 for reviewability)

Cargo **workspace**:
- `crates/libscheme/` — library crate. Modules: `value`, `interner`, `env`, `error`,
  `interp` (register*/make_type/basic_env), `reader`(`scheme_read.c`),
  `printer`(`scheme_print.c`), `eval`(`scheme_eval.c`+`scheme_fun.c`),
  `number`(+`nummacs`), `list`, `string`, `char`, `vector`, `boolean`, `symbol`,
  `port`, `promise`, `structs`, `syntax`(`scheme_syntax.c`), `repl`(`main.c`).
  `scheme_hash.c`→`HashMap`, `scheme_alloc.c`→`gc` crate, `scheme_type.c`→`Interp::make_type`
  (absorbed; note this in the mapping doc).
- `crates/scheme/` — REPL binary (`main.c`).
- `crates/posix/` — **separate crate** (not a feature; the whole point is "extension lives
  outside the core") with `init_posix_file`/`init_posix_proc` (`posix_file.c`/`posix_proc.c`)
  and a `posix_scheme` binary. `libc`/`nix` confined here so the core stays `unsafe`-free.

`Interp::basic_env()` calls each module's `init(&mut it)` in the **same order** as
`scheme_basic_env` (`scheme_env.c:43`), reproducing the "add to the end, not the
beginning" rule. A user extension registers identically:
```
let mut it = libscheme::Interp::basic_env();
posix::init_posix_file(&mut it);   // == posix/main.c
posix::init_posix_proc(&mut it);
libscheme::repl::run(&mut it, std::env::args());
```

## Implementation phases (respecting bootstrap dependency order)

0. **Foundations (3–4d):** `value`, `interner`, `error`, `interp` skeleton, `gc` wiring,
   `eq?`/`eqv?`/`equal?`. **De-risk first:** a `#[test]` proving `rust-gc` collects a
   `set-cdr!` cycle.
1. **Reader + printer (3d):** round-trip `write-test-obj` (`test.scm:844`).
2. **Core eval + env + fun (4–5d):** lookup, combination eval, `apply`, tail-call loop,
   rest-args, internal-define rewriting.
3. **Special forms (4–5d):** `syntax.rs` — all forms incl. named-let, `do`, `cond =>`,
   `case`, `quasiquote` splicing, `defmacro`. (Largest file; second-riskiest.)
4. **Typed subsystems (5–6d, parallelizable):** `number` (exact/inexact contagion),
   `list`, `string`, `char`, `vector`, `boolean`, `symbol` — each with units.
5. **Ports + promises + structs + call/cc (4–5d):** file/string ports (heavily used
   in `test.scm:818-867`), `delay`/`force`, `define-struct`, escape-continuation machinery.
6. **REPL + posix crate (3d):** `#!` skip, big-stack thread, then the extension crate.
7. **Acceptance + CI (3–4d):** port `test.scm` as a golden test asserting "Passed all
   tests" plus clean `(test-sc4)`/`(test-inexact)`; wire golden + unit tests into CI.

### Riskiest parts (de-risk in this order)
1. **GC viability** — validate `rust-gc` in Phase 0; fallback is `Rc` + globals-in-Interp
   (already less leaky than naive `Rc`).
2. **Numeric exact/inexact rules** for `test-inexact` (`scheme_number.c`, `nummacs`).
3. **`quasiquote` nesting/splicing** + **internal-define rewriting** (subtle, well-exercised).
4. **call/cc id-matching** for nested cases; confirm `(test-cont)` fails *gracefully*.
5. **Char/string byte-vs-`char`** semantics for any non-ASCII `test-sc4` checks.

## Critical reference files (the C oracle to mirror)

- `src/scheme.h` — full type taxonomy + public extension API to reproduce.
- `src/scheme_fun.c` — apply, closures, call/cc, internal-define (eval/apply core).
- `src/scheme_syntax.c` — all special forms, `defmacro`, `quasiquote`.
- `src/scheme_env.c` — frame/globals semantics and `define`/`set!` rules to preserve.
- `src/posix/posix_file.c` — the extension-registration pattern the public API must support.
- `src/test.scm` — the acceptance oracle.

## Verification

- `cargo test` — per-subsystem unit tests (interning identity, GC cycle collection,
  reader round-trip, numeric tower, each primitive family, call/cc escape).
- **Golden acceptance:** a test that loads the ported `test.scm` through the new
  interpreter and asserts the harness reports "Passed all tests", and that `(test-sc4)`
  and `(test-inexact)` run clean. `(test-cont)` is expected to fail (upward continuations
  unsupported) and is asserted to fail *gracefully* (a reported error, not a crash).
- Manual parity spot-check: run the C `./scheme` and the Rust `scheme` REPL on the same
  inputs and diff output.
- `posix` crate has its own integration tests proving a third-party extension registers
  primitives and nominal types through the public API alone.

## Scope notes / non-goals (matching C)

No bignums (fixnum `i64` + `f64` only), no `syntax-rules` (only `defmacro`), no module
system, escape-only `call/cc` (no upward continuations). These are intentional parity
constraints, not omissions.
