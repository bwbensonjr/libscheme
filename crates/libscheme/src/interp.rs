//! The interpreter — owns the global state the C version kept in process
//! globals (`symbol_table`, `scheme_env`, the type singletons) and exposes the
//! extension API that is the heart of the "library, not application" design:
//! `register*` (= `scheme_make_prim` + `scheme_add_global`) and `make_type`
//! (= `scheme_make_type`).
//!
//! Each subsystem module supplies an `init(&mut Interp)` function, and
//! [`Interp::basic_env`] calls them in the same deliberate order as
//! `scheme_basic_env` (scheme_env.c:43) — types before the primitives that
//! reference them. As in the C header comment: add new init calls to the END
//! of that list, not the beginning.

use crate::error::ContId;
use crate::interner::{Interner, Symbol};
use crate::value::{Arity, Primitive, SyntaxForm, SyntaxHandler, TypeObject, Value};
use gc::Gc;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Interp {
    interner: Interner,
    /// The single global binding table (keyed by interned [`Symbol`]). In C this
    /// was a string-keyed hash copied into every frame; here it lives once.
    globals: HashMap<Symbol, Value>,
    /// Interned `define`/`lambda`, cached so the evaluator's hot path
    /// (internal-define handling in `eval_body_tail`) need not re-intern them on
    /// every body evaluation.
    sym_define: Symbol,
    sym_lambda: Symbol,
    next_type_id: u64,
    next_cont_id: u64,
    /// The current input/output ports (`cur_in_port`/`cur_out_port`,
    /// scheme_port.c:48). Set up by the port subsystem; `None` until then.
    cur_in: Option<Value>,
    cur_out: Option<Value>,
}

impl Interp {
    /// A bare interpreter with no bindings. Use [`Interp::basic_env`] for the
    /// standard environment.
    pub fn new() -> Self {
        let mut interner = Interner::new();
        let sym_define = interner.intern("define");
        let sym_lambda = interner.intern("lambda");
        Interp {
            interner,
            globals: HashMap::new(),
            sym_define,
            sym_lambda,
            next_type_id: 0,
            next_cont_id: 0,
            cur_in: None,
            cur_out: None,
        }
    }

    /// The interned `define` symbol (cached at construction).
    pub(crate) fn sym_define(&self) -> Symbol {
        self.sym_define
    }

    /// The interned `lambda` symbol (cached at construction).
    pub(crate) fn sym_lambda(&self) -> Symbol {
        self.sym_lambda
    }

    /// The current input port (`current-input-port`).
    pub fn cur_in(&self) -> Option<Value> {
        self.cur_in.clone()
    }
    /// The current output port (`current-output-port`).
    pub fn cur_out(&self) -> Option<Value> {
        self.cur_out.clone()
    }
    pub fn set_cur_in(&mut self, port: Value) {
        self.cur_in = Some(port);
    }
    pub fn set_cur_out(&mut self, port: Value) {
        self.cur_out = Some(port);
    }

    // --- interning (delegates to the owned Interner) ---

    pub fn intern(&mut self, name: &str) -> Symbol {
        self.interner.intern(name)
    }

    pub fn resolve(&self, sym: Symbol) -> &str {
        self.interner.resolve(sym)
    }

    // --- globals ---

    pub fn lookup_global(&self, sym: Symbol) -> Option<Value> {
        self.globals.get(&sym).cloned()
    }

    pub fn set_global(&mut self, sym: Symbol, val: Value) {
        self.globals.insert(sym, val);
    }

    /// True if a global binding exists — used by `set!` to distinguish "unbound"
    /// from "assignable global" (scheme_env.c:153).
    pub fn has_global(&self, sym: Symbol) -> bool {
        self.globals.contains_key(&sym)
    }

    // --- the extension API: register* and make_type ---

