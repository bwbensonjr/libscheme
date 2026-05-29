//! I/O ports — the Rust analogue of `scheme_port.c`.
//!
//! Input ports are backed by a char buffer (a string, or a file read up front);
//! output ports by an [`OutputSink`] (stdout/stderr/file/in-memory buffer). The
//! current input/output ports live on [`Interp`] (`cur_in`/`cur_out`,
//! scheme_port.c:48). `read` drives the [`Reader`] over a port's remaining
//! characters and advances the port's cursor by however much it consumed.
//!
//! `with-input-from-*` / `with-output-to-*` swap the current port for the
//! dynamic extent of a thunk and restore it afterward — restored even if the
//! thunk errors or escapes via a continuation, so the swap is exception-safe.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::printer::{display_to_string, write_to_string};
use crate::reader::Reader;
use crate::value::{Arity, OutputSink, Value};
use std::fs;

pub fn init(it: &mut Interp) {
    let eof_ty = it.make_type("<eof>");
    let in_ty = it.make_type("<input-port>");
    let out_ty = it.make_type("<output-port>");
    it.register_value("<eof>", Value::TypeObject(eof_ty));
    it.register_value("<input-port>", Value::TypeObject(in_ty));
    it.register_value("<output-port>", Value::TypeObject(out_ty));

    // Establish the standard current ports (scheme_port.c:95).
    let stdin_port = Value::make_input_port(Vec::new());
    let stdout_port = Value::make_output_port(OutputSink::Stdout);
    it.set_cur_in(stdin_port);
    it.set_cur_out(stdout_port);

    it.register("input-port?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::InputPort(_))))
    });
    it.register("output-port?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::OutputPort(_))))
    });
    it.register("eof-object?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Eof)))
    });
    it.register("current-input-port", Arity::Exact(0), |it, _a| {
        Ok(it.cur_in().expect("cur_in set"))
    });
    it.register("current-output-port", Arity::Exact(0), |it, _a| {
        Ok(it.cur_out().expect("cur_out set"))
    });

    // opening ports.
    it.register("open-input-file", Arity::Exact(1), |_it, a| {
        let name = str_arg(&a[0], "open-input-file")?;
        let contents = fs::read_to_string(&name)
            .map_err(|e| SchemeError::msg(format!("Cannot open input file {name}: {e}")))?;
        Ok(Value::make_input_port(contents.chars().collect()))
    });
    it.register("open-input-string", Arity::Exact(1), |_it, a| {
        let s = str_arg(&a[0], "open-input-string")?;
        Ok(Value::make_input_port(s.chars().collect()))
    });
    it.register("open-output-file", Arity::Exact(1), |_it, a| {
        let name = str_arg(&a[0], "open-output-file")?;
        let f = fs::File::create(&name)
            .map_err(|e| SchemeError::msg(format!("Cannot open output file {name}: {e}")))?;
        Ok(Value::make_output_port(OutputSink::File(f)))
    });
    it.register("close-input-port", Arity::Exact(1), |_it, a| match &a[0] {
        Value::InputPort(p) => {
            p.borrow_mut().open = false;
            Ok(Value::Bool(true))
        }
        _ => Err(SchemeError::msg("close-input-port: arg must be an input port")),
    });
    it.register("close-output-port", Arity::Exact(1), |_it, a| match &a[0] {
        Value::OutputPort(p) => {
            p.borrow_mut().open = false;
            Ok(Value::Bool(true))
        }
        _ => Err(SchemeError::msg("close-output-port: arg must be an output port")),
    });

    // call-with-* : open, apply the proc to the port, close.
    it.register("call-with-input-file", Arity::Exact(2), |it, a| {
        let name = str_arg(&a[0], "call-with-input-file")?;
        let contents = fs::read_to_string(&name)
            .map_err(|e| SchemeError::msg(format!("cannot open file for input: {name}: {e}")))?;
        let port = Value::make_input_port(contents.chars().collect());
        it.apply(a[1].clone(), &[port])
    });
    it.register("call-with-output-file", Arity::Exact(2), |it, a| {
        let name = str_arg(&a[0], "call-with-output-file")?;
        let f = fs::File::create(&name)
            .map_err(|e| SchemeError::msg(format!("cannot open file for output: {name}: {e}")))?;
        let port = Value::make_output_port(OutputSink::File(f));
        it.apply(a[1].clone(), &[port])
    });

    // with-*: rebind the current port for the thunk's dynamic extent.
    it.register("with-input-from-string", Arity::Exact(2), |it, a| {
        let s = str_arg(&a[0], "with-input-from-string")?;
        let port = Value::make_input_port(s.chars().collect());
        with_swapped(it, true, port, a[1].clone())
    });
    it.register("with-input-from-file", Arity::Exact(2), |it, a| {
        let name = str_arg(&a[0], "with-input-from-file")?;
        let contents = fs::read_to_string(&name)
            .map_err(|e| SchemeError::msg(format!("cannot open file for input: {name}: {e}")))?;
        let port = Value::make_input_port(contents.chars().collect());
        with_swapped(it, true, port, a[1].clone())
    });
    it.register("with-output-to-file", Arity::Exact(2), |it, a| {
        let name = str_arg(&a[0], "with-output-to-file")?;
        let f = fs::File::create(&name)
            .map_err(|e| SchemeError::msg(format!("cannot open file for output: {name}: {e}")))?;
        let port = Value::make_output_port(OutputSink::File(f));
        with_swapped(it, false, port, a[1].clone())
    });

    // reading.
    it.register("read", Arity::Range(0, 1), read_prim);
    it.register("read-char", Arity::Range(0, 1), |it, a| {
        let port = in_port(it, a, "read-char")?;
        match &port {
            Value::InputPort(p) => Ok(p.borrow_mut().getc().map(Value::Char).unwrap_or(Value::Eof)),
            _ => unreachable!(),
        }
    });
    it.register("peek-char", Arity::Range(0, 1), |it, a| {
        let port = in_port(it, a, "peek-char")?;
        match &port {
            Value::InputPort(p) => Ok(p.borrow().peek().map(Value::Char).unwrap_or(Value::Eof)),
            _ => unreachable!(),
        }
    });
    it.register("char-ready?", Arity::Range(0, 1), |it, a| {
        let port = in_port(it, a, "char-ready?")?;
        match &port {
            Value::InputPort(p) => Ok(Value::Bool(p.borrow().char_ready())),
            _ => unreachable!(),
        }
    });

    // writing.
    it.register("write", Arity::Range(1, 2), |it, a| out_write(it, a, true));
    it.register("display", Arity::Range(1, 2), |it, a| out_write(it, a, false));
    it.register("newline", Arity::Range(0, 1), |it, a| {
        let port = out_port(it, a, 0, "newline")?;
        write_to_port(&port, "\n")?;
        Ok(Value::Bool(true))
    });
    it.register("write-char", Arity::Range(1, 2), |it, a| {
        let c = match &a[0] {
            Value::Char(c) => *c,
            _ => return Err(SchemeError::msg("write-char: first arg must be a character")),
        };
        let port = out_port(it, a, 1, "write-char")?;
        write_to_port(&port, &c.to_string())?;
        Ok(Value::Bool(true))
    });

    // string conversion (non-standard, exposed by C).
    it.register("write-to-string", Arity::Exact(1), |it, a| {
        Ok(Value::make_string(write_to_string(it, &a[0])))
    });
    it.register("display-to-string", Arity::Exact(1), |it, a| {
        Ok(Value::make_string(display_to_string(it, &a[0])))
    });

    // load: read and evaluate every form in a file (scheme_port.c:722).
    it.register("load", Arity::Exact(1), |it, a| {
        let name = str_arg(&a[0], "load")?;
        let contents = fs::read_to_string(&name)
            .map_err(|e| SchemeError::msg(format!("load: could not open file: {name}: {e}")))?;
        let mut reader = Reader::new(&contents);
        let forms = reader.read_all(it)?;
        let mut last = Value::Bool(false);
        for f in forms {
            last = it.eval(f, crate::env::Env::root())?;
        }
        Ok(last)
    });
}

