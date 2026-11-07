//! Monomorphization of checked generic bodies for lowering.
//!
//! A generic function, or a method of a generic type, is recorded once as a template whose types
//! may be `Generic`. Lowering a module builds a *world* for it: a copy of the module's checked
//! file in which
//!
//! * every type is concrete (a generic nominal type applied to arguments is a layout of its own,
//!   with no parameters, so `register_width` and friends work on it unchanged),
//! * every call of a template, and every call through a bound, is a plain call of an *instance*:
//!   one function per (template, concrete type arguments), appended to the function table and
//!   named after its canonical key, and
//! * a function of another module that a body calls is a stub that lowering declares.
//!
//! Instances are discovered from the module's non-generic bodies and, transitively, from the
//! template bodies of whichever capsules define what they request, so a module needs no other
//! module to have instantiated anything. Each module emits its own copies as `linkonce_odr` under
//! one symbol, and the linker keeps one.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use typecheck::{
    AggregateKind, CheckedSourceBody, CheckedSourceBodyKind, CheckedSourceCallee,
    CheckedSourceFile, CheckedSourceFunction, CheckedSourceInstance, CheckedSourceInstanceHead,
    CheckedSourceModule, SourceType,
};

/// What instantiation may cost. A generic function itself has no size; each instance does.
#[derive(Debug, Clone, Copy)]
pub struct MonoLimits {
    /// Deepest nesting of any type argument.
    pub type_depth_max: u32,
    /// Most nodes in the arguments of one instance.
    pub type_nodes_max: u32,
    /// Most instances one module emits.
    pub instances_max: u32,
    /// Most body nodes all the instances of a module may have together.
    pub body_nodes_max: u64,
    /// Frames of the request chain an error prints.
    pub chain_report: usize,
}

impl Default for MonoLimits {
    fn default() -> Self {
        Self {
            type_depth_max: 16,
            type_nodes_max: 256,
            instances_max: 2_048,
            body_nodes_max: 2_000_000,
            chain_report: 8,
        }
    }
}

/// The longest symbol an instance is named by before its arguments are replaced by a hash.
const SYMBOL_LEN_MAX: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum MonoError {
    #[error("{what} exceeds the limit of {limit} while instantiating {chain}")]
    Limit {
        what: &'static str,
        limit: u64,
        chain: String,
    },
    #[error("`{ty}` does not provide `{method}`, which a bound of {chain} calls")]
    BoundNotSatisfied {
        ty: String,
        method: String,
        chain: String,
    },
    #[error("no body was recorded for the template `{function}` requested by {chain}")]
    MissingBody { function: String, chain: String },
    #[error("{what}, requested by {chain}")]
    Unsupported { what: String, chain: String },
}

/// Every module of the build, by name.
pub struct Program<'a> {
    modules: Vec<&'a CheckedSourceModule>,
}

impl<'a> Program<'a> {
    pub fn new(modules: impl IntoIterator<Item = &'a CheckedSourceModule>) -> Self {
        Self {
            modules: modules.into_iter().collect(),
        }
    }

    pub fn module(&self, name: &str) -> Option<&'a CheckedSourceModule> {
        self.modules.iter().copied().find(|module| module.name() == name)
    }

    /// The dependencies of `module`, in the order its bodies number them.
    pub fn dependencies(&self, module: &CheckedSourceModule) -> Vec<&'a CheckedSourceModule> {
        (0..)
            .map_while(|slot| module.dependency_name(slot))
            .filter_map(|name| self.module(name))
            .collect()
    }
}

/// A type with no parameter left, spelled by where its pieces were declared so that every
/// module names it alike.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum MonoType {
    Unit,
    Integer(SourceType),
    Nominal {
        home: String,
        name: String,
        args: Vec<MonoType>,
    },
    Array {
        extent: u32,
        element: Box<MonoType>,
    },
    /// `[element; len]` whose length is a sibling field: the address of the first element.
    Slice(Box<MonoType>),
    Sequence(Vec<MonoType>),
    Pointer(Box<MonoType>),
}

impl MonoType {
    fn depth(&self) -> u32 {
        1 + match self {
            Self::Nominal { args, .. } => args.iter().map(Self::depth).max().unwrap_or(0),
            Self::Sequence(elements) => elements.iter().map(Self::depth).max().unwrap_or(0),
            Self::Array { element, .. } | Self::Slice(element) | Self::Pointer(element) => element.depth(),
            Self::Unit | Self::Integer(_) => 0,
        }
    }

