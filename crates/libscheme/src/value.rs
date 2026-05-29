//! The Scheme value representation — the Rust analogue of the `Scheme_Object`
//! tagged union + `type` pointer from `scheme.h:60`.
//!
//! In C every value is a union plus a pointer to a singleton "type" object, and
//! type checks are pointer-identity comparisons. Here the [`Value`] enum's
//! discriminant *is* the type, so most type checks are a `match`. The one place
//! that isn't enough is dynamic nominal types — `define-struct` and extension
//! types like posix's `<stat>`/`<dir>` are minted at runtime — so those carry a
//! [`Gc`]`<`[`TypeObject`]`>` whose identity (compared with [`ptr_eq`]) plays the
//! role of the C type pointer.
//!
//! Mutable and potentially-cyclic payloads (pairs, vectors, strings, struct
//! instances, promises, ports) live behind `Gc<GcCell<..>>`: the `gc` crate's
//! tracing collector reclaims `set-car!`/`vector-set!` cycles that an `Rc` could
//! not, matching the original's reliance on the Boehm collector (`scheme.h:177`).

use crate::env::Env;
use crate::error::{ContId, SchemeResult};
use crate::interner::Symbol;
use crate::interp::Interp;
use gc::{Finalize, Gc, GcCell, Trace};
use std::any::Any;
use std::rc::Rc;

/// A Scheme value. Cheap to clone — every heap variant is a `Gc` handle.
#[derive(Clone, Trace, Finalize)]
pub enum Value {
    // --- immutable scalars (the C union's scalar arms, scheme.h:62) ---
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    Char(char),
    Eof,
    Symbol(Symbol),

    // --- mutable / cyclic aggregates: Gc<GcCell<..>> ---
    Str(Gc<GcCell<String>>),
    Pair(Gc<GcCell<Pair>>),
    Vector(Gc<GcCell<Vec<Value>>>),
    Promise(Gc<GcCell<Promise>>),
    InputPort(Gc<GcCell<InputPort>>),
    OutputPort(Gc<GcCell<OutputPort>>),
    Struct(Gc<GcCell<StructInstance>>),

    // --- immutable handles ---
    Closure(Gc<Closure>),
    Prim(Gc<Primitive>),
    Continuation(Gc<Continuation>),
    Syntax(Gc<SyntaxForm>),
    Macro(Gc<Closure>),
    TypeObject(Gc<TypeObject>),
    StructProc(Gc<StructProc>),

    // --- extension payloads (the C `void* port_data`/`close_fun` pattern) ---
    Foreign(Gc<GcCell<dyn ForeignData>>),
}

/// A cons cell (`scheme_pair_type`). `car`/`cdr` are mutable so `set-car!` /
/// `set-cdr!` work in place and can form cycles the GC will reclaim.
#[derive(Trace, Finalize)]
pub struct Pair {
    pub car: Value,
    pub cdr: Value,
}

/// A user closure (`scheme_closure_type`, scheme_fun.c:71): its defining
/// environment plus the (unevaluated) parameter list and body.
#[derive(Trace, Finalize)]
pub struct Closure {
    pub env: Gc<Env>,
    pub params: Value,
    pub body: Value,
    pub name: Option<Symbol>,
}

/// A delayed computation (`scheme_promise_type`, scheme_promise.c).
#[derive(Trace, Finalize)]
pub struct Promise {
    pub forced: bool,
    pub value: Option<Value>,
    pub thunk: Option<Gc<Closure>>,
}

/// An escape continuation (`scheme_cont_type`). Carries only its id; the actual
/// escape travels through the error channel (see [`crate::error`]).
#[derive(Trace, Finalize)]
pub struct Continuation {
    #[unsafe_ignore_trace]
    pub id: ContId,
}

/// A runtime-minted nominal type — the Rust analogue of the singleton objects
/// produced by `scheme_make_type` (scheme_type.c). Identity is the `id`.
#[derive(Trace, Finalize)]
pub struct TypeObject {
    pub name: String,
    pub id: u64,
}

