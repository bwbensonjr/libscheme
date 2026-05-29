//! The evaluator and applicator — the Rust analogue of `scheme_eval.c` and the
//! `scheme_apply` core of `scheme_fun.c`.
//!
//! The C version is a plain recursive tree-walker with no tail-call
//! optimization, so deep recursion overflows the C stack. A naive Rust port
//! would overflow *sooner* (heavier frames) and, worse, a Rust stack overflow
//! aborts uncatchably. So this port adds one behavior-preserving improvement: a
//! **trampoline**. [`Interp::eval`] runs a loop; anything in tail position is
//! returned as [`Tail::Eval`] and re-driven by the loop instead of recursing.
//! Closure bodies and the tail branches of `if`/`begin`/`cond`/… therefore run
//! in constant native stack depth — proper tail calls — without changing any
//! observable result.
//!
//! Errors and escape continuations both travel the `Result` channel
//! ([`crate::error`]): a primitive signalling an error returns `Err(User..)`,
//! and invoking a continuation returns `Err(ContinuationInvoked..)` that unwinds
//! until the matching `call/cc` frame catches it.
//!
//! Hot-path notes (see the Performance section of `RUST_PORT_FEASIBILITY.md`):
//! `eval_combination` walks the operand pair-chain straight into the `args`
//! vector rather than first collecting the unevaluated operands with
//! `list_to_vec` — that intermediate allocation + per-operand clone was the
//! single largest avoidable cost. Relatedly, [`Closure`] caches its parsed
//! parameter names and body forms (computed once in [`build_closure`]), so
//! `closure_frame` is just an arity check and a frame allocation.

use crate::env::Env;
use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::{Arity, Closure, Value};
use gc::Gc;

/// The result of one evaluation step. Either we have a final value, or we have
/// an expression to evaluate in a given environment — the latter lets the eval
/// loop continue in tail position without growing the native stack.
pub enum Tail {
    Done(Value),
    Eval { expr: Value, env: Gc<Env> },
}

impl Tail {
    /// Convenience: a finished value.
    pub fn done(v: Value) -> Tail {
        Tail::Done(v)
    }
}

impl Interp {
    /// Evaluate `expr` in `env` (the `scheme_eval` entry point), trampolining on
    /// tail positions so tail calls do not grow the stack.
    pub fn eval(&mut self, expr: Value, env: Gc<Env>) -> SchemeResult {
        let mut expr = expr;
        let mut env = env;
        loop {
            match self.eval_step(&expr, &env)? {
                Tail::Done(v) => return Ok(v),
                Tail::Eval { expr: e, env: n } => {
                    expr = e;
                    env = n;
                }
            }
        }
    }

    /// One non-looping evaluation step. Self-evaluating data and variable
    /// references finish immediately; combinations dispatch to syntax, macros,
    /// or application (`scheme_eval_combination`).
    fn eval_step(&mut self, expr: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
        match expr {
            // Variable reference: lexical chain, then globals (scheme_eval.c:42).
            Value::Symbol(sym) => match env.lookup(*sym).or_else(|| self.lookup_global(*sym)) {
                Some(v) => Ok(Tail::Done(v)),
                None => Err(SchemeError::msg(format!(
                    "reference to unbound symbol: {}",
                    self.resolve(*sym)
                ))),
            },
            // Combination.
            Value::Pair(_) => self.eval_combination(expr, env),
            // Everything else is self-evaluating.
            _ => Ok(Tail::Done(expr.clone())),
        }
    }

