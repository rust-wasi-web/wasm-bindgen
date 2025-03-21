//! The `wasm-bindgen` wait transformation.
//!
//! This crate provides a transformation to turn the instruction
//! `i32.atomic.wait` into a function call.
//! The function checks via a global if blocking
//! is allowed on the current thread. If not, it spins instead of
//! issuing the wait operation.
//!

#![deny(missing_docs, missing_debug_implementations)]

use anyhow::{anyhow, Context, Error, Result};

use walrus::ir::{
    dfs_pre_order_mut, BinaryOp, Call, Instr, LoadKind, MemArg, UnaryOp, Value, VisitorMut,
};
use walrus::{
    ConstExpr, ExportItem, FunctionBuilder, FunctionId, GlobalId, InstrLocId, MemoryId, Module,
    ValType,
};

/// Import name of function returning monotonic clock in nanoseconds.
pub const CLOCK_NS_IMPORT: &str = "__wbindgen_clock_ns";

/// Import name of function that is called when maximum spin time is exceeded.
pub const SPIN_TIMEOUT_IMPORT: &str = "__wbindgen_spin_timeout";

/// Export name of global that informs us whether waiting is prohibited.
pub const WAIT_PROHIBITED_GLOBAL: &str = "wait_prohibited";

/// Export name of global that sets the maximum spin time.
pub const MAX_SPIN_NS_GLOBAL: &str = "max_spin_ns";

/// Default maximum spin time.
const MAX_SPIN_NS: i64 = 10 * 1000 * 1000 * 1000;

/// Supported memory argument.
const MEM_ARG: MemArg = MemArg {
    align: 4,
    offset: 0,
};

/// Adds the `__wbindgen_clock_ns` function import.
fn add_clock_ns_import(module: &mut Module, placeholder_module: &str) -> FunctionId {
    let ty = module.types.add(&[], &[ValType::I64]);
    let (func, _) = module.add_import_func(placeholder_module, CLOCK_NS_IMPORT, ty);
    module.funcs.get_mut(func).name = Some(CLOCK_NS_IMPORT.to_string());
    func
}

/// Adds the `__wbindgen_spin_timeout` function import.
fn add_spin_timeout_import(module: &mut Module, placeholder_module: &str) -> FunctionId {
    let ty = module.types.add(&[], &[]);
    let (func, _) = module.add_import_func(placeholder_module, SPIN_TIMEOUT_IMPORT, ty);
    module.funcs.get_mut(func).name = Some(SPIN_TIMEOUT_IMPORT.to_string());
    func
}

/// Adds the wait prohibited global.
fn add_wait_prohibited_global(module: &mut Module) -> GlobalId {
    let global =
        module
            .globals
            .add_local(ValType::I32, true, false, ConstExpr::Value(Value::I32(0)));
    module.globals.get_mut(global).name = Some(WAIT_PROHIBITED_GLOBAL.into());
    module
        .exports
        .add(WAIT_PROHIBITED_GLOBAL, ExportItem::Global(global));
    global
}

/// Adds the maximum spin time global.
fn add_max_spin_ns_global(module: &mut Module) -> GlobalId {
    let global = module.globals.add_local(
        ValType::I64,
        true,
        false,
        ConstExpr::Value(Value::I64(MAX_SPIN_NS)),
    );
    module.globals.get_mut(global).name = Some(MAX_SPIN_NS_GLOBAL.into());
    module
        .exports
        .add(MAX_SPIN_NS_GLOBAL, ExportItem::Global(global));
    global
}

/// Adds the `__atomic_wait32` function to the module.
fn add_atomic_wait32_func(
    module: &mut Module,
    memory: MemoryId,
    wait_prohibited: GlobalId,
    atomic_spin32: FunctionId,
) -> FunctionId {
    let mut builder = FunctionBuilder::new(
        &mut module.types,
        &[ValType::I32, ValType::I32, ValType::I64],
        &[ValType::I32],
    );

    builder.name("__atomic_wait32".into());

    // Parameters.
    let ptr = module.locals.add(ValType::I32);
    let expected = module.locals.add(ValType::I32);
    let timeout = module.locals.add(ValType::I64);

    builder.func_body().global_get(wait_prohibited).if_else(
        ValType::I32,
        |then| {
            then.local_get(ptr)
                .local_get(expected)
                .local_get(timeout)
                .call(atomic_spin32);
        },
        |else_| {
            else_
                .local_get(ptr)
                .local_get(expected)
                .local_get(timeout)
                .atomic_wait(memory, MEM_ARG, false);
        },
    );

    builder.finish(vec![ptr, expected, timeout], &mut module.funcs)
}