    fn nodes(&self) -> u32 {
        1 + match self {
            Self::Nominal { args, .. } => args.iter().map(Self::nodes).sum(),
            Self::Sequence(elements) => elements.iter().map(Self::nodes).sum(),
            Self::Array { element, .. } | Self::Slice(element) | Self::Pointer(element) => element.nodes(),
            Self::Unit | Self::Integer(_) => 0,
        }
    }

    pub fn spell(&self) -> String {
        match self {
            Self::Unit => "()".to_owned(),
            Self::Integer(SourceType::Integer { lower, upper }) => format!("i<{lower}..{upper}>"),
            Self::Integer(_) => "i<?>".to_owned(),
            Self::Nominal { home, name, args } if args.is_empty() => format!("{home}.{name}"),
            Self::Nominal { home, name, args } => {
                format!("{home}.{name}<{}>", spell_all(args))
            }
            Self::Array { extent, element } => format!("[{extent}]{}", element.spell()),
            Self::Slice(element) => format!("[..]{}", element.spell()),
            Self::Sequence(elements) => format!("seq({})", spell_all(elements)),
            Self::Pointer(element) => format!("ptr({})", element.spell()),
        }
    }
}

fn spell_all(types: &[MonoType]) -> String {
    types.iter().map(MonoType::spell).collect::<Vec<_>>().join(",")
}

/// One function applied to concrete type arguments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstanceKey {
    /// The module that defines the function, and the function's index in its own functions.
    pub module: String,
    pub function: u32,
    pub arguments: Vec<MonoType>,
}

/// Who asked for an instance, for the chain an error prints.
#[derive(Debug, Clone)]
enum Requester {
    Function(String),
    Instance(usize),
}

/// A request not yet materialized.
struct Pending {
    index: usize,
    key: InstanceKey,
}

/// The concrete world one module is lowered in. See the module comment.
pub struct World<'a> {
    program: &'a Program<'a>,
    root: &'a CheckedSourceModule,
    limits: MonoLimits,
    pub file: CheckedSourceFile,
    /// Function-table indexes that are only declared here, defined by another module.
    pub external: BTreeSet<usize>,
    types: BTreeMap<MonoType, SourceType>,
    instances: BTreeMap<InstanceKey, usize>,
    requesters: BTreeMap<usize, Requester>,
    stubs: BTreeMap<(String, u32), usize>,
    symbols: BTreeMap<String, InstanceKey>,
    queue: VecDeque<Pending>,
    body_nodes: u64,
}

/// A name that is a link symbol as it stands, not one qualified by the module it appears in.
pub const ABSOLUTE: &str = "::";

/// Whether the module needs a world: it has a template, or applies one.
pub fn needs_world(module: &CheckedSourceModule) -> bool {
    let file = module.checked();
    file.functions.iter().any(|function| !function.type_parameters.is_empty())
        || file.bodies.iter().flatten().any(|body| {
            body.expressions.iter().any(|expr| {
                matches!(&expr.kind, CheckedSourceBodyKind::Call { type_arguments, .. } if !type_arguments.is_empty())
            })
        })
}

impl<'a> World<'a> {
    pub fn build(
        program: &'a Program<'a>,
        root: &'a CheckedSourceModule,
        limits: MonoLimits,
    ) -> Result<Self, MonoError> {
        let mut world = Self {
            program,
            root,
            limits,
            file: root.checked().clone(),
            external: BTreeSet::new(),
            types: BTreeMap::new(),
            instances: BTreeMap::new(),
            requesters: BTreeMap::new(),
            stubs: BTreeMap::new(),
            symbols: BTreeMap::new(),
            queue: VecDeque::new(),
            body_nodes: 0,
        };
        world.seed();
        let own = root.checked().functions.len();
        for index in 0..own {
            let function = &root.checked().functions[index];
            let chain = || format!("`{}`", function.name);
            // Templates are lowered per instance; an instruction is applied where it is called.
            if !function.type_parameters.is_empty()
                || function.instruction.is_some()
                || function.result == SourceType::Text
                || function.parameters.contains(&SourceType::Text)
            {
                continue;
            }
            let Some(body) = &root.checked().bodies[index] else {
                continue;
            };
            let requester = Requester::Function(function.name.clone());
            let parameters = world.translate_all(root, &function.parameters, &[], &chain)?;
            let result = world.translate(root, function.result, &[], &chain)?;
            let rewritten = world.rewrite_body(root, body, &[], &requester)?;
            world.file.functions[index].parameters = parameters;
            world.file.functions[index].result = result;
            world.file.bodies[index] = Some(rewritten);
        }
        while let Some(pending) = world.queue.pop_front() {
            world.materialize(pending)?;
        }
        Ok(world)
    }

