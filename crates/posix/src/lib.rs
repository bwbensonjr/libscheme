//! POSIX bindings — the Rust analogue of `src/posix/posix_file.c` and
//! `posix_proc.c`.
//!
//! This crate exists to demonstrate the central libscheme design principle: an
//! extension lives *outside* the core and adds primitives and nominal types
//! through the same public API (`Interp::register`, `Interp::make_type`,
//! `Value::make_struct`) that the built-in subsystems use. It depends only on
//! `libscheme`'s public surface plus `libc` for the system calls — the core
//! stays `unsafe`-free; what little `unsafe` exists is confined here.
//!
//! [`init_posix_file`] mirrors `init_posix_file`: filesystem operations, the
//! `<stat>` nominal type with its accessors, and the `O_*`/`S_*` constants.
//! [`init_posix_proc`] mirrors `init_posix_proc`: process operations.

use libscheme::{Arity, Gc, Interp, SchemeError, SchemeResult, TypeObject, Value};
use std::cell::RefCell;

// Field order for the `<stat>` record. Accessors index by these slots.
mod stat_slot {
    pub const MODE: usize = 0;
    pub const INO: usize = 1;
    pub const DEV: usize = 2;
    pub const NLINK: usize = 3;
    pub const UID: usize = 4;
    pub const GID: usize = 5;
    pub const SIZE: usize = 6;
    pub const ATIME: usize = 7;
    pub const MTIME: usize = 8;
    pub const CTIME: usize = 9;
    pub const COUNT: usize = 10;
}

thread_local! {
    /// The `<stat>` nominal type, stashed so the accessor primitives (which only
    /// receive `&mut Interp` and args) can check instance identity. Each
    /// `init_posix_file` call refreshes it for the interpreter being set up.
    static STAT_TYPE: RefCell<Option<Gc<TypeObject>>> = const { RefCell::new(None) };
}

/// `init_posix_file`: register the filesystem primitives, the `<stat>` type and
/// its accessors, and the `O_*`/`S_*`/`SEEK_*` integer constants.
pub fn init_posix_file(it: &mut Interp) {
    let stat_ty = it.make_type("<stat>");
    it.register_value("<stat>", Value::TypeObject(stat_ty.clone()));
    STAT_TYPE.with(|s| *s.borrow_mut() = Some(stat_ty));

    it.register("posix-getcwd", Arity::Exact(0), |_it, _a| {
        let cwd = std::env::current_dir()
            .map_err(|e| SchemeError::msg(format!("posix-getcwd: failed: {e}")))?;
        Ok(Value::make_string(cwd.to_string_lossy().into_owned()))
    });
    it.register("posix-chdir", Arity::Exact(1), |_it, a| {
        let path = str_arg(&a[0], "posix-chdir")?;
        std::env::set_current_dir(&path)
            .map_err(|e| SchemeError::msg(format!("posix-chdir: {path}: {e}")))?;
        Ok(Value::Bool(true))
    });
    it.register("posix-mkdir", Arity::Exact(2), |_it, a| {
        let path = str_arg(&a[0], "posix-mkdir")?;
        let _mode = int_arg(&a[1], "posix-mkdir")?;
        std::fs::create_dir(&path)
            .map_err(|e| SchemeError::msg(format!("posix-mkdir: {path}: {e}")))?;
        Ok(Value::Bool(true))
    });
    it.register("posix-rmdir", Arity::Exact(1), |_it, a| {
        let path = str_arg(&a[0], "posix-rmdir")?;
        std::fs::remove_dir(&path)
            .map_err(|e| SchemeError::msg(format!("posix-rmdir: {path}: {e}")))?;
        Ok(Value::Bool(true))
    });
    it.register("posix-link", Arity::Exact(2), |_it, a| {
        let old = str_arg(&a[0], "posix-link")?;
        let new = str_arg(&a[1], "posix-link")?;
        std::fs::hard_link(&old, &new)
            .map_err(|e| SchemeError::msg(format!("posix-link: {old} -> {new}: {e}")))?;
        Ok(Value::Bool(true))
    });
    it.register("posix-unlink", Arity::Exact(1), |_it, a| {
        let path = str_arg(&a[0], "posix-unlink")?;
        std::fs::remove_file(&path)
            .map_err(|e| SchemeError::msg(format!("posix-unlink: {path}: {e}")))?;
        Ok(Value::Bool(true))
    });
    it.register("posix-rename", Arity::Exact(2), |_it, a| {
        let old = str_arg(&a[0], "posix-rename")?;
        let new = str_arg(&a[1], "posix-rename")?;
        std::fs::rename(&old, &new)
            .map_err(|e| SchemeError::msg(format!("posix-rename: {old} -> {new}: {e}")))?;
        Ok(Value::Bool(true))
    });

    // posix-stat builds a <stat> record (a nominal struct instance).
    it.register("posix-stat", Arity::Exact(1), |_it, a| {
        let path = str_arg(&a[0], "posix-stat")?;
        let md = std::fs::metadata(&path)
            .map_err(|e| SchemeError::msg(format!("posix-stat: {path}: {e}")))?;
        Ok(make_stat(&md))
    });

    // stat accessors — pull a field from a <stat> instance by slot.
    register_stat_accessor(it, "stat-mode", stat_slot::MODE);
    register_stat_accessor(it, "stat-ino", stat_slot::INO);
    register_stat_accessor(it, "stat-dev", stat_slot::DEV);
    register_stat_accessor(it, "stat-nlink", stat_slot::NLINK);
    register_stat_accessor(it, "stat-uid", stat_slot::UID);
    register_stat_accessor(it, "stat-gid", stat_slot::GID);
    register_stat_accessor(it, "stat-size", stat_slot::SIZE);
    register_stat_accessor(it, "stat-atime", stat_slot::ATIME);
    register_stat_accessor(it, "stat-mtime", stat_slot::MTIME);
    register_stat_accessor(it, "stat-ctime", stat_slot::CTIME);

    // a few representative open/seek constants (libc values).
    for (name, val) in [
        ("o_rdonly", libc::O_RDONLY),
        ("o_wronly", libc::O_WRONLY),
        ("o_rdwr", libc::O_RDWR),
        ("o_append", libc::O_APPEND),
        ("o_creat", libc::O_CREAT),
        ("o_excl", libc::O_EXCL),
        ("o_trunc", libc::O_TRUNC),
        ("seek_set", libc::SEEK_SET),
        ("seek_cur", libc::SEEK_CUR),
        ("seek_end", libc::SEEK_END),
    ] {
        it.register_value(name, Value::Int(val as i64));
    }
}

