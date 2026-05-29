//! Special forms — the Rust analogue of `scheme_syntax.c`.
//!
//! Every handler has the [`crate::value::SyntaxHandler`] signature and returns a
//! [`Tail`] so its tail position (the chosen `if`/`cond` branch, the last
//! `begin`/body form) trampolines in the eval loop rather than recursing.
//!
//! The `let`/`let*`/`letrec`/named-`let`/`do` forms build a binding frame and
//! then defer to [`Interp::eval_body_tail`], reusing the *same* internal-define
//! hoisting the evaluator applies to closure bodies (scheme_syntax.c:316) — so
//! internal defines behave identically everywhere.

use crate::env::Env;
use crate::error::{SchemeError, SchemeResult};
use crate::eval::{make_closure, Tail};
use crate::interner::Symbol;
use crate::interp::Interp;
use crate::value::{Closure, Promise, Value};
use gc::{Gc, GcCell};

/// `scheme_init_syntax`: register the syntax/macro type objects and all forms.
pub fn init(it: &mut Interp) {
    let syntax_ty = it.make_type("<syntax>");
    let macro_ty = it.make_type("<macro>");
    it.register_value("<syntax>", Value::TypeObject(syntax_ty));
    it.register_value("<macro>", Value::TypeObject(macro_ty));

    it.register_syntax("quote", quote);
    it.register_syntax("if", if_form);
    it.register_syntax("lambda", lambda);
    it.register_syntax("define", define);
    it.register_syntax("set!", set_bang);
    it.register_syntax("begin", begin);
    it.register_syntax("cond", cond);
    it.register_syntax("case", case);
    it.register_syntax("and", and);
    it.register_syntax("or", or);
    it.register_syntax("let", let_form);
    it.register_syntax("let*", let_star);
    it.register_syntax("letrec", letrec);
    it.register_syntax("do", do_form);
    it.register_syntax("delay", delay);
    it.register_syntax("quasiquote", quasiquote);
    it.register_syntax("defmacro", defmacro);
}

// --- core forms (unchanged from Phase 2) ---

fn quote(_it: &mut Interp, form: &Value, _env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() != 2 {
        return Err(SchemeError::msg("quote: wrong number of args"));
    }
    Ok(Tail::done(parts[1].clone()))
}

fn if_form(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() < 3 || parts.len() > 4 {
        return Err(SchemeError::msg("badly formed if statement"));
    }
    let test = it.eval(parts[1].clone(), env.clone())?;
    if test.is_truthy() {
        tail_eval(parts[2].clone(), env)
    } else if parts.len() == 4 {
        tail_eval(parts[3].clone(), env)
    } else {
        Ok(Tail::done(Value::Bool(false)))
    }
}

fn lambda(_it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let params = form
        .cdr()
        .and_then(|d| d.car())
        .ok_or_else(|| SchemeError::msg("badly formed lambda"))?;
    let body = form
        .cdr()
        .and_then(|d| d.cdr())
        .ok_or_else(|| SchemeError::msg("badly formed lambda"))?;
    if body.is_null() {
        return Err(SchemeError::msg("lambda: empty body"));
    }
    Ok(Tail::done(make_closure(env.clone(), params, body, None)))
}

fn define(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() < 2 {
        return Err(SchemeError::msg("define: malformed"));
    }
    match &parts[1] {
        Value::Symbol(name) => {
            let val_expr = parts.get(2).cloned().unwrap_or(Value::Bool(false));
            let val = it.eval(val_expr, env.clone())?;
            bind_define(it, env, *name, val);
            Ok(Tail::done(Value::Symbol(*name)))
        }
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
        _ => Err(SchemeError::msg(
            "define: second arg must be symbol or list",
        )),
    }
}

fn bind_define(it: &mut Interp, env: &Gc<Env>, name: Symbol, val: Value) {
    if !env.set(name, val.clone()) {
        it.set_global(name, val);
    }
}