    /// The module's own types that have no parameters stand for themselves.
    fn seed(&mut self) {
        let file = self.root.checked();
        for (schema, aggregate) in file.aggregates.iter().enumerate() {
            if !aggregate.parameters.is_empty() {
                continue;
            }
            let Some(home) = self.root.layout_home(schema) else {
                continue;
            };
            let found = file.instances.iter().position(|instance| {
                instance.arguments.is_empty()
                    && instance.head == CheckedSourceInstanceHead::Nominal { schema: schema as u32 }
            });
            let Some(id) = found else {
                continue;
            };
            let id = id as u32;
            let ty = match aggregate.kind {
                AggregateKind::Sum => SourceType::Sum { id },
                _ => SourceType::Aggregate { id },
            };
            let key = MonoType::Nominal {
                home: home.to_owned(),
                name: aggregate.name.clone(),
                args: Vec::new(),
            };
            self.types.insert(key, ty);
        }
    }

    /// The chain of requests that led to `requester`, nearest first.
    fn chain_of(&self, requester: &Requester) -> String {
        let mut frames = Vec::new();
        let mut current = requester.clone();
        loop {
            match current {
                Requester::Function(name) => {
                    frames.push(format!("`{name}`"));
                    break;
                }
                Requester::Instance(index) => {
                    frames.push(format!("`{}`", self.file.functions[index].name.trim_start_matches(ABSOLUTE)));
                    match self.requesters.get(&index) {
                        Some(next) => current = next.clone(),
                        None => break,
                    }
                }
            }
        }
        let total = frames.len();
        frames.truncate(self.limits.chain_report);
        let mut text = frames.join(" <- ");
        if total > frames.len() {
            text.push_str(&format!(" <- ... ({} more)", total - frames.len()));
        }
        text
    }

    fn translate_all(
        &mut self,
        frame: &CheckedSourceModule,
        types: &[SourceType],
        arguments: &[MonoType],
        chain: &dyn Fn() -> String,
    ) -> Result<Vec<SourceType>, MonoError> {
        types
            .iter()
            .map(|ty| self.translate(frame, *ty, arguments, chain))
            .collect()
    }

    /// `ty`, written in the universe of `frame` with its type parameters standing for `arguments`,
    /// as a type of this world.
    fn translate(
        &mut self,
        frame: &CheckedSourceModule,
        ty: SourceType,
        arguments: &[MonoType],
        chain: &dyn Fn() -> String,
    ) -> Result<SourceType, MonoError> {
        match ty {
            SourceType::Text => Ok(SourceType::Text),
            _ => {
                let mono = self.canonical(frame, ty, arguments, 0, chain)?;
                self.intern(&mono, chain)
            }
        }
    }

    fn canonical(
        &self,
        frame: &CheckedSourceModule,
        ty: SourceType,
        arguments: &[MonoType],
        depth: u32,
        chain: &dyn Fn() -> String,
    ) -> Result<MonoType, MonoError> {
        let unsupported = |what: &str| MonoError::Unsupported {
            what: what.to_owned(),
            chain: chain(),
        };
        if depth > self.limits.type_depth_max * 4 {
            return Err(unsupported("a type nested too deeply"));
        }
        let id = match ty {
            SourceType::Unit => return Ok(MonoType::Unit),
            SourceType::Text => return Err(unsupported("text cannot be a type argument")),
            SourceType::Integer { .. } => return Ok(MonoType::Integer(ty)),
            SourceType::Generic { index } => {
                return arguments
                    .get(index as usize)
                    .cloned()
                    .ok_or_else(|| unsupported("a type parameter with no argument"));
            }
            SourceType::Aggregate { id }
            | SourceType::Sum { id }
            | SourceType::Array { id }
            | SourceType::Sequence { id }
            | SourceType::Pointer { id } => id,
        };
        let file = frame.checked();
        let instance = file
            .instances
            .get(id as usize)
            .ok_or_else(|| unsupported("an unknown type"))?;
        let mut args = Vec::with_capacity(instance.arguments.len());
        for argument in &instance.arguments {
            args.push(self.canonical(frame, *argument, arguments, depth + 1, chain)?);
        }
        Ok(match instance.head {
            CheckedSourceInstanceHead::Nominal { schema } => MonoType::Nominal {
                home: frame
                    .layout_home(schema as usize)
                    .ok_or_else(|| unsupported("a type with no home"))?
                    .to_owned(),
                name: file.aggregates[schema as usize].name.clone(),
                args,
            },
            CheckedSourceInstanceHead::Array { extent } if args.len() == 1 => MonoType::Array {
                extent,
                element: Box::new(args.remove(0)),
            },
            CheckedSourceInstanceHead::Slice if args.len() == 1 => MonoType::Slice(Box::new(args.remove(0))),
            CheckedSourceInstanceHead::Sequence => MonoType::Sequence(args),
            CheckedSourceInstanceHead::Pointer if args.len() == 1 => {
                MonoType::Pointer(Box::new(args.remove(0)))
            }
            _ => return Err(unsupported("a type with no machine form")),
        })
    }

