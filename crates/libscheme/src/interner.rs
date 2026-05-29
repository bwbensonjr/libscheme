//! Symbol interning — the Rust analogue of the interning half of `scheme_symbol.c`.
//!
//! In C, symbols are interned in a global hash table so that `eq?` (and the
//! environment's symbol lookup) can compare them by pointer identity. Here we
//! intern names into an [`Interner`] owned by the interpreter and hand back a
//! `Copy` [`Symbol`] id. `eq?` on symbols then reduces to a `u32` comparison,
//! exactly mirroring the C pointer-identity semantics.
//!
//! Like C (`scheme_intern_symbol` downcases via `scheme_add_global`), names are
//! lowercased before interning so the reader and `add_global` agree on identity.

use gc::{Finalize, Trace};
use std::collections::HashMap;

/// An interned symbol. Identity (`eq?`) is the wrapped id.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Symbol(pub u32);

// `Symbol` is `Copy` and holds no `Gc` pointers, so the `Trace` derive (which
// would add a destructor and conflict with `Copy`) is replaced by an empty
// manual impl.
impl Finalize for Symbol {}
unsafe impl Trace for Symbol {
    gc::unsafe_empty_trace!();
}

/// Owns the canonical name for every interned [`Symbol`].
#[derive(Default)]
pub struct Interner {
    names: Vec<Box<str>>,
    map: HashMap<Box<str>, Symbol>,
}

impl Interner {
    pub fn new() -> Self {
        Interner::default()
    }

    /// Intern `name` (lowercased), returning its canonical [`Symbol`].
    pub fn intern(&mut self, name: &str) -> Symbol {
        let lowered = name.to_ascii_lowercase();
        if let Some(&sym) = self.map.get(lowered.as_str()) {
            return sym;
        }
        let sym = Symbol(self.names.len() as u32);
        let boxed: Box<str> = lowered.into_boxed_str();
        self.names.push(boxed.clone());
        self.map.insert(boxed, sym);
        sym
    }

    /// Resolve a [`Symbol`] back to its canonical (lowercased) name.
    pub fn resolve(&self, sym: Symbol) -> &str {
        &self.names[sym.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_identity() {
        let mut it = Interner::new();
        let a = it.intern("foo");
        let b = it.intern("foo");
        assert_eq!(a, b, "same name must intern to the same Symbol");
    }

    #[test]
    fn interning_is_case_folded() {
        let mut it = Interner::new();
        assert_eq!(it.intern("Foo"), it.intern("foo"));
        let car = it.intern("CAR");
        assert_eq!(it.resolve(car), "car");
    }

    #[test]
    fn distinct_names_distinct_symbols() {
        let mut it = Interner::new();
        assert_ne!(it.intern("foo"), it.intern("bar"));
    }
}