fn set_bang(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() != 3 {
        return Err(SchemeError::msg("bad set! form"));
    }
    let name = match &parts[1] {
        Value::Symbol(s) => *s,
        _ => return Err(SchemeError::msg("second arg to `set!' must be symbol")),
    };
    let val = it.eval(parts[2].clone(), env.clone())?;
    if env.set(name, val.clone()) || {
        // global fallback
        if it.has_global(name) {
            it.set_global(name, val.clone());
            true
        } else {
            false
        }
    } {
        // C returns the assigned value.
        Ok(Tail::done(val))
    } else {
        Err(SchemeError::msg(format!(
            "set!: var unbound: {}",
            it.resolve(name)
        )))
    }
}

fn begin(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    eval_sequence_tail(it, &parts[1..], env)
}

// --- new Phase 3 forms ---

/// `(cond (test body...) ... [(else body...)])` with `=>` support.
fn cond(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let else_sym = it.intern("else");
    let arrow_sym = it.intern("=>");
    let clauses = list_parts(form)?;
    for clause in &clauses[1..] {
        let parts = clause
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("cond: bad clause"))?;
        if parts.is_empty() {
            return Err(SchemeError::msg("cond: empty clause"));
        }
        // else => test is true
        let is_else = matches!(&parts[0], Value::Symbol(s) if *s == else_sym);
        let test_val = if is_else {
            Value::Bool(true)
        } else {
            it.eval(parts[0].clone(), env.clone())?
        };
        if test_val.is_truthy() {
            let body = &parts[1..];
            if body.is_empty() {
                // (test) with no body: yield the test value.
                return Ok(Tail::done(test_val));
            }
            // `=>`: apply the procedure to the test value.
            if let Value::Symbol(s) = &body[0] {
                if *s == arrow_sym {
                    if body.len() != 2 {
                        return Err(SchemeError::msg("cond: bad `=>' clause"));
                    }
                    let proc = it.eval(body[1].clone(), env.clone())?;
                    return it.apply_tail(proc, vec![test_val]);
                }
            }
            return eval_sequence_tail(it, body, env);
        }
    }
    Ok(Tail::done(Value::Bool(false)))
}

/// `(case key (data body...) ... [(else body...)])`, matching with `eqv?`.
fn case(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let else_sym = it.intern("else");
    let parts = list_parts(form)?;
    if parts.len() < 2 {
        return Err(SchemeError::msg("case: malformed"));
    }
    let key = it.eval(parts[1].clone(), env.clone())?;
    for clause in &parts[2..] {
        let cparts = clause
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("case: bad clause"))?;
        if cparts.is_empty() {
            return Err(SchemeError::msg("case: empty clause"));
        }
        let is_else = matches!(&cparts[0], Value::Symbol(s) if *s == else_sym);
        let matched = if is_else {
            true
        } else {
            let data = cparts[0]
                .list_to_vec()
                .ok_or_else(|| SchemeError::msg("case: first thing in clause must be a list"))?;
            data.iter().any(|d| d.eqv(&key))
        };
        if matched {
            return eval_sequence_tail(it, &cparts[1..], env);
        }
    }
    Ok(Tail::done(Value::Bool(false)))
}

/// `(and ...)` — last form is the tail; short-circuits on `#f`.
fn and(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    let forms = &parts[1..];
    if forms.is_empty() {
        return Ok(Tail::done(Value::Bool(true)));
    }
    for f in &forms[..forms.len() - 1] {
        if !it.eval(f.clone(), env.clone())?.is_truthy() {
            return Ok(Tail::done(Value::Bool(false)));
        }
    }
    tail_eval(forms[forms.len() - 1].clone(), env)
}

/// `(or ...)` — last form is the tail; short-circuits on the first truthy value.
fn or(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    let forms = &parts[1..];
    if forms.is_empty() {
        return Ok(Tail::done(Value::Bool(false)));
    }
    for f in &forms[..forms.len() - 1] {
        let v = it.eval(f.clone(), env.clone())?;
        if v.is_truthy() {
            return Ok(Tail::done(v));
        }
    }
    tail_eval(forms[forms.len() - 1].clone(), env)
}