    /// The type of this world that is `mono`, made if it is new.
    fn intern(&mut self, mono: &MonoType, chain: &dyn Fn() -> String) -> Result<SourceType, MonoError> {
        if let Some(ty) = self.types.get(mono) {
            return Ok(*ty);
        }
        let unsupported = |what: &str| MonoError::Unsupported {
            what: what.to_owned(),
            chain: chain(),
        };
        let ty = match mono {
            MonoType::Unit => SourceType::Unit,
            MonoType::Integer(ty) => *ty,
            MonoType::Nominal { home, name, args } => {
                let module = self
                    .program
                    .module(home)
                    .ok_or_else(|| unsupported(&format!("module `{home}` is not part of the build")))?;
                let file = module.checked();
                let (_, aggregate) = file
                    .aggregates
                    .iter()
                    .enumerate()
                    .find(|(schema, aggregate)| {
                        aggregate.name == *name && module.layout_home(*schema) == Some(home.as_str())
                    })
                    .ok_or_else(|| unsupported(&format!("type `{name}` is not declared by `{home}`")))?;
                if aggregate.parameters.len() != args.len() {
                    return Err(unsupported(&format!("`{name}` applied to the wrong number of types")));
                }
                // The layout is registered before its fields are made: a field may name it.
                let index = self.file.aggregates.len() as u32;
                let id = self.file.instances.len() as u32;
                let mut shell = aggregate.clone();
                shell.parameters.clear();
                shell.defaults.clear();
                shell.bounds.clear();
                shell.fields.iter_mut().for_each(|field| field.ty = SourceType::Unit);
                shell.variants.clear();
                self.file.aggregates.push(shell);
                self.file.instances.push(CheckedSourceInstance {
                    head: CheckedSourceInstanceHead::Nominal { schema: index },
                    arguments: Vec::new(),
                });
                let ty = match aggregate.kind {
                    AggregateKind::Sum => SourceType::Sum { id },
                    _ => SourceType::Aggregate { id },
                };
                self.types.insert(mono.clone(), ty);
                for (position, field) in aggregate.fields.iter().enumerate() {
                    let field_ty = self.translate(module, field.ty, args, chain)?;
                    self.file.aggregates[index as usize].fields[position].ty = field_ty;
                }
                for variant in &aggregate.variants {
                    let variant = self.translate(module, *variant, args, chain)?;
                    self.file.aggregates[index as usize].variants.push(variant);
                }
                return Ok(ty);
            }
            MonoType::Array { extent, element } => {
                self.structural(CheckedSourceInstanceHead::Array { extent: *extent }, &[element], chain)?
            }
            MonoType::Slice(element) => {
                self.structural(CheckedSourceInstanceHead::Slice, &[element], chain)?
            }
            MonoType::Sequence(elements) => {
                let elements: Vec<&MonoType> = elements.iter().collect();
                self.structural(CheckedSourceInstanceHead::Sequence, &elements, chain)?
            }
            MonoType::Pointer(element) => {
                self.structural(CheckedSourceInstanceHead::Pointer, &[element], chain)?
            }
        };
        self.types.insert(mono.clone(), ty);
        Ok(ty)
    }

    fn structural(
        &mut self,
        head: CheckedSourceInstanceHead,
        elements: &[&MonoType],
        chain: &dyn Fn() -> String,
    ) -> Result<SourceType, MonoError> {
        let mut arguments = Vec::new();
        for element in elements {
            arguments.push(self.intern(element, chain)?);
        }
        let id = self.file.instances.len() as u32;
        self.file.instances.push(CheckedSourceInstance { head, arguments });
        Ok(match head {
            CheckedSourceInstanceHead::Sequence => SourceType::Sequence { id },
            CheckedSourceInstanceHead::Pointer => SourceType::Pointer { id },
            _ => SourceType::Array { id },
        })
    }