    /// Register a primitive under `name` — the Rust `scheme_add_global` +
    /// `scheme_make_prim`. The same call built-ins and user extensions use.
    pub fn register<F>(&mut self, name: &str, arity: Arity, f: F)
    where
        F: Fn(&mut Interp, &[Value]) -> crate::error::SchemeResult + 'static,
    {
        let sym = self.intern(name);
        let prim = Value::Prim(Gc::new(Primitive {
            name: sym,
            arity,
            f: Rc::new(f),
        }));
        self.set_global(sym, prim);
    }

    /// Bind a ready-made value (a constant, a type object, etc.) to `name`.
    pub fn register_value(&mut self, name: &str, val: Value) {
        let sym = self.intern(name);
        self.set_global(sym, val);
    }

    /// Register a special form (`scheme_make_syntax`).
    pub fn register_syntax(&mut self, name: &str, handler: SyntaxHandler) {
        let sym = self.intern(name);
        let form = Value::Syntax(Gc::new(SyntaxForm { name: sym, handler }));
        self.set_global(sym, form);
    }

    /// Mint a fresh nominal type — the Rust `scheme_make_type` (scheme_type.c).
    /// Each call yields a distinct identity, so predicates over it are exact.
    pub fn make_type(&mut self, name: &str) -> Gc<TypeObject> {
        let id = self.next_type_id;
        self.next_type_id += 1;
        Gc::new(TypeObject {
            name: name.to_string(),
            id,
        })
    }

    /// Allocate a fresh continuation id for `call/cc` (the `setjmp` analogue).
    pub fn fresh_cont_id(&mut self) -> ContId {
        let id = self.next_cont_id;
        self.next_cont_id += 1;
        ContId(id)
    }

    /// Build the standard environment by running each subsystem's `init` in the
    /// deliberate bootstrap order (scheme_basic_env, scheme_env.c:43).
    ///
    /// As subsystems land in later phases, add their `init` calls to the END of
    /// this list — never the beginning — because later inits depend on the type
    /// objects and primitives earlier ones register.
    pub fn basic_env() -> Interp {
        let mut it = Interp::new();
        // Bootstrap order mirrors scheme_basic_env (scheme_env.c:43). Type
        // registration is folded into make_type, so there is no separate
        // type init.
        crate::fun::init(&mut it);
        crate::symbol::init(&mut it);
        crate::list::init(&mut it);
        crate::number::init(&mut it);
        crate::port::init(&mut it);
        crate::string::init(&mut it);
        crate::vector::init(&mut it);
        crate::char::init(&mut it);
        crate::boolean::init(&mut it);
        crate::syntax::init(&mut it);
        crate::eval::init(&mut it);
        crate::promise::init(&mut it);
        crate::structs::init(&mut it);
        it
    }
}

impl Default for Interp {
    fn default() -> Self {
        Interp::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_call_a_primitive() {
        let mut it = Interp::basic_env();
        it.register("double", Arity::Exact(1), |_it, args| match args[0] {
            Value::Int(n) => Ok(Value::Int(n * 2)),
            _ => Err(crate::error::SchemeError::msg("double: not an int")),
        });
        let sym = it.intern("double");
        let prim = it.lookup_global(sym).expect("prim should be bound");
        if let Value::Prim(ref p) = prim {
            let out = (p.f)(&mut it, &[Value::Int(21)]).unwrap();
            assert!(out.eq(&Value::Int(42)));
            assert!(p.arity.accepts(1) && !p.arity.accepts(2));
        } else {
            panic!("expected a primitive");
        }
    }

    #[test]
    fn make_type_yields_distinct_identities() {
        let mut it = Interp::new();
        let a = it.make_type("<stat>");
        let b = it.make_type("<stat>");
        assert_ne!(a.id, b.id, "each make_type call is a fresh nominal type");
    }

    #[test]
    fn cont_ids_are_monotonic() {
        let mut it = Interp::new();
        assert_eq!(it.fresh_cont_id(), ContId(0));
        assert_eq!(it.fresh_cont_id(), ContId(1));
    }
}
