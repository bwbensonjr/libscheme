//! Procedure primitives and continuations — the Rust analogue of the
//! primitive-registration half of `scheme_fun.c`.
//!
//! Registers the procedure type objects (`<primitive>`, `<closure>`,
//! `<continuation>`) and the primitives `procedure?`, `apply`, `map`,
//! `for-each`, and `call-with-current-continuation` / `call/cc`.
//!
//! `call/cc` is escape-only, exactly as in C (which used `setjmp`/`longjmp`).
//! Here it mints a [`crate::error::ContId`], hands the user procedure a
//! [`Value::Continuation`], and catches the matching `ContinuationInvoked`
//! error that invoking the continuation raises. Invoking it after `call/cc` has
//! returned (an upward continuation, unsupported as in C) propagates to the
//! REPL as an error rather than undefined behavior.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::{Arity, Continuation, Value};
use gc::Gc;

/// `scheme_init_fun`: type objects + procedure primitives.
pub fn init(it: &mut Interp) {
    let prim_ty = it.make_type("<primitive>");
    let clos_ty = it.make_type("<closure>");
    let cont_ty = it.make_type("<continuation>");
    it.register_value("<primitive>", Value::TypeObject(prim_ty));
    it.register_value("<closure>", Value::TypeObject(clos_ty));
    it.register_value("<continuation>", Value::TypeObject(cont_ty));

    it.register("procedure?", Arity::Exact(1), |_it, args| {
        Ok(Value::Bool(is_procedure(&args[0])))
    });

    it.register("apply", Arity::AtLeast(2), apply_prim);
    it.register("map", Arity::AtLeast(2), map_prim);
    it.register("for-each", Arity::AtLeast(2), for_each_prim);
    it.register("call-with-current-continuation", Arity::Exact(1), call_cc);
    it.register("call/cc", Arity::Exact(1), call_cc);
}

fn is_procedure(v: &Value) -> bool {
    matches!(
        v,
        Value::Prim(_) | Value::Closure(_) | Value::Continuation(_) | Value::StructProc(_)
    )
}

/// `(apply proc arg1 ... args-list)` — spread the final list argument
/// (scheme_fun.c:254).
fn apply_prim(it: &mut Interp, args: &[Value]) -> SchemeResult {
    let proc = args[0].clone();
    if !is_procedure(&proc) {
        return Err(SchemeError::msg("apply: first arg must be a procedure"));
    }
    let mut call_args: Vec<Value> = args[1..args.len() - 1].to_vec();
    let last = &args[args.len() - 1];
    let tail = last
        .list_to_vec()
        .ok_or_else(|| SchemeError::msg("apply: last arg must be a list"))?;
    call_args.extend(tail);
    it.apply(proc, &call_args)
}

/// `(map proc list1 list2 ...)` — apply across lists of equal length
/// (scheme_fun.c:289).
fn map_prim(it: &mut Interp, args: &[Value]) -> SchemeResult {
    let proc = args[0].clone();
    if !is_procedure(&proc) {
        return Err(SchemeError::msg("map: first arg must be a procedure"));
    }
    let lists = collect_lists(&args[1..], "map")?;
    let len = common_len(&lists, "map")?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let call_args: Vec<Value> = lists.iter().map(|l| l[i].clone()).collect();
        out.push(it.apply(proc.clone(), &call_args)?);
    }
    Ok(Value::list(&out))
}

/// `(for-each proc list1 list2 ...)` — like `map` but for effect; returns
/// unspecified (scheme_fun.c:345). C only supports the 2-arg form, but the
/// n-list generalization is harmless and matches `map`.
fn for_each_prim(it: &mut Interp, args: &[Value]) -> SchemeResult {
    let proc = args[0].clone();
    if !is_procedure(&proc) {
        return Err(SchemeError::msg("for-each: first arg must be a procedure"));
    }
    let lists = collect_lists(&args[1..], "for-each")?;
    let len = common_len(&lists, "for-each")?;
    for i in 0..len {
        let call_args: Vec<Value> = lists.iter().map(|l| l[i].clone()).collect();
        it.apply(proc.clone(), &call_args)?;
    }
    Ok(Value::Bool(false))
}

/// `(call/cc proc)` — escape-only continuation via the error channel.
fn call_cc(it: &mut Interp, args: &[Value]) -> SchemeResult {
    let proc = args[0].clone();
    if !is_procedure(&proc) {
        return Err(SchemeError::msg("call/cc: arg must be a procedure"));
    }
    let id = it.fresh_cont_id();
    let cont = Value::Continuation(Gc::new(Continuation { id }));
    match it.apply(proc, &[cont]) {
        Ok(v) => Ok(v),
        // Our own continuation was invoked: that value is the call/cc result.
        Err(SchemeError::ContinuationInvoked { id: got, value }) if got == id => Ok(value),
        // A different continuation, or a real error: keep unwinding.
        Err(other) => Err(other),
    }
}

/// Validate and collect a set of list arguments into Vecs.
fn collect_lists(args: &[Value], who: &str) -> SchemeResult<Vec<Vec<Value>>> {
    args.iter()
        .map(|a| {
            a.list_to_vec()
                .ok_or_else(|| SchemeError::msg(format!("{who}: arguments must be lists")))
        })
        .collect()
}

/// All lists must share a length (scheme_fun.c:303).
fn common_len(lists: &[Vec<Value>], who: &str) -> SchemeResult<usize> {
    let len = lists.first().map(|l| l.len()).unwrap_or(0);
    if lists.iter().any(|l| l.len() != len) {
        return Err(SchemeError::msg(format!("{who}: all lists must have same size")));
    }
    Ok(len)
}