    /// The (module, own index) of the function a callee of a body of `frame` names.
    fn resolve(&self, frame: &CheckedSourceModule, callee: CheckedSourceCallee) -> Option<(&'a CheckedSourceModule, u32)> {
        let (home, index) = match callee {
            CheckedSourceCallee::Local { function } => frame.function_home(function as usize)?,
            CheckedSourceCallee::Dependency { dependency, function } => {
                let dependency = self.program.module(frame.dependency_name(dependency as usize)?)?;
                dependency.function_home(function as usize)?
            }
        };
        Some((self.program.module(home)?, index))
    }

    /// The callee a rewritten call names: a template becomes its instance, a function of another
    /// module a stub, and one of this module stays as it is.
    fn callee(
        &mut self,
        (module, index): (&'a CheckedSourceModule, u32),
        arguments: &[MonoType],
        requester: &Requester,
    ) -> Result<CheckedSourceCallee, MonoError> {
        let function = &module.checked().functions[index as usize];
        if !function.type_parameters.is_empty() {
            let key = InstanceKey {
                module: module.name().to_owned(),
                function: index,
                arguments: arguments.to_vec(),
            };
            let at = self.request(key, requester)?;
            return Ok(CheckedSourceCallee::Local { function: at as u32 });
        }
        if !arguments.is_empty() {
            return Err(MonoError::Unsupported {
                what: format!("`{}` takes no type arguments", function.name),
                chain: self.chain_of(requester),
            });
        }
        if module.name() == self.root.name() {
            return Ok(CheckedSourceCallee::Local { function: index });
        }
        let key = (module.name().to_owned(), index);
        if let Some(at) = self.stubs.get(&key) {
            return Ok(CheckedSourceCallee::Local { function: *at as u32 });
        }
        let chain_text = self.chain_of(requester);
        let chain = move || chain_text.clone();
        let mut stub = function.clone();
        stub.name = format!("{ABSOLUTE}{}.{}", module.name(), function.name);
        stub.parameters = self.translate_all(module, &function.parameters, &[], &chain)?;
        stub.result = self.translate(module, function.result, &[], &chain)?;
        let mut body = module.checked().bodies[index as usize].clone();
        if let Some(body) = &mut body {
            body.expressions.clear();
        }
        let at = self.file.functions.len();
        self.file.functions.push(stub);
        self.file.bodies.push(body);
        self.external.insert(at);
        self.stubs.insert(key, at);
        Ok(CheckedSourceCallee::Local { function: at as u32 })
    }

    fn request(&mut self, key: InstanceKey, requester: &Requester) -> Result<usize, MonoError> {
        if let Some(at) = self.instances.get(&key) {
            return Ok(*at);
        }
        let module = self.program.module(&key.module).expect("a resolved module is in the program");
        let template = &module.checked().functions[key.function as usize];
        let limit = |what: &'static str, limit: u64| MonoError::Limit {
            what,
            limit,
            chain: format!(
                "`{}<{}>` requested by {}",
                template.name,
                spell_all(&key.arguments),
                self.chain_of(requester)
            ),
        };
        if key.arguments.len() != template.type_parameters.len() {
            return Err(MonoError::Unsupported {
                what: format!("`{}` needs {} type arguments", template.name, template.type_parameters.len()),
                chain: self.chain_of(requester),
            });
        }
        for argument in &key.arguments {
            if matches!(argument, MonoType::Unit) {
                return Err(MonoError::Unsupported {
                    what: "unit cannot be a type argument".to_owned(),
                    chain: self.chain_of(requester),
                });
            }
            if argument.depth() > self.limits.type_depth_max {
                return Err(limit("the nesting depth of a type argument", self.limits.type_depth_max.into()));
            }
        }
        if key.arguments.iter().map(MonoType::nodes).sum::<u32>() > self.limits.type_nodes_max {
            return Err(limit("the size of the type arguments", self.limits.type_nodes_max.into()));
        }
        if self.instances.len() >= self.limits.instances_max as usize {
            return Err(limit("the number of instances", self.limits.instances_max.into()));
        }
        let symbol = self.mangle(&key, &template.name);
        let at = self.file.functions.len();
        let mut function = template.clone();
        function.name = symbol;
        function.type_parameters.clear();
        function.type_bounds.clear();
        self.file.functions.push(function);
        self.file.bodies.push(None);
        self.instances.insert(key.clone(), at);
        self.requesters.insert(at, requester.clone());
        self.queue.push_back(Pending { index: at, key });
        Ok(at)
    }

    fn mangle(&mut self, key: &InstanceKey, name: &str) -> String {
        let full = format!("{}.{name}<{}>", key.module, spell_all(&key.arguments));
        let symbol = if full.len() > SYMBOL_LEN_MAX {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(full.as_bytes());
            let hex: String = digest[..16].iter().map(|byte| format!("{byte:02x}")).collect();
            format!("{}.{name}<#{hex}>", key.module)
        } else {
            full
        };
        if let Some(other) = self.symbols.insert(symbol.clone(), key.clone()) {
            assert!(&other == key, "two instances share the symbol `{symbol}`");
        }
        format!("{ABSOLUTE}{symbol}")
    }

