//! Vector primitives — the Rust analogue of `scheme_vector.c`.
//!
//! Vectors are mutable (`vector-set!`, `vector-fill!`), held in
//! `Gc<GcCell<Vec<Value>>>`. `make-vector` defaults its fill to `#f`, matching
//! C. Includes the non-standard `vector-append` the C version also exposes.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::{Arity, Value};

pub fn init(it: &mut Interp) {
    let vec_ty = it.make_type("<vector>");
    it.register_value("<vector>", Value::TypeObject(vec_ty));

    it.register("vector?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Vector(_))))
    });
    it.register("make-vector", Arity::Range(1, 2), |_it, a| {
        let len = int_arg(&a[0], "make-vector")?;
        if len < 0 {
            return Err(SchemeError::msg("make-vector: length must be non-negative"));
        }
        let fill = a.get(1).cloned().unwrap_or(Value::Bool(false));
        Ok(Value::make_vector(vec![fill; len as usize]))
    });
    it.register("vector", Arity::AtLeast(0), |_it, a| Ok(Value::make_vector(a.to_vec())));
    it.register("vector-length", Arity::Exact(1), |_it, a| match &a[0] {
        Value::Vector(v) => Ok(Value::Int(v.borrow().len() as i64)),
        _ => Err(SchemeError::msg("vector-length: arg must be a vector")),
    });
    it.register("vector-ref", Arity::Exact(2), |_it, a| match &a[0] {
        Value::Vector(v) => {
            let i = int_arg(&a[1], "vector-ref")?;
            let v = v.borrow();
            if i < 0 || i as usize >= v.len() {
                return Err(SchemeError::msg("vector-ref: index out of range"));
            }
            Ok(v[i as usize].clone())
        }
        _ => Err(SchemeError::msg("vector-ref: first arg must be a vector")),
    });
    it.register("vector-set!", Arity::Exact(3), |_it, a| match &a[0] {
        Value::Vector(v) => {
            let i = int_arg(&a[1], "vector-set!")?;
            let mut v = v.borrow_mut();
            if i < 0 || i as usize >= v.len() {
                return Err(SchemeError::msg("vector-set!: index out of range"));
            }
            v[i as usize] = a[2].clone();
            Ok(a[0].clone())
        }
        _ => Err(SchemeError::msg("vector-set!: first arg must be a vector")),
    });
    it.register("vector->list", Arity::Exact(1), |_it, a| match &a[0] {
        Value::Vector(v) => Ok(Value::list(&v.borrow())),
        _ => Err(SchemeError::msg("vector->list: arg must be a vector")),
    });
    it.register("list->vector", Arity::Exact(1), |_it, a| {
        let items = a[0]
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("list->vector: arg must be a list"))?;
        Ok(Value::make_vector(items))
    });
    it.register("vector-fill!", Arity::Exact(2), |_it, a| match &a[0] {
        Value::Vector(v) => {
            let mut v = v.borrow_mut();
            for slot in v.iter_mut() {
                *slot = a[1].clone();
            }
            Ok(a[0].clone())
        }
        _ => Err(SchemeError::msg("vector-fill!: first arg must be a vector")),
    });
    it.register("vector-append", Arity::Exact(2), |_it, a| match (&a[0], &a[1]) {
        (Value::Vector(x), Value::Vector(y)) => {
            let mut out = x.borrow().clone();
            out.extend(y.borrow().iter().cloned());
            Ok(Value::make_vector(out))
        }
        _ => Err(SchemeError::msg("vector-append: both args must be vectors")),
    });
}

fn int_arg(v: &Value, who: &str) -> SchemeResult<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        _ => Err(SchemeError::msg(format!("{who}: expected an integer"))),
    }
}
