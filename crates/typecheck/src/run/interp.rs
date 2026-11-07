//! The evaluator.
//!
//! A recorded body is a dataflow graph in a postorder arena (`CheckedSourceBody`). It is run by
//! *demand*: the result and the `mut` outputs are asked for, and each node asks for its
//! operands. That gives the semantics the language needs without a separate rule for each
//! construct:
//!
//! * `Select` and `Match` run only the arm that is taken (so an arm that would be refused or
//!   would not terminate costs nothing when not taken, as in a compiled branch);
//! * a loop body is the node range `body_start..loop`, cleared and re-demanded once per pass;
//! * a call runs the callee's body in a fresh frame; the `mut` parameters come back through
//!   `CallOut` nodes.
//!
//! Generic functions run as templates: the frame carries the concrete types the function was
//! applied to, and only `Call` type arguments and `BoundCall` look at them.

use std::collections::HashMap;
use std::rc::Rc;

use i256::I256;
use crate::{
    CheckedSourceBody, CheckedSourceBodyKind as Kind, CheckedSourceCallee, CheckedSourceFunction,
    CheckedSourceInstanceHead, CheckedSourceModule, SourceType, apply, wrap_into,
};

use super::{Error, RtType, Tag, Value};

/// What a run may cost.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Most nodes evaluated, over the whole run.
    pub fuel: u64,
    /// Most nested calls.
    pub calls: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { fuel: 50_000_000, calls: 256 }
    }
}

/// What a function gave back: its result, and the value each `mut` parameter holds after it,
/// in the order of the function's `mutated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub value: Value,
    pub mutated: Vec<Value>,
}

pub struct Interpreter<'p> {
    modules: &'p [CheckedSourceModule],
    limits: Limits,
    spent: u64,
    depth: usize,
    /// The functions being run, outermost first.
    stack: Vec<String>,
    tags: HashMap<(usize, u32), Tag>,
}

/// One function being run.
struct Frame<'p> {
    module: usize,
    function: &'p CheckedSourceFunction,
    body: &'p CheckedSourceBody,
    arguments: Vec<Value>,
    env: Rc<Vec<RtType>>,
    values: Vec<Option<Value>>,
    /// For a `Call` node: the `mut` parameters after the call.
    outs: Vec<Option<Vec<Value>>>,
    /// For a `Loop`/`While` node: the carried values when it ended.
    finals: Vec<Option<Vec<Value>>>,
    /// How many loop bodies and effectful arms each node sits in. Only for a body that
    /// reaches the machine (`CheckedSourceBody::effects`); see [`Interpreter::run_effects`].
    nesting: Option<Vec<u32>>,
}

impl<'p> Interpreter<'p> {
    /// An interpreter over checked modules, dependencies first and the root last.
    pub fn new<M: AsRef<[CheckedSourceModule]> + ?Sized>(modules: &'p M) -> Self {
        Self::with_limits(modules, Limits::default())
    }

    pub fn with_limits<M: AsRef<[CheckedSourceModule]> + ?Sized>(modules: &'p M, limits: Limits) -> Self {
        Self { modules: modules.as_ref(), limits, spent: 0, depth: 0, stack: Vec::new(), tags: HashMap::new() }
    }

    /// Nodes evaluated so far.
    pub fn steps(&self) -> u64 {
        self.spent
    }

    /// Run the root module's `#entry` function.
    pub fn run_entry(&mut self) -> Result<Outcome, Error> {
        let root = self.modules.len() - 1;
        let entry = self.modules[root].checked().entry;
        let Some(index) = entry else {
            return Err(Error::NoSuchFunction("#entry".to_owned()));
        };
        self.invoke(root, index as usize, Vec::new(), Vec::new())
    }