    /// Evaluate a combination `(rator . rands)` (`scheme_eval_combination`).
    fn eval_combination(&mut self, comb: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
        let head = comb.car().expect("combination is a pair");
        let rands = comb.cdr().expect("combination is a pair");

        // A symbol in operator position that names a syntax/macro binding is
        // special-cased WITHOUT evaluating it as a variable, matching how C
        // evaluates the operator and dispatches on its type: a syntax or macro
        // object short-circuits before argument evaluation.
        let rator = if let Value::Symbol(s) = &head {
            // Look up; if it resolves to syntax/macro, dispatch on that.
            env.lookup(*s).or_else(|| self.lookup_global(*s))
        } else {
            None
        };

        // If head wasn't a bound symbol resolving to something, evaluate it.
        let rator = match rator {
            Some(v) => v,
            None => self.eval(head, env.clone())?,
        };

        match &rator {
            // Special form: hand the whole combination to the handler, which
            // returns a Tail so its tail position trampolines here.
            Value::Syntax(form) => (form.handler)(self, comb, env),
            // defmacro macro: expand by applying the transformer to the
            // *unevaluated* operands, then evaluate the result in tail position.
            Value::Macro(closure) => {
                let operands = rands.list_to_vec().ok_or_else(|| {
                    SchemeError::msg("eval: macro call operands must be a proper list")
                })?;
                let expanded = self.apply_closure(closure.clone(), &operands)?;
                Ok(Tail::Eval {
                    expr: expanded,
                    env: env.clone(),
                })
            }
            // Ordinary application: evaluate operands left-to-right, then apply.
            // Walk the operand pair-chain directly rather than materializing an
            // intermediate Vec of unevaluated operands (this combination runs on
            // every call, so the extra allocation + per-operand clone showed up
            // in profiles).
            _ => {
                let mut args = Vec::new();
                let mut cur = rands;
                while let Value::Pair(p) = &cur {
                    let (car, cdr) = {
                        let b = p.borrow();
                        (b.car.clone(), b.cdr.clone())
                    };
                    args.push(self.eval(car, env.clone())?);
                    cur = cdr;
                }
                if !matches!(cur, Value::Null) {
                    return Err(SchemeError::msg(
                        "eval: combination operands must be a proper list",
                    ));
                }
                self.apply_tail(rator, args)
            }
        }
    }

    /// Apply `rator` to already-evaluated `args`, returning a [`Tail`] so a
    /// closure's final body form trampolines instead of recursing. This is the
    /// tail-call seam — the C `scheme_apply`, split so the body's last form is
    /// returned as `Tail::Eval`.
    pub fn apply_tail(&mut self, rator: Value, args: Vec<Value>) -> SchemeResult<Tail> {
        match &rator {
            Value::Prim(p) => {
                if !p.arity.accepts(args.len()) {
                    return Err(SchemeError::msg(format!(
                        "{}: wrong number of arguments ({} given)",
                        self.resolve(p.name),
                        args.len()
                    )));
                }
                let f = p.f.clone();
                Ok(Tail::Done(f(self, &args)?))
            }
            Value::Closure(c) => {
                let (frame, body) = self.closure_frame(c.clone(), &args)?;
                // Evaluate all but the last body form; return the last as a tail.
                self.eval_body_tail(body, frame)
            }
            Value::Continuation(k) => {
                // Invoking a continuation: escape via the error channel
                // (scheme_fun.c:198). Exactly one argument, like C.
                if args.len() != 1 {
                    return Err(SchemeError::msg(
                        "continuation: wrong number of args (expected 1)",
                    ));
                }
                Err(SchemeError::ContinuationInvoked {
                    id: k.id,
                    value: args.into_iter().next().unwrap(),
                })
            }
            Value::StructProc(sp) => Ok(Tail::Done(self.apply_struct_proc(sp, &args)?)),
            _ => Err(SchemeError::msg("apply: bad procedure")),
        }
    }

    /// Non-tail apply — used by primitives like `map`/`apply`/`force` that need
    /// a concrete value, not a trampoline step.
    pub fn apply(&mut self, rator: Value, args: &[Value]) -> SchemeResult {
        match self.apply_tail(rator, args.to_vec())? {
            Tail::Done(v) => Ok(v),
            Tail::Eval { expr, env } => self.eval(expr, env),
        }
    }

