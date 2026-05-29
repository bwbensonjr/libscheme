//! Special forms — the Rust analogue of `scheme_syntax.c`.
//!
//! Phase 2 registers the irreducible core needed to exercise the evaluator:
//! `quote`, `if`, `lambda`, `define`, `set!`, and `begin`. The remaining forms
//! (`cond`, `case`, `and`, `or`, `let`/`let*`/`letrec`, `do`, `delay`,
//! `quasiquote`, `defmacro`) are added in Phase 3.
//!
//! Each handler has the [`crate::value::SyntaxHandler`] signature and returns a
//! [`Tail`] so its tail position (the chosen `if` branch, the last `begin`
//! form, a closure body) trampolines in the eval loop rather than recursing.

use crate::env::Env;
use crate::error::{SchemeError, SchemeResult};
use crate::eval::{make_closure, Tail};
use crate::interp::Interp;
use crate::value::Value;
use gc::Gc;

/// `scheme_init_syntax` (core subset): register the special forms.
pub fn init(it: &mut Interp) {
    it.register_syntax("quote", quote);
    it.register_syntax("if", if_form);
    it.register_syntax("lambda", lambda);
    it.register_syntax("define", define);
    it.register_syntax("set!", set_bang);
    it.register_syntax("begin", begin);
}

/// `(quote datum)` — return the datum unevaluated.
fn quote(_it: &mut Interp, form: &Value, _env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() != 2 {
        return Err(SchemeError::msg("quote: wrong number of parts"));
    }
    Ok(Tail::done(parts[1].clone()))
}

/// `(if test then [else])` — the chosen branch is returned as a tail.
fn if_form(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() < 3 || parts.len() > 4 {
        return Err(SchemeError::msg("if: wrong number of parts"));
    }
    let test = it.eval(parts[1].clone(), env.clone())?;
    if test.is_truthy() {
        Ok(Tail::Eval {
            expr: parts[2].clone(),
            env: env.clone(),
        })
    } else if parts.len() == 4 {
        Ok(Tail::Eval {
            expr: parts[3].clone(),
            env: env.clone(),
        })
    } else {
        // No else branch: unspecified value (C returns the false object).
        Ok(Tail::done(Value::Bool(false)))
    }
}

/// `(lambda params body...)` — capture the current environment.
fn lambda(_it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let params = form
        .cdr()
        .and_then(|d| d.car())
        .ok_or_else(|| SchemeError::msg("lambda: missing parameter list"))?;
    let body = form
        .cdr()
        .and_then(|d| d.cdr())
        .ok_or_else(|| SchemeError::msg("lambda: missing body"))?;
    if body.is_null() {
        return Err(SchemeError::msg("lambda: empty body"));
    }
    Ok(Tail::done(make_closure(env.clone(), params, body, None)))
}

/// `(define name expr)` or `(define (f . args) body...)`.
///
/// As in C (`define_syntax` always calls `scheme_add_global`, scheme_syntax.c:132),
/// a `define` evaluated at top level installs into the global table. When
/// evaluated inside a frame it binds into that frame via `set!`-style mutation
/// of an existing binding; true internal defines at a body head are handled
/// separately by the evaluator (`eval_body_tail`).
fn define(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() < 2 {
        return Err(SchemeError::msg("define: malformed"));
    }
    match &parts[1] {
        // (define name expr)
        Value::Symbol(name) => {
            let val_expr = parts.get(2).cloned().unwrap_or(Value::Bool(false));
            let val = it.eval(val_expr, env.clone())?;
            // Name closures for nicer printing.
            bind_define(it, env, *name, val);
            Ok(Tail::done(Value::Symbol(*name)))
        }
        // (define (f . args) body...) => define f as a lambda
        Value::Pair(p) => {
            let (name, lambda_args) = {
                let b = p.borrow();
                let name = match &b.car {
                    Value::Symbol(s) => *s,
                    _ => return Err(SchemeError::msg("define: bad procedure name")),
                };
                (name, b.cdr.clone())
            };
            let body = Value::list(&parts[2..]);
            let closure = make_closure(env.clone(), lambda_args, body, Some(name));
            bind_define(it, env, name, closure);
            Ok(Tail::done(Value::Symbol(name)))
        }
        _ => Err(SchemeError::msg("define: malformed")),
    }
}

/// Install a `define` binding: into the current lexical frame if one exists
/// there, otherwise into globals.
fn bind_define(it: &mut Interp, env: &Gc<Env>, name: crate::interner::Symbol, val: Value) {
    // If the name is already lexically bound, update it in place; otherwise
    // define it as a global (matching C's top-level define going to globals).
    if !env.set(name, val.clone()) {
        it.set_global(name, val);
    }
}

/// `(set! name expr)` — assign an existing variable (lexical or global).
fn set_bang(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() != 3 {
        return Err(SchemeError::msg("set!: wrong number of parts"));
    }
    let name = match &parts[1] {
        Value::Symbol(s) => *s,
        _ => return Err(SchemeError::msg("set!: first arg must be a symbol")),
    };
    let val = it.eval(parts[2].clone(), env.clone())?;
    if env.set(name, val.clone()) {
        Ok(Tail::done(Value::Bool(false)))
    } else if it.has_global(name) {
        it.set_global(name, val);
        Ok(Tail::done(Value::Bool(false)))
    } else {
        Err(SchemeError::msg(format!(
            "set!: var unbound: {}",
            it.resolve(name)
        )))
    }
}

/// `(begin form...)` — evaluate in order; the last form is the tail.
fn begin(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    let body = &parts[1..];
    if body.is_empty() {
        return Ok(Tail::done(Value::Bool(false)));
    }
    for f in &body[..body.len() - 1] {
        it.eval(f.clone(), env.clone())?;
    }
    Ok(Tail::Eval {
        expr: body[body.len() - 1].clone(),
        env: env.clone(),
    })
}

/// Flatten a special-form combination into its parts, erroring on improper lists.
fn list_parts(form: &Value) -> SchemeResult<Vec<Value>> {
    form.list_to_vec()
        .ok_or_else(|| SchemeError::msg("special form: improper list"))
}