/// `init_posix_proc`: process operations (fork/exit/wait/execv).
pub fn init_posix_proc(it: &mut Interp) {
    it.register("posix-fork", Arity::Exact(0), |_it, _a| {
        // SAFETY: fork() with no allocation between fork and return; the child
        // branch returns immediately to Scheme, matching the C binding.
        let pid = unsafe { libc::fork() };
        if pid == -1 {
            return Err(SchemeError::msg("posix-fork: could not fork"));
        }
        // Parent gets the child pid; child gets #f (as in C).
        Ok(if pid == 0 {
            Value::Bool(false)
        } else {
            Value::Int(pid as i64)
        })
    });
    it.register("posix-exit", Arity::Exact(1), |_it, a| {
        let status = int_arg(&a[0], "posix-exit")?;
        std::process::exit(status as i32);
    });
    it.register("posix-wait", Arity::Exact(0), |_it, _a| {
        let mut status: libc::c_int = 0;
        // SAFETY: wait() into a local status int.
        let pid = unsafe { libc::wait(&mut status) };
        if pid == -1 {
            return Err(SchemeError::msg("posix-wait: could not wait"));
        }
        Ok(Value::Int(pid as i64))
    });
    it.register("posix-getpid", Arity::Exact(0), |_it, _a| {
        // SAFETY: getpid() has no preconditions.
        Ok(Value::Int(unsafe { libc::getpid() } as i64))
    });
}

// --- helpers ---

fn str_arg(v: &Value, who: &str) -> SchemeResult<String> {
    match v {
        Value::Str(s) => Ok(s.borrow().clone()),
        _ => Err(SchemeError::msg(format!("{who}: arg must be a string"))),
    }
}

fn int_arg(v: &Value, who: &str) -> SchemeResult<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        _ => Err(SchemeError::msg(format!("{who}: arg must be an integer"))),
    }
}

/// Build a `<stat>` instance from filesystem metadata.
fn make_stat(md: &std::fs::Metadata) -> Value {
    use std::os::unix::fs::MetadataExt;
    let ty = STAT_TYPE
        .with(|s| s.borrow().clone())
        .expect("init_posix_file installed <stat> type");
    let mut fields = vec![Value::Bool(false); stat_slot::COUNT];
    fields[stat_slot::MODE] = Value::Int(md.mode() as i64);
    fields[stat_slot::INO] = Value::Int(md.ino() as i64);
    fields[stat_slot::DEV] = Value::Int(md.dev() as i64);
    fields[stat_slot::NLINK] = Value::Int(md.nlink() as i64);
    fields[stat_slot::UID] = Value::Int(md.uid() as i64);
    fields[stat_slot::GID] = Value::Int(md.gid() as i64);
    fields[stat_slot::SIZE] = Value::Int(md.size() as i64);
    fields[stat_slot::ATIME] = Value::Int(md.atime());
    fields[stat_slot::MTIME] = Value::Int(md.mtime());
    fields[stat_slot::CTIME] = Value::Int(md.ctime());
    Value::make_struct(ty, fields)
}

/// Register a `stat-FIELD` accessor reading `slot` from a `<stat>` instance.
fn register_stat_accessor(it: &mut Interp, name: &str, slot: usize) {
    let who = name.to_string();
    it.register(name, Arity::Exact(1), move |_it, a| {
        let ty = STAT_TYPE
            .with(|s| s.borrow().clone())
            .expect("<stat> type installed");
        a[0].struct_field(&ty, slot)
            .ok_or_else(|| SchemeError::msg(format!("{who}: arg must be a stat object")))
    });
}
