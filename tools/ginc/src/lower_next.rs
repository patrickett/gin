//! Temporary fail-closed lowering from the new checker's checked bodies to MLIR
//! text. This is a narrow replacement-path slice, not a general backend, and it
//! must not call the legacy parser, checker, or typed-AST lowering.
//!
//! One Gin module lowers to one MLIR module. A call into another Gin module becomes a
//! `func.call` to a module-qualified symbol plus a private declaration, so modules lower
//! independently and are joined at link time.
//!
//! Integer storage is mapped to `i64` as a placeholder until target-width
//! selection reaches this path; the domain bounds are already checked upstream.
//!
//! A named domain holds exactly one integer, so a domain value lowers to that integer:
//! building one and reading its integer are both the identity, and only reducing into a
//! wrapping domain emits arithmetic. Records with several fields have no lowering yet,
//! except at a `#instruction` boundary, where a record of `#register` fields is the
//! register list of one `llvm.inline_asm`. The compiler knows no register, ABI or
//! instruction here: every name comes from Gin, and LLVM refuses an unknown register
//! because each one is a constraint on an operand. There is never a clobber list, whose
//! unknown names LLVM ignores silently; what an instruction destroys is an output.

use crate::mono::{self, MonoLimits, Program};
use num_traits::ToPrimitive;
use std::collections::BTreeSet;
use typecheck::{
    AggregateKind, CheckedArithmetic, CheckedSourceBody, CheckedSourceBodyKind,
    CheckedSourceCallee, CheckedSourceField, CheckedSourceFile, CheckedSourceFunction,
    CheckedSourceInstanceHead, CheckedSourceModule, SourceType,
};

#[derive(Debug, thiserror::Error)]
pub enum LowerNextError {
    #[error("no executable body was recorded for {0}")]
    MissingBody(String),
    #[error("lowering `{function}` hit an unsupported construct")]
    Unsupported { function: String },
    #[error("emitted MLIR failed to verify: {0}")]
    InvalidMlir(String),
    #[error("{0}")]
    Instantiation(#[from] crate::mono::MonoError),
    #[error("{0}")]
    Generic(String),
}

/// A single file has no module name, so its symbols stay bare.
pub fn lower_file(file: &CheckedSourceFile) -> Result<String, LowerNextError> {
    lower_checked("", file, &[], &BTreeSet::new(), None)
}

/// As `lower_program`, for a library module of an executable: a function that cannot be
/// lowered is left out and listed, to fail the build only if the program calls it.
pub fn lower_library(
    program: &Program<'_>,
    module: &CheckedSourceModule,
    skipped: &mut Vec<Skipped>,
) -> Result<String, LowerNextError> {
    let dependencies = program.dependencies(module);
    if !mono::needs_world(module) {
        return lower_checked_tolerating(
            module.name(),
            module.checked(),
            &dependencies,
            &BTreeSet::new(),
            None,
            Some(skipped),
        );
    }
    let world = mono::World::build(program, module, MonoLimits::default())?;
    lower_checked_tolerating(
        module.name(),
        &world.file,
        &dependencies,
        &world.external,
        Some(&world),
        Some(skipped),
    )
}

/// Lower `module` with every module of the build in view, so that what it instantiates from
/// the generic functions of any of them can be found.
pub fn lower_program(
    program: &Program<'_>,
    module: &CheckedSourceModule,
) -> Result<String, LowerNextError> {
    let dependencies = program.dependencies(module);
    if !mono::needs_world(module) {
        return lower_checked(
            module.name(),
            module.checked(),
            &dependencies,
            &BTreeSet::new(),
            None,
        );
    }
    let world = mono::World::build(program, module, MonoLimits::default())?;
    lower_checked(
        module.name(),
        &world.file,
        &dependencies,
        &world.external,
        Some(&world),
    )
}

struct Context<'a> {
    module: &'a str,
    file: &'a CheckedSourceFile,
    dependencies: &'a [&'a CheckedSourceModule],
    /// Functions of `file` that another module defines and this one only calls.
    externals: &'a BTreeSet<usize>,
    /// Text literals used at run time, by content: one read-only global each, per module.
    texts: std::cell::RefCell<Vec<Vec<u8>>>,
    /// The file is a world, where the instances of every module are one set of ids.
    world: bool,
    /// Stack slots of the function being lowered: `llvm.alloca`s that go in its entry block.
    slots: std::cell::RefCell<Vec<String>>,
}

/// A function this module calls but does not define.
struct Declaration {
    symbol: String,
    parameters: usize,
    /// How many numbers it gives back.
    results: usize,
    /// A link symbol named at a `#instruction` call: it has no signature, only an address.
    external: bool,
}

/// The most numbers a record may hold and still travel in registers; see `register_width`.
const REGISTER_LIMIT: usize = 16;

/// The maximum flattened register width accepted for a value held in a stack slot.
const SLOT_REGISTERS_MAX: usize = 512;

fn lower_checked(
    module: &str,
    file: &CheckedSourceFile,
    dependencies: &[&CheckedSourceModule],
    externals: &BTreeSet<usize>,
    world: Option<&mono::World<'_>>,
) -> Result<String, LowerNextError> {
    lower_checked_tolerating(module, file, dependencies, externals, world, None)
}

/// A function that could not be lowered, by symbol, and why. A library module of an executable
/// may hold functions the program never calls; they are left out, and the build fails only if
/// some module calls one.
pub struct Skipped {
    pub symbol: String,
    pub error: String,
}

fn lower_checked_tolerating(
    module: &str,
    file: &CheckedSourceFile,
    dependencies: &[&CheckedSourceModule],
    externals: &BTreeSet<usize>,
    world: Option<&mono::World<'_>>,
    mut skipped: Option<&mut Vec<Skipped>>,
) -> Result<String, LowerNextError> {
    assert_eq!(file.functions.len(), file.bodies.len());
    let context = Context {
        module,
        file,
        dependencies,
        externals,
        texts: std::cell::RefCell::new(Vec::new()),
        world: world.is_some(),
        slots: std::cell::RefCell::new(Vec::new()),
    };
    if let Some(entry) = file.entry
        && file
            .functions
            .get(entry as usize)
            .is_some_and(|f| !f.type_parameters.is_empty())
    {
        return Err(LowerNextError::Generic(
            "a generic function cannot be the `#entry`: it has no type arguments to be given"
                .to_owned(),
        ));
    }
    // Say which instance does not fit before any function that merely calls it fails.
    if let Some(world) = world {
        for index in (0..file.functions.len()).filter(|index| world.is_instance(*index)) {
            context.check_instance_fits(world, index)?;
        }
    }
    let mut declarations = Vec::new();
    let mut pieces: Vec<(String, String)> = Vec::new();
    for (index, function) in file.functions.iter().enumerate() {
        // An instruction is applied where it is called; there is nothing to define. A
        // function of text is only ever run by the compiler, so it has no machine form. A
        // template is lowered once per instance, and a stub is defined by its own module.
        if function.instruction.is_some()
            || compile_time_only(file, function)
            || !function.type_parameters.is_empty()
            || externals.contains(&index)
        {
            continue;
        }

        let lowered = match file.bodies[index].as_ref() {
            None => Err(LowerNextError::MissingBody(
                match file.declines.iter().find(|(at, _)| *at as usize == index) {
                    Some((_, decline)) => format!(
                        "`{}` (the recorder declined {}{})",
                        function.name,
                        decline.construct,
                        decline
                            .span
                            .map(|span| format!(" at bytes {}..{}", span.start(), span.end()))
                            .unwrap_or_default()
                    ),
                    None => format!("`{}`", function.name),
                },
            )),
            Some(body) => context.lower_function(function, body, &mut declarations),
        };
        match (lowered, skipped.as_deref_mut()) {
            (Ok(lowered), _) => pieces.push((context.symbol(&function.name), lowered)),
            (Err(error), None) => return Err(error),
            (Err(error), Some(skipped)) => skipped.push(Skipped {
                symbol: context.symbol(&function.name),
                error: error.to_string(),
            }),
        }
    }
    // A function that calls a left-out one is left out too, to a fixed point: only what the
    // program calls must lower, and a caller that is itself never called is not one of those.
    if let Some(skipped) = skipped {
        loop {
            let position = pieces.iter().position(|(_, lowered)| {
                skipped
                    .iter()
                    .any(|left_out| lowered.contains(&format!("{}(", left_out.symbol)))
            });
            let Some(position) = position else { break };
            let (symbol, lowered) = pieces.remove(position);
            let cause = skipped
                .iter()
                .find(|left_out| lowered.contains(&format!("{}(", left_out.symbol)))
                .map(|left_out| left_out.error.clone())
                .unwrap_or_default();
            skipped.push(Skipped {
                symbol,
                error: cause,
            });
        }
    }
    let text: String = pieces.iter().map(|(_, lowered)| lowered.as_str()).collect();
    let mut output = String::from("module {\n");
    for (index, bytes) in context.texts.borrow().iter().enumerate() {
        let written: String = bytes.iter().map(|byte| format!("\\{byte:02X}")).collect();
        output.push_str(&format!(
            "  llvm.mlir.global private constant @gin.text.{index}(\"{written}\") {{addr_space = 0 : i32}} : !llvm.array<{} x i8>\n",
            bytes.len()
        ));
    }
    for declaration in &declarations {
        output.push_str(&declare(declaration));
    }
    output.push_str(&text);
    if let Some(entry) = file.entry {
        output.push_str(&context.entry_shim(entry)?);
    }
    output.push_str("}\n");
    verify_mlir(&output)?;
    Ok(output)
}

/// The written form of one `#instruction` call. `id` names its SSA values.
struct Instruction<'a> {
    id: usize,
    text: &'a str,
    /// The compile-time text given for each `$name` of `text`, as (name, symbol).
    symbols: &'a [(String, String)],
}