/// An instance of a [`TypeObject`] (`define-struct`, scheme_struct.c:150).
#[derive(Trace, Finalize)]
pub struct StructInstance {
    pub ty: Gc<TypeObject>,
    pub fields: Vec<Value>,
}

/// One of the procedures `define-struct` generates (constructor / predicate /
/// accessor / mutator). Fleshed out in Phase 5; kept opaque for now.
#[derive(Trace, Finalize)]
pub struct StructProc {
    pub ty: Gc<TypeObject>,
    #[unsafe_ignore_trace]
    pub kind: StructProcKind,
}

#[derive(Copy, Clone, Debug)]
pub enum StructProcKind {
    Constructor,
    Predicate,
    Accessor(usize),
    Mutator(usize),
}

// --- ports (scheme_port.c) ---

/// An input port (`scheme_input_port_type`). Backed by either an in-memory
/// string (the `scheme_string_input_port_type`) or a file's contents read up
/// front. Both expose the `getc`/`ungetc`/`peek` interface the reader needs;
/// modeling files as a pre-read char buffer keeps the port `'static` and avoids
/// threading a live `File` handle through the GC.
#[derive(Trace, Finalize)]
pub struct InputPort {
    #[unsafe_ignore_trace]
    pub chars: Vec<char>,
    pub index: usize,
    pub open: bool,
}

impl InputPort {
    /// Read the next character, advancing the cursor. `None` at end of input.
    pub fn getc(&mut self) -> Option<char> {
        let c = self.chars.get(self.index).copied();
        if c.is_some() {
            self.index += 1;
        }
        c
    }
    /// Put the last character back (one position of lookahead), like C's ungetc.
    pub fn ungetc(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        }
    }
    /// Peek at the next character without consuming it.
    pub fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }
    /// `char-ready?`: more input is buffered.
    pub fn char_ready(&self) -> bool {
        self.index < self.chars.len()
    }
}

/// An output port (`scheme_output_port_type`). Either a real file/stdout/stderr
/// sink or an in-memory accumulator (for the string-port extensions).
#[derive(Trace, Finalize)]
pub struct OutputPort {
    #[unsafe_ignore_trace]
    pub sink: OutputSink,
    pub open: bool,
}

/// Where an output port's bytes go.
pub enum OutputSink {
    Stdout,
    Stderr,
    File(std::fs::File),
    /// In-memory accumulator (string output port).
    Buffer(String),
}

impl OutputSink {
    /// Append `s` to the sink (the C `write_string_fun`).
    pub fn write_str(&mut self, s: &str) -> std::io::Result<()> {
        use std::io::Write;
        match self {
            OutputSink::Stdout => {
                print!("{s}");
                std::io::stdout().flush()
            }
            OutputSink::Stderr => {
                eprint!("{s}");
                std::io::stderr().flush()
            }
            OutputSink::File(f) => f.write_all(s.as_bytes()),
            OutputSink::Buffer(buf) => {
                buf.push_str(s);
                Ok(())
            }
        }
    }
}

// --- primitives & syntax: the extension API surface ---

/// The signature every primitive (built-in or user extension) implements — the
/// Rust analogue of `Scheme_Prim` (scheme.h:119). Takes `&mut Interp` because
/// primitives need the interner, the type-minter, and eval/apply.
pub type PrimFn = dyn Fn(&mut Interp, &[Value]) -> SchemeResult;

/// A primitive procedure (`scheme_prim_type`). Arity is checked centrally by
/// the interpreter before `f` runs, replacing C's scattered `SCHEME_ASSERT`s.
#[derive(Trace, Finalize)]
pub struct Primitive {
    pub name: Symbol,
    pub arity: Arity,
    #[unsafe_ignore_trace]
    pub f: Rc<PrimFn>,
}

/// Expected argument count for a primitive.
#[derive(Copy, Clone, Debug)]
pub enum Arity {
    Exact(usize),
    AtLeast(usize),
    Range(usize, usize),
}

// `Arity` is `Copy` and holds no `Gc` pointers; empty manual `Trace`.
impl Finalize for Arity {}
unsafe impl Trace for Arity {
    gc::unsafe_empty_trace!();
}