    /// Apply a closure to args and fully evaluate its body (non-tail). Used when
    /// a concrete value is required (e.g. macro expansion).
    pub fn apply_closure(&mut self, c: Gc<Closure>, args: &[Value]) -> SchemeResult {
        let (frame, body) = self.closure_frame(c, args)?;
        match self.eval_body_tail(body, frame)? {
            Tail::Done(v) => Ok(v),
            Tail::Eval { expr, env } => self.eval(expr, env),
        }
    }

    /// Bind a closure's parameters to `args`, producing the call frame and the
    /// body forms. The parameter shape and body were parsed once at
    /// construction (`make_closure`); this is only the per-call arity check and
    /// frame allocation (the binding half of `scheme_apply`, scheme_fun.c:106).
    fn closure_frame(
        &mut self,
        c: Gc<Closure>,
        args: &[Value],
    ) -> SchemeResult<(Gc<Env>, Vec<Value>)> {
        let fixed = c.param_names.len();
        if let Some(rest_sym) = c.rest_param {
            if args.len() < fixed {
                return Err(SchemeError::msg("too few arguments to procedure"));
            }
            let mut names = c.param_names.clone();
            let mut vals: Vec<Value> = args[..fixed].to_vec();
            names.push(rest_sym);
            vals.push(Value::list(&args[fixed..]));
            let frame = Env::new(names, vals, Some(c.env.clone()));
            Ok((frame, c.body_forms.clone()))
        } else {
            if args.len() < fixed {
                return Err(SchemeError::msg("too few arguments to procedure"));
            }
            if args.len() > fixed {
                return Err(SchemeError::msg("too many arguments to procedure"));
            }
            let frame = Env::new(c.param_names.clone(), args.to_vec(), Some(c.env.clone()));
            Ok((frame, c.body_forms.clone()))
        }
    }

    /// Evaluate a body (a list of forms) in `env`, returning its last form as a
    /// [`Tail`] so it trampolines. Internal defines at the head are hoisted into
    /// a fresh frame extending `env` (the letrec semantics of scheme_fun.c:174).
    ///
    /// Exposed to the crate so the `let`/`let*`/`letrec`/`do` special forms can
    /// reuse the exact same internal-define handling for their bodies
    /// (scheme_syntax.c:316), rather than re-implementing it.
    pub(crate) fn eval_body_tail(&mut self, forms: Vec<Value>, env: Gc<Env>) -> SchemeResult<Tail> {
        // Split leading internal defines.
        let define_sym = self.sym_define();
        let lambda_sym = self.sym_lambda();
        let mut idx = 0;
        let mut def_names: Vec<crate::interner::Symbol> = Vec::new();
        let mut def_exprs: Vec<Value> = Vec::new();
        while idx < forms.len() {
            let form = &forms[idx];
            let is_define = matches!(form.car(), Some(Value::Symbol(s)) if s == define_sym);
            if !is_define {
                break;
            }
            let (name, val_expr) = self.parse_internal_define(form, lambda_sym)?;
            def_names.push(name);
            def_exprs.push(val_expr);
            idx += 1;
        }

        let body_env = if def_names.is_empty() {
            env
        } else {
            // letrec frame: pre-bind names to a placeholder, then assign.
            let placeholders = vec![Value::Bool(false); def_names.len()];
            let frame = Env::new(def_names.clone(), placeholders, Some(env));
            for (name, expr) in def_names.iter().zip(def_exprs) {
                let v = self.eval(expr, frame.clone())?;
                frame.set(*name, v);
            }
            frame
        };

        // Evaluate the residual body; last form is the tail.
        let rest = &forms[idx..];
        if rest.is_empty() {
            // A body of only internal defines yields an unspecified value.
            return Ok(Tail::Done(Value::Null));
        }
        for f in &rest[..rest.len() - 1] {
            self.eval(f.clone(), body_env.clone())?;
        }
        Ok(Tail::Eval {
            expr: rest[rest.len() - 1].clone(),
            env: body_env,
        })
    }