// --- helpers ---

fn str_arg(v: &Value, who: &str) -> SchemeResult<String> {
    match v {
        Value::Str(s) => Ok(s.borrow().clone()),
        _ => Err(SchemeError::msg(format!("{who}: arg must be a string"))),
    }
}

/// Resolve the input port for a 0-or-1-arg reader primitive: the explicit arg,
/// or the current input port.
fn in_port(it: &Interp, args: &[Value], who: &str) -> SchemeResult<Value> {
    match args.first() {
        Some(p @ Value::InputPort(_)) => Ok(p.clone()),
        Some(_) => Err(SchemeError::msg(format!("{who}: arg must be an input port"))),
        None => Ok(it.cur_in().expect("cur_in set")),
    }
}

/// Resolve the output port for a writer primitive whose port arg (if any) is at
/// index `port_idx`.
fn out_port(it: &Interp, args: &[Value], port_idx: usize, who: &str) -> SchemeResult<Value> {
    match args.get(port_idx) {
        Some(p @ Value::OutputPort(_)) => Ok(p.clone()),
        Some(_) => Err(SchemeError::msg(format!("{who}: bad output port"))),
        None => Ok(it.cur_out().expect("cur_out set")),
    }
}

fn write_to_port(port: &Value, s: &str) -> SchemeResult<()> {
    match port {
        Value::OutputPort(p) => p
            .borrow_mut()
            .sink
            .write_str(s)
            .map_err(|e| SchemeError::msg(format!("write error: {e}"))),
        _ => Err(SchemeError::msg("not an output port")),
    }
}

/// `(read [port])` — read one datum, advancing the port's cursor.
fn read_prim(it: &mut Interp, args: &[Value]) -> SchemeResult {
    let port = in_port(it, args, "read")?;
    let (chars, start) = match &port {
        Value::InputPort(p) => {
            let b = p.borrow();
            (b.chars.clone(), b.index)
        }
        _ => unreachable!(),
    };
    let mut reader = Reader::from_chars(chars, start);
    let datum = reader.read(it)?;
    // Advance the port past whatever the reader consumed.
    if let Value::InputPort(p) = &port {
        p.borrow_mut().index = reader.pos();
    }
    Ok(datum)
}

/// Shared body of `write`/`display`.
fn out_write(it: &mut Interp, args: &[Value], escaped: bool) -> SchemeResult {
    let s = if escaped {
        write_to_string(it, &args[0])
    } else {
        display_to_string(it, &args[0])
    };
    let port = out_port(it, args, 1, if escaped { "write" } else { "display" })?;
    write_to_port(&port, &s)?;
    Ok(Value::Bool(true))
}

/// Run `thunk` with the current input (or output) port swapped to `port`,
/// restoring the previous port afterward even on error/escape.
fn with_swapped(it: &mut Interp, is_input: bool, port: Value, thunk: Value) -> SchemeResult {
    let saved = if is_input {
        let old = it.cur_in();
        it.set_cur_in(port);
        old
    } else {
        let old = it.cur_out();
        it.set_cur_out(port);
        old
    };
    let result = it.apply(thunk, &[]);
    // Restore regardless of how the thunk finished.
    if let Some(old) = saved {
        if is_input {
            it.set_cur_in(old);
        } else {
            it.set_cur_out(old);
        }
    }
    result
}