    fn materialize(&mut self, pending: Pending) -> Result<(), MonoError> {
        let Pending { index, key } = pending;
        let module = self.program.module(&key.module).expect("a resolved module is in the program");
        let template = &module.checked().functions[key.function as usize];
        let requester = Requester::Instance(index);
        let Some(body) = &module.checked().bodies[key.function as usize] else {
            let reason = module
                .checked()
                .declines
                .iter()
                .find(|(at, _)| *at == key.function)
                .map(|(_, decline)| match decline.span {
                    Some(span) => format!(" (the recorder declined {} at bytes {}..{})", decline.construct, span.start(), span.end()),
                    None => format!(" (the recorder declined {})", decline.construct),
                })
                .unwrap_or_default();
            return Err(MonoError::MissingBody {
                function: format!("{}{reason}", template.name),
                chain: self.chain_of(&requester),
            });
        };
        let chain_text = self.chain_of(&requester);
        let chain = move || chain_text.clone();
        let parameters = self.translate_all(module, &template.parameters, &key.arguments, &chain)?;
        let result = self.translate(module, template.result, &key.arguments, &chain)?;
        let rewritten = self.rewrite_body(module, body, &key.arguments, &requester)?;
        self.file.functions[index].parameters = parameters;
        self.file.functions[index].result = result;
        self.file.bodies[index] = Some(rewritten);
        Ok(())
    }

