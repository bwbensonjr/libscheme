//! libscheme — a Scheme interpreter packaged as an extensible library.
//!
//! This is a Rust port of the C `libscheme`. The central design point survives
//! the port: the same extension API used to add new primitives
//! ([`Interp::register`]) is how the interpreter implements its own built-ins.
//! Each Scheme subsystem lives in its own module mirroring a `scheme_*.c` file,
//! and [`Interp::basic_env`] wires them together in the original bootstrap order.
//!
//! Phase 0 establishes the foundations: the [`value::Value`] representation, the
//! [`interner`], the [`error`] channel (errors + escape continuations), the
//! [`env`] frames, and the [`interp::Interp`] extension API. Reader, eval, and
//! the typed subsystems land in later phases.

// The `gc_derive 0.5` macros emit impls inside an anonymous const, which trips
// the `non_local_definitions` lint on modern rustc. The generated code is
// correct; silence the noise until the dependency is updated.
#![allow(non_local_definitions)]

pub mod boolean;
pub mod char;
pub mod env;
pub mod error;
pub mod eval;
pub mod fun;
pub mod interner;
pub mod interp;
pub mod list;
pub mod number;
pub mod port;
pub mod printer;
pub mod promise;
pub mod reader;
pub mod string;
pub mod structs;
pub mod symbol;
pub mod syntax;
pub mod value;
pub mod vector;

pub use error::{SchemeError, SchemeResult};
pub use interner::Symbol;
pub use interp::Interp;
pub use printer::{display_to_string, write_to_string};
pub use reader::Reader;
pub use value::Value;

#[cfg(test)]
mod gc_tests {
    //! Phase 0 de-risking: prove the `gc` crate actually reclaims a cyclic
    //! structure built with `set-cdr!`-style mutation. This is the assumption
    //! the whole memory strategy rests on (the plan's riskiest item), so we
    //! verify it before building anything on top. The node here stands in for a
    //! cons cell with a mutable `cdr` (the `GcCell<Option<..>>` link).

    use gc::{Finalize, Gc, GcCell, Trace};
    use std::cell::Cell;

    thread_local! {
        static DROPPED: Cell<usize> = const { Cell::new(0) };
    }

    #[derive(Trace, Finalize)]
    struct Node {
        // A mutable link, like a cons cell's `cdr` under `set-cdr!`.
        next: GcCell<Option<Gc<Node>>>,
        // Records finalization so the test can observe collection of the cycle.
        #[unsafe_ignore_trace]
        _probe: DropProbe,
    }

    struct DropProbe;

    impl Drop for DropProbe {
        fn drop(&mut self) {
            DROPPED.with(|d| d.set(d.get() + 1));
        }
    }

    impl Finalize for DropProbe {}
    // SAFETY: DropProbe owns no Gc pointers, so it has nothing to trace.
    unsafe impl Trace for DropProbe {
        gc::unsafe_empty_trace!();
    }

    #[test]
    fn gc_collects_a_setcdr_cycle() {
        DROPPED.with(|d| d.set(0));

        {
            let a = Gc::new(Node {
                next: GcCell::new(None),
                _probe: DropProbe,
            });
            let b = Gc::new(Node {
                next: GcCell::new(Some(a.clone())),
                _probe: DropProbe,
            });
            // set-cdr! a -> b : now a <-> b is a cycle that Rc could never free.
            *a.next.borrow_mut() = Some(b.clone());
        }

        // Force a collection; the cycle must be reclaimable.
        gc::force_collect();
        let dropped = DROPPED.with(|d| d.get());
        assert_eq!(
            dropped, 2,
            "gc must finalize both probes in the cycle (got {dropped})"
        );
    }
}
