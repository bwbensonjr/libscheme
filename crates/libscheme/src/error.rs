//! Error handling and escape continuations — the Rust analogue of
//! `scheme_error.c`'s `setjmp`/`longjmp` machinery.
//!
//! In C, `scheme_signal_error` prints to stderr and `longjmp`s back to the
//! REPL-level `setjmp` (`SCHEME_CATCH_ERROR`). Escape continuations (`call/cc`)
//! `longjmp` with a stashed return value. Here both are carried in the `Result`
//! channel: every eval/apply returns [`SchemeResult`] and propagates with `?`,
//! which unwinds the native stack frame-by-frame — the safe-Rust replacement
//! for `longjmp`.

use crate::value::Value;

/// Identifies one `call/cc` invocation. Monotonic per [`crate::interp::Interp`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ContId(pub u64);

/// A user-visible error: `(error ...)` / a primitive's type or arity check.
#[derive(Clone)]
pub struct UserError {
    pub message: String,
    pub irritants: Vec<Value>,
}

/// The error channel. Two arms, mirroring the two reasons C does a `longjmp`:
/// a real error, or an escape continuation being invoked.
#[derive(Clone)]
pub enum SchemeError {
    /// A genuine error — reported at the REPL boundary (the `SCHEME_CATCH_ERROR`
    /// analogue), like `scheme_signal_error`.
    User(UserError),
    /// An escape continuation was invoked. Propagates up via `?` until the
    /// originating `call/cc` frame matches its own `id` and absorbs it. If no
    /// frame matches (an upward continuation, unsupported as in C), it reaches
    /// the REPL and is reported as an error — gracefully, without UB.
    ContinuationInvoked { id: ContId, value: Value },
}

pub type SchemeResult<T = Value> = Result<T, SchemeError>;

// Minimal `Debug` impls — `Value` has no `Debug` until the Phase 1 printer
// lands, so we summarize structurally without printing values.
impl std::fmt::Debug for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserError")
            .field("message", &self.message)
            .field("irritants", &self.irritants.len())
            .finish()
    }
}

impl std::fmt::Debug for SchemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemeError::User(e) => write!(f, "SchemeError::User({e:?})"),
            SchemeError::ContinuationInvoked { id, .. } => {
                write!(f, "SchemeError::ContinuationInvoked {{ id: {id:?}, .. }}")
            }
        }
    }
}

impl SchemeError {
    /// Build a [`SchemeError::User`] with no irritants.
    pub fn msg(message: impl Into<String>) -> SchemeError {
        SchemeError::User(UserError {
            message: message.into(),
            irritants: Vec::new(),
        })
    }

    /// Build a [`SchemeError::User`] with irritants (the offending values).
    pub fn with_irritants(message: impl Into<String>, irritants: Vec<Value>) -> SchemeError {
        SchemeError::User(UserError {
            message: message.into(),
            irritants,
        })
    }
}