/// `(let ((v init)...) body...)` or named `(let name ((v init)...) body...)`.
fn let_form(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() >= 2 {
        if let Value::Symbol(_) = parts[1] {
            return named_let(it, &parts, env);
        }
    }
    if parts.len() < 3 {
        return Err(SchemeError::msg("badly formed `let' form"));
    }
    let (names, init_exprs) = parse_bindings(&parts[1])?;
    // `let`: all inits evaluated in the OUTER env.
    let mut vals = Vec::with_capacity(names.len());
    for e in init_exprs {
        vals.push(it.eval(e, env.clone())?);
    }
    let frame = Env::new(names, vals, Some(env.clone()));
    it.eval_body_tail(parts[2..].to_vec(), frame)
}

/// Named let: `(let loop ((v init)...) body...)` becomes a self-recursive
/// closure bound to `loop` (scheme_syntax.c:379).
fn named_let(it: &mut Interp, parts: &[Value], env: &Gc<Env>) -> SchemeResult<Tail> {
    if parts.len() < 4 {
        return Err(SchemeError::msg("badly formed named `let' form"));
    }
    let name = match &parts[1] {
        Value::Symbol(s) => *s,
        _ => unreachable!(),
    };
    let (names, init_exprs) = parse_bindings(&parts[2])?;
    let body = Value::list(&parts[3..]);

    // Frame holding the loop binding (placeholder until the closure is built).
    let loop_frame = Env::new(vec![name], vec![Value::Bool(false)], Some(env.clone()));
    let params = Value::list(&names.iter().map(|s| Value::Symbol(*s)).collect::<Vec<_>>());
    let proc = make_closure(loop_frame.clone(), params, body, Some(name));
    loop_frame.set(name, proc.clone());

    // Evaluate the inits in the OUTER env and apply the loop procedure.
    let mut args = Vec::with_capacity(init_exprs.len());
    for e in init_exprs {
        args.push(it.eval(e, env.clone())?);
    }
    it.apply_tail(proc, args)
}

/// `(let* ((v init)...) body...)` — each init sees the previous bindings.
fn let_star(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() < 3 {
        return Err(SchemeError::msg("badly formed `let*' form"));
    }
    let (names, init_exprs) = parse_bindings(&parts[1])?;
    // Build one frame, binding each name as its init is evaluated, so later
    // inits see earlier bindings (C installs into a single growing frame).
    let mut cur = env.clone();
    for (name, e) in names.into_iter().zip(init_exprs) {
        let v = it.eval(e, cur.clone())?;
        cur = Env::new(vec![name], vec![v], Some(cur));
    }
    it.eval_body_tail(parts[2..].to_vec(), cur)
}

/// `(letrec ((v init)...) body...)` — all names visible to all inits.
fn letrec(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() < 3 {
        return Err(SchemeError::msg("badly formed `letrec' form"));
    }
    let (names, init_exprs) = parse_bindings(&parts[1])?;
    // Pre-bind all names to a placeholder, then evaluate inits in that frame.
    let placeholders = vec![Value::Bool(false); names.len()];
    let frame = Env::new(names.clone(), placeholders, Some(env.clone()));
    for (name, e) in names.iter().zip(init_exprs) {
        let v = it.eval(e, frame.clone())?;
        frame.set(*name, v);
    }
    it.eval_body_tail(parts[2..].to_vec(), frame)
}