/// Adds the `__atomic_spin32` function to the module.
fn add_atomic_spin32_func(
    module: &mut Module,
    memory: MemoryId,
    clock_ns: FunctionId,
    max_spin_ns: GlobalId,
    spin_timeout: FunctionId,
) -> FunctionId {
    let mut builder = FunctionBuilder::new(
        &mut module.types,
        &[ValType::I32, ValType::I32, ValType::I64],
        &[ValType::I32],
    );

    builder.name("__atomic_spin32".into());

    // Parameters.
    let ptr = module.locals.add(ValType::I32);
    let expected = module.locals.add(ValType::I32);
    let timeout = module.locals.add(ValType::I64);

    // Locals.
    let start_time = module.locals.add(ValType::I64);
    let elapsed = module.locals.add(ValType::I64);
    let max_spin_ns_local = module.locals.add(ValType::I64);

    builder
        .func_body()
        // check initial value
        .local_get(ptr)
        .load(memory, LoadKind::I32 { atomic: true }, MEM_ARG)
        .local_get(expected)
        .binop(BinaryOp::I32Ne)
        .if_else(
            None,
            |then| {
                // memory != expected, return 0
                then.i32_const(0).return_();
            },
            |_| (),
        )
        // memory == expected, record start time
        .call(clock_ns)
        .local_set(start_time)
        // cache max_spin_ns in local
        .global_get(max_spin_ns)
        .local_set(max_spin_ns_local)
        // spin loop
        .loop_(None, |spin_loop| {
            let id = spin_loop.id();
            spin_loop
                // check if memory still equals expected
                .local_get(ptr)
                .load(memory, LoadKind::I32 { atomic: true }, MEM_ARG)
                .local_get(expected)
                .binop(BinaryOp::I32Ne)
                .if_else(
                    None,
                    |then| {
                        // value changed, return 1
                        then.i32_const(1).return_();
                    },
                    |_| (),
                )
                // get time
                .call(clock_ns)
                .local_get(start_time)
                .binop(BinaryOp::I64Sub)
                .local_tee(elapsed)
                .local_get(timeout)
                .binop(BinaryOp::I64GeU)
                .if_else(
                    None,
                    |then| {
                        // timeout exceeded, return 2
                        then.i32_const(2).return_();
                    },
                    |_| (),
                )
                // check for global spin timeout
                .local_get(max_spin_ns_local)
                .unop(UnaryOp::I64Eqz)
                .if_else(
                    None,
                    |_| (),
                    |else_| {
                        // global spin timeout enabled
                        else_
                            .local_get(elapsed)
                            .local_get(max_spin_ns_local)
                            .binop(BinaryOp::I64GeU)
                            .if_else(
                                None,
                                |then| {
                                    // global spin timeout exceeded
                                    then.call(spin_timeout).unreachable();
                                },
                                |_| (),
                            );
                    },
                )
                // repeat loop
                .br(id);
        })
        .unreachable();

    builder.finish(vec![ptr, expected, timeout], &mut module.funcs)
}

/// Replaces `atomic.wait32` by the specified function.
struct ReplaceAtomicWait {
    wait32_func: FunctionId,
    memory: MemoryId,
    failed: Option<Error>,
}

impl VisitorMut for ReplaceAtomicWait {
    fn visit_instr_mut(&mut self, instr: &mut Instr, instr_loc: &mut InstrLocId) {
        if self.failed.is_some() {
            return;
        }

        if let Some(wait) = instr.atomic_wait_mut() {
            if wait.memory != self.memory {
                self.failed = Some(anyhow!(
                    "unsupported wait memory index {} at {}",
                    wait.memory.index(),
                    instr_loc.data()
                ));
            }

            if wait.arg.align != MEM_ARG.align || wait.arg.offset != MEM_ARG.offset {
                self.failed = Some(anyhow!(
                    "unsupported wait memory argument {:?} at {}",
                    wait.arg,
                    instr_loc.data()
                ));
            }

            if wait.sixty_four {
                self.failed = Some(anyhow!("unsupported wait64 at {}", instr_loc.data()));
            }

            *instr = Instr::Call(Call {
                func: self.wait32_func,
            });
        }
    }
}

/// Run the transformation.
///
/// See the module-level docs for details on the transformation.
pub fn run(module: &mut Module, placeholder_module: &str) -> Result<()> {
    // For now only one memory is supported.
    let memory = module
        .memories
        .iter()
        .next()
        .context("module has no memory")?
        .id();

    // Add necessary items to module.
    let clock_ns = add_clock_ns_import(module, placeholder_module);
    let spin_timeout = add_spin_timeout_import(module, placeholder_module);
    let max_spin_ns = add_max_spin_ns_global(module);
    let spin32_func = add_atomic_spin32_func(module, memory, clock_ns, max_spin_ns, spin_timeout);
    let wait_prohibited = add_wait_prohibited_global(module);
    let wait32_func = add_atomic_wait32_func(module, memory, wait_prohibited, spin32_func);

    // Replace all atomic.wait32 instructions by calls to function.
    for (id, func) in module.funcs.iter_local_mut() {
        if id == wait32_func || id == spin32_func {
            continue;
        }

        let mut visitor = ReplaceAtomicWait {
            wait32_func,
            memory,
            failed: None,
        };

        dfs_pre_order_mut(&mut visitor, func, func.entry_block());

        if let Some(err) = visitor.failed {
            return Err(err).with_context(|| format!("processing function {} failed", id.index()));
        }
    }

    Ok(())
}
