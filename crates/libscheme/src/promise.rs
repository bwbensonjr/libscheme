//! Promises — the Rust analogue of `scheme_promise.c`.
//!
//! `delay` (in [`crate::syntax`]) already builds a [`Value::Promise`] wrapping a
//! zero-argument thunk closure. `force` applies that thunk the first time and
//! memoizes the result. Unlike the C `force` — which never sets its `forced`
//! flag and merely overwrites `val` with the (self-evaluating) result — we set
//! `forced` properly, giving correct memoization for any value type.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::{Arity, Value};

pub fn init(it: &mut Interp) {
    let promise_ty = it.make_type("<promise>");
    it.register_value("<promise>", Value::TypeObject(promise_ty));
    it.register("force", Arity::Exact(1), force);
}

fn force(it: &mut Interp, args: &[Value]) -> SchemeResult {
    let promise = match &args[0] {
        Value::Promise(p) => p.clone(),
        // R4RS: force of a non-promise returns it unchanged.
        other => return Ok(other.clone()),
    };

    // Already forced? Return the memoized value.
    if let Some(v) = {
        let b = promise.borrow();
        if b.forced {
            b.value.clone()
        } else {
            None
        }
    } {
        return Ok(v);
    }

    // Otherwise apply the thunk, then memoize.
    let thunk = promise
        .borrow()
        .thunk
        .clone()
        .ok_or_else(|| SchemeError::msg("force: promise has no thunk"))?;
    let value = it.apply_closure(thunk, &[])?;
    {
        let mut b = promise.borrow_mut();
        b.forced = true;
        b.value = Some(value.clone());
        b.thunk = None;
    }
    Ok(value)
}