    /// Parse one internal `(define ...)` into (name, value-expression),
    /// desugaring the `(define (f args) body)` procedure form into a lambda
    /// (scheme_fun.c:146-164).
    fn parse_internal_define(
        &mut self,
        form: &Value,
        lambda_sym: crate::interner::Symbol,
    ) -> SchemeResult<(crate::interner::Symbol, Value)> {
        let parts = form
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("define: malformed"))?;
        if parts.len() < 2 {
            return Err(SchemeError::msg("define: malformed"));
        }
        match &parts[1] {
            // (define name expr)
            Value::Symbol(name) => {
                let val = parts.get(2).cloned().unwrap_or(Value::Bool(false));
                Ok((*name, val))
            }
            // (define (f . args) body...) => (define f (lambda args body...))
            Value::Pair(p) => {
                let header = p.borrow();
                let name = match &header.car {
                    Value::Symbol(s) => *s,
                    _ => return Err(SchemeError::msg("define: bad procedure name")),
                };
                let lambda_args = header.cdr.clone();
                // (lambda lambda_args body...)
                let mut lam = vec![Value::Symbol(lambda_sym), lambda_args];
                lam.extend_from_slice(&parts[2..]);
                Ok((name, Value::list(&lam)))
            }
            _ => Err(SchemeError::msg("define: malformed")),
        }
    }
}

/// Parse a lambda parameter list into (fixed names, optional rest name).
/// Accepts `(a b c)`, `(a b . rest)`, and a bare symbol `rest`
/// (scheme_fun.c:111-122 + the improper-tail rest case).
fn parse_params(
    params: &Value,
) -> SchemeResult<(
    Vec<crate::interner::Symbol>,
    Option<crate::interner::Symbol>,
)> {
    let mut names = Vec::new();
    let mut cur = params.clone();
    loop {
        match &cur {
            Value::Null => return Ok((names, None)),
            Value::Symbol(s) => return Ok((names, Some(*s))),
            Value::Pair(p) => {
                let (car, cdr) = {
                    let b = p.borrow();
                    (b.car.clone(), b.cdr.clone())
                };
                match car {
                    Value::Symbol(s) => names.push(s),
                    _ => return Err(SchemeError::msg("lambda: parameter names must be symbols")),
                }
                cur = cdr;
            }
            _ => return Err(SchemeError::msg("lambda: malformed parameter list")),
        }
    }
}

/// `scheme_init_eval`: registers the `eval` primitive. The single-arg `eval`
/// evaluates in the global environment, like C (which uses `scheme_env`).
pub fn init(it: &mut Interp) {
    it.register("eval", Arity::Exact(1), |it, args| {
        it.eval(args[0].clone(), Env::root())
    });
}

/// Build a [`Closure`] from a parameter list, body forms, and a defining
/// environment, parsing and validating both once. Returns the `Gc<Closure>` so
/// callers can wrap it as a procedure ([`make_closure`]), a `delay` thunk, or a
/// `defmacro` transformer (`Value::Macro`).
pub fn build_closure(
    env: Gc<Env>,
    params: Value,
    body: Value,
    name: Option<crate::interner::Symbol>,
) -> SchemeResult<Gc<Closure>> {
    let (param_names, rest_param) = parse_params(&params)?;
    let body_forms = body
        .list_to_vec()
        .ok_or_else(|| SchemeError::msg("closure body must be a proper list"))?;
    if body_forms.is_empty() {
        return Err(SchemeError::msg("closure body has no forms"));
    }
    Ok(Gc::new(Closure {
        env,
        param_names,
        rest_param,
        body_forms,
        name,
    }))
}

/// Construct a closure *value* from a parameter list, body forms, and a defining
/// environment. Shared by `lambda` and `define`'s procedure form. The parameter
/// list and body are parsed and validated here, once, so that applying the
/// closure later costs only an arity check and a frame allocation.
pub fn make_closure(
    env: Gc<Env>,
    params: Value,
    body: Value,
    name: Option<crate::interner::Symbol>,
) -> SchemeResult {
    Ok(Value::Closure(build_closure(env, params, body, name)?))
}