    /// `body`, written in the universe of `frame` with its type parameters standing for
    /// `arguments`, as a body of this world.
    fn rewrite_body(
        &mut self,
        frame: &'a CheckedSourceModule,
        body: &CheckedSourceBody,
        arguments: &[MonoType],
        requester: &Requester,
    ) -> Result<CheckedSourceBody, MonoError> {
        self.body_nodes = self.body_nodes.saturating_add(body.expressions.len() as u64);
        if self.body_nodes > self.limits.body_nodes_max {
            return Err(MonoError::Limit {
                what: "the nodes of instantiated bodies",
                limit: self.limits.body_nodes_max,
                chain: self.chain_of(requester),
            });
        }
        let chain_text = self.chain_of(requester);
        let chain = move || chain_text.clone();
        let mut rewritten = body.clone();
        for (id, expr) in body.expressions.iter().enumerate() {
            let ty = self.translate(frame, expr.ty, arguments, &chain)?;
            let out = &mut rewritten.expressions[id];
            out.ty = ty;
            match &expr.kind {
                CheckedSourceBodyKind::Call {
                    callee,
                    arguments: operands,
                    type_arguments,
                } => {
                    let target = self.resolve(frame, *callee).ok_or_else(|| MonoError::Unsupported {
                        what: "a call to a function that is not part of the build".to_owned(),
                        chain: chain(),
                    })?;
                    let applied = type_arguments
                        .iter()
                        .map(|ty| self.canonical(frame, *ty, arguments, 0, &chain))
                        .collect::<Result<Vec<_>, _>>()?;
                    let callee = self.callee(target, &applied, requester)?;
                    rewritten.expressions[id].kind = CheckedSourceBodyKind::Call {
                        callee,
                        arguments: operands.clone(),
                        type_arguments: Vec::new(),
                    };
                }
                // `#size(T)` once `T` is known: the layout of the instance's type.
                CheckedSourceBodyKind::Layout { queried, alignment } => {
                    let concrete = self.translate(frame, *queried, arguments, &chain)?;
                    let (size, align) = typecheck::layout_in_file(&self.file, concrete).ok_or_else(|| MonoError::Unsupported {
                        what: "the size of this type".to_owned(),
                        chain: chain(),
                    })?;
                    let (kind, ty) = typecheck::integer_node(if *alignment { align } else { size });
                    rewritten.expressions[id].kind = kind;
                    rewritten.expressions[id].ty = ty;
                }
                CheckedSourceBodyKind::BoundCall {
                    generic,
                    method,
                    arguments: operands,
                    trait_arguments,
                } => {
                    let Some(MonoType::Nominal { home, name, args }) = arguments.get(*generic as usize)
                    else {
                        return Err(MonoError::BoundNotSatisfied {
                            ty: arguments.get(*generic as usize).map_or("?".to_owned(), MonoType::spell),
                            method: method.clone(),
                            chain: chain(),
                        });
                    };
                    let provider = self.program.module(home).and_then(|module| {
                        let wanted = format!("{name}.{method}");
                        let index = module.checked().functions.iter().position(|f| f.name == wanted)?;
                        Some((module, index as u32))
                    });
                    let Some(provider) = provider else {
                        return Err(MonoError::BoundNotSatisfied {
                            ty: arguments[*generic as usize].spell(),
                            method: method.clone(),
                            chain: chain(),
                        });
                    };
                    // The provider's type parameters are its type's own (a method of a generic
                    // type reads them from the receiver), then the ones the call wrote.
                    let wanted = provider.0.checked().functions[provider.1 as usize].type_parameters.len();
                    let mut implicit = if wanted == 0 { Vec::new() } else { args.clone() };
                    if wanted > implicit.len() {
                        for written in trait_arguments {
                            implicit.push(self.canonical(frame, *written, arguments, 0, &chain)?);
                        }
                    }
                    let callee = self.callee(provider, &implicit, requester)?;
                    rewritten.expressions[id].kind = CheckedSourceBodyKind::Call {
                        callee,
                        arguments: operands.clone(),
                        type_arguments: Vec::new(),
                    };
                }
                CheckedSourceBodyKind::Binary { operator, left, right } => {
                    // A size that became a number narrows the arithmetic written over it.
                    let narrowed = typecheck::arithmetic_type(
                        *operator,
                        rewritten.expressions[*left as usize].ty,
                        rewritten.expressions[*right as usize].ty,
                    );
                    if let (Some(narrowed), SourceType::Integer { lower, upper }) = (narrowed, ty)
                        && let SourceType::Integer { lower: n_lower, upper: n_upper } = narrowed
                        && n_lower >= lower
                        && n_upper <= upper
                    {
                        rewritten.expressions[id].ty = narrowed;
                    }
                }
                CheckedSourceBodyKind::Construct { fields, .. } => {
                    if let Some(schema) = self.schema_of(ty) {
                        rewritten.expressions[id].kind = CheckedSourceBodyKind::Construct {
                            schema,
                            fields: fields.clone(),
                        };
                    }
                }
                CheckedSourceBodyKind::Match { arms, .. } => {
                    let subject = match &expr.kind {
                        CheckedSourceBodyKind::Match { subject, .. } => *subject,
                        _ => unreachable!(),
                    };
                    let subject_ty = body.expressions[subject as usize].ty;
                    let mut mapped = Vec::with_capacity(arms.len());
                    for arm in arms {
                        let mut arm = *arm;
                        arm.schema = self.member_schema(frame, subject_ty, arm.schema, arguments, &chain)?;
                        mapped.push(arm);
                    }
                    if let CheckedSourceBodyKind::Match { arms, .. } = &mut rewritten.expressions[id].kind {
                        *arms = mapped;
                    }
                }
                CheckedSourceBodyKind::VariantField { subject, schema, .. } => {
                    let subject_ty = body.expressions[*subject as usize].ty;
                    let mapped = self.member_schema(frame, subject_ty, *schema, arguments, &chain)?;
                    if let CheckedSourceBodyKind::VariantField { schema, .. } = &mut rewritten.expressions[id].kind {
                        *schema = mapped;
                    }
                }
                _ => {}
            }
        }
        Ok(rewritten)
    }

    /// The layout of this world that a node of type `ty` (of this world) is built from.
    fn schema_of(&self, ty: SourceType) -> Option<u32> {
        let (SourceType::Aggregate { id } | SourceType::Sum { id }) = ty else {
            return None;
        };
        match self.file.instances.get(id as usize)?.head {
            CheckedSourceInstanceHead::Nominal { schema } => Some(schema),
            _ => None,
        }
    }

    /// The layout of this world of member `schema` (of `frame`) of a subject of type `subject`
    /// (of `frame`, with its parameters standing for `arguments`).
    fn member_schema(
        &mut self,
        frame: &CheckedSourceModule,
        subject: SourceType,
        schema: u32,
        arguments: &[MonoType],
        chain: &dyn Fn() -> String,
    ) -> Result<u32, MonoError> {
        let unsupported = |what: &str| MonoError::Unsupported {
            what: what.to_owned(),
            chain: chain(),
        };
        let file = frame.checked();
        let member = match subject {
            SourceType::Sum { id } => {
                let instance = &file.instances[id as usize];
                let CheckedSourceInstanceHead::Nominal { schema: sum } = instance.head else {
                    return Err(unsupported("a match over something that is not a sum"));
                };
                let sum_arguments = instance
                    .arguments
                    .iter()
                    .map(|ty| self.canonical(frame, *ty, arguments, 0, chain))
                    .collect::<Result<Vec<_>, _>>()?;
                let variant = file.aggregates[sum as usize].variants.iter().find(|variant| {
                    matches!(variant, SourceType::Aggregate { id }
                        if file.instances[*id as usize].head == CheckedSourceInstanceHead::Nominal { schema })
                });
                let variant = variant.ok_or_else(|| unsupported("a match arm that is not a member"))?;
                self.canonical(frame, *variant, &sum_arguments, 0, chain)?
            }
            other => self.canonical(frame, other, arguments, 0, chain)?,
        };
        let ty = self.intern(&member, chain)?;
        self.schema_of(ty).ok_or_else(|| unsupported("a member with no layout"))
    }