impl Arity {
    pub fn accepts(&self, n: usize) -> bool {
        match *self {
            Arity::Exact(k) => n == k,
            Arity::AtLeast(k) => n >= k,
            Arity::Range(lo, hi) => n >= lo && n <= hi,
        }
    }
}

/// The handler for a special form (`scheme_syntax_type`). Receives the whole
/// form (including the keyword) and the current environment, like the C
/// `Scheme_Object* fn(Scheme_Object* form, Scheme_Env* env)` (scheme.h).
///
/// Returns a [`crate::eval::Tail`] rather than a bare value so that forms with a
/// tail position (`if`, `begin`, `cond`, …) can hand their final expression back
/// to the eval loop to be evaluated *in place*, giving proper tail calls.
pub type SyntaxHandler = fn(&mut Interp, &Value, &Gc<Env>) -> SchemeResult<crate::eval::Tail>;

/// A special form (`scheme_syntax_type`). Populated in Phase 3.
#[derive(Trace, Finalize)]
pub struct SyntaxForm {
    pub name: Symbol,
    #[unsafe_ignore_trace]
    pub handler: SyntaxHandler,
}

/// Opaque, downcastable payload for extension types (e.g. posix `DIR*`/`stat`).
/// The C `void*` port/foreign data becomes a trait object collected by the GC.
pub trait ForeignData: Trace + Finalize + Any {
    fn as_any(&self) -> &dyn Any;
}

impl Value {
    // --- constructors ---

    pub fn make_string(s: impl Into<String>) -> Value {
        Value::Str(Gc::new(GcCell::new(s.into())))
    }

    pub fn cons(car: Value, cdr: Value) -> Value {
        Value::Pair(Gc::new(GcCell::new(Pair { car, cdr })))
    }

    pub fn make_vector(items: Vec<Value>) -> Value {
        Value::Vector(Gc::new(GcCell::new(items)))
    }

    /// An input port over the given characters (a string or file's contents).
    pub fn make_input_port(chars: Vec<char>) -> Value {
        Value::InputPort(Gc::new(GcCell::new(InputPort {
            chars,
            index: 0,
            open: true,
        })))
    }

    /// An output port over the given sink.
    pub fn make_output_port(sink: OutputSink) -> Value {
        Value::OutputPort(Gc::new(GcCell::new(OutputPort { sink, open: true })))
    }

    /// Build a proper list from a slice (the `()`-terminated chain).
    pub fn list(items: &[Value]) -> Value {
        let mut acc = Value::Null;
        for v in items.iter().rev() {
            acc = Value::cons(v.clone(), acc);
        }
        acc
    }

    /// `car`, if this is a pair.
    pub fn car(&self) -> Option<Value> {
        match self {
            Value::Pair(p) => Some(p.borrow().car.clone()),
            _ => None,
        }
    }

    /// `cdr`, if this is a pair.
    pub fn cdr(&self) -> Option<Value> {
        match self {
            Value::Pair(p) => Some(p.borrow().cdr.clone()),
            _ => None,
        }
    }

    /// Collect a proper list into a `Vec`. Returns `None` if this value is not a
    /// proper list (i.e. it ends in a non-null, non-pair tail).
    pub fn list_to_vec(&self) -> Option<Vec<Value>> {
        let mut out = Vec::new();
        let mut cur = self.clone();
        loop {
            match &cur {
                Value::Null => return Some(out),
                Value::Pair(p) => {
                    let (car, cdr) = {
                        let b = p.borrow();
                        (b.car.clone(), b.cdr.clone())
                    };
                    out.push(car);
                    cur = cdr;
                }
                _ => return None,
            }
        }
    }

    /// Length of a proper list, or `None` if improper.
    pub fn list_len(&self) -> Option<usize> {
        self.list_to_vec().map(|v| v.len())
    }

    // --- predicates mirroring the SCHEME_*P macros ---

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_pair(&self) -> bool {
        matches!(self, Value::Pair(_))
    }