    /// Run the function the root module defines under `name` (`Type.method` for a method) on
    /// `arguments`; `types` are the type arguments of a generic function.
    pub fn call(&mut self, name: &str, types: &[RtType], arguments: &[Value]) -> Result<Outcome, Error> {
        let root = self.modules.len() - 1;
        let module = &self.modules[root];
        let index = module
            .checked()
            .functions
            .iter()
            .enumerate()
            .position(|(index, function)| {
                function.name == name
                    && module
                        .function_home(index)
                        .is_some_and(|(home, own)| home == module.name() && own as usize == index)
            })
            .ok_or_else(|| Error::NoSuchFunction(name.to_owned()))?;
        self.invoke(root, index, types.to_vec(), arguments.to_vec())
    }

    /// Run function `index` of the table of `module`.
    fn invoke(
        &mut self,
        module: usize,
        index: usize,
        types: Vec<RtType>,
        arguments: Vec<Value>,
    ) -> Result<Outcome, Error> {
        let checked = self.modules[module].checked();
        let function = &checked.functions[index];
        if function.type_parameters.len() != types.len() {
            return Err(Error::TypeArity {
                function: function.name.clone(),
                expected: function.type_parameters.len(),
                given: types.len(),
            });
        }
        if function.parameters.len() != arguments.len() {
            return Err(Error::Arity {
                function: function.name.clone(),
                expected: function.parameters.len(),
                given: arguments.len(),
            });
        }
        if function.instruction.is_some() {
            return Err(Error::Unsupported {
                what: "a machine instruction",
                function: function.name.clone(),
            });
        }
        let Some(body) = &checked.bodies[index] else {
            return Err(Error::NoBody(function.name.clone()));
        };
        if self.depth >= self.limits.calls {
            return Err(Error::DepthExceeded(self.limits.calls));
        }
        let count = body.expressions.len();
        let mut frame = Frame {
            module,
            function,
            body,
            arguments,
            env: Rc::new(types),
            values: vec![None; count],
            outs: vec![None; count],
            finals: vec![None; count],
            nesting: None,
        };
        self.depth += 1;
        self.stack.push(function.name.clone());
        let result = self.start(&mut frame).and_then(|()| self.finish(&mut frame));
        self.stack.pop();
        self.depth -= 1;
        result
    }

    /// A body that reaches the machine runs its effects whether or not a value depends on them,
    /// in order, each only when the region it sits in runs. The interpreter has no machine, so
    /// reaching one is an error, found before anything else runs.
    fn start(&mut self, frame: &mut Frame<'p>) -> Result<(), Error> {
        if !frame.body.effects {
            return Ok(());
        }
        let body = frame.body;
        let module = frame.module;
        // The same regions the lowering branches on: one definition (`crate::regions`).
        let nesting = crate::regions::nesting(body, &|kind| self.effectful(module, kind));
        frame.nesting = Some(nesting);
        self.run_effects(frame, 0..body.expressions.len(), 0)
    }

    /// Run the effectful nodes of `range` that sit exactly `level` regions deep, in order, whether
    /// or not a value depends on them. An instruction among them is an error, so is a call that
    /// reaches one; calls that turn out pure just run.
    fn run_effects(&mut self, frame: &mut Frame<'p>, range: std::ops::Range<usize>, level: u32) -> Result<(), Error> {
        let Some(nesting) = &frame.nesting else { return Ok(()) };
        let due: Vec<u32> = range
            .filter(|&id| nesting[id] == level && self.effectful(frame.module, &frame.body.expressions[id].kind))
            .map(|id| id as u32)
            .collect();
        for id in due {
            self.eval(frame, id)?;
        }
        Ok(())
    }

    fn effectful(&self, module: usize, kind: &Kind) -> bool {
        match kind {
            Kind::Instruction { .. } | Kind::Load { .. } | Kind::Store { .. } | Kind::BoundCall { .. } => true,
            Kind::Call { callee, .. } => {
                self.resolve(module, *callee).is_none_or(|(home, index)| {
                    let checked = self.modules[home].checked();
                    checked.functions[index].instruction.is_some()
                        || checked.bodies[index].as_ref().is_none_or(|body| body.effects)
                })
            }
            _ => false,
        }
    }

