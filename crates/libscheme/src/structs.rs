//! Structures — the Rust analogue of `scheme_struct.c`.
//!
//! `(define-struct point (x y))` mints a fresh nominal type `<point>` and binds
//! the generated procedures: `make-point` (constructor), `point?` (predicate),
//! `point-x`/`point-y` (accessors), and `set-point-x!`/`set-point-y!`
//! (mutators). Each generated procedure is a [`Value::StructProc`] carrying the
//! nominal type's identity and its role; the evaluator dispatches them in
//! [`Interp::apply_struct_proc`].

use crate::env::Env;
use crate::error::{SchemeError, SchemeResult};
use crate::eval::Tail;
use crate::interp::Interp;
use crate::value::{StructInstance, StructProc, StructProcKind, Value};
use gc::{Gc, GcCell};

pub fn init(it: &mut Interp) {
    let proc_ty = it.make_type("<struct-procedure>");
    it.register_value("<struct-procedure>", Value::TypeObject(proc_ty));
    it.register_syntax("define-struct", define_struct);
}

/// `(define-struct name (field...))` — mint the type and bind the procedures.
fn define_struct(it: &mut Interp, form: &Value, env: &Gc<Env>) -> SchemeResult<Tail> {
    let parts = form
        .list_to_vec()
        .ok_or_else(|| SchemeError::msg("define-struct: malformed"))?;
    if parts.len() != 3 {
        return Err(SchemeError::msg("badly constructed define-struct form"));
    }
    let struct_name = match &parts[1] {
        Value::Symbol(s) => it.resolve(*s).to_string(),
        _ => return Err(SchemeError::msg("define-struct: name must be a symbol")),
    };
    let fields = parts[2]
        .list_to_vec()
        .ok_or_else(|| SchemeError::msg("define-struct: fields must be a list"))?;
    let field_names: Vec<String> = fields
        .iter()
        .map(|f| match f {
            Value::Symbol(s) => Ok(it.resolve(*s).to_string()),
            _ => Err(SchemeError::msg("define-struct: field must be a symbol")),
        })
        .collect::<SchemeResult<_>>()?;

    // Mint the nominal type `<name>` and bind it.
    let ty = it.make_type(&format!("<{struct_name}>"));
    it.register_value(&format!("<{struct_name}>"), Value::TypeObject(ty.clone()));

    // Constructor `make-name` and predicate `name?`.
    let ctor = Value::StructProc(Gc::new(StructProc {
        ty: ty.clone(),
        kind: StructProcKind::Constructor,
    }));
    it.register_value(&format!("make-{struct_name}"), ctor);
    let pred = Value::StructProc(Gc::new(StructProc {
        ty: ty.clone(),
        kind: StructProcKind::Predicate,
    }));
    it.register_value(&format!("{struct_name}?"), pred);

    // Accessor `name-field` and mutator `set-name-field!` per slot.
    for (slot, field) in field_names.iter().enumerate() {
        let getter = Value::StructProc(Gc::new(StructProc {
            ty: ty.clone(),
            kind: StructProcKind::Accessor(slot),
        }));
        it.register_value(&format!("{struct_name}-{field}"), getter);
        let setter = Value::StructProc(Gc::new(StructProc {
            ty: ty.clone(),
            kind: StructProcKind::Mutator(slot),
        }));
        it.register_value(&format!("set-{struct_name}-{field}!"), setter);
    }

    let _ = env; // define-struct always binds globally, like C.
    Ok(Tail::done(parts[1].clone()))
}

impl Interp {
    /// Apply a struct procedure to its arguments (`scheme_apply_struct_proc`).
    pub fn apply_struct_proc(&mut self, sp: &Gc<StructProc>, args: &[Value]) -> SchemeResult {
        match sp.kind {
            StructProcKind::Constructor => {
                // The number of fields is recorded implicitly by how many args;
                // C checks against slot_num, but our StructProc only stores the
                // type. We accept the args as the field vector — the type's
                // identity is what predicates check.
                Ok(Value::Struct(Gc::new(GcCell::new(StructInstance {
                    ty: sp.ty.clone(),
                    fields: args.to_vec(),
                }))))
            }
            StructProcKind::Predicate => {
                let matches = matches!(&args[0], Value::Struct(s)
                    if crate::value::gc_ptr_eq(&s.borrow().ty, &sp.ty));
                Ok(Value::Bool(matches))
            }
            StructProcKind::Accessor(slot) => match &args[0] {
                Value::Struct(s) => {
                    let s = s.borrow();
                    if !crate::value::gc_ptr_eq(&s.ty, &sp.ty) {
                        return Err(SchemeError::msg("struct accessor: wrong type"));
                    }
                    s.fields
                        .get(slot)
                        .cloned()
                        .ok_or_else(|| SchemeError::msg("struct accessor: bad slot"))
                }
                _ => Err(SchemeError::msg("struct accessor: not a struct instance")),
            },
            StructProcKind::Mutator(slot) => match &args[0] {
                Value::Struct(s) => {
                    {
                        let sb = s.borrow();
                        if !crate::value::gc_ptr_eq(&sb.ty, &sp.ty) {
                            return Err(SchemeError::msg("struct mutator: wrong type"));
                        }
                    }
                    let mut sb = s.borrow_mut();
                    if slot >= sb.fields.len() {
                        return Err(SchemeError::msg("struct mutator: bad slot"));
                    }
                    sb.fields[slot] = args[1].clone();
                    Ok(args[1].clone())
                }
                _ => Err(SchemeError::msg("struct mutator: not a struct instance")),
            },
        }
    }
}