    pub fn is_truthy(&self) -> bool {
        // Only #f is false in Scheme (scheme_bool.c).
        !matches!(self, Value::Bool(false))
    }

    // --- equivalence predicates (scheme.h:285) ---

    /// `eq?` — identity. Scalars compare by value; heap values by pointer.
    /// Named to mirror Scheme's `eq?`; not the [`PartialEq`] method.
    #[allow(clippy::should_implement_trait)]
    pub fn eq(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Null, Null) | (Eof, Eof) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Char(a), Char(b)) => a == b,
            (Symbol(a), Symbol(b)) => a == b,
            (Str(a), Str(b)) => gc_ptr_eq(a, b),
            (Pair(a), Pair(b)) => gc_ptr_eq(a, b),
            (Vector(a), Vector(b)) => gc_ptr_eq(a, b),
            (Promise(a), Promise(b)) => gc_ptr_eq(a, b),
            (InputPort(a), InputPort(b)) => gc_ptr_eq(a, b),
            (OutputPort(a), OutputPort(b)) => gc_ptr_eq(a, b),
            (Struct(a), Struct(b)) => gc_ptr_eq(a, b),
            (Closure(a), Closure(b)) => gc_ptr_eq(a, b),
            (Prim(a), Prim(b)) => gc_ptr_eq(a, b),
            (Continuation(a), Continuation(b)) => gc_ptr_eq(a, b),
            (Syntax(a), Syntax(b)) => gc_ptr_eq(a, b),
            (Macro(a), Macro(b)) => gc_ptr_eq(a, b),
            (TypeObject(a), TypeObject(b)) => gc_ptr_eq(a, b),
            (StructProc(a), StructProc(b)) => gc_ptr_eq(a, b),
            (Foreign(a), Foreign(b)) => gc_ptr_eq(a, b),
            _ => false,
        }
    }

    /// `eqv?` — like `eq?` but numbers and chars compare by value.
    pub fn eqv(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Double(a), Double(b)) => a == b,
            // Int/Char already compare by value in `eq`.
            _ => self.eq(other),
        }
    }

    /// `equal?` — recursive structural equivalence.
    pub fn equal(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Str(a), Str(b)) => *a.borrow() == *b.borrow(),
            (Pair(a), Pair(b)) => {
                let (pa, pb) = (a.borrow(), b.borrow());
                pa.car.equal(&pb.car) && pa.cdr.equal(&pb.cdr)
            }
            (Vector(a), Vector(b)) => {
                let (va, vb) = (a.borrow(), b.borrow());
                va.len() == vb.len() && va.iter().zip(vb.iter()).all(|(x, y)| x.equal(y))
            }
            _ => self.eqv(other),
        }
    }
}

/// Pointer-identity for two `Gc` handles — the Rust analogue of comparing two
/// `Scheme_Object*`. Compares the address of the referent.
pub fn gc_ptr_eq<T: Trace + ?Sized>(a: &Gc<T>, b: &Gc<T>) -> bool {
    std::ptr::eq(a.as_ref() as *const T as *const (), b.as_ref() as *const T as *const ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_on_scalars() {
        assert!(Value::Int(3).eq(&Value::Int(3)));
        assert!(!Value::Int(3).eq(&Value::Int(4)));
        assert!(Value::Null.eq(&Value::Null));
        assert!(Value::Bool(true).eq(&Value::Bool(true)));
    }

    #[test]
    fn eqv_distinguishes_doubles_from_eq() {
        assert!(Value::Double(1.5).eqv(&Value::Double(1.5)));
    }

    #[test]
    fn equal_is_structural() {
        let a = Value::list(&[Value::Int(1), Value::make_string("x")]);
        let b = Value::list(&[Value::Int(1), Value::make_string("x")]);
        assert!(!a.eq(&b), "distinct pairs are not eq?");
        assert!(a.equal(&b), "structurally identical lists are equal?");
    }

    #[test]
    fn pair_pointer_identity() {
        let p = Value::cons(Value::Int(1), Value::Null);
        let q = p.clone();
        assert!(p.eq(&q), "a clone shares the same Gc pointer");
    }
}