impl Context<'_> {
    fn lower_function(
        &self,
        function: &CheckedSourceFunction,
        body: &CheckedSourceBody,
        declarations: &mut Vec<Declaration>,
    ) -> Result<String, LowerNextError> {
        let unsupported = || LowerNextError::Unsupported {
            function: function.name.clone(),
        };
        if body
            .expressions
            .iter()
            .any(|expr| matches!(expr.kind, CheckedSourceBodyKind::Hole))
        {
            return Err(LowerNextError::Generic(format!(
                "`{}` contains a `#todo` hole; a program with a hole can be run by the interpreter but not compiled",
                function.name
            )));
        }
        // A value too wide for the registers is named as such, not as an unsupported construct.
        for ty in function
            .parameters
            .iter()
            .chain(std::iter::once(&function.result))
        {
            if self.register_width(ty).is_none()
                && let Some(need) = self.flat_width(ty, 0)
                && need > REGISTER_LIMIT
            {
                return Err(LowerNextError::Generic(format!(
                    "`{}` passes a value of {need} registers and only {REGISTER_LIMIT} are passed (a record over the limit needs the by-address class, which is not built)",
                    function.name
                )));
            }
        }
        let mut parameters = Vec::new();
        for (index, ty) in function.parameters.iter().enumerate() {
            match self.register_width(ty).ok_or_else(unsupported)? {
                1 => parameters.push(format!("%arg{index}: i64")),
                width => parameters.extend((0..width).map(|k| format!("%arg{index}_{k}: i64"))),
            }
        }
        // An owned array in the result is given back in a slot of the caller's, whose address
        // comes after the parameters: the function's own slot ends with it.
        let result_slots = self.array_slots(&function.result).ok_or_else(unsupported)?;
        for k in 0..result_slots.len() {
            parameters.push(format!("%sret{k}: i64"));
        }
        self.slots.borrow_mut().clear();
        // What a function gives back: its result, then the value each `mut` parameter holds
        // when it returns. Which class a value passes in is decided here, by its type alone,
        // so a caller and a callee always agree: numbers go by register.
        let mut results: Vec<&str> = Vec::new();
        results.extend(std::iter::repeat_n(
            "i64",
            self.register_width(&function.result)
                .ok_or_else(unsupported)?,
        ));
        for &index in &function.mutated {
            let ty = function
                .parameters
                .get(index as usize)
                .ok_or_else(unsupported)?;
            results.extend(std::iter::repeat_n(
                "i64",
                self.register_width(ty).ok_or_else(unsupported)?,
            ));
        }
        let result = match results.len() {
            0 => String::new(),
            1 => format!(" -> {}", results[0]),
            _ => format!(" -> ({})", results.join(", ")),
        };
        let returned = returned_value(body).ok_or_else(unsupported)?;
        let mut ops = Vec::new();
        let values = self
            .lower_nodes(body, &mut ops, declarations)
            .ok_or_else(unsupported)?;
        let mut returning = Vec::new();
        if function.result != SourceType::Unit {
            let value = values[returned as usize].clone().ok_or_else(unsupported)?;
            let given = &body.expressions[returned as usize].ty;
            let value = self
                .coerce(
                    &value,
                    given,
                    &function.result,
                    body.expressions.len(),
                    &mut ops,
                )
                .ok_or_else(unsupported)?;
            returning.push(value);
        }
        for output in &body.outputs {
            returning.push(values[*output as usize].clone().ok_or_else(unsupported)?);
        }
        // A record is several registers: flatten them into the list.
        let returning: Vec<String> = returning
            .iter()
            .flat_map(|value| value.split(", ").map(str::to_owned))
            .collect();
        if returning.len() != results.len() {
            return Err(unsupported());
        }
        let mut returning = returning;
        for (k, (register, bytes)) in result_slots.iter().enumerate() {
            let source = returning.get(*register).cloned().ok_or_else(unsupported)?;
            copy_bytes(
                &format!("r{k}"),
                &source,
                &format!("%sret{k}"),
                *bytes,
                &mut ops,
            );
            returning[*register] = format!("%sret{k}");
        }
        // A `mut` parameter hands back the array it was given, written in place: an array put in
        // its place by the body is copied into the given one (its own slot ends here).
        let mut position = self
            .register_width(&function.result)
            .ok_or_else(unsupported)?;
        for &index in &function.mutated {
            let ty = function
                .parameters
                .get(index as usize)
                .ok_or_else(unsupported)?;
            let width = self.register_width(ty).ok_or_else(unsupported)?;
            for (k, (register, bytes)) in self
                .array_slots(ty)
                .ok_or_else(unsupported)?
                .iter()
                .enumerate()
            {
                let given = if width == 1 {
                    format!("%arg{index}")
                } else {
                    format!("%arg{index}_{register}")
                };
                let now = returning
                    .get(position + register)
                    .cloned()
                    .ok_or_else(unsupported)?;
                if now != given {
                    copy_bytes(&format!("m{index}_{k}"), &now, &given, *bytes, &mut ops);
                    returning[position + register] = given;
                }
            }
            position += width;
        }
        if returning.is_empty() {
            ops.push("    return".to_string());
        } else {
            ops.push(format!(
                "    return {} : {}",
                returning.join(", "),
                results.join(", ")
            ));
        }
        // An instance is emitted by every module that needs it, under one symbol.
        let linkage = match function.name.starts_with(mono::ABSOLUTE) {
            true => " attributes {llvm.linkage = #llvm.linkage<linkonce_odr>}",
            false => "",
        };
        let mut slots = self.slots.borrow_mut();
        slots.extend(ops);
        let ops = std::mem::take(&mut *slots);
        Ok(format!(
            "  func.func {}({}){}{linkage} {{\n{}\n  }}\n",
            self.symbol(&function.name),
            parameters.join(", "),
            result,
            ops.join("\n")
        ))
    }

    /// The arena is postorder, so one forward pass sees every operand before its user.
    /// Entry `i` of the result names the SSA value of node `i`, or `None` for unit.
    fn lower_nodes(
        &self,
        body: &CheckedSourceBody,
        ops: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
    ) -> Option<Vec<Option<String>>> {
        // Loop bodies, and the arms of a choice that must not run both, are regions of their
        // own: one definition, shared with the evaluator.
        let nesting = typecheck::regions::nesting(body, &|kind| self.guarded(kind));
        let mut values = vec![None; body.expressions.len()];
        let all = 0..body.expressions.len();
        self.lower_range(body, all, 0, &nesting, &mut values, ops, declarations)?;
        Some(values)
    }

    /// The nodes of `range` that are `level` loop bodies deep. A loop lowers the nodes of its
    /// own body, one level deeper, inside its region.
    #[allow(clippy::too_many_arguments)] // the lowering walk threads its state explicitly
    fn lower_range(
        &self,
        body: &CheckedSourceBody,
        range: std::ops::Range<usize>,
        level: u32,
        nesting: &[u32],
        values: &mut Vec<Option<String>>,
        ops: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
    ) -> Option<()> {
        for id in range {
            if nesting[id] != level {
                continue;
            }
            let expr = &body.expressions[id];
            let value = match &expr.kind {
                CheckedSourceBodyKind::Parameter { index } => {
                    match self.register_width(&expr.ty)? {
                        0 => None,
                        1 => Some(format!("%arg{index}")),
                        width => Some(
                            (0..width)
                                .map(|k| format!("%arg{index}_{k}"))
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    }
                }
                CheckedSourceBodyKind::Load { pointer, index } => {
                    let (bits, signed) = self.memory_element(&expr.ty)?;
                    let address = self.memory_address(
                        id,
                        values.get(*pointer as usize)?.as_ref()?,
                        values.get(*index as usize)?.as_ref()?,
                        bits,
                        ops,
                    );
                    let name = format!("%load{id}");
                    if bits == 64 {
                        ops.push(format!(
                            "    {name} = llvm.load {address} : !llvm.ptr -> i64"
                        ));
                    } else {
                        ops.push(format!(
                            "    %raw{id} = llvm.load {address} : !llvm.ptr -> i{bits}"
                        ));
                        let extend = if signed { "arith.extsi" } else { "arith.extui" };
                        ops.push(format!("    {name} = {extend} %raw{id} : i{bits} to i64"));
                    }
                    Some(name)
                }
                CheckedSourceBodyKind::Store {
                    pointer,
                    index,
                    value,
                } => {
                    let element = &body.expressions[*value as usize].ty;
                    let (bits, _) = self.memory_element(element)?;
                    let address = self.memory_address(
                        id,
                        values.get(*pointer as usize)?.as_ref()?,
                        values.get(*index as usize)?.as_ref()?,
                        bits,
                        ops,
                    );
                    let stored = values.get(*value as usize)?.as_ref()?.clone();
                    if bits == 64 {
                        ops.push(format!(
                            "    llvm.store {stored}, {address} : i64, !llvm.ptr"
                        ));
                    } else {
                        ops.push(format!(
                            "    %narrow{id} = arith.trunci {stored} : i64 to i{bits}"
                        ));
                        ops.push(format!(
                            "    llvm.store %narrow{id}, {address} : i{bits}, !llvm.ptr"
                        ));
                    }
                    None
                }
                CheckedSourceBodyKind::Integer(value) => {
                    let bits = register_bits(value.to_i128()?)?;
                    let name = format!("%const{}", ops.len());
                    ops.push(format!("    {name} = arith.constant {bits} : i64"));
                    Some(name)
                }
                CheckedSourceBodyKind::Binary {
                    operator,
                    left,
                    right,
                } => {
                    // The checked domains prove the exact result fits, so the machine
                    // operation needs no overflow check; it must still fit the i64 placeholder.
                    let operands = [*left, *right].map(|id| &body.expressions[id as usize].ty);
                    // A sum, difference or product of two numbers can be wider than a register
                    // (`left + right` of two 64-bit values): it is worked in 128 bits, exactly,
                    // and brought back by whoever proved it fits.
                    // Non-negative numbers whose result still fits an unsigned register: the machine
                    // operation on the same bits is exact (`previous + current` of two numbers
                    // below 2^63 is below 2^64).
                    if matches!(
                        operator,
                        CheckedArithmetic::Add
                            | CheckedArithmetic::Subtract
                            | CheckedArithmetic::Multiply
                    ) && operands.iter().all(|ty| fits_u64(ty))
                        && fits_u64(&expr.ty)
                        && !(operands.iter().all(|ty| fits_i64(ty)) && fits_i64(&expr.ty))
                    {
                        let operation = match operator {
                            CheckedArithmetic::Add => "arith.addi",
                            CheckedArithmetic::Subtract => "arith.subi",
                            _ => "arith.muli",
                        };
                        let left = values.get(*left as usize)?.clone()?;
                        let right = values.get(*right as usize)?.clone()?;
                        let name = format!("%t{id}");
                        ops.push(format!("    {name} = {operation} {left}, {right} : i64"));
                        values[id] = Some(name);
                        continue;
                    }
                    if operands.iter().all(|ty| fits_wide(ty))
                        && fits_wide(&expr.ty)
                        && (is_wide(&expr.ty) || operands.iter().any(|ty| is_wide(ty)))
                    {
                        let operation = match operator {
                            CheckedArithmetic::Add => "arith.addi",
                            CheckedArithmetic::Subtract => "arith.subi",
                            CheckedArithmetic::Multiply => "arith.muli",
                            _ => return None,
                        };
                        let left = widened(
                            values.get(*left as usize)?.as_ref()?,
                            operands[0],
                            id * 2,
                            ops,
                        );
                        let right = widened(
                            values.get(*right as usize)?.as_ref()?,
                            operands[1],
                            id * 2 + 1,
                            ops,
                        );
                        let name = format!("%t{id}");
                        ops.push(format!("    {name} = {operation} {left}, {right} : i128"));
                        // A result that fits a register is held in one.
                        if !is_wide(&expr.ty) {
                            let narrow = format!("%tn{id}");
                            ops.push(format!("    {narrow} = arith.trunci {name} : i128 to i64"));
                            values[id] = Some(narrow);
                        } else {
                            values[id] = Some(name);
                        }
                        continue;
                    }
                    if !operands.iter().all(|ty| fits_i64(ty)) || !fits_i64(&expr.ty) {
                        return None;
                    }
                    let left = values.get(*left as usize)?.clone()?;
                    let right = values.get(*right as usize)?.clone()?;
                    let name = format!("%t{id}");
                    let operation = match operator {
                        CheckedArithmetic::Add => "arith.addi",
                        CheckedArithmetic::Subtract => "arith.subi",
                        CheckedArithmetic::Multiply => "arith.muli",
                        // Signed division truncates toward zero and a remainder follows its
                        // dividend, as in the checker; `>>` keeps the sign.
                        CheckedArithmetic::Divide => "arith.divsi",
                        CheckedArithmetic::Remainder => "arith.remsi",
                        CheckedArithmetic::BitAnd => "arith.andi",
                        CheckedArithmetic::BitOr => "arith.ori",
                        CheckedArithmetic::BitXor => "arith.xori",
                        CheckedArithmetic::ShiftLeft => "arith.shli",
                        CheckedArithmetic::ShiftRight => "arith.shrsi",
                    };
                    // A shift by 64 or more bits is undefined in the machine: the amount must be
                    // provably below the register width, not only below what the checker allows.
                    if matches!(
                        operator,
                        CheckedArithmetic::ShiftLeft | CheckedArithmetic::ShiftRight
                    ) {
                        let SourceType::Integer { lower, upper } = *operands[1] else {
                            return None;
                        };
                        let (lower, upper) = (lower.to_i128()?, upper.to_i128()?);
                        if lower < 0 || upper > 63 {
                            return None;
                        }
                    }
                    ops.push(format!("    {name} = {operation} {left}, {right} : i64"));
                    Some(name)
                }
                CheckedSourceBodyKind::Call {
                    callee, arguments, ..
                } => {
                    let target = self.callee(*callee)?.1;
                    let mut operands = Vec::new();
                    for (position, argument) in arguments.iter().enumerate() {
                        let value = values.get(*argument as usize)?.as_ref()?;
                        let expected = target.parameters.get(position)?;
                        let given = &body.expressions[*argument as usize].ty;
                        operands.push(self.coerce_argument(
                            *callee,
                            value,
                            given,
                            expected,
                            id * 64 + position,
                            ops,
                        )?);
                    }
                    // A result given back in a slot of this function's is not made in a loop: each
                    // pass would write over the slot an earlier pass's value is still in.
                    if inside_loop(body, id)
                        && self
                            .array_slots(&expr.ty)
                            .is_none_or(|slots| !slots.is_empty())
                    {
                        return None;
                    }
                    let returned = self.register_width(&expr.ty)?;
                    let count = returned + self.mutated_width(*callee)?;
                    let name = (count > 0).then(|| format!("%call{id}"));
                    self.lower_call(
                        *callee,
                        &operands,
                        name.as_deref().map(|name| (name, count)),
                        ops,
                        declarations,
                    )?;
                    // The result is the first registers when the call gives back several.
                    // A call that gives back nothing (a unit call made for its effect) has no value.
                    let Some(name) = name else {
                        values[id] = None;
                        continue;
                    };
                    match (returned, count) {
                        (0, _) => None,
                        (_, 1) => Some(name),
                        (width, _) => Some(
                            (0..width)
                                .map(|k| format!("{name}#{k}"))
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    }
                }
                CheckedSourceBodyKind::CallOut { call, index } => {
                    let CheckedSourceBodyKind::Call { callee, .. } =
                        &body.expressions[*call as usize].kind
                    else {
                        return None;
                    };
                    let returned = self.register_width(&body.expressions[*call as usize].ty)?;
                    let target = self.callee(*callee)?.1;
                    // The callee's parameter types are in its own module's arena.
                    let width_of = |parameter: u32| {
                        self.callee_parameter_width(
                            *callee,
                            std::iter::once(*target.parameters.get(parameter as usize)?),
                        )
                    };
                    let mut position = returned;
                    for &parameter in target.mutated.iter().take(*index as usize) {
                        position += width_of(parameter)?;
                    }
                    let parameter = *target.mutated.get(*index as usize)?;
                    let width = width_of(parameter)?;
                    let count = returned + self.mutated_width(*callee)?;
                    Some(if count == 1 {
                        format!("%call{call}")
                    } else {
                        (position..position + width)
                            .map(|k| format!("%call{call}#{k}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                }
                // A domain is its integer: building one and reading it are the identity.
                CheckedSourceBodyKind::Construct { schema, fields } => {
                    // Registers going into an instruction are read from this node there.
                    if !fields.is_empty() && self.registers(*schema).is_some() {
                        None
                    } else if fields.is_empty() && self.nullary(*schema) {
                        // A record with nothing in it is only which one it is: its tag.
                        let name = format!("%tag{id}");
                        let tag = self.tag(*schema)?;
                        ops.push(format!("    {name} = arith.constant {tag} : i64"));
                        Some(name)
                    } else if !self.is_domain(*schema) && self.slotted_record(&expr.ty).is_some() {
                        // A record wider than the registers: its registers go in a slot, one after
                        // another, and the record is the slot's address (made once: not in a loop).
                        if inside_loop(body, id) {
                            return None;
                        }
                        let width = self.slotted_record(&expr.ty)?;
                        let declared = &self.file.aggregates.get(*schema as usize)?.fields;
                        let mut names = Vec::new();
                        for (position, field) in fields.iter().enumerate() {
                            let value = values.get(*field as usize)?.clone()?;
                            let given = &body.expressions[*field as usize].ty;
                            let expected = &declared.get(position)?.ty;
                            let value =
                                self.coerce(&value, given, expected, id * 64 + position, ops)?;
                            names.extend(value.split(", ").map(str::to_owned));
                        }
                        if names.len() != width {
                            return None;
                        }
                        let mut slots = self.slots.borrow_mut();
                        slots.push(format!(
                            "    %recn{id} = llvm.mlir.constant({} : i64) : i64",
                            8 * width
                        ));
                        slots.push(format!(
                            "    %rec{id} = llvm.alloca %recn{id} x i8 : (i64) -> !llvm.ptr"
                        ));
                        drop(slots);
                        for (k, name) in names.iter().enumerate() {
                            ops.push(format!("    %recp{id}_{k} = llvm.getelementptr %rec{id}[{k}] : (!llvm.ptr) -> !llvm.ptr, i64"));
                            ops.push(format!(
                                "    llvm.store {name}, %recp{id}_{k} : i64, !llvm.ptr"
                            ));
                        }
                        ops.push(format!(
                            "    %reca{id} = llvm.ptrtoint %rec{id} : !llvm.ptr to i64"
                        ));
                        Some(format!("%reca{id}"))
                    } else if !self.is_domain(*schema) {
                        // A record of numbers travels as its fields' registers, in order.
                        self.register_width(&expr.ty)?;
                        let declared = &self.file.aggregates.get(*schema as usize)?.fields;
                        let mut names = Vec::new();
                        for (position, field) in fields.iter().enumerate() {
                            let value = values.get(*field as usize)?.clone()?;
                            let given = &body.expressions[*field as usize].ty;
                            let expected = &declared.get(position)?.ty;
                            names.push(self.coerce(
                                &value,
                                given,
                                expected,
                                id * 64 + position,
                                ops,
                            )?);
                        }
                        Some(names.join(", "))
                    } else if fields.len() != 1 {
                        return None;
                    } else {
                        values.get(fields[0] as usize)?.clone()
                    }
                }
                CheckedSourceBodyKind::Instruction {
                    text,
                    symbols,
                    input,
                } => {
                    let call = Instruction { id, text, symbols };
                    self.lower_instruction(
                        call,
                        (&body.expressions[*input as usize], &expr.ty),
                        values,
                        ops,
                        declarations,
                    )?
                }
                // Never read: any registers of the right count.
                CheckedSourceBodyKind::Undefined => match self.register_width(&expr.ty)? {
                    0 => None,
                    width => {
                        ops.push(format!("    %undef{id} = arith.constant 0 : i64"));
                        Some(vec![format!("%undef{id}"); width].join(", "))
                    }
                },
                CheckedSourceBodyKind::Bundle { parts } => {
                    // A part may be a member where the bundle holds the sum (an exit returning
                    // one variant): it takes the sum's shape, as a branch of a choice does.
                    let declared = match &expr.ty {
                        SourceType::Sequence { id } => self
                            .file
                            .instances
                            .get(*id as usize)
                            .map(|instance| instance.arguments.clone()),
                        _ => None,
                    };
                    let mut registers = Vec::new();
                    for (index, part) in parts.iter().enumerate() {
                        if let Some(value) = values.get(*part as usize)?.clone() {
                            let given = &body.expressions[*part as usize].ty;
                            let value = match declared.as_ref().and_then(|types| types.get(index)) {
                                Some(want) if want != given => {
                                    self.coerce(&value, given, want, id * 64 + index, ops)?
                                }
                                _ => value,
                            };
                            registers.push(value);
                        }
                    }
                    (!registers.is_empty()).then(|| registers.join(", "))
                }
                CheckedSourceBodyKind::Part { bundle, index } => {
                    let offsets = self.bundle_offsets(&body.expressions[*bundle as usize].ty)?;
                    let (start, end) = (
                        *offsets.get(*index as usize)?,
                        *offsets.get(*index as usize + 1)?,
                    );
                    if start == end {
                        None
                    } else {
                        let source = values.get(*bundle as usize)?.clone()?;
                        let registers: Vec<&str> = source.split(", ").collect();
                        if registers.len() != *offsets.last()? {
                            return None;
                        }
                        Some(registers[start..end].join(", "))
                    }
                }
                CheckedSourceBodyKind::Field { record, index } => {
                    let record_ty = &body.expressions[*record as usize].ty;
                    if let CheckedSourceBodyKind::Instruction { .. } =
                        body.expressions[*record as usize].kind
                    {
                        let registers = self.record_registers(record_ty)?.len();
                        let source = values.get(*record as usize)?.clone()?;
                        if registers == 1 {
                            values[id] = Some(source);
                        } else {
                            let name = format!("%f{id}");
                            let ty = register_struct(registers);
                            ops.push(format!(
                                "    {name} = llvm.extractvalue {source}[{index}] : {ty}"
                            ));
                            values[id] = Some(name);
                        }
                        continue;
                    }
                    if self.slotted_record(record_ty).is_some() {
                        // A field of a record in a slot is read from it.
                        let (offset, width) = self.field_slot(record_ty, *index as usize)?;
                        let source = values.get(*record as usize)?.clone()?;
                        ops.push(format!(
                            "    %fb{id} = llvm.inttoptr {source} : i64 to !llvm.ptr"
                        ));
                        let mut parts = Vec::new();
                        for k in 0..width {
                            let at = offset + k;
                            ops.push(format!("    %fp{id}_{k} = llvm.getelementptr %fb{id}[{at}] : (!llvm.ptr) -> !llvm.ptr, i64"));
                            ops.push(format!(
                                "    %fv{id}_{k} = llvm.load %fp{id}_{k} : !llvm.ptr -> i64"
                            ));
                            parts.push(format!("%fv{id}_{k}"));
                        }
                        Some(parts.join(", "))
                    } else if self.domain_of(record_ty).is_none() {
                        // A field of a record of numbers is its slice of the registers.
                        let (offset, width) = self.field_slot(record_ty, *index as usize)?;
                        let source = values.get(*record as usize)?.clone()?;
                        let parts: Vec<&str> = source.split(", ").collect();
                        Some(parts.get(offset..offset + width)?.join(", "))
                    } else if *index != 0 {
                        return None;
                    } else {
                        values.get(*record as usize)?.clone()
                    }
                }
                // A payload field of a sum: its slice of the registers after the tag.
                CheckedSourceBodyKind::VariantField {
                    subject,
                    schema,
                    index,
                } => {
                    let fields = &self.file.aggregates.get(*schema as usize)?.fields;
                    let mut offset = 1;
                    for field in fields.get(..*index as usize)? {
                        offset += self.field_width(field)?;
                    }
                    let width = self.field_width(fields.get(*index as usize)?)?;
                    let source = values.get(*subject as usize)?.clone()?;
                    let parts: Vec<&str> = source.split(", ").collect();
                    Some(parts.get(offset..offset + width)?.join(", "))
                }
                CheckedSourceBodyKind::Wrap { operand } => {
                    let SourceType::Integer { lower, upper } = expr.ty else {
                        return None;
                    };
                    let operand_ty = &body.expressions[*operand as usize].ty;
                    let x = values.get(*operand as usize)?.clone()?;
                    let bounds = (lower.to_i128()?, upper.to_i128()?);
                    if is_wide(operand_ty) {
                        // Reducing a 128-bit number into the signed 64-bit range is cutting it.
                        if bounds != (i128::from(i64::MIN), i128::from(i64::MAX)) {
                            return None;
                        }
                        let name = format!("%cut{id}");
                        ops.push(format!("    {name} = arith.trunci {x} : i128 to i64"));
                        values[id] = Some(name);
                        continue;
                    }
                    lower_wrap(id, bounds, operand_ty, &x, ops)?
                }
                // Both branches are computed, so a branch that would run an instruction cannot
                // be selected from: it would run when it is not taken.
                CheckedSourceBodyKind::Select {
                    comparison,
                    left,
                    right,
                    then_branch,
                    else_branch,
                    region,
                } => {
                    if let Some(region) = region
                        && self.range_effectful(body, region.then_start as usize..id)
                    {
                        // An arm that runs an instruction must run it only when taken, so its
                        // nodes go in a branch. Arms a chain of `and`/`or` shares cannot move.
                        let width = self.register_width(&expr.ty)?;
                        if !region.exclusive {
                            return None;
                        }
                        compare_nodes(*comparison, (*left, *right), body, values, id, ops)?;
                        self.lower_branches(
                            body,
                            id,
                            (
                                region.then_start as usize..region.else_start as usize,
                                *then_branch,
                            ),
                            (region.else_start as usize..id, *else_branch),
                            (width, &expr.ty),
                            nesting,
                            values,
                            ops,
                            declarations,
                        )?
                    } else {
                        // With no region the arms are values only, and a value that reaches an
                        // instruction cannot be selected from: both would run.
                        if region.is_none()
                            && self.reaches_effect(body, [*then_branch, *else_branch])
                        {
                            return None;
                        }
                        if self.register_width(&expr.ty) == Some(0) {
                            // A unit choice run for nothing it computes has no value to select.
                            None
                        } else {
                            let then_value = values.get(*then_branch as usize)?.clone()?;
                            let else_value = values.get(*else_branch as usize)?.clone()?;
                            let then_value = self.coerce(
                                &then_value,
                                &body.expressions[*then_branch as usize].ty,
                                &expr.ty,
                                id * 64,
                                ops,
                            )?;
                            let else_value = self.coerce(
                                &else_value,
                                &body.expressions[*else_branch as usize].ty,
                                &expr.ty,
                                id * 64 + 1,
                                ops,
                            )?;
                            compare_nodes(*comparison, (*left, *right), body, values, id, ops)?;
                            if is_wide(&expr.ty) {
                                // A choice of numbers one of which needs 128 bits: all are held in 128.
                                let then_value = widened(
                                    &then_value,
                                    &body.expressions[*then_branch as usize].ty,
                                    id * 4,
                                    ops,
                                );
                                let else_value = widened(
                                    &else_value,
                                    &body.expressions[*else_branch as usize].ty,
                                    id * 4 + 1,
                                    ops,
                                );
                                let name = format!("%s{id}");
                                ops.push(format!(
                                "    {name} = arith.select %c{id}, {then_value}, {else_value} : i128"
                            ));
                                Some(name)
                            } else {
                                self.select_registers(
                                    &format!("s{id}"),
                                    &format!("%c{id}"),
                                    &then_value,
                                    &else_value,
                                    ops,
                                )
                            }
                        }
                    }
                }
                // A `when` over variants that are only tags: one compare and select per arm but
                // the last, which every other tag falls to. Arms are computed together, so an
                // arm that would run an instruction cannot be lowered.
                CheckedSourceBodyKind::Match {
                    subject,
                    arms,
                    region,
                } => {
                    if let Some(region) = region
                        && self.range_effectful(body, region.member_start as usize..id)
                    {
                        // A test of one member whose arm runs an instruction: two branches. Both
                        // yield one number, or nothing (a unit `when` run for its effects).
                        let width = self.register_width(&expr.ty)?;
                        let member = arms.iter().find(|arm| arm.body == region.member_body)?;
                        let others: Vec<u32> = arms
                            .iter()
                            .filter(|arm| arm.body != region.member_body)
                            .map(|arm| arm.body)
                            .collect();
                        let other = *others.first()?;
                        if others.iter().any(|body| *body != other) {
                            return None;
                        }
                        let subject_ty = &body.expressions[*subject as usize].ty;
                        if !matches!(subject_ty, SourceType::Sum { .. }) {
                            return None;
                        }
                        let tag = self.tag(member.schema)?;
                        // A sum with payloads travels as its tag, then the payload registers.
                        let subject = values.get(*subject as usize)?.clone()?;
                        let tag_register = subject.split(", ").next()?.to_owned();
                        ops.push(format!("    %mt{id} = arith.constant {tag} : i64"));
                        ops.push(format!(
                            "    %c{id} = arith.cmpi eq, {tag_register}, %mt{id} : i64"
                        ));
                        self.lower_branches(
                            body,
                            id,
                            (
                                region.member_start as usize..region.other_start as usize,
                                region.member_body,
                            ),
                            (region.other_start as usize..id, other),
                            (width, &expr.ty),
                            nesting,
                            values,
                            ops,
                            declarations,
                        )?
                    } else {
                        if region.is_none()
                            && self.reaches_effect(body, arms.iter().map(|arm| arm.body))
                        {
                            return None;
                        }
                        if self.register_width(&expr.ty)? == 0 {
                            // A unit `when` whose arms run nothing has no value to choose.
                            None
                        } else {
                            let subject_ty = &body.expressions[*subject as usize].ty;
                            if !matches!(subject_ty, SourceType::Sum { .. }) {
                                return None;
                            }
                            let subject = values.get(*subject as usize)?.clone()?;
                            let tag_register = subject.split(", ").next()?.to_owned();
                            let (last, rest) = arms.split_last()?;
                            let arm_value = |arm: &typecheck::CheckedSourceMatchArm,
                                             ops: &mut Vec<String>,
                                             k: usize| {
                                let value = values.get(arm.body as usize)?.clone()?;
                                let ty = &body.expressions[arm.body as usize].ty;
                                self.coerce(&value, ty, &expr.ty, id * 64 + k, ops)
                            };
                            let mut chosen = arm_value(last, ops, rest.len())?;
                            for (index, arm) in rest.iter().enumerate().rev() {
                                let tag = self.tag(arm.schema)?;
                                let body_value = arm_value(arm, ops, index)?;
                                ops.push(format!(
                                    "    %m{id}_{index}t = arith.constant {tag} : i64"
                                ));
                                ops.push(format!(
                                "    %m{id}_{index}c = arith.cmpi eq, {tag_register}, %m{id}_{index}t : i64"
                            ));
                                chosen = self.select_registers(
                                    &format!("m{id}_{index}s"),
                                    &format!("%m{id}_{index}c"),
                                    &body_value,
                                    &chosen,
                                    ops,
                                )?;
                            }
                            Some(chosen)
                        }
                    }
                }
                // Text has no machine form and so no value: whatever would need one is left
                // unlowered, but a text nothing reads costs nothing.
                CheckedSourceBodyKind::Text(_) => None,
                // The value is unchanged; only its declared range differs.
                CheckedSourceBodyKind::Widen { operand } => {
                    let from = &body.expressions[*operand as usize].ty;
                    let value = values.get(*operand as usize)?.clone();
                    // A number held in 128 bits that is now known to fit a register is cut back.
                    if let Some(value) = &value
                        && is_wide(from)
                        && !is_wide(&expr.ty)
                    {
                        let name = format!("%narrow{id}");
                        ops.push(format!("    {name} = arith.trunci {value} : i128 to i64"));
                        values[id] = Some(name);
                        continue;
                    }
                    value
                }
                // Set by the loop that lowers the pass.
                CheckedSourceBodyKind::LoopCounter | CheckedSourceBodyKind::LoopCarried => {
                    values[id].clone()
                }
                CheckedSourceBodyKind::Loop {
                    start,
                    end,
                    counter,
                    body_start,
                    carried,
                } => {
                    self.lower_loop(
                        body,
                        id,
                        (*start, *end, *counter, *body_start),
                        carried,
                        nesting,
                        values,
                        ops,
                        declarations,
                    )?;
                    None
                }
                CheckedSourceBodyKind::While {
                    comparison,
                    negated,
                    left,
                    right,
                    body_start,
                    split,
                    carried,
                    exit,
                } => {
                    self.lower_while(
                        body,
                        id,
                        (*comparison, *negated, *left, *right, *body_start, *split),
                        *exit,
                        carried,
                        nesting,
                        values,
                        ops,
                        declarations,
                    )?;
                    None
                }
                CheckedSourceBodyKind::LoopResult { looped, index } => {
                    let (CheckedSourceBodyKind::Loop { carried, .. }
                    | CheckedSourceBodyKind::While { carried, .. }) =
                        &body.expressions[*looped as usize].kind
                    else {
                        return None;
                    };
                    // A carried record is several registers: this one starts after the earlier ones.
                    let widths = self.carry_widths(body, carried)?;
                    let total: usize = widths.iter().sum();
                    let offset: usize = widths[..*index as usize].iter().sum();
                    let width = widths[*index as usize];
                    Some(if total == 1 {
                        format!("%loop{looped}")
                    } else {
                        (offset..offset + width)
                            .map(|k| format!("%loop{looped}#{k}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                }
                // A block is its result: the effects before it are nodes of their own, which
                // were lowered in order.
                CheckedSourceBodyKind::Block { result, .. } => {
                    values.get(*result as usize)?.clone()
                }
                CheckedSourceBodyKind::Unit | CheckedSourceBodyKind::Return { .. } => None,
                // Replaced by a number when the instance was made; one left here has no type.
                CheckedSourceBodyKind::Layout { .. } => return None,
                // The array moved on by `start` elements: its address plus `start` times the size of
                // one element, scaled as a load of that element would be.
                // A slot in the entry block, filled with the element: one per array made, so it is
                // not made inside a loop (a later pass would write over an earlier pass's array).
                CheckedSourceBodyKind::RepeatArray { element, count } => {
                    if inside_loop(body, id) {
                        return None;
                    }
                    let (bits, _) = self.owned_array(&expr.ty)?;
                    let bytes = u64::from(bits / 8) * u64::from(*count);
                    let mut slots = self.slots.borrow_mut();
                    slots.push(format!(
                        "    %slotn{id} = llvm.mlir.constant({bytes} : i64) : i64"
                    ));
                    slots.push(format!(
                        "    %slot{id} = llvm.alloca %slotn{id} x i8 : (i64) -> !llvm.ptr"
                    ));
                    drop(slots);
                    let value = values.get(*element as usize)?.clone()?;
                    let stored = if bits == 64 {
                        value
                    } else {
                        ops.push(format!(
                            "    %fill{id} = arith.trunci {value} : i64 to i{bits}"
                        ));
                        format!("%fill{id}")
                    };
                    ops.push(format!("    %flb{id} = arith.constant 0 : index"));
                    ops.push(format!("    %fub{id} = arith.constant {count} : index"));
                    ops.push(format!("    %fst{id} = arith.constant 1 : index"));
                    ops.push(format!(
                        "    scf.for %fi{id} = %flb{id} to %fub{id} step %fst{id} {{"
                    ));
                    ops.push(format!(
                        "      %fix{id} = arith.index_cast %fi{id} : index to i64"
                    ));
                    ops.push(format!("      %fat{id} = llvm.getelementptr %slot{id}[%fix{id}] : (!llvm.ptr, i64) -> !llvm.ptr, i{bits}"));
                    ops.push(format!(
                        "      llvm.store {stored}, %fat{id} : i{bits}, !llvm.ptr"
                    ));
                    ops.push("    }".to_owned());
                    ops.push(format!(
                        "    %arr{id} = llvm.ptrtoint %slot{id} : !llvm.ptr to i64"
                    ));
                    Some(format!("%arr{id}"))
                }
                CheckedSourceBodyKind::SubArray { base, start } => {
                    let SourceType::Array { id: array } = expr.ty else {
                        return None;
                    };
                    let element = *self.file.instances.get(array as usize)?.arguments.first()?;
                    let (bits, _) = self.memory_element(&element)?;
                    let address = values.get(*base as usize)?.clone()?;
                    let start = values.get(*start as usize)?.clone()?;
                    ops.push(format!(
                        "    %sbase{id} = llvm.inttoptr {address} : i64 to !llvm.ptr"
                    ));
                    ops.push(format!(
                        "    %smoved{id} = llvm.getelementptr %sbase{id}[{start}] : (!llvm.ptr, i64) -> !llvm.ptr, i{bits}"
                    ));
                    ops.push(format!(
                        "    %sint{id} = llvm.ptrtoint %smoved{id} : !llvm.ptr to i64"
                    ));
                    Some(format!("%sint{id}"))
                }
                // The literal's length, and the address of its bytes in a read-only global.
                CheckedSourceBodyKind::TextBytes { text } => {
                    let index = {
                        let mut texts = self.texts.borrow_mut();
                        match texts
                            .iter()
                            .position(|known| known.as_slice() == text.as_bytes())
                        {
                            Some(index) => index,
                            None => {
                                texts.push(text.as_bytes().to_vec());
                                texts.len() - 1
                            }
                        }
                    };
                    ops.push(format!(
                        "    %tlen{id} = arith.constant {} : i64",
                        text.len()
                    ));
                    ops.push(format!(
                        "    %tptr{id} = llvm.mlir.addressof @gin.text.{index} : !llvm.ptr"
                    ));
                    ops.push(format!(
                        "    %taddr{id} = llvm.ptrtoint %tptr{id} : !llvm.ptr to i64"
                    ));
                    Some(format!("%tlen{id}, %taddr{id}"))
                }
                // Refused up front in `lower_function`; never lowered to anything.
                CheckedSourceBodyKind::Hole => return None,
                // Resolved to a call of the concrete provider before lowering; none remains here.
                CheckedSourceBodyKind::BoundCall { .. } => return None,
            };
            values[id] = value;
        }
        Some(())
    }

    /// `for counter in start to end`, inclusive, as an `scf.for` that yields what it carries.
    #[allow(clippy::too_many_arguments)] // the lowering walk threads its state explicitly
    fn lower_loop(
        &self,
        body: &CheckedSourceBody,
        id: usize,
        (start, end, counter, body_start): (u32, u32, u32, u32),
        carried: &[typecheck::CheckedSourceLoopCarry],
        nesting: &[u32],
        values: &mut Vec<Option<String>>,
        ops: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
    ) -> Option<()> {
        if ![start, end, counter]
            .into_iter()
            .all(|node| fits_i64(&body.expressions[node as usize].ty))
        {
            return None;
        }
        // The upper bound of `scf.for` is exclusive, so the loop is run to `end + 1`; that
        // would wrap if the domain of `end` reaches the largest 64-bit number.
        let SourceType::Integer { upper, .. } = body.expressions[end as usize].ty else {
            return None;
        };
        if upper.to_i128()? >= i128::from(i64::MAX) {
            return None;
        }
        // Each carried value is one register or, for a record, several.
        let widths = self.carry_widths(body, carried)?;
        for (carry, width) in carried.iter().zip(&widths) {
            if *width == 1
                && matches!(
                    body.expressions[carry.placeholder as usize].ty,
                    SourceType::Integer { .. }
                )
                && !fits_i64(&body.expressions[carry.placeholder as usize].ty)
            {
                return None;
            }
        }
        let total: usize = widths.iter().sum();
        let named = |prefix: &str, index: usize| -> Vec<String> {
            if widths[index] == 1 {
                vec![format!("%{prefix}{id}_{index}")]
            } else {
                (0..widths[index])
                    .map(|r| format!("%{prefix}{id}_{index}_{r}"))
                    .collect()
            }
        };
        let low = values.get(start as usize)?.clone()?;
        let high = values.get(end as usize)?.clone()?;
        ops.push(format!("    %one{id} = arith.constant 1 : i64"));
        ops.push(format!("    %last{id} = arith.addi {high}, %one{id} : i64"));
        ops.push(format!(
            "    %lb{id} = arith.index_cast {low} : i64 to index"
        ));
        ops.push(format!(
            "    %ub{id} = arith.index_cast %last{id} : i64 to index"
        ));
        ops.push(format!("    %step{id} = arith.constant 1 : index"));
        // A loop that carries nothing has no results to name.
        let mut header = if total > 0 {
            format!("    %loop{id}")
        } else {
            "    ".to_owned()
        };
        if total > 1 {
            header.push_str(&format!(":{total}"));
        }
        if total > 0 {
            header.push_str(" = ");
        }
        header.push_str(&format!(
            "scf.for %iv{id} = %lb{id} to %ub{id} step %step{id}"
        ));
        if total > 0 {
            let mut accumulators = Vec::new();
            for (index, carry) in carried.iter().enumerate() {
                let initial = values.get(carry.initial as usize)?.clone()?;
                let parts: Vec<&str> = initial.split(", ").collect();
                if parts.len() != widths[index] {
                    return None;
                }
                for (name, part) in named("acc", index).into_iter().zip(parts) {
                    accumulators.push(format!("{name} = {part}"));
                }
            }
            header.push_str(&format!(
                " iter_args({}) -> ({})",
                accumulators.join(", "),
                vec!["i64"; total].join(", ")
            ));
        }
        header.push_str(" {");
        ops.push(header);
        ops.push(format!(
            "      %cnt{id} = arith.index_cast %iv{id} : index to i64"
        ));
        values[counter as usize] = Some(format!("%cnt{id}"));
        for (index, carry) in carried.iter().enumerate() {
            values[carry.placeholder as usize] = Some(named("acc", index).join(", "));
        }
        self.lower_range(
            body,
            body_start as usize..id,
            nesting[id] + 1,
            nesting,
            values,
            ops,
            declarations,
        )?;
        if total > 0 {
            let mut nexts = Vec::new();
            for (index, carry) in carried.iter().enumerate() {
                let next = values.get(carry.next as usize)?.clone()?;
                let parts: Vec<String> = next.split(", ").map(str::to_owned).collect();
                if parts.len() != widths[index] {
                    return None;
                }
                nexts.extend(parts);
            }
            ops.push(format!(
                "      scf.yield {} : {}",
                nexts.join(", "),
                vec!["i64"; total].join(", ")
            ));
        }
        ops.push("    }".to_string());
        Some(())
    }

    /// `while left <comparison> right do body` as an `scf.while` that carries what the body
    /// assigns. The condition is computed from the carried values in the `before` region and
    /// forwarded with `scf.condition`; the body runs in the `do` region on what it was given.
    /// A region cannot see the other's values, so each names the carried values for itself.
    #[allow(clippy::too_many_arguments)] // the lowering walk threads its state explicitly
    fn lower_while(
        &self,
        body: &CheckedSourceBody,
        id: usize,
        (comparison, negated, left, right, body_start, split): (
            typecheck::InfixOperator,
            bool,
            u32,
            u32,
            u32,
            u32,
        ),
        exit: Option<u32>,
        carried: &[typecheck::CheckedSourceLoopCarry],
        nesting: &[u32],
        values: &mut Vec<Option<String>>,
        ops: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
    ) -> Option<()> {
        let count = carried.len();
        if !fits_i64(&body.expressions[left as usize].ty)
            || !fits_i64(&body.expressions[right as usize].ty)
        {
            return None;
        }
        // Each carried value is one register or, for a record, several.
        let widths = self.carry_widths(body, carried)?;
        for (carry, width) in carried.iter().zip(&widths) {
            if *width == 1 && !fits_i64(&body.expressions[carry.placeholder as usize].ty) {
                // A domain or pointer is one register; a plain number must fit one.
                if matches!(
                    body.expressions[carry.placeholder as usize].ty,
                    SourceType::Integer { .. }
                ) {
                    return None;
                }
            }
        }
        let total: usize = widths.iter().sum();
        let predicate = match (comparison, negated) {
            (typecheck::InfixOperator::Equal, false) => "eq",
            (typecheck::InfixOperator::Equal, true) => "ne",
            (typecheck::InfixOperator::Less, _) => "slt",
            (typecheck::InfixOperator::LessOrEqual, _) => "sle",
            (typecheck::InfixOperator::Greater, _) => "sgt",
            (typecheck::InfixOperator::GreaterOrEqual, _) => "sge",
            _ => return None,
        };
        let types = vec!["i64"; total].join(", ");
        // The registers of carry `index`, named `prefix{id}_{index}` (`_{r}` when several).
        let named = |prefix: &str, index: usize| -> Vec<String> {
            if widths[index] == 1 {
                vec![format!("%{prefix}{id}_{index}")]
            } else {
                (0..widths[index])
                    .map(|r| format!("%{prefix}{id}_{index}_{r}"))
                    .collect()
            }
        };
        let mut initials = Vec::new();
        for (index, carry) in carried.iter().enumerate() {
            let initial = values.get(carry.initial as usize)?.clone()?;
            let parts: Vec<&str> = initial.split(", ").collect();
            if parts.len() != widths[index] {
                return None;
            }
            for (name, part) in named("acc", index).into_iter().zip(parts) {
                initials.push(format!("{name} = {part}"));
            }
        }
        // A loop that carries nothing has no results to name and no types to forward.
        let mut header = if total > 0 {
            format!("    %loop{id}")
        } else {
            "   ".to_owned()
        };
        if total > 1 {
            header.push_str(&format!(":{total}"));
        }
        if total > 0 {
            header.push_str(&format!(
                " = scf.while ({}) : ({types}) -> ({types}) {{",
                initials.join(", ")
            ));
        } else {
            header.push_str(" scf.while : () -> () {");
        }
        ops.push(header);
        // The condition, from the values carried into the pass.
        for (index, carry) in carried.iter().enumerate() {
            values[carry.placeholder as usize] = Some(named("acc", index).join(", "));
        }
        self.lower_range(
            body,
            body_start as usize..split as usize,
            nesting[id] + 1,
            nesting,
            values,
            ops,
            declarations,
        )?;
        let left = values.get(left as usize)?.clone()?;
        let right = values.get(right as usize)?.clone()?;
        ops.push(format!(
            "      %wc{id} = arith.cmpi {predicate}, {left}, {right} : i64"
        ));
        // A `return` in the body sets the flag; the loop goes on only while it is still zero.
        let mut condition = format!("%wc{id}");
        if let Some(flag) = exit {
            ops.push(format!("      %wz{id} = arith.constant 0 : i64"));
            ops.push(format!(
                "      %wn{id} = arith.cmpi eq, {}, %wz{id} : i64",
                named("acc", flag as usize).join(", ")
            ));
            ops.push(format!("      %wa{id} = arith.andi %wc{id}, %wn{id} : i1"));
            condition = format!("%wa{id}");
        }
        let forwarded = (0..count)
            .flat_map(|index| named("acc", index))
            .collect::<Vec<_>>()
            .join(", ");
        if total > 0 {
            ops.push(format!(
                "      scf.condition({condition}) {forwarded} : {types}"
            ));
        } else {
            ops.push(format!("      scf.condition({condition})"));
        }
        // The body, on the values the condition let through.
        let arguments = (0..count)
            .flat_map(|index| named("arg", index))
            .map(|name| format!("{name}: i64"))
            .collect::<Vec<_>>()
            .join(", ");
        ops.push("    } do {".to_string());
        if total > 0 {
            ops.push(format!("    ^bb0({arguments}):"));
        }
        for (index, carry) in carried.iter().enumerate() {
            values[carry.placeholder as usize] = Some(named("arg", index).join(", "));
        }
        self.lower_range(
            body,
            split as usize..id,
            nesting[id] + 1,
            nesting,
            values,
            ops,
            declarations,
        )?;
        let mut nexts = Vec::new();
        for (index, carry) in carried.iter().enumerate() {
            let next = values.get(carry.next as usize)?.clone()?;
            let parts: Vec<String> = next.split(", ").map(str::to_owned).collect();
            if parts.len() != widths[index] {
                return None;
            }
            nexts.extend(parts);
        }
        if total > 0 {
            ops.push(format!("      scf.yield {} : {types}", nexts.join(", ")));
        } else {
            ops.push("      scf.yield".to_string());
        }
        ops.push("    }".to_string());
        Some(())
    }

    /// The registers each carried value of a loop takes.
    fn carry_widths(
        &self,
        body: &CheckedSourceBody,
        carried: &[typecheck::CheckedSourceLoopCarry],
    ) -> Option<Vec<usize>> {
        carried
            .iter()
            .map(|carry| self.register_width(&body.expressions[carry.placeholder as usize].ty))
            .collect()
    }

    /// The width in bits an element of type `ty` takes in memory, and whether it is signed: the
    /// narrowest of 8, 16, 32 and 64 bits that holds its range. A raw address takes 64.
    /// An owned array of a known length whose element is a number in memory: its element's width
    /// in bits and its length.
    fn owned_array(&self, ty: &SourceType) -> Option<(u32, u32)> {
        let SourceType::Array { id } = ty else {
            return None;
        };
        let instance = self.file.instances.get(*id as usize)?;
        let CheckedSourceInstanceHead::Array { extent } = instance.head else {
            return None;
        };
        let (bits, _) = self.memory_element(instance.arguments.first()?)?;
        Some((bits, extent))
    }

    /// The owned arrays a value of `ty` holds, by the register that holds each one's address and
    /// its size in bytes. `None` when an owned array sits where no register names it (inside a
    /// sum): such a value is not passed at all, so no address can outlive its slot.
    fn array_slots(&self, ty: &SourceType) -> Option<Vec<(usize, u64)>> {
        let mut found = Vec::new();
        self.collect_array_slots(ty, 0, &mut found, 0)?;
        Some(found)
    }

    fn collect_array_slots(
        &self,
        ty: &SourceType,
        base: usize,
        found: &mut Vec<(usize, u64)>,
        depth: usize,
    ) -> Option<()> {
        if depth > 8 {
            return None;
        }
        if let Some((bits, extent)) = self.owned_array(ty) {
            found.push((base, u64::from(bits / 8) * u64::from(extent)));
            return Some(());
        }
        if let Some(width) = self.slotted_record(ty) {
            found.push((base, 8 * width as u64));
            return Some(());
        }
        match ty {
            SourceType::Aggregate { id } => {
                let instance = self.file.instances.get(*id as usize)?;
                let CheckedSourceInstanceHead::Nominal { schema } = instance.head else {
                    return Some(());
                };
                let aggregate = self.file.aggregates.get(schema as usize)?;
                if aggregate.kind != AggregateKind::Record {
                    return Some(());
                }
                let mut offset = base;
                for field in &aggregate.fields {
                    let field_ty = match field.ty {
                        SourceType::Generic { index } => *instance.arguments.get(index as usize)?,
                        other => other,
                    };
                    if field.borrow.is_none() {
                        self.collect_array_slots(&field_ty, offset, found, depth + 1)?;
                    }
                    offset +=
                        if field.borrow.is_some() && matches!(field.ty, SourceType::Array { .. }) {
                            1
                        } else {
                            self.register_width(&field_ty).unwrap_or(0)
                        };
                }
                Some(())
            }
            SourceType::Sum { .. } => {
                // A member holding an owned array would carry its address out: refused.
                let schema = self.schema_of(ty)?;
                for variant in &self.file.aggregates.get(schema as usize)?.variants {
                    let mut inner = Vec::new();
                    self.collect_array_slots(variant, 0, &mut inner, depth + 1)?;
                    if !inner.is_empty() {
                        return None;
                    }
                }
                Some(())
            }
            _ => Some(()),
        }
    }

    fn memory_element(&self, ty: &SourceType) -> Option<(u32, bool)> {
        let range = match ty {
            SourceType::Integer { lower, upper } => Some((*lower, *upper)),
            SourceType::Pointer { .. } => return Some((64, false)),
            _ => {
                let schema = self.domain_of(ty)?;
                match self.file.aggregates.get(schema)?.fields.first()?.ty {
                    SourceType::Integer { lower, upper } => Some((lower, upper)),
                    _ => None,
                }
            }
        }?;
        let element = SourceType::Integer {
            lower: range.0,
            upper: range.1,
        };
        if !fits_register(&element) {
            return None;
        }
        let (lower, upper) = (range.0.to_i128()?, range.1.to_i128()?);
        let signed = lower < 0;
        let fits = |bits: u32| {
            if signed {
                lower >= -(1i128 << (bits - 1)) && upper < (1i128 << (bits - 1))
            } else {
                lower >= 0 && upper < (1i128 << bits)
            }
        };
        [8, 16, 32, 64]
            .into_iter()
            .find(|bits| fits(*bits))
            .map(|bits| (bits, signed))
    }

    /// The address of element `index` of `pointer`, an `i64` register holding an address.
    fn memory_address(
        &self,
        id: usize,
        pointer: &str,
        index: &str,
        bits: u32,
        ops: &mut Vec<String>,
    ) -> String {
        ops.push(format!(
            "    %base{id} = llvm.inttoptr {pointer} : i64 to !llvm.ptr"
        ));
        ops.push(format!(
            "    %addr{id} = llvm.getelementptr %base{id}[{index}] : (!llvm.ptr, i64) -> !llvm.ptr, i{bits}"
        ));
        format!("%addr{id}")
    }

    /// One `llvm.inline_asm`. Outputs number first, then the symbols in the order the
    /// function names them, then the registers going in; `$name` in the text becomes the
    /// number of that name's operand.
    fn lower_instruction(
        &self,
        call: Instruction<'_>,
        (input, output): (&typecheck::CheckedSourceBodyExpr, &SourceType),
        values: &[Option<String>],
        ops: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
    ) -> Option<Option<String>> {
        let CheckedSourceBodyKind::Construct { schema, fields } = &input.kind else {
            return None;
        };
        let inputs = self.registers(*schema)?;
        let outputs = self.record_registers(output)?;
        if inputs.len() != fields.len() {
            return None;
        }
        let mut constraints: Vec<String> =
            outputs.iter().map(|name| format!("={{{name}}}")).collect();
        let mut operands = Vec::new();
        let mut types = Vec::new();
        for (index, (_, symbol)) in call.symbols.iter().enumerate() {
            let address = format!("%symbol{}_{index}", call.id);
            self.symbol_address(symbol, &address, ops, declarations)?;
            constraints.push("s".to_owned());
            operands.push(address);
            types.push("!llvm.ptr".to_owned());
        }
        for (register, field) in inputs.iter().zip(fields) {
            constraints.push(format!("{{{register}}}"));
            operands.push(values.get(*field as usize)?.clone()?);
            types.push("i64".to_owned());
        }
        constraints.extend(["~{memory}".to_owned(), "~{cc}".to_owned()]);
        // `;` separates instructions in the text, and each goes on its own line: a newline is
        // what every assembler accepts, and some (Apple's) read `;` as a comment. The text
        // sits in an MLIR string, where a quote and a newline are written as their codes.
        let template = place_symbols(call.text, outputs.len(), call.symbols)?
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| part.replace('"', "\\22"))
            .collect::<Vec<_>>()
            .join("\\0A");
        let (name, result) = match outputs.len() {
            0 => (String::new(), "()".to_owned()),
            1 => (format!("%asm{} = ", call.id), "i64".to_owned()),
            count => (format!("%asm{} = ", call.id), register_struct(count)),
        };
        ops.push(format!(
            "    {name}llvm.inline_asm has_side_effects \"{template}\", \"{}\" {} : ({}) -> {result}",
            constraints.join(","),
            operands.join(", "),
            types.join(", "),
        ));
        Some((!outputs.is_empty()).then(|| format!("%asm{}", call.id)))
    }

    /// The address of the symbol named `symbol` as the pointer `address`. A function this
    /// module defines is taken through a constant of its own type, which the lowering to
    /// LLVM reconciles into its address; any other name is an external symbol, declared
    /// with no signature because only its address is wanted.
    fn symbol_address(
        &self,
        symbol: &str,
        address: &str,
        ops: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
    ) -> Option<()> {
        // A function of this module, or of a module it imports: the address of one that the
        // module also calls must come from the one declaration the call uses.
        let local = self.file.functions.iter().find(|function| {
            function.instruction.is_none()
                && !compile_time_only(self.file, function)
                && self.defined_name(&function.name) == symbol
        });
        let imported = self.dependencies.iter().find_map(|module| {
            let function = module.checked().functions.iter().find(|function| {
                function.instruction.is_none()
                    && !compile_time_only(module.checked(), function)
                    && format!("{}.{}", module.name(), function.name) == symbol
            })?;
            Some((qualified(module.name(), &function.name), function))
        });
        let found = local
            .map(|function| (self.symbol(&function.name), function, false))
            .or_else(|| imported.map(|(target, function)| (target, function, true)));
        if let Some((target, function, declare)) = found {
            let parameters = vec!["i64"; function.parameters.len()].join(", ");
            let result = match &function.result {
                SourceType::Unit => "()".to_owned(),
                ty => self.machine_type(ty)?.to_owned(),
            };
            if declare && !declarations.iter().any(|known| known.symbol == target) {
                declarations.push(Declaration {
                    symbol: target.clone(),
                    parameters: function.parameters.len(),
                    results: self.register_width(&function.result)?,
                    external: false,
                });
            }
            ops.push(format!(
                "    {address}_f = func.constant {target} : ({parameters}) -> {result}"
            ));
            ops.push(format!(
                "    {address} = builtin.unrealized_conversion_cast {address}_f : ({parameters}) -> {result} to !llvm.ptr"
            ));
            return Some(());
        }
        let target = mlir_symbol(symbol);
        if !declarations.iter().any(|known| known.symbol == target) {
            declarations.push(Declaration {
                symbol: target.clone(),
                parameters: 0,
                results: 0,
                external: true,
            });
        }
        ops.push(format!(
            "    {address} = llvm.mlir.addressof {target} : !llvm.ptr"
        ));
        Some(())
    }

    /// The name a function of this module has at link level, without `@`.
    fn defined_name(&self, name: &str) -> String {
        match self.module.is_empty() {
            true => name.to_owned(),
            false => format!("{}.{name}", self.module),
        }
    }

    /// The registers of a record's fields in order, when every field names one and fits it.
    fn registers(&self, schema: u32) -> Option<Vec<&str>> {
        let aggregate = self.file.aggregates.get(schema as usize)?;
        let fields = aggregate.fields.iter();
        fields
            .map(|field| {
                self.machine_type(&field.ty)?;
                field.register.as_deref()
            })
            .collect()
    }

    fn record_registers(&self, ty: &SourceType) -> Option<Vec<&str>> {
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let CheckedSourceInstanceHead::Nominal { schema } =
            self.file.instances.get(*id as usize)?.head
        else {
            return None;
        };
        self.registers(schema)
    }

    fn lower_call(
        &self,
        callee: CheckedSourceCallee,
        operands: &[String],
        results: Option<(&str, usize)>,
        ops: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
    ) -> Option<()> {
        let (symbol, target) = self.callee(callee)?;
        // A record is several registers, written joined: pass each on its own.
        let operands: Vec<String> = operands
            .iter()
            .flat_map(|operand| operand.split(", ").map(str::to_owned))
            .collect();
        let mut operands = operands;
        let expected = self.callee_parameter_width(callee, target.parameters.iter().copied())?;
        if expected != operands.len() {
            return None;
        }
        // A result holding owned arrays is given back in slots of this function's.
        let slots_wanted = match callee {
            CheckedSourceCallee::Dependency { dependency, .. } if self.module_world_differs() => {
                let module = self.dependencies.get(dependency as usize)?;
                let context = Context {
                    module: module.name(),
                    file: module.checked(),
                    dependencies: &[],
                    externals: &BTreeSet::new(),
                    texts: std::cell::RefCell::new(Vec::new()),
                    world: false,
                    slots: std::cell::RefCell::new(Vec::new()),
                };
                context.array_slots(&target.result)?
            }
            _ => self.array_slots(&target.result)?,
        };
        for (_, bytes) in &slots_wanted {
            let mut slots = self.slots.borrow_mut();
            let n = slots.len();
            slots.push(format!(
                "    %rsn{n} = llvm.mlir.constant({bytes} : i64) : i64"
            ));
            slots.push(format!(
                "    %rsp{n} = llvm.alloca %rsn{n} x i8 : (i64) -> !llvm.ptr"
            ));
            drop(slots);
            ops.push(format!(
                "    %rsa{n} = llvm.ptrtoint %rsp{n} : !llvm.ptr to i64"
            ));
            operands.push(format!("%rsa{n}"));
        }
        let operands = &operands[..];
        let foreign = match callee {
            CheckedSourceCallee::Dependency { .. } => true,
            CheckedSourceCallee::Local { function } => {
                self.externals.contains(&(function as usize))
            }
        };
        if foreign && !declarations.iter().any(|known| known.symbol == symbol) {
            declarations.push(Declaration {
                symbol: symbol.clone(),
                parameters: operands.len(),
                results: results.map_or(0, |(_, count)| count),
                external: false,
            });
        }
        let types = vec!["i64"; operands.len()].join(", ");
        let (prefix, result) = match results {
            Some((name, 1)) => (format!("{name} = "), " -> i64".to_string()),
            Some((name, count)) => (
                format!("{name}:{count} = "),
                format!(" -> ({})", vec!["i64"; count].join(", ")),
            ),
            None => (String::new(), " -> ()".to_string()),
        };
        ops.push(format!(
            "    {prefix}func.call {symbol}({}) : ({types}){result}",
            operands.join(", ")
        ));
        Some(())
    }

    fn callee(&self, callee: CheckedSourceCallee) -> Option<(String, &CheckedSourceFunction)> {
        match callee {
            CheckedSourceCallee::Local { function } => {
                let target = self.file.functions.get(function as usize)?;
                Some((self.symbol(&target.name), target))
            }
            CheckedSourceCallee::Dependency {
                dependency,
                function,
            } => {
                let module = self.dependencies.get(dependency as usize)?;
                let target = module.checked().functions.get(function as usize)?;
                Some((qualified(module.name(), &target.name), target))
            }
        }
    }

    /// The schema of the linear or unrestricted domain `ty` names, when it is one.
    fn domain_of(&self, ty: &SourceType) -> Option<usize> {
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let CheckedSourceInstanceHead::Nominal { schema } =
            self.file.instances.get(*id as usize)?.head
        else {
            return None;
        };
        self.is_domain(schema).then_some(schema as usize)
    }

    fn is_domain(&self, schema: u32) -> bool {
        self.file
            .aggregates
            .get(schema as usize)
            .is_some_and(|aggregate| {
                aggregate.kind == AggregateKind::Domain
                    && aggregate
                        .fields
                        .first()
                        .is_some_and(|field| fits_register(&field.ty))
            })
    }

    /// The MLIR type of a value of `ty`: an integer, a domain, which is its integer, or a
    /// value that is only which variant it is, which is its tag.
    /// How many numbers a value of this type is passed as: one for a number, a domain or a
    /// tag, none for unit, and for a record of such the sum of its fields.
    ///
    /// A record travels in registers only up to `REGISTER_LIMIT` numbers; wider values use
    /// stack slots. The representation depends on the type alone so callers and callees
    /// in different modules always agree.
    ///
    /// The limit is 16: wide records are passed by their stack-slot address.
    /// How many numbers a value of `ty` is, with no limit; `None` where it has no register form.
    fn flat_width(&self, ty: &SourceType, depth: usize) -> Option<usize> {
        if depth > 16 {
            return None;
        }
        if matches!(ty, SourceType::Unit) {
            return Some(0);
        }
        if self.machine_type(ty).is_some() {
            return Some(1);
        }
        let SourceType::Aggregate { .. } = ty else {
            return None;
        };
        let aggregate = self.file.aggregates.get(self.schema_of(ty)? as usize)?;
        if aggregate.kind != AggregateKind::Record {
            return None;
        }
        let mut total = 0;
        for field in &aggregate.fields {
            total += if (field.borrow.is_some() && matches!(field.ty, SourceType::Array { .. }))
                || self.owned_array(&field.ty).is_some()
            {
                1
            } else {
                self.flat_width(&field.ty, depth + 1)?
            };
        }
        Some(total)
    }

    /// A field's registers. A borrowed array (`data ref [element; len]`) is the address of its
    /// first element, one register; its length is the sibling field.
    fn field_width(&self, field: &CheckedSourceField) -> Option<usize> {
        if field.borrow.is_some() && matches!(field.ty, SourceType::Array { .. }) {
            return Some(1);
        }
        self.register_width(&field.ty)
    }

    fn register_width(&self, ty: &SourceType) -> Option<usize> {
        if matches!(ty, SourceType::Unit) {
            return Some(0);
        }
        // An owned array of a known length is held in a stack slot and passed by its address.
        if self.owned_array(ty).is_some() {
            return Some(1);
        }
        if self.machine_type(ty).is_some() {
            return Some(1);
        }
        if let SourceType::Sum { .. } = ty {
            return self.sum_width(ty);
        }
        // A bundle of an exit's result and outputs: its parts' registers, no tag.
        if let SourceType::Sequence { .. } = ty {
            return self
                .bundle_offsets(ty)
                .map(|offsets| *offsets.last().unwrap_or(&0));
        }
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let instance = self.file.instances.get(*id as usize)?;
        let CheckedSourceInstanceHead::Nominal { schema } = instance.head else {
            return None;
        };
        let aggregate = self.file.aggregates.get(schema as usize)?;
        if aggregate.kind != AggregateKind::Record {
            return None;
        }
        let total = self.spread_width(ty)?;
        if total > REGISTER_LIMIT {
            // Wider than the registers: held in a stack slot and passed by its address.
            return (total <= SLOT_REGISTERS_MAX).then_some(1);
        }
        (1..=REGISTER_LIMIT).contains(&total).then_some(total)
    }

    /// Where each part of a bundle (a `Sequence`) begins in its registers, then its width.
    fn bundle_offsets(&self, ty: &SourceType) -> Option<Vec<usize>> {
        let SourceType::Sequence { id } = ty else {
            return None;
        };
        let instance = self.file.instances.get(*id as usize)?;
        let mut offsets = vec![0];
        for part in &instance.arguments {
            let next = offsets.last()? + self.register_width(part)?;
            offsets.push(next);
        }
        Some(offsets)
    }

    /// The registers a record's fields take laid out one after another, whatever its class.
    fn spread_width(&self, ty: &SourceType) -> Option<usize> {
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let instance = self.file.instances.get(*id as usize)?;
        let CheckedSourceInstanceHead::Nominal { schema } = instance.head else {
            return None;
        };
        let aggregate = self.file.aggregates.get(schema as usize)?;
        if aggregate.kind != AggregateKind::Record {
            return None;
        }
        let mut total = 0;
        for field in &aggregate.fields {
            total += match field.ty {
                // A field of a type parameter is the type the instance was made with.
                SourceType::Generic { index } if field.borrow.is_none() => {
                    self.register_width(instance.arguments.get(index as usize)?)?
                }
                _ => self.field_width(field)?,
            };
        }
        Some(total)
    }

    /// A record held in a stack slot because it is wider than the registers: its spread width.
    fn slotted_record(&self, ty: &SourceType) -> Option<usize> {
        let width = self.spread_width(ty)?;
        (width > REGISTER_LIMIT && width <= SLOT_REGISTERS_MAX).then_some(width)
    }

    /// A sum that holds payloads travels as its tag then the widest payload any member has,
    /// shorter ones padded with zeros: one shape for every member, so a caller and a callee
    /// agree on it by the type alone. One whose members all have no payload is its tag only.
    fn sum_width(&self, ty: &SourceType) -> Option<usize> {
        let schema = self.schema_of(ty)?;
        let variants = &self.file.aggregates.get(schema as usize)?.variants;
        let mut tags = Vec::new();
        let mut widest = 0;
        for variant in variants {
            let SourceType::Aggregate { .. } = variant else {
                return None;
            };
            let member = self.schema_of(variant)?;
            let tag = self.tag(member)?;
            if tags.contains(&tag) {
                return None;
            }
            tags.push(tag);
            let payload = self.payload_width(member)?;
            widest = widest.max(payload);
        }
        if tags.is_empty() {
            return None;
        }
        let total = 1 + widest;
        (total <= REGISTER_LIMIT).then_some(total)
    }

    /// The registers the fields of member `schema` take together.
    fn payload_width(&self, schema: u32) -> Option<usize> {
        let aggregate = self.file.aggregates.get(schema as usize)?;
        // A domain member of a sum is its number: one register.
        if aggregate.kind == AggregateKind::Domain && aggregate.parameters.is_empty() {
            return aggregate
                .fields
                .first()
                .and_then(|field| self.field_width(field))
                .filter(|width| *width == 1);
        }
        if aggregate.kind != AggregateKind::Record || !aggregate.parameters.is_empty() {
            return None;
        }
        let mut total = 0;
        for field in &aggregate.fields {
            total += self.field_width(field)?;
        }
        Some(total)
    }

    /// [`Self::coerce`] for an argument of `callee`. A parameter of another module's function is
    /// typed in that module's instance table, not this file's: a member going where an imported
    /// sum is expected is placed by that module's view of the sum, found through the member's name.
    fn coerce_argument(
        &self,
        callee: CheckedSourceCallee,
        value: &str,
        given: &SourceType,
        expected: &SourceType,
        id: usize,
        ops: &mut Vec<String>,
    ) -> Option<String> {
        let CheckedSourceCallee::Dependency { dependency, .. } = callee else {
            return self.coerce(value, given, expected, id, ops);
        };
        if self.schema_of(expected).is_some() || !matches!(expected, SourceType::Sum { .. }) {
            return self.coerce(value, given, expected, id, ops);
        }
        // A sum going where the foreign sum is expected is already in its shape (a member is not).
        if matches!(given, SourceType::Sum { .. }) {
            return Some(value.to_owned());
        }
        let module = self.dependencies.get(dependency as usize)?;
        let local = self.schema_of(given)?;
        let member_name = &self.file.aggregates.get(local as usize)?.name;
        let foreign = module.checked();
        let member = foreign
            .aggregates
            .iter()
            .position(|aggregate| &aggregate.name == member_name)? as u32;
        let from = foreign.instances.iter().position(|instance| {
            matches!(instance.head, CheckedSourceInstanceHead::Nominal { schema } if schema == member)
                && instance.arguments.is_empty()
        })?;
        let there = Context {
            module: module.name(),
            file: foreign,
            dependencies: &[],
            externals: self.externals,
            texts: std::cell::RefCell::new(Vec::new()),
            world: false,
            slots: std::cell::RefCell::new(Vec::new()),
        };
        there.coerce(
            value,
            &SourceType::Aggregate { id: from as u32 },
            expected,
            id,
            ops,
        )
    }

    /// `value`, held as a `from`, in the shape a `to` is passed in. A member going where its
    /// sum is expected becomes tag, payload and zero padding; anything else must already be so.
    fn coerce(
        &self,
        value: &str,
        from: &SourceType,
        to: &SourceType,
        id: usize,
        ops: &mut Vec<String>,
    ) -> Option<String> {
        if from == to {
            return Some(value.to_owned());
        }
        if !matches!(to, SourceType::Sum { .. }) || !matches!(from, SourceType::Aggregate { .. }) {
            return Some(value.to_owned());
        }
        let member = self.schema_of(from)?;
        let sum = self.schema_of(to)?;
        let holds = self
            .file
            .aggregates
            .get(sum as usize)?
            .variants
            .iter()
            .any(|variant| self.schema_of(variant) == Some(member));
        if !holds {
            return None;
        }
        let width = self.sum_width(to)?;
        let mut parts = Vec::with_capacity(width);
        if self.nullary(member) {
            // Already only its tag.
            parts.push(value.to_owned());
        } else {
            let name = format!("%tag{id}");
            ops.push(format!(
                "    {name} = arith.constant {} : i64",
                self.tag(member)?
            ));
            parts.push(name);
            parts.extend(
                value
                    .split(", ")
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned),
            );
        }
        while parts.len() < width {
            let name = format!("%pad{id}_{}", parts.len());
            ops.push(format!("    {name} = arith.constant 0 : i64"));
            parts.push(name);
        }
        (parts.len() == width).then(|| parts.join(", "))
    }

    /// `then_value` or `else_value` by `condition`, register by register.
    fn select_registers(
        &self,
        id: &str,
        condition: &str,
        then_value: &str,
        else_value: &str,
        ops: &mut Vec<String>,
    ) -> Option<String> {
        let then_parts: Vec<&str> = then_value.split(", ").collect();
        let else_parts: Vec<&str> = else_value.split(", ").collect();
        if then_parts.len() != else_parts.len() {
            return None;
        }
        if then_parts.len() == 1 {
            let name = format!("%{id}");
            ops.push(format!(
                "    {name} = arith.select {condition}, {then_value}, {else_value} : i64"
            ));
            return Some(name);
        }
        let mut names = Vec::new();
        for (k, (then_part, else_part)) in then_parts.iter().zip(&else_parts).enumerate() {
            let name = format!("%{id}_{k}");
            ops.push(format!(
                "    {name} = arith.select {condition}, {then_part}, {else_part} : i64"
            ));
            names.push(name);
        }
        Some(names.join(", "))
    }

    /// Where field `index` of a record of numbers sits in its registers: (first, count).
    fn field_slot(&self, ty: &SourceType, index: usize) -> Option<(usize, usize)> {
        self.register_width(ty)?;
        let SourceType::Aggregate { id } = ty else {
            return None;
        };
        let CheckedSourceInstanceHead::Nominal { schema } =
            self.file.instances.get(*id as usize)?.head
        else {
            return None;
        };
        let fields = &self.file.aggregates.get(schema as usize)?.fields;
        let mut offset = 0;
        for field in fields.get(..index)? {
            offset += self.field_width(field)?;
        }
        Some((offset, self.field_width(fields.get(index)?)?))
    }

    /// The registers a call gives back for the places it was lent `mut`.
    fn mutated_width(&self, callee: CheckedSourceCallee) -> Option<usize> {
        let target = self.callee(callee)?.1;
        let mut chosen = Vec::new();
        for &parameter in &target.mutated {
            chosen.push(*target.parameters.get(parameter as usize)?);
        }
        self.callee_parameter_width(callee, chosen.into_iter())
    }

    /// The registers of parameter types of `callee`, read in the arena they are written in: a
    /// function of another module speaks of that module's instances.
    fn callee_parameter_width(
        &self,
        callee: CheckedSourceCallee,
        types: impl Iterator<Item = SourceType>,
    ) -> Option<usize> {
        match callee {
            CheckedSourceCallee::Dependency { dependency, .. } if self.module_world_differs() => {
                let module = self.dependencies.get(dependency as usize)?;
                let context = Context {
                    module: module.name(),
                    file: module.checked(),
                    dependencies: &[],
                    externals: &BTreeSet::new(),
                    texts: std::cell::RefCell::new(Vec::new()),
                    world: false,
                    slots: std::cell::RefCell::new(Vec::new()),
                };
                types
                    .map(|ty| context.register_width(&ty))
                    .sum::<Option<usize>>()
            }
            _ => types
                .map(|ty| self.register_width(&ty))
                .sum::<Option<usize>>(),
        }
    }

    /// Whether this context lowers a module's own file (a function of a dependency is then
    /// written in the dependency's arena), as opposed to a world file where all instances unify.
    fn module_world_differs(&self) -> bool {
        !self.world
    }

    fn machine_type(&self, ty: &SourceType) -> Option<&'static str> {
        integer_type(ty)
            .or_else(|| self.domain_of(ty).map(|_| "i64"))
            .or_else(|| self.tagged(ty).then_some("i64"))
            // A raw address is one register.
            .or_else(|| matches!(ty, SourceType::Pointer { .. }).then_some("i64"))
    }

    /// A record with no fields and no parameters: nothing to carry but its identity.
    fn nullary(&self, schema: u32) -> bool {
        self.file
            .aggregates
            .get(schema as usize)
            .is_some_and(|aggregate| {
                aggregate.kind == AggregateKind::Record
                    && aggregate.fields.is_empty()
                    && aggregate.parameters.is_empty()
                    && aggregate.variants.is_empty()
            })
    }

    /// The schema of an instance, when it is a nominal one.
    fn schema_of(&self, ty: &SourceType) -> Option<u32> {
        let (SourceType::Aggregate { id } | SourceType::Sum { id }) = ty else {
            return None;
        };
        let CheckedSourceInstanceHead::Nominal { schema } =
            self.file.instances.get(*id as usize)?.head
        else {
            return None;
        };
        Some(schema)
    }

    /// Whether a value of `ty` is only a tag: a record with nothing in it, or a union of
    /// them whose tags all differ.
    fn tagged(&self, ty: &SourceType) -> bool {
        let Some(schema) = self.schema_of(ty) else {
            return false;
        };
        match ty {
            SourceType::Aggregate { .. } => self.nullary(schema),
            SourceType::Sum { .. } => {
                let variants = &self.file.aggregates[schema as usize].variants;
                let mut tags = Vec::new();
                for variant in variants {
                    let Some(member) = self.schema_of(variant) else {
                        return false;
                    };
                    let Some(tag) = self.nullary(member).then(|| self.tag(member)).flatten() else {
                        return false;
                    };
                    if tags.contains(&tag) {
                        return false;
                    }
                    tags.push(tag);
                }
                !tags.is_empty()
            }
            _ => false,
        }
    }

    /// The runtime value of a variant: a 64-bit hash of its own name, so it means the same
    /// in every module and wherever a union expects it. Two variants of one union that hash
    /// alike make the union unlowerable, never ambiguous. A variant with an integer
    /// discriminant (`Success = 0`) is that integer instead: the layout is guaranteed.
    fn tag(&self, schema: u32) -> Option<i64> {
        let aggregate = self.file.aggregates.get(schema as usize)?;
        if let Some(typecheck::Discriminant::Integer(value)) = &aggregate.discriminant {
            return value.to_i64();
        }
        let name = &aggregate.name;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in name.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Some(hash as i64)
    }

    /// Whether any node in `range` runs an instruction, directly or through a call that does.
    fn range_effectful(&self, body: &CheckedSourceBody, range: std::ops::Range<usize>) -> bool {
        range
            .into_iter()
            .any(|id| self.guarded(&body.expressions[id].kind))
    }

    /// Whether running this node where its operands are not the ones it was checked for could be
    /// wrong: an instruction, a call that may reach one or divide, a division, a wide shift.
    fn guarded(&self, kind: &CheckedSourceBodyKind) -> bool {
        match kind {
            CheckedSourceBodyKind::Instruction { .. }
            | CheckedSourceBodyKind::Load { .. }
            | CheckedSourceBodyKind::Store { .. } => true,
            CheckedSourceBodyKind::Call { callee, .. } => self.callee_has_effects(*callee),
            CheckedSourceBodyKind::Binary { operator, .. } => !operator.speculation_safe(),
            _ => false,
        }
    }

    /// `%c{id}` chooses between two arms, each lowered inside its own branch of an `scf.if`
    /// that yields the arm's value: an arm that runs an instruction runs it only when taken.
    #[allow(clippy::too_many_arguments)] // the lowering walk threads its state explicitly
    fn lower_branches(
        &self,
        body: &CheckedSourceBody,
        id: usize,
        (then_range, then_value): (std::ops::Range<usize>, u32),
        (else_range, else_value): (std::ops::Range<usize>, u32),
        (width, result_ty): (usize, &SourceType),
        nesting: &[u32],
        values: &mut Vec<Option<String>>,
        ops: &mut Vec<String>,
        declarations: &mut Vec<Declaration>,
    ) -> Option<Option<String>> {
        let level = nesting[id] + 1;
        if width == 0 {
            // Nothing to yield: the branches run for their effects.
            ops.push(format!("    scf.if %c{id} {{"));
            self.lower_range(body, then_range, level, nesting, values, ops, declarations)?;
            ops.push("    } else {".to_string());
            self.lower_range(body, else_range, level, nesting, values, ops, declarations)?;
            ops.push("    }".to_string());
            return Some(None);
        }
        // Each branch yields the registers of the result: an arm of another member is padded to
        // the result's shape inside its own branch.
        let types = vec!["i64"; width].join(", ");
        let results = if width > 1 {
            format!("%s{id}:{width}")
        } else {
            format!("%s{id}")
        };
        ops.push(format!("    {results} = scf.if %c{id} -> ({types}) {{"));
        for (branch, range, value, last) in [
            (0, then_range, then_value, false),
            (1, else_range, else_value, true),
        ] {
            self.lower_range(body, range, level, nesting, values, ops, declarations)?;
            let given = values.get(value as usize)?.clone()?;
            let given = self.coerce(
                &given,
                &body.expressions[value as usize].ty,
                result_ty,
                id * 64 + 2 + branch,
                ops,
            )?;
            ops.push(format!("      scf.yield {given} : {types}"));
            if !last {
                ops.push("    } else {".to_string());
            }
        }
        ops.push("    }".to_string());
        Some(Some(if width > 1 {
            (0..width)
                .map(|k| format!("%s{id}#{k}"))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            format!("%s{id}")
        }))
    }

    /// Whether running any of `roots`, or what they depend on, would run an instruction.
    fn reaches_effect(
        &self,
        body: &CheckedSourceBody,
        roots: impl IntoIterator<Item = u32>,
    ) -> bool {
        let mut pending: Vec<u32> = roots.into_iter().collect();
        let mut seen = vec![false; body.expressions.len()];
        while let Some(id) = pending.pop() {
            let Some(expr) = body.expressions.get(id as usize) else {
                continue;
            };
            if std::mem::replace(&mut seen[id as usize], true) {
                continue;
            }
            match &expr.kind {
                CheckedSourceBodyKind::Layout { .. } | CheckedSourceBodyKind::TextBytes { .. } => {}
                CheckedSourceBodyKind::SubArray { base, start } => pending.extend([*base, *start]),
                CheckedSourceBodyKind::RepeatArray { element, .. } => pending.push(*element),
                CheckedSourceBodyKind::Instruction { .. }
                | CheckedSourceBodyKind::Load { .. }
                | CheckedSourceBodyKind::Store { .. } => return true,
                CheckedSourceBodyKind::Call {
                    callee, arguments, ..
                } => {
                    if self.callee_has_effects(*callee) {
                        return true;
                    }
                    pending.extend(arguments);
                }
                CheckedSourceBodyKind::Return { value } => pending.push(*value),
                CheckedSourceBodyKind::Binary {
                    operator,
                    left,
                    right,
                } => {
                    // A division by a number that is at least 1 cannot trap (not zero, not -1),
                    // a shift by an amount in 0..=63 cannot be undefined: either may be speculated.
                    let divisor = match body.expressions[*right as usize].ty {
                        SourceType::Integer { lower, upper } => (lower.to_i128(), upper.to_i128()),
                        _ => (None, None),
                    };
                    let harmless = match operator {
                        CheckedArithmetic::Divide | CheckedArithmetic::Remainder => {
                            divisor.0.is_some_and(|lower| lower >= 1)
                        }
                        CheckedArithmetic::ShiftLeft | CheckedArithmetic::ShiftRight => {
                            divisor.0.is_some_and(|lower| lower >= 0)
                                && divisor.1.is_some_and(|upper| upper <= 63)
                        }
                        _ => operator.speculation_safe(),
                    };
                    if !harmless {
                        return true;
                    }
                    pending.extend([*left, *right])
                }
                CheckedSourceBodyKind::Wrap { operand } => pending.push(*operand),
                CheckedSourceBodyKind::Construct { fields, .. } => pending.extend(fields),
                CheckedSourceBodyKind::Field { record, .. } => pending.push(*record),
                CheckedSourceBodyKind::Bundle { parts } => pending.extend(parts),
                CheckedSourceBodyKind::Undefined => {}
                CheckedSourceBodyKind::Part { bundle, .. } => pending.push(*bundle),
                CheckedSourceBodyKind::VariantField { subject, .. } => pending.push(*subject),
                CheckedSourceBodyKind::Match { subject, arms, .. } => {
                    pending.push(*subject);
                    pending.extend(arms.iter().map(|arm| arm.body));
                }
                CheckedSourceBodyKind::Select {
                    left,
                    right,
                    then_branch,
                    else_branch,
                    ..
                } => pending.extend([*left, *right, *then_branch, *else_branch]),
                CheckedSourceBodyKind::Block { statements, result } => {
                    pending.extend(statements);
                    pending.push(*result);
                }
                CheckedSourceBodyKind::Widen { operand } => pending.push(*operand),
                CheckedSourceBodyKind::CallOut { call, .. } => pending.push(*call),
                CheckedSourceBodyKind::LoopResult { looped, .. } => pending.push(*looped),
                // A loop runs its whole body, which is every node up to it.
                CheckedSourceBodyKind::Loop {
                    start,
                    end,
                    body_start,
                    carried,
                    ..
                } => {
                    pending.extend([*start, *end]);
                    pending.extend(carried.iter().flat_map(|carry| [carry.initial, carry.next]));
                    pending.extend(*body_start..id);
                }
                CheckedSourceBodyKind::While {
                    left,
                    right,
                    body_start,
                    carried,
                    ..
                } => {
                    pending.extend([*left, *right]);
                    pending.extend(carried.iter().flat_map(|carry| [carry.initial, carry.next]));
                    pending.extend(*body_start..id);
                }
                CheckedSourceBodyKind::BoundCall { .. } => return true,
                CheckedSourceBodyKind::LoopCounter
                | CheckedSourceBodyKind::LoopCarried
                | CheckedSourceBodyKind::Parameter { .. }
                | CheckedSourceBodyKind::Integer(_)
                | CheckedSourceBodyKind::Text(_)
                | CheckedSourceBodyKind::Hole
                | CheckedSourceBodyKind::Unit => {}
            }
        }
        false
    }

    /// Whether a function's recorded body reaches an instruction. A function with no
    /// recorded body is assumed to, since nothing says otherwise.
    fn callee_has_effects(&self, callee: CheckedSourceCallee) -> bool {
        let bodies = match callee {
            CheckedSourceCallee::Local { function } => self.file.bodies.get(function as usize),
            CheckedSourceCallee::Dependency {
                dependency,
                function,
            } => self
                .dependencies
                .get(dependency as usize)
                .and_then(|module| module.checked().bodies.get(function as usize)),
        };
        bodies
            .and_then(Option::as_ref)
            .is_none_or(|body| body.effects || self.body_speculation_unsafe(body, 0))
    }

    /// Whether running `body` on operands it was not checked for could be undefined: it divides,
    /// takes a remainder or shifts, or calls something that does.
    fn body_speculation_unsafe(&self, body: &CheckedSourceBody, depth: usize) -> bool {
        body.expressions.iter().any(|expr| match &expr.kind {
            CheckedSourceBodyKind::Binary { operator, .. } => !operator.speculation_safe(),
            CheckedSourceBodyKind::Call { callee, .. } => {
                let found = match callee {
                    CheckedSourceCallee::Local { function } => {
                        self.file.bodies.get(*function as usize)
                    }
                    CheckedSourceCallee::Dependency {
                        dependency,
                        function,
                    } => self
                        .dependencies
                        .get(*dependency as usize)
                        .and_then(|module| module.checked().bodies.get(*function as usize)),
                };
                depth >= 8
                    || found
                        .and_then(Option::as_ref)
                        .is_none_or(|inner| self.body_speculation_unsafe(inner, depth + 1))
            }
            _ => false,
        })
    }

    /// The C `main` the linker needs: call the entry and return its exit status.
    fn entry_shim(&self, entry: u32) -> Result<String, LowerNextError> {
        let function = self
            .file
            .functions
            .get(entry as usize)
            .ok_or_else(|| LowerNextError::MissingBody("`#entry`".to_owned()))?;
        let symbol = self.symbol(&function.name);
        let unsupported = || LowerNextError::Unsupported {
            function: function.name.clone(),
        };
        let body = match &function.result {
            SourceType::Unit => format!(
                "    func.call {symbol}() : () -> ()\n    %status = arith.constant 0 : i32\n"
            ),
            ty => {
                integer_type(ty).ok_or_else(unsupported)?;
                format!(
                    "    %value = func.call {symbol}() : () -> i64\n\
                     \x20   %status = arith.trunci %value : i64 to i32\n"
                )
            }
        };
        assert!(function.parameters.is_empty());
        Ok(format!(
            "  func.func @main() -> i32 {{\n{body}    return %status : i32\n  }}\n"
        ))
    }

    fn symbol(&self, name: &str) -> String {
        match name.strip_prefix(mono::ABSOLUTE) {
            Some(absolute) => format!("@\"{absolute}\""),
            None => qualified(self.module, name),
        }
    }

    /// An instance whose signature does not travel in registers cannot be lowered: say which
    /// instance and who asked for it, not only that a construct is unsupported.
    fn check_instance_fits(
        &self,
        world: &mono::World<'_>,
        index: usize,
    ) -> Result<(), LowerNextError> {
        let function = world.function(index);
        let mut types: Vec<(&str, SourceType)> = vec![("result", function.result)];
        types.extend(function.parameters.iter().map(|ty| ("a parameter", *ty)));
        for (what, ty) in types {
            if self.register_width(&ty).is_some() {
                continue;
            }
            let need = world.flat_width(ty);
            let detail = match need {
                Some(need) if need > REGISTER_LIMIT => {
                    format!("{what} needs {need} registers and only {REGISTER_LIMIT} are passed")
                }
                _ => format!("{what} has no register form"),
            };
            return Err(LowerNextError::Generic(format!(
                "the instance `{}` cannot be lowered: {detail} (requested by {})",
                function.name.trim_start_matches(mono::ABSOLUTE),
                world.chain(index)
            )));
        }
        Ok(())
    }
}

/// `@name` for a bare single-file symbol, `@"module.name"` otherwise.
fn qualified(module: &str, name: &str) -> String {
    match module.is_empty() {
        true => format!("@{name}"),
        false => format!("@\"{module}.{name}\""),
    }
}

fn declare(declaration: &Declaration) -> String {
    if declaration.external {
        return format!("  llvm.func {}()\n", declaration.symbol);
    }
    let types = vec!["i64"; declaration.parameters].join(", ");
    let result = match declaration.results {
        0 => String::new(),
        1 => " -> i64".to_owned(),
        count => format!(" -> ({})", vec!["i64"; count].join(", ")),
    };
    format!(
        "  func.func private {}({types}){result}\n",
        declaration.symbol
    )
}

/// The node the single `return` statement yields, when the body is that one shape.
fn returned_value(body: &CheckedSourceBody) -> Option<u32> {
    let root = body.expressions.get(body.root as usize)?;
    let CheckedSourceBodyKind::Block { statements, result } = &root.kind else {
        // A body written as one expression returns that expression.
        return Some(body.root);
    };
    if statements.len() != 1 || *result != statements[0] {
        return None;
    }
    let last = statements[0];
    match body.expressions.get(last as usize)?.kind {
        CheckedSourceBodyKind::Return { value } => Some(value),
        _ => None,
    }
}

/// Reduce `x` into `lower..=upper` modulo the range width: the congruent value inside it.
/// `operand` is the interval `x` was proved to lie in; every intermediate must fit `i64`.
fn lower_wrap(
    id: usize,
    (low, high): (i128, i128),
    operand: &SourceType,
    x: &str,
    ops: &mut Vec<String>,
) -> Option<Option<String>> {
    let width = high.checked_sub(low)?.checked_add(1)?;
    let SourceType::Integer {
        lower: from,
        upper: to,
    } = operand
    else {
        return None;
    };
    let (from, to) = (from.to_i128()?, to.to_i128()?);
    // x - low, then the remainder plus one width, must stay inside i64.
    let reach = [from.checked_sub(low)?, to.checked_sub(low)?]
        .into_iter()
        .chain([width.checked_mul(2)?])
        .map(i128::unsigned_abs);
    if width > i128::from(i64::MAX) / 2 || reach.into_iter().any(|r| r > i64::MAX as u128 / 2) {
        return None;
    }
    let (low, width) = (i64::try_from(low).ok()?, i64::try_from(width).ok()?);
    ops.push(format!("    %w{id}_lo = arith.constant {low} : i64"));
    ops.push(format!("    %w{id}_wd = arith.constant {width} : i64"));
    ops.push(format!("    %w{id}_d = arith.subi {x}, %w{id}_lo : i64"));
    ops.push(format!(
        "    %w{id}_r = arith.remsi %w{id}_d, %w{id}_wd : i64"
    ));
    ops.push(format!(
        "    %w{id}_p = arith.addi %w{id}_r, %w{id}_wd : i64"
    ));
    ops.push(format!(
        "    %w{id}_m = arith.remsi %w{id}_p, %w{id}_wd : i64"
    ));
    ops.push(format!(
        "    %w{id}_out = arith.addi %w{id}_m, %w{id}_lo : i64"
    ));
    Some(Some(format!("%w{id}_out")))
}

/// `text` with each `$name` replaced by the number of that name's operand, which follow the
/// `outputs` results. A name that is not given a symbol makes it `None`.
fn place_symbols(text: &str, outputs: usize, symbols: &[(String, String)]) -> Option<String> {
    let mut placed = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        placed.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let end = rest
            .bytes()
            .position(|byte| !(byte.is_ascii_alphanumeric() || byte == b'_'))
            .unwrap_or(rest.len());
        let index = symbols.iter().position(|(name, _)| name == &rest[..end])?;
        placed.push_str(&format!("${{{}}}", outputs + index));
        rest = &rest[end..];
    }
    placed.push_str(rest);
    Some(placed)
}

/// `@name`, quoted when it holds more than letters, digits and `_`.
fn mlir_symbol(name: &str) -> String {
    let plain = !name.starts_with(|first: char| first.is_ascii_digit())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    match plain {
        true => format!("@{name}"),
        false => format!("@\"{name}\""),
    }
}

/// Whether `function` takes or returns text, or a record that holds text, which exists only
/// while compiling.
fn compile_time_only(
    file: &typecheck::CheckedSourceFile,
    function: &CheckedSourceFunction,
) -> bool {
    let holds_text = |ty: &SourceType| match ty {
        SourceType::Text => true,
        SourceType::Aggregate { id } => file
            .instances
            .get(*id as usize)
            .and_then(|instance| match instance.head {
                typecheck::CheckedSourceInstanceHead::Nominal { schema } => {
                    file.aggregates.get(schema as usize)
                }
                _ => None,
            })
            .is_some_and(|aggregate| {
                aggregate
                    .fields
                    .iter()
                    .any(|field| field.ty == SourceType::Text)
            }),
        _ => false,
    };
    holds_text(&function.result) || function.parameters.iter().any(holds_text)
}

/// The comparison for operands of these types: signed when both fit `i64`, unsigned when both
/// are within `0..=u64::MAX` (an address-sized number), otherwise none.
fn comparison_predicate(
    comparison: typecheck::InfixOperator,
    operands: [&SourceType; 2],
) -> Option<&'static str> {
    use typecheck::InfixOperator as Op;
    let (signed, unsigned) = if operands.iter().all(|ty| fits_i64(ty)) {
        (true, false)
    } else {
        (false, operands.iter().all(|ty| fits_u64(ty)))
    };
    match (comparison, signed, unsigned) {
        (Op::Equal, true, _) | (Op::Equal, _, true) => Some("eq"),
        (Op::Less, true, _) => Some("slt"),
        (Op::LessOrEqual, true, _) => Some("sle"),
        (Op::Greater, true, _) => Some("sgt"),
        (Op::GreaterOrEqual, true, _) => Some("sge"),
        (Op::Less, _, true) => Some("ult"),
        (Op::LessOrEqual, _, true) => Some("ule"),
        (Op::Greater, _, true) => Some("ugt"),
        (Op::GreaterOrEqual, _, true) => Some("uge"),
        _ => None,
    }
}

fn fits_u64(ty: &SourceType) -> bool {
    let SourceType::Integer { lower, upper } = ty else {
        return false;
    };
    [lower, upper]
        .into_iter()
        .all(|bound| bound.to_u64().is_some())
}

/// `%c{id}`: the comparison of two nodes, in 128 bits when either is wider than a register.
fn compare_nodes(
    comparison: typecheck::InfixOperator,
    (left, right): (u32, u32),
    body: &CheckedSourceBody,
    values: &[Option<String>],
    id: usize,
    ops: &mut Vec<String>,
) -> Option<()> {
    use typecheck::InfixOperator as Op;
    let types = [left, right].map(|node| &body.expressions[node as usize].ty);
    let (left_value, right_value) = (
        values.get(left as usize)?.clone()?,
        values.get(right as usize)?.clone()?,
    );
    let predicate = comparison_predicate(comparison, types);
    if types.iter().all(|ty| fits_wide(ty))
        && (types.iter().any(|ty| is_wide(ty)) || predicate.is_none())
    {
        let predicate = match comparison {
            Op::Equal => "eq",
            Op::Less => "slt",
            Op::LessOrEqual => "sle",
            Op::Greater => "sgt",
            Op::GreaterOrEqual => "sge",
            _ => return None,
        };
        let left_value = widened(&left_value, types[0], id * 2 + 100_000, ops);
        let right_value = widened(&right_value, types[1], id * 2 + 100_001, ops);
        ops.push(format!(
            "    %c{id} = arith.cmpi {predicate}, {left_value}, {right_value} : i128"
        ));
        return Some(());
    }
    let predicate = predicate?;
    ops.push(format!(
        "    %c{id} = arith.cmpi {predicate}, {left_value}, {right_value} : i64"
    ));
    Some(())
}

/// Copy `bytes` bytes from the address in `source` to the address in `target` (both `i64`).
/// Whether node `id` is inside the body of a loop of `body`.
fn inside_loop(body: &CheckedSourceBody, id: usize) -> bool {
    body.expressions
        .iter()
        .enumerate()
        .any(|(at, expr)| match &expr.kind {
            CheckedSourceBodyKind::Loop { body_start, .. }
            | CheckedSourceBodyKind::While { body_start, .. } => {
                (*body_start as usize..at).contains(&id)
            }
            _ => false,
        })
}

fn copy_bytes(name: &str, source: &str, target: &str, bytes: u64, ops: &mut Vec<String>) {
    ops.push(format!(
        "    %cps{name} = llvm.inttoptr {source} : i64 to !llvm.ptr"
    ));
    ops.push(format!(
        "    %cpt{name} = llvm.inttoptr {target} : i64 to !llvm.ptr"
    ));
    ops.push(format!(
        "    %cpn{name} = llvm.mlir.constant({bytes} : i64) : i64"
    ));
    ops.push(format!("    \"llvm.intr.memcpy\"(%cpt{name}, %cps{name}, %cpn{name}) <{{isVolatile = false}}> : (!llvm.ptr, !llvm.ptr, i64) -> ()"));
}

/// A number that fits 128 bits (and may be wider than a register).
fn fits_wide(ty: &SourceType) -> bool {
    let SourceType::Integer { lower, upper } = ty else {
        return false;
    };
    [lower, upper]
        .into_iter()
        .all(|bound| bound.to_i128().is_some())
}

/// Whether a number of this type is held in 128 bits: it fits no register, signed or unsigned.
fn is_wide(ty: &SourceType) -> bool {
    fits_wide(ty) && !fits_i64(ty) && !fits_u64(ty)
}

/// `value` as a 128-bit number: itself when its type is wider than a register, else extended
/// (sign for a signed register, zero for an unsigned one).
fn widened(value: &str, ty: &SourceType, id: usize, ops: &mut Vec<String>) -> String {
    if is_wide(ty) {
        return value.to_owned();
    }
    let name = format!("%wide{id}");
    let extend = if fits_i64(ty) {
        "arith.extsi"
    } else {
        "arith.extui"
    };
    ops.push(format!("    {name} = {extend} {value} : i64 to i128"));
    name
}

fn fits_i64(ty: &SourceType) -> bool {
    let SourceType::Integer { lower, upper } = ty else {
        return false;
    };
    [lower, upper]
        .into_iter()
        .all(|bound| bound.to_i64().is_some())
}

/// A signed or unsigned 64-bit domain: one register, with no check needed to read it.
fn fits_register(ty: &SourceType) -> bool {
    fits_i64(ty) || fits_u64(ty)
}

/// The register holding `value`: unsigned values above `i64::MAX` keep their bit pattern.
fn register_bits(value: i128) -> Option<i64> {
    i64::try_from(value)
        .ok()
        .or_else(|| u64::try_from(value).ok().map(|bits| bits as i64))
}

fn register_struct(count: usize) -> String {
    format!("!llvm.struct<({})>", vec!["i64"; count].join(", "))
}

fn integer_type(ty: &SourceType) -> Option<&'static str> {
    fits_register(ty).then_some("i64")
}

fn verify_mlir(text: &str) -> Result<(), LowerNextError> {
    // A developer aid: the text that failed to parse or verify, to read the line a message names.
    let dump = std::env::var_os("GIN_DUMP_MLIR");
    let context = backend::NativeCompiler::create_context();
    let module = melior::ir::Module::parse(&context, text).ok_or_else(|| {
        if let Some(path) = &dump {
            let _ = std::fs::write(path, text);
        }
        LowerNextError::InvalidMlir("parse failed".to_string())
    })?;
    use melior::ir::operation::OperationLike;
    if !module.as_operation().verify() {
        if let Some(path) = &dump {
            let _ = std::fs::write(path, text);
        }
        return Err(LowerNextError::InvalidMlir(
            "module verification failed".to_string(),
        ));
    }
    Ok(())
}

// -----------------------------------------------------------------------------------------
// Layout as lowering sees it, for tools that describe a program (`ginterpreter`). These
// are views of the lowering's own functions, not a second derivation: what is reported here is
// what `--emit-new` emits.

/// How many numbers a value travels as, and why not when it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterLayout {
    /// The most numbers a value may travel as.
    pub limit: usize,
    /// How many numbers a value of this type travels as (`None`: it has no register form).
    pub width: Option<usize>,
    /// The `#register` names of a record's fields in order, when every field names one.
    pub registers: Option<Vec<String>>,
    /// For a sum: the tag, the payloads and the padding.
    pub sum: Option<SumLayout>,
    /// Why there is no `width`.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SumLayout {
    /// The numbers every member travels as: the tag, then the widest payload.
    pub total: Option<usize>,
    pub widest_payload: Option<usize>,
    pub members: Vec<MemberLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberLayout {
    pub name: String,
    /// The 64-bit tag a member is, derived from its name.
    pub tag: i64,
    /// The numbers its fields take; shorter members are padded to the widest.
    pub payload_width: Option<usize>,
}

/// The register layout of parameter `index` of the function `function` of `module`, as the
/// lowering sees it: through monomorphization, so a generic type applied to arguments has the
/// concrete layout it would be emitted with. `None` when there is no such parameter.
pub fn parameter_layout(
    module: &CheckedSourceModule,
    function: &str,
    index: usize,
) -> Option<RegisterLayout> {
    let program = mono::Program::new([module]);
    let world = mono::World::build(&program, module, MonoLimits::default()).ok()?;
    let at = world
        .file
        .functions
        .iter()
        .position(|f| f.name == function)?;
    let ty = *world.file.functions[at].parameters.get(index)?;
    let context = Context {
        module: module.name(),
        file: &world.file,
        dependencies: &[],
        externals: &world.external,
        texts: std::cell::RefCell::new(Vec::new()),
        world: true,
        slots: std::cell::RefCell::new(Vec::new()),
    };
    let width = context.register_width(&ty);
    let registers = context
        .record_registers(&ty)
        .map(|names| names.into_iter().map(str::to_owned).collect());
    let sum = match ty {
        SourceType::Sum { .. } => context.schema_of(&ty).and_then(|schema| {
            let aggregate = context.file.aggregates.get(schema as usize)?;
            let members: Vec<MemberLayout> = aggregate
                .variants
                .iter()
                .filter_map(|variant| {
                    let member = context.schema_of(variant)?;
                    Some(MemberLayout {
                        name: context.file.aggregates[member as usize].name.clone(),
                        tag: context.tag(member)?,
                        payload_width: context.payload_width(member),
                    })
                })
                .collect();
            let widest = members
                .iter()
                .map(|m| m.payload_width)
                .collect::<Option<Vec<_>>>()
                .and_then(|w| w.into_iter().max());
            Some(SumLayout {
                total: width,
                widest_payload: widest,
                members,
            })
        }),
        _ => None,
    };
    let reason = width.is_none().then(|| match world.flat_width(ty) {
        Some(need) if need > REGISTER_LIMIT => {
            format!("it needs {need} registers and only {REGISTER_LIMIT} are passed")
        }
        _ => "it has no register form".to_owned(),
    });
    Some(RegisterLayout {
        limit: REGISTER_LIMIT,
        width,
        registers,
        sum,
        reason,
    })
}

/// One instance of a generic function that the module's code requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceLayout {
    pub name: String,
    pub requested_by: String,
    pub parameters: Vec<(String, Option<usize>)>,
    pub result: (String, Option<usize>),
}

fn spell(file: &CheckedSourceFile, ty: SourceType) -> String {
    match ty {
        SourceType::Unit => "()".to_owned(),
        SourceType::Text => "text".to_owned(),
        SourceType::Integer { lower, upper } => format!("{lower} to {upper}"),
        SourceType::Generic { index } => format!("type parameter {index}"),
        SourceType::Aggregate { id }
        | SourceType::Sum { id }
        | SourceType::Array { id }
        | SourceType::Sequence { id }
        | SourceType::Pointer { id } => {
            let Some(instance) = file.instances.get(id as usize) else {
                return "?".to_owned();
            };
            let args: Vec<String> = instance.arguments.iter().map(|t| spell(file, *t)).collect();
            match instance.head {
                CheckedSourceInstanceHead::Nominal { schema } => {
                    let name = file
                        .aggregates
                        .get(schema as usize)
                        .map_or("?", |a| a.name.as_str());
                    if args.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{name}({})", args.join(", "))
                    }
                }
                CheckedSourceInstanceHead::Array { extent } => {
                    format!("[{}; {extent}]", args.join(", "))
                }
                CheckedSourceInstanceHead::Sequence => format!("seq({})", args.join(", ")),
                CheckedSourceInstanceHead::Pointer => format!("ptr({})", args.join(", ")),
                _ => "array".to_owned(),
            }
        }
    }
}

/// The instances of generic functions that `modules` (the root last) request, with the register
/// width of each parameter and result, as monomorphization (`mono.rs`) decides them.
pub fn instance_layouts(modules: &[&CheckedSourceModule]) -> Result<Vec<InstanceLayout>, String> {
    let Some(root) = modules.last().copied() else {
        return Ok(Vec::new());
    };
    if !mono::needs_world(root) {
        return Ok(Vec::new());
    }
    let program = mono::Program::new(modules.iter().copied());
    let world =
        mono::World::build(&program, root, MonoLimits::default()).map_err(|e| e.to_string())?;
    let context = Context {
        module: root.name(),
        file: &world.file,
        dependencies: &[],
        externals: &world.external,
        texts: std::cell::RefCell::new(Vec::new()),
        world: true,
        slots: std::cell::RefCell::new(Vec::new()),
    };
    Ok((0..world.file.functions.len())
        .filter(|index| world.is_instance(*index))
        .map(|index| {
            let function = world.function(index);
            let part = |ty: SourceType| (spell(&world.file, ty), context.register_width(&ty));
            InstanceLayout {
                name: function.name.trim_start_matches(mono::ABSOLUTE).to_owned(),
                requested_by: world.chain(index),
                parameters: function.parameters.iter().map(|t| part(*t)).collect(),
                result: part(function.result),
            }
        })
        .collect())
}
