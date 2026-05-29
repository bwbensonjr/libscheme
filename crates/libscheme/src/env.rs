//! Lexical environments — the Rust analogue of `scheme_env.c`.
//!
//! An [`Env`] is one lexical frame: parallel `vars`/`vals` (as C used parallel
//! symbol/value arrays, scheme_env.c:88) plus a link to the enclosing frame.
//! Crucially — and unlike C, which copied the shared `globals` pointer into
//! every frame (scheme_env.c:112) — the global table lives once in
//! [`crate::interp::Interp`], not here. A closure therefore captures only its
//! lexical chain (bottoming out at `parent = None`), so the dominant
//! closure→env→globals→closure cycle of the C version simply does not form.
//! User-built cycles (via `set-cdr!`) remain, and the GC reclaims those.

use crate::interner::Symbol;
use crate::value::Value;
use gc::{Finalize, Gc, GcCell, Trace};

/// One lexical frame. `vars` is fixed for the life of the frame; `vals` is
/// interior-mutable so `set!`, `let*`, and `do` can rebind in place.
#[derive(Trace, Finalize)]
pub struct Env {
    vars: Vec<Symbol>,
    vals: GcCell<Vec<Value>>,
    parent: Option<Gc<Env>>,
}

impl Env {
    /// Create a new frame extending `parent` with the given bindings.
    /// `vars` and `vals` must have equal length.
    pub fn new(vars: Vec<Symbol>, vals: Vec<Value>, parent: Option<Gc<Env>>) -> Gc<Env> {
        debug_assert_eq!(vars.len(), vals.len());
        Gc::new(Env {
            vars,
            vals: GcCell::new(vals),
            parent,
        })
    }

    /// An empty root frame (no lexical bindings).
    pub fn root() -> Gc<Env> {
        Env::new(Vec::new(), Vec::new(), None)
    }

    /// Look up `sym` in this frame's lexical chain. Returns `None` if unbound
    /// lexically — the caller (the interpreter) then consults globals, matching
    /// the fallback in scheme_env.c:192.
    pub fn lookup(&self, sym: Symbol) -> Option<Value> {
        if let Some(i) = self.vars.iter().position(|&s| s == sym) {
            return Some(self.vals.borrow()[i].clone());
        }
        match &self.parent {
            Some(p) => p.lookup(sym),
            None => None,
        }
    }

    /// `set!` a lexically-bound variable. Returns `true` if `sym` was found and
    /// updated somewhere in the chain; `false` if not lexically bound (the
    /// caller then tries globals), mirroring scheme_env.c:153.
    pub fn set(&self, sym: Symbol, val: Value) -> bool {
        if let Some(i) = self.vars.iter().position(|&s| s == sym) {
            self.vals.borrow_mut()[i] = val;
            return true;
        }
        match &self.parent {
            Some(p) => p.set(sym, val),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interner::Symbol;

    #[test]
    fn lookup_walks_parent_chain() {
        let outer = Env::new(vec![Symbol(0)], vec![Value::Int(1)], None);
        let inner = Env::new(vec![Symbol(1)], vec![Value::Int(2)], Some(outer));
        assert!(matches!(inner.lookup(Symbol(1)), Some(Value::Int(2))));
        assert!(matches!(inner.lookup(Symbol(0)), Some(Value::Int(1))));
        assert!(inner.lookup(Symbol(2)).is_none());
    }

    #[test]
    fn set_mutates_in_place_and_shadows_correctly() {
        let outer = Env::new(vec![Symbol(0)], vec![Value::Int(1)], None);
        let inner = Env::new(vec![Symbol(0)], vec![Value::Int(9)], Some(outer.clone()));
        // inner shadows symbol 0; set! must hit the inner binding.
        assert!(inner.set(Symbol(0), Value::Int(42)));
        assert!(matches!(inner.lookup(Symbol(0)), Some(Value::Int(42))));
        assert!(matches!(outer.lookup(Symbol(0)), Some(Value::Int(1))));
    }

    #[test]
    fn set_reports_unbound() {
        let env = Env::root();
        assert!(!env.set(Symbol(7), Value::Null));
    }
}
