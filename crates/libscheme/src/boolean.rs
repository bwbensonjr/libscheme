//! Boolean and equivalence primitives — the Rust analogue of `scheme_bool.c`.
//!
//! `not` is true only for `#f`. `eq?`/`eqv?`/`equal?` delegate to the
//! corresponding [`Value`] methods, which mirror the C identity/value/structural
//! semantics.

use crate::interp::Interp;
use crate::value::{Arity, Value};

pub fn init(it: &mut Interp) {
    let true_ty = it.make_type("<true>");
    let false_ty = it.make_type("<false>");
    it.register_value("<true>", Value::TypeObject(true_ty));
    it.register_value("<false>", Value::TypeObject(false_ty));

    it.register("not", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Bool(false))))
    });
    it.register("boolean?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Bool(_))))
    });
    it.register("eq?", Arity::Exact(2), |_it, a| {
        Ok(Value::Bool(a[0].eq(&a[1])))
    });
    it.register("eqv?", Arity::Exact(2), |_it, a| {
        Ok(Value::Bool(a[0].eqv(&a[1])))
    });
    it.register("equal?", Arity::Exact(2), |_it, a| {
        Ok(Value::Bool(a[0].equal(&a[1])))
    });
}