/// `(do ((v init step)...) (test final...) body...)`.
fn do_form(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() < 3 {
        return Err(SchemeError::msg("badly formed `do' form"));
    }
    // Parse spec clauses: (var init [step]).
    let specs = parts[1]
        .list_to_vec()
        .ok_or_else(|| SchemeError::msg("do: bad variable spec"))?;
    let mut names = Vec::with_capacity(specs.len());
    let mut inits = Vec::with_capacity(specs.len());
    let mut steps: Vec<Option<Value>> = Vec::with_capacity(specs.len());
    for spec in &specs {
        let sp = spec
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("do: bad variable spec"))?;
        if sp.len() < 2 {
            return Err(SchemeError::msg("do: variable spec needs name and init"));
        }
        match &sp[0] {
            Value::Symbol(s) => names.push(*s),
            _ => return Err(SchemeError::msg("do: variable name must be a symbol")),
        }
        inits.push(sp[1].clone());
        steps.push(sp.get(2).cloned());
    }

    // Test clause: (test final...).
    let test_clause = parts[2]
        .list_to_vec()
        .ok_or_else(|| SchemeError::msg("do: bad test clause"))?;
    if test_clause.is_empty() {
        return Err(SchemeError::msg("do: empty test clause"));
    }
    let test = test_clause[0].clone();
    let finals = &test_clause[1..];
    let body = &parts[3..];

    // Bind vars to their initial values (evaluated in the outer env).
    let mut vals = Vec::with_capacity(names.len());
    for e in &inits {
        vals.push(it.eval(e.clone(), env.clone())?);
    }
    let frame = Env::new(names.clone(), vals, Some(env.clone()));

    // Iterate: while test is false, run body, then rebind via steps.
    loop {
        if it.eval(test.clone(), frame.clone())?.is_truthy() {
            // Evaluate the finals; last is the tail.
            return eval_sequence_tail(it, finals, &frame);
        }
        for f in body {
            it.eval(f.clone(), frame.clone())?;
        }
        // Evaluate all steps in the current frame, then rebind together.
        let mut new_vals = Vec::with_capacity(names.len());
        for (i, step) in steps.iter().enumerate() {
            match step {
                Some(e) => new_vals.push(it.eval(e.clone(), frame.clone())?),
                // No step: keep the current value (scheme_syntax.c:597).
                None => new_vals.push(frame.lookup(names[i]).unwrap()),
            }
        }
        for (name, v) in names.iter().zip(new_vals) {
            frame.set(*name, v);
        }
    }
}

/// `(delay expr)` — wrap `expr` in a promise capturing `env`. The thunk is a
/// zero-arg closure; `force` (Phase 5) applies it once and memoizes.
fn delay(_it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() != 2 {
        return Err(SchemeError::msg("delay: bad form"));
    }
    // body = (expr); params = () ; so applying it evaluates expr.
    let thunk = Gc::new(Closure {
        env: env.clone(),
        params: Value::Null,
        body: Value::list(&[parts[1].clone()]),
        name: None,
    });
    Ok(Tail::done(Value::Promise(Gc::new(GcCell::new(Promise {
        forced: false,
        value: None,
        thunk: Some(thunk),
    })))))
}

/// `` (quasiquote template) `` — handles nested quasiquote and unquote-splicing.
fn quasiquote(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() != 2 {
        return Err(SchemeError::msg("quasiquote(`): wrong number of args"));
    }
    Ok(Tail::done(quasi(it, &parts[1], 0, env)?))
}

/// The quasiquote walker (scheme_syntax.c:682). `level` tracks nesting depth;
/// `unquote` only evaluates at level 0.
fn quasi(it: &mut Interp, x: &Value, level: u32, env: &Gc<Env>) -> SchemeResult {
    let unquote = it.intern("unquote");
    let unquote_splicing = it.intern("unquote-splicing");
    let quasiquote_sym = it.intern("quasiquote");

    // Vectors: walk as a list, then rebuild.
    if let Value::Vector(v) = x {
        let items = v.borrow().clone();
        let as_list = Value::list(&items);
        let walked = quasi(it, &as_list, level, env)?;
        let out = walked
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("quasiquote: vector template did not yield a list"))?;
        return Ok(Value::make_vector(out));
    }

    // Non-pairs are literal.
    let Value::Pair(_) = x else {
        return Ok(x.clone());
    };

    let head = x.car().unwrap();

    // (unquote e)
    if matches!(&head, Value::Symbol(s) if *s == unquote) {
        let rest = x.cdr().unwrap();
        if !rest.is_pair() {
            return Err(SchemeError::msg("bad unquote form"));
        }
        let e = rest.car().unwrap();
        if level > 0 {
            let inner = quasi(it, &Value::list(&[e]), level - 1, env)?;
            return Ok(Value::cons(Value::Symbol(unquote), inner));
        }
        return it.eval(e, env.clone());
    }

    // ((unquote-splicing e) . rest)
    if let Value::Pair(hp) = &head {
        let hhead = hp.borrow().car.clone();
        if matches!(&hhead, Value::Symbol(s) if *s == unquote_splicing) {
            let qcdr = x.cdr().unwrap();
            let inner = head.cdr().unwrap();
            if !inner.is_pair() {
                return Err(SchemeError::msg("bad unquote-splicing form"));
            }
            if level > 0 {
                let spliced = quasi(it, &inner, level - 1, env)?;
                let with_kw = Value::cons(Value::Symbol(unquote_splicing), spliced);
                let rest = quasi(it, &qcdr, level, env)?;
                return Ok(Value::cons(with_kw, rest));
            }
            // level 0: evaluate the splice and prepend its elements.
            let spliced = it.eval(inner.car().unwrap(), env.clone())?;
            let mut items = spliced
                .list_to_vec()
                .ok_or_else(|| SchemeError::msg("unquote-splicing: not a list"))?;
            let rest = quasi(it, &qcdr, level, env)?;
            let mut acc = rest;
            items.reverse();
            for v in items {
                acc = Value::cons(v, acc);
            }
            return Ok(acc);
        }
    }

    // Otherwise recurse into car/cdr. A nested quasiquote bumps the level for
    // BOTH the car and cdr recursion, matching C's `++level` before both calls
    // (scheme_syntax.c:735).
    let new_level = if matches!(&head, Value::Symbol(s) if *s == quasiquote_sym) {
        level + 1
    } else {
        level
    };
    let qcar = quasi(it, &head, new_level, env)?;
    let qcdr = quasi(it, &x.cdr().unwrap(), new_level, env)?;
    Ok(Value::cons(qcar, qcdr))
}