    /// The two arms of a choice that must not both run, as node ranges, when an arm reaches an
    /// instruction and its nodes belong to it alone (the rule `lower_next` lowers by).
    fn lazy_arms(
        &self,
        module: usize,
        body: &CheckedSourceBody,
        id: usize,
    ) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
        crate::regions::lazy_arms(body, id, &|kind| self.effectful(module, kind))
    }

    fn finish(&mut self, frame: &mut Frame<'p>) -> Result<Outcome, Error> {
        let value = self.eval(frame, frame.body.root)?;
        let mut mutated = Vec::with_capacity(frame.body.outputs.len());
        for output in &frame.body.outputs {
            mutated.push(self.eval(frame, *output)?);
        }
        Ok(Outcome { value, mutated })
    }

    fn inconsistent(frame: &Frame<'_>, node: u32, what: impl Into<String>) -> Error {
        Error::Inconsistent { function: frame.function.name.clone(), node, what: what.into() }
    }

    fn integer(&mut self, frame: &mut Frame<'p>, node: u32) -> Result<I256, Error> {
        match self.eval(frame, node)? {
            Value::Integer(value) => Ok(value),
            other => Err(Self::inconsistent(frame, node, format!("expected an integer, found {other}"))),
        }
    }

    fn eval(&mut self, frame: &mut Frame<'p>, id: u32) -> Result<Value, Error> {
        if let Some(value) = &frame.values[id as usize] {
            return Ok(value.clone());
        }
        self.spent += 1;
        if self.spent > self.limits.fuel {
            return Err(Error::OutOfFuel(self.limits.fuel));
        }
        let body = frame.body;
        let expr = &body.expressions[id as usize];
        let value = match &expr.kind {
            Kind::Return { value } => self.eval(frame, *value)?,
            Kind::Block { result, .. } => self.eval(frame, *result)?,
            Kind::Parameter { index } => {
                let value = frame.arguments[*index as usize].clone();
                if let (SourceType::Integer { lower, upper }, Value::Integer(n)) = (expr.ty, &value) {
                    if *n < lower || *n > upper {
                        return Err(Error::ArgumentOutOfDomain {
                            function: frame.function.name.clone(),
                            value: *n,
                            lower,
                            upper,
                        });
                    }
                }
                value
            }
            Kind::Integer(value) => Value::Integer(*value),
            Kind::Text(text) => Value::Text(text.clone()),
            Kind::Unit => Value::Unit,
            Kind::TextBytes { .. } | Kind::SubArray { .. } | Kind::RepeatArray { .. } => {
                return Err(Error::Unsupported { what: "text as bytes in memory", function: frame.function.name.clone() });
            }
            Kind::Layout { .. } => {
                return Err(Error::Unsupported { what: "a size that depends on a type parameter", function: frame.function.name.clone() });
            }
            Kind::Hole => {
                return Err(Error::HoleReached {
                    function: frame.function.name.clone(),
                    node: id,
                    stack: self.stack.clone(),
                });
            }
            Kind::Binary { operator, left, right } => {
                let left = self.integer(frame, *left)?;
                let right = self.integer(frame, *right)?;
                let result = apply(*operator, left, right)
                    .ok_or_else(|| Self::inconsistent(frame, id, "arithmetic left the checker's integer width"))?;
                Value::Integer(result)
            }
            Kind::Wrap { operand } => {
                let SourceType::Integer { lower, upper } = expr.ty else {
                    return Err(Self::inconsistent(frame, id, "a wrap into a type that is not an integer domain"));
                };
                let operand = self.integer(frame, *operand)?;
                let wrapped = wrap_into(operand, lower, upper)
                    .ok_or_else(|| Self::inconsistent(frame, id, "a wrap wider than the checker's integers"))?;
                Value::Integer(wrapped)
            }
            Kind::Widen { operand } => self.eval(frame, *operand)?,
            Kind::Construct { schema, fields } => {
                let tag = self.tag_of(frame.module, *schema)?;
                let mut values = Vec::with_capacity(fields.len());
                for field in fields {
                    values.push(self.eval(frame, *field)?);
                }
                Value::Record { tag, fields: values }
            }
            Kind::Undefined => return Err(Self::inconsistent(frame, id, "a value that is never read was read")),
            // A bundle is an untagged record of its parts.
            Kind::Bundle { parts } => {
                let mut values = Vec::with_capacity(parts.len());
                for part in parts {
                    values.push(self.eval(frame, *part)?);
                }
                Value::Record { tag: Tag { home: "".into(), name: "(bundle)".into() }, fields: values }
            }
            Kind::Part { bundle, index } => match self.eval(frame, *bundle)? {
                Value::Record { mut fields, .. } if (*index as usize) < fields.len() => {
                    fields.swap_remove(*index as usize)
                }
                other => return Err(Self::inconsistent(frame, id, format!("no part {index} in {other}"))),
            },
            Kind::Field { record, index } => match self.eval(frame, *record)? {
                Value::Record { mut fields, .. } if (*index as usize) < fields.len() => {
                    fields.swap_remove(*index as usize)
                }
                other => return Err(Self::inconsistent(frame, id, format!("no field {index} in {other}"))),
            },
            Kind::VariantField { subject, schema, index } => {
                let expected = self.tag_of(frame.module, *schema)?;
                match self.eval(frame, *subject)? {
                    Value::Record { tag, mut fields } if tag == expected && (*index as usize) < fields.len() => {
                        fields.swap_remove(*index as usize)
                    }
                    other => {
                        return Err(Self::inconsistent(
                            frame,
                            id,
                            format!("payload field {index} of {} read from {other}", expected.name),
                        ));
                    }
                }
            }
            Kind::Match { subject, arms, .. } => {
                let Value::Record { tag, .. } = self.eval(frame, *subject)? else {
                    return Err(Self::inconsistent(frame, id, "a match over something that is not a record"));
                };
                let mut chosen = None;
                for arm in arms {
                    if self.tag_of(frame.module, arm.schema)? == tag {
                        chosen = Some(arm.body);
                        break;
                    }
                }
                let Some(arm) = chosen else {
                    return Err(Self::inconsistent(frame, id, format!("no arm for member {}", tag.name)));
                };
                if let (Some(nesting), Some(region)) = (&frame.nesting, Self::match_region(body, id)) {
                    if self.lazy_arms(frame.module, body, id as usize).is_some() {
                        let range = if arm == region.member_body {
                            region.member_start as usize..region.other_start as usize
                        } else {
                            region.other_start as usize..id as usize
                        };
                        let level = nesting[id as usize] + 1;
                        self.run_effects(frame, range, level)?;
                    }
                }
                self.eval(frame, arm)?
            }
            Kind::Select { comparison, left, right, then_branch, else_branch, region } => {
                let left = self.integer(frame, *left)?;
                let right = self.integer(frame, *right)?;
                let holds = crate::compare(*comparison, left, right)
                    .ok_or_else(|| Self::inconsistent(frame, id, "a selection by a non-comparison"))?;
                if let (Some(nesting), Some(region)) = (&frame.nesting, region) {
                    if self.lazy_arms(frame.module, body, id as usize).is_some() {
                        let range = if holds {
                            region.then_start as usize..region.else_start as usize
                        } else {
                            region.else_start as usize..id as usize
                        };
                        let level = nesting[id as usize] + 1;
                        self.run_effects(frame, range, level)?;
                    }
                }
                self.eval(frame, if holds { *then_branch } else { *else_branch })?
            }
            Kind::Call { callee, arguments, type_arguments } => {
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(self.eval(frame, *argument)?);
                }
                let mut types = Vec::with_capacity(type_arguments.len());
                for ty in type_arguments {
                    types.push(self.canonical(frame.module, *ty, &frame.env, 0)?);
                }
                let (module, index) = self.resolve(frame.module, *callee).ok_or_else(|| {
                    Self::inconsistent(frame, id, "a callee that is not part of the program")
                })?;
                let outcome = self.invoke(module, index, types, values)?;
                frame.outs[id as usize] = Some(outcome.mutated);
                outcome.value
            }
            Kind::BoundCall { generic, method, arguments, .. } => {
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(self.eval(frame, *argument)?);
                }
                let given = frame.env.get(*generic as usize).cloned();
                let Some(RtType::Nominal { home, name, args }) = given else {
                    return Err(Error::BoundNotSatisfied {
                        ty: frame.env.get(*generic as usize).map_or("?".to_owned(), RtType::spell),
                        method: method.clone(),
                    });
                };
                let wanted = format!("{name}.{method}");
                let found = self
                    .modules
                    .iter()
                    .position(|module| module.name() == home)
                    .and_then(|module| {
                        let table = &self.modules[module].checked().functions;
                        Some((module, table.iter().position(|function| function.name == wanted)?))
                    });
                let Some((module, index)) = found else {
                    return Err(Error::BoundNotSatisfied {
                        ty: RtType::Nominal { home, name, args }.spell(),
                        method: method.clone(),
                    });
                };
                let provider = &self.modules[module].checked().functions[index];
                let implicit = if provider.type_parameters.is_empty() { Vec::new() } else { args };
                let outcome = self.invoke(module, index, implicit, values)?;
                frame.outs[id as usize] = Some(outcome.mutated);
                outcome.value
            }
            Kind::CallOut { call, index } => {
                self.eval(frame, *call)?;
                let outs = frame.outs[*call as usize].as_ref();
                match outs.and_then(|outs| outs.get(*index as usize)) {
                    Some(value) => value.clone(),
                    None => return Err(Self::inconsistent(frame, id, "a call out that the call did not give")),
                }
            }
            Kind::Loop { start, end, counter, body_start, carried } => {
                let first = self.integer(frame, *start)?;
                let last = self.integer(frame, *end)?;
                let mut carry = Vec::with_capacity(carried.len());
                for slot in carried {
                    carry.push(self.eval(frame, slot.initial)?);
                }
                let mut number = first;
                while number <= last {
                    self.spent += 1;
                    if self.spent > self.limits.fuel {
                        return Err(Error::OutOfFuel(self.limits.fuel));
                    }
                    Self::clear(frame, *body_start, id);
                    if let Some(nesting) = &frame.nesting {
                        let level = nesting[id as usize] + 1;
                        self.run_effects(frame, *body_start as usize..id as usize, level)?;
                    }
                    frame.values[*counter as usize] = Some(Value::Integer(number));
                    for (slot, value) in carried.iter().zip(&carry) {
                        frame.values[slot.placeholder as usize] = Some(value.clone());
                    }
                    let mut next = Vec::with_capacity(carried.len());
                    for slot in carried {
                        next.push(self.eval(frame, slot.next)?);
                    }
                    carry = next;
                    number = number
                        .checked_add(I256::ONE)
                        .ok_or_else(|| Self::inconsistent(frame, id, "a loop counter left the checker's width"))?;
                }
                frame.finals[id as usize] = Some(carry);
                Value::Unit
            }
            Kind::While { comparison, negated, left, right, body_start, split: _, carried, exit } => {
                let mut carry = Vec::with_capacity(carried.len());
                for slot in carried {
                    carry.push(self.eval(frame, slot.initial)?);
                }
                loop {
                    self.spent += 1;
                    if self.spent > self.limits.fuel {
                        return Err(Error::OutOfFuel(self.limits.fuel));
                    }
                    Self::clear(frame, *body_start, id);
                    for (slot, value) in carried.iter().zip(&carry) {
                        frame.values[slot.placeholder as usize] = Some(value.clone());
                    }
                    let left = self.integer(frame, *left)?;
                    let right = self.integer(frame, *right)?;
                    let holds = crate::compare(*comparison, left, right)
                        .ok_or_else(|| Self::inconsistent(frame, id, "a while over a non-comparison"))?;
                    let mut go = holds != *negated;
                    if let Some(flag) = exit {
                        go &= carry.get(*flag as usize) == Some(&Value::integer(0));
                    }
                    if !go {
                        break;
                    }
                    if let Some(nesting) = &frame.nesting {
                        let level = nesting[id as usize] + 1;
                        self.run_effects(frame, *body_start as usize..id as usize, level)?;
                    }
                    let mut next = Vec::with_capacity(carried.len());
                    for slot in carried {
                        next.push(self.eval(frame, slot.next)?);
                    }
                    carry = next;
                }
                frame.finals[id as usize] = Some(carry);
                Value::Unit
            }
            Kind::LoopCounter | Kind::LoopCarried => {
                return Err(Self::inconsistent(frame, id, "a loop value read outside its pass"));
            }
            Kind::LoopResult { looped, index } => {
                self.eval(frame, *looped)?;
                match frame.finals[*looped as usize].as_ref().and_then(|f| f.get(*index as usize)) {
                    Some(value) => value.clone(),
                    None => return Err(Self::inconsistent(frame, id, "a loop result the loop did not carry")),
                }
            }
            Kind::Load { .. } | Kind::Store { .. } => {
                // Memory is not byte-addressed here yet: refuse rather than invent a model.
                return Err(Error::Unsupported {
                    what: "raw memory (`#load`/`#store`) before byte-addressed memory exists",
                    function: frame.function.name.clone(),
                });
            }
            Kind::Instruction { .. } => {
                return Err(Error::Unsupported {
                    what: "a machine instruction",
                    function: frame.function.name.clone(),
                });
            }
        };
        // The checker proved every integer node inside its domain. Re-check that claim.
        if let (SourceType::Integer { lower, upper }, Value::Integer(n)) = (expr.ty, &value) {
            if *n < lower || *n > upper {
                return Err(Error::DomainViolated {
                    function: frame.function.name.clone(),
                    node: id,
                    value: *n,
                    lower,
                    upper,
                });
            }
        }
        frame.values[id as usize] = Some(value.clone());
        Ok(value)
    }

    fn match_region(body: &CheckedSourceBody, id: u32) -> Option<crate::MatchRegion> {
        match &body.expressions[id as usize].kind {
            Kind::Match { region, .. } => *region,
            _ => None,
        }
    }

    /// Forget what the nodes `from..to` held, so the next pass of a loop recomputes them.
    fn clear(frame: &mut Frame<'_>, from: u32, to: u32) {
        for node in from as usize..to as usize {
            frame.values[node] = None;
            frame.outs[node] = None;
            frame.finals[node] = None;
        }
    }

    /// The module and table index of the function a callee of a body of `module` names.
    fn resolve(&self, module: usize, callee: CheckedSourceCallee) -> Option<(usize, usize)> {
        let frame = &self.modules[module];
        let (home, index) = match callee {
            CheckedSourceCallee::Local { function } => frame.function_home(function as usize)?,
            CheckedSourceCallee::Dependency { dependency, function } => {
                let name = frame.dependency_name(dependency as usize)?;
                let dependency = self.modules.iter().find(|m| m.name() == name)?;
                dependency.function_home(function as usize)?
            }
        };
        let home = self.modules.iter().position(|m| m.name() == home)?;
        Some((home, index as usize))
    }

    fn tag_of(&mut self, module: usize, layout: u32) -> Result<Tag, Error> {
        if let Some(tag) = self.tags.get(&(module, layout)) {
            return Ok(tag.clone());
        }
        let frame = &self.modules[module];
        let (Some(home), Some(aggregate)) =
            (frame.layout_home(layout as usize), frame.checked().aggregates.get(layout as usize))
        else {
            return Err(Error::Inconsistent {
                function: frame.name().to_owned(),
                node: layout,
                what: "a layout the module does not have".to_owned(),
            });
        };
        let tag = Tag { home: home.into(), name: aggregate.name.as_str().into() };
        self.tags.insert((module, layout), tag.clone());
        Ok(tag)
    }

    /// `ty`, written in `module` with its type parameters standing for `env`, as a concrete type.
    fn canonical(&self, module: usize, ty: SourceType, env: &[RtType], depth: u32) -> Result<RtType, Error> {
        let unsupported = |what: &str| Error::Inconsistent {
            function: self.modules[module].name().to_owned(),
            node: 0,
            what: what.to_owned(),
        };
        if depth > 64 {
            return Err(unsupported("a type nested too deeply"));
        }
        let id = match ty {
            SourceType::Unit => return Ok(RtType::Unit),
            SourceType::Text => return Err(unsupported("text as a type argument")),
            SourceType::Integer { .. } => return Ok(RtType::Integer(ty)),
            SourceType::Generic { index } => {
                return env.get(index as usize).cloned().ok_or_else(|| unsupported("a type parameter with no argument"));
            }
            SourceType::Aggregate { id }
            | SourceType::Sum { id }
            | SourceType::Array { id }
            | SourceType::Sequence { id }
            | SourceType::Pointer { id } => id,
        };
        let frame = &self.modules[module];
        let file = frame.checked();
        let instance = file.instances.get(id as usize).ok_or_else(|| unsupported("an unknown type"))?;
        let mut args = Vec::with_capacity(instance.arguments.len());
        for argument in &instance.arguments {
            args.push(self.canonical(module, *argument, env, depth + 1)?);
        }
        Ok(match instance.head {
            CheckedSourceInstanceHead::Nominal { schema } => RtType::Nominal {
                home: frame
                    .layout_home(schema as usize)
                    .ok_or_else(|| unsupported("a type with no home"))?
                    .to_owned(),
                name: file.aggregates[schema as usize].name.clone(),
                args,
            },
            CheckedSourceInstanceHead::Array { extent } if args.len() == 1 => {
                RtType::Array { extent, element: Box::new(args.remove(0)) }
            }
            CheckedSourceInstanceHead::Sequence => RtType::Sequence(args),
            CheckedSourceInstanceHead::Pointer if args.len() == 1 => RtType::Pointer(Box::new(args.remove(0))),
            _ => return Err(unsupported("a type with no run-time form")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::SpanId;
    use crate::{CheckedSourceBodyExpr, Limits as CheckLimits, check_module};

    /// The one thing the interpreter re-checks: a node whose value leaves the domain the
    /// checker recorded for it. The checker never produces such a body, so this one is forged.
    #[test]
    fn a_value_outside_the_recorded_domain_is_reported_as_a_checker_bug() {
        let file = parser::parse_strict("f(n in 0 to 9) in 0 to 9: return n\n").unwrap();
        let program = vec![check_module("main", &file, &[], CheckLimits::default()).unwrap()];
        let checked = program[0].checked();
        let forged = CheckedSourceBody {
            effects: false,
            expressions: vec![CheckedSourceBodyExpr {
                span: SpanId::from(0),
                ty: SourceType::Integer { lower: I256::ZERO, upper: I256::from(3) },
                kind: Kind::Integer(I256::from(5)),
            }],
            root: 0,
            outputs: Vec::new(),
        };
        let mut frame = Frame {
            module: 0,
            function: &checked.functions[0],
            body: &forged,
            arguments: Vec::new(),
            env: Rc::new(Vec::new()),
            values: vec![None],
            outs: vec![None],
            finals: vec![None],
            nesting: None,
        };
        let mut interpreter = Interpreter::new(&program);
        let error = interpreter.eval(&mut frame, 0).unwrap_err();
        assert!(matches!(error, Error::DomainViolated { node: 0, .. }), "{error:?}");
    }
}
