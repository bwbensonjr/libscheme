//! Symbol primitives — the Rust analogue of the primitive half of
//! `scheme_symbol.c` (the interning half lives in [`crate::interner`]).
//!
//! `string->symbol` interns through the [`Interp`], so
//! `(eq? (string->symbol "x") 'x)` holds — the R4RS-correct behavior.
//! `symbol->string` resolves the interned (lowercased) name.

use crate::error::SchemeError;
use crate::interp::Interp;
use crate::value::{Arity, Value};

pub fn init(it: &mut Interp) {
    let sym_ty = it.make_type("<symbol>");
    it.register_value("<symbol>", Value::TypeObject(sym_ty));

    it.register("symbol?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Symbol(_))))
    });
    it.register("string->symbol", Arity::Exact(1), |it, a| match &a[0] {
        Value::Str(s) => {
            let name = s.borrow().clone();
            Ok(Value::Symbol(it.intern(&name)))
        }
        _ => Err(SchemeError::msg("string->symbol: arg must be a string")),
    });
    it.register("symbol->string", Arity::Exact(1), |it, a| match &a[0] {
        Value::Symbol(s) => Ok(Value::make_string(it.resolve(*s).to_string())),
        _ => Err(SchemeError::msg("symbol->string: arg must be a symbol")),
    });
}