/// `(defmacro name (args...) body...)` — non-standard macro definition.
/// Binds `name` to a [`Value::Macro`] whose transformer is applied to the
/// *unevaluated* operands by the evaluator, the result then re-evaluated.
fn defmacro(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = list_parts(form)?;
    if parts.len() < 4 {
        return Err(SchemeError::msg("badly formed defmacro"));
    }
    let name = match &parts[1] {
        Value::Symbol(s) => *s,
        _ => return Err(SchemeError::msg("defmacro: second arg must be a symbol")),
    };
    let params = parts[2].clone();
    let body = Value::list(&parts[3..]);
    let closure = Gc::new(Closure {
        env: env.clone(),
        params,
        body,
        name: Some(name),
    });
    let macro_val = Value::Macro(closure);
    bind_define(it, env, name, macro_val.clone());
    Ok(Tail::done(macro_val))
}

// --- helpers ---

/// Evaluate a sequence of forms; all but the last are evaluated for effect, and
/// the last is returned as a [`Tail`] so it trampolines.
fn eval_sequence_tail(it: &mut Interp, forms: &[Value], env: &Gc<Env>) -> SchemeResult<Tail> {
    if forms.is_empty() {
        return Ok(Tail::done(Value::Bool(false)));
    }
    for f in &forms[..forms.len() - 1] {
        it.eval(f.clone(), env.clone())?;
    }
    tail_eval(forms[forms.len() - 1].clone(), env)
}

fn tail_eval(expr: Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    Ok(Tail::Eval {
        expr,
        env: env.clone(),
    })
}

/// Parse a `let`-style binding list `((name init)...)` into parallel name and
/// init-expression vectors.
fn parse_bindings(bindings: &Value) -> SchemeResult<(Vec<Symbol>, Vec<Value>)> {
    let items = bindings
        .list_to_vec()
        .ok_or_else(|| SchemeError::msg("let: bad binding list"))?;
    let mut names = Vec::with_capacity(items.len());
    let mut inits = Vec::with_capacity(items.len());
    for b in items {
        let pair = b
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("let: each binding must be a list"))?;
        if pair.len() != 2 {
            return Err(SchemeError::msg("let: each binding must be (name init)"));
        }
        match &pair[0] {
            Value::Symbol(s) => names.push(*s),
            _ => return Err(SchemeError::msg("let: binding name must be a symbol")),
        }
        inits.push(pair[1].clone());
    }
    Ok((names, inits))
}

/// Flatten a special-form combination into its parts, erroring on improper lists.
fn list_parts(form: &Value) -> SchemeResult<Vec<Value>> {
    form.list_to_vec()
        .ok_or_else(|| SchemeError::msg("special form: improper list"))
}