    /// How many numbers a value of `ty` is passed as, without the register limit.
    pub fn flat_width(&self, ty: SourceType) -> Option<usize> {
        match ty {
            SourceType::Unit => Some(0),
            SourceType::Integer { .. } => Some(1),
            SourceType::Aggregate { .. } | SourceType::Sum { .. } => {
                let schema = self.schema_of(ty)?;
                let aggregate = &self.file.aggregates[schema as usize];
                match aggregate.kind {
                    AggregateKind::Domain => Some(1),
                    AggregateKind::Record => {
                        let mut total = 0;
                        for field in &aggregate.fields {
                            total += self.flat_width(field.ty)?;
                        }
                        Some(total.max(usize::from(aggregate.fields.is_empty())))
                    }
                    AggregateKind::Sum => {
                        let mut widest = 0;
                        for variant in &aggregate.variants {
                            let member = self.schema_of(*variant)?;
                            let mut total = 0;
                            for field in &self.file.aggregates[member as usize].fields {
                                total += self.flat_width(field.ty)?;
                            }
                            widest = widest.max(total);
                        }
                        Some(1 + widest)
                    }
                }
            }
            _ => None,
        }
    }

    /// The request chain that produced the instance at function `index`, for an error.
    pub fn chain(&self, index: usize) -> String {
        self.chain_of(&Requester::Instance(index))
    }

    pub fn is_instance(&self, index: usize) -> bool {
        self.requesters.contains_key(&index)
    }

    pub fn function(&self, index: usize) -> &CheckedSourceFunction {
        &self.file.functions[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use typecheck::{Limits, check_module};

    // Recursion between functions is refused by the checker, so a program cannot ask for an
    // ever larger instance; the limits are exercised with small ones instead.
    const NESTED: &str = "#unrestricted\nWord has value in 0 to 255\n\nPair(element) has\n    left element\n    right element\n\ninner(shape, p shape) shape: return p\n\nouter(element, a element, b element) Pair(element):\n    return inner(Pair(element), Pair(left: a, right: b))\n\n#entry\nstart() in 0 to 255:\n    r := outer(Word, Word(value: 4), Word(value: 6))\n    Pair(l, m) := r\n    return m.value\n";

    fn build(limits: MonoLimits) -> Result<usize, String> {
        let source = parser::parse_strict(NESTED).expect("parses");
        let module = check_module("m", &source, &[], Limits::default()).expect("checks");
        let program = Program::new([&module]);
        World::build(&program, &module, limits)
            .map(|world| world.file.functions.len())
            .map_err(|error| error.to_string())
    }

    #[test]
    fn the_defaults_admit_a_nested_instantiation() {
        assert!(build(MonoLimits::default()).is_ok());
    }

    #[test]
    fn a_type_argument_nested_too_deeply_is_refused_naming_the_chain() {
        let limits = MonoLimits { type_depth_max: 1, ..MonoLimits::default() };
        let error = build(limits).unwrap_err();
        assert!(error.contains("nesting depth"), "{error}");
        assert!(error.contains("inner<m.Pair<m.Word>>") && error.contains("outer"), "{error}");
    }

    #[test]
    fn too_many_instances_are_refused() {
        let limits = MonoLimits { instances_max: 1, ..MonoLimits::default() };
        assert!(build(limits).unwrap_err().contains("number of instances"));
    }

    #[test]
    fn too_many_body_nodes_are_refused() {
        let limits = MonoLimits { body_nodes_max: 5, ..MonoLimits::default() };
        assert!(build(limits).unwrap_err().contains("nodes of instantiated bodies"));
    }

    #[test]
    fn the_nodes_of_a_type_argument_list_are_limited() {
        let limits = MonoLimits { type_nodes_max: 1, ..MonoLimits::default() };
        assert!(build(limits).unwrap_err().contains("size of the type arguments"));
    }
}
