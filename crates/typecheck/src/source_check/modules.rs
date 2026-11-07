//! Protected checked snapshots. No dependency AST is retained. The recorded bodies of a
//! module's own functions are, as shared immutable data, so an importer can evaluate a call
//! whose arguments are all known (the compile-time evaluation rule) instead of emitting it.
use super::*;
use diagnostics::{DiagnosticResult, FunctionDeclaration};

/// An immutable, body-verified dependency capsule. Raw checked-file data cannot create one.
#[derive(Debug)]
pub struct CheckedSourceModule {
    pub(super) name: String,
    pub(super) checked: CheckedSourceFile,
    pub(super) layouts: Vec<contracts::Layout>,
    pub(super) functions: Vec<calls::Function>,
    pub(super) exports: Vec<(String, Export)>,
    /// The module's constants as the numbers they evaluated to.
    pub(super) constants: Vec<(String, I256)>,
    /// The module's type aliases and the types they stand for.
    pub(super) aliases: Vec<(String, SourceType)>,
    pub(super) homes: homes::Homes,
    /// The declarations of the traits this module defines, for importers that name them.
    pub(super) traits: Vec<imported_traits::TraitSource>,
    /// Inline nullary accessors of this module's types and its dependencies': (type, method,
    /// field), so a claim in an importer can name `x.initialized_count` (see `numbers.rs`).
    pub(super) accessors: Vec<(String, String, String)>,
    /// Those accessors that are exactly `return self.field` (read as the field itself).
    pub(super) plain_accessors: Vec<(String, String)>,
}

impl CheckedSourceModule {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn checked(&self) -> &CheckedSourceFile {
        &self.checked
    }

    /// The module's constants as the numbers they evaluated to (folded at check time).
    pub fn constants(&self) -> &[(String, I256)] {
        &self.constants
    }

    /// The module's type aliases and the types they stand for.
    pub fn aliases(&self) -> &[(String, SourceType)] {
        &self.aliases
    }

    #[cfg(test)]
    pub(super) fn summaries(&self, name: &str) -> &[summaries::Summary] {
        &self
            .functions
            .iter()
            .find(|f| f.signature.name == name)
            .unwrap()
            .summaries
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Export {
    Type(usize),
    Function(usize),
}

struct Dependency {
    instances: Vec<SourceType>,
    types: Vec<usize>,
    functions: Vec<usize>,
}

/// Verify a named source snapshot against already-verified dependency capsules.
///
/// Only explicit selected-name imports are supported; this API never loads files.
/// Entry contracts are call-site goals, and dependency calls use body-derived
/// origins rather than rechecking or retaining the dependency's source body.
/// A fresh nominal origin is allocated for this verification, independently of
/// the module's display name. Duplicate or self dependency names are rejected.
pub fn check_module(
    name: &str,
    source: &SourceFile,
    dependencies: &[&CheckedSourceModule],
    limits: crate::Limits,
) -> Result<CheckedSourceModule> {
    check_module_diagnostic(name, source, dependencies, limits).map_err(|failure| failure.error)
}

/// Verify one module spread over several files, in the given order.
///
/// File `index` must have been parsed with `parser::parse_file(.., index, ..)` so
/// that its span ids name it; a file parsed for another slot is rejected as malformed.
/// All files share one declaration namespace, and every other rule is that of
/// [`check_module`]. Failures name the file by index in [`SourceCheckFailure::file`].
pub fn check_module_files(
    name: &str,
    files: &[SourceFile],
    dependencies: &[&CheckedSourceModule],
    limits: crate::Limits,
) -> Result<CheckedSourceModule> {
    check_module_files_diagnostic(name, files, dependencies, limits)
        .map_err(|failure| failure.error)
}

/// [`check_module_files`] with the first failure located in its file.
pub fn check_module_files_diagnostic(
    name: &str,
    files: &[SourceFile],
    dependencies: &[&CheckedSourceModule],
    limits: crate::Limits,
) -> DiagnosticResult<CheckedSourceModule> {
    if name.is_empty() {
        return Err(SourceCheckError::Malformed.into());
    }
    check(name, files, dependencies, limits)
}

/// Verify a named snapshot with optional first-failure context in `source`.
///
/// Dependencies remain source-free capsules. Module-name and dependency-list
/// failures are unlocated; declaration/import contexts never refer to dependencies.
/// Acceptance and work limits are identical to [`check_module`].
pub fn check_module_diagnostic(
    name: &str,
    source: &SourceFile,
    dependencies: &[&CheckedSourceModule],
    limits: crate::Limits,
) -> DiagnosticResult<CheckedSourceModule> {
    if name.is_empty() {
        return Err(SourceCheckError::Malformed.into());
    }
    check(name, std::slice::from_ref(source), dependencies, limits)
}

/// Every top-level item of file `index` must carry ids that name file `index`.
fn files_name_themselves(files: &[SourceFile]) -> bool {
    files.iter().enumerate().all(|(index, file)| {
        file.items
            .iter()
            .all(|item| item.span_id.file() as usize == index)
    })
}

pub(super) fn check(
    name: &str,
    files: &[SourceFile],
    dependencies: &[&CheckedSourceModule],
    limits: crate::Limits,
) -> DiagnosticResult<CheckedSourceModule> {
    check_files(name, files, dependencies, limits).map_err(|mut failure| {
        // A failure inside an imported trait's extra file names no file of this module.
        if failure.file.is_some_and(|file| file >= files.len()) {
            failure.file = None;
            failure.span = None;
        }
        failure
    })
}

fn check_files(
    name: &str,
    files: &[SourceFile],
    dependencies: &[&CheckedSourceModule],
    limits: crate::Limits,
) -> DiagnosticResult<CheckedSourceModule> {
    if files.is_empty() || files.len() >= ast::SpanId::FILES_MAX as usize {
        return Err(SourceCheckError::Malformed.into());
    }
    if !files_name_themselves(files) {
        return Err(SourceCheckError::Malformed.into());
    }
    let (extended, trait_work, imported) = imported_traits::extend(files, dependencies)?;
    let all: &[SourceFile] = extended.as_deref().unwrap_or(files);
    let mut checker = Checker::new_module(all, limits);
    checker.charge(trait_work)?;
    checker.module_names(name, dependencies)?;
    let item_count = checker.item_count();
    checker.charge(item_count)?;
    if item_count > limits.definitions_max as usize {
        return Err(SourceCheckError::Budget.into());
    }
    checker.file_syntax()?;
    checker.collect()?;
    checker.prepare_layouts()?;
    let bindings = checker.function_declarations()?;
    let mut remaps = Vec::new();
    for (slot, dependency) in dependencies.iter().enumerate() {
        remaps.push(checker.import_interface(slot, dependency)?);
    }
    checker.import_names(dependencies, &remaps)?;
    checker.import_trait_names(dependencies, &remaps)?;
    checker.sort_symbols()?;
    checker.evaluate_constants()?;
    checker.declare_union_variants()?;
    let schemas = checker.validate_source_schemas()?;
    checker.compile_layouts(schemas);
    checker.resolve_variant_payloads()?;
    checker.validate_unrestricted()?;
    checker.validate_layouts()?;
    checker.resolve_aliases()?;
    let graph = checker.call_graph(&bindings)?;
    checker.check_bodies(&bindings, &graph, limits.parameters_max)?;
    checker.entry_contract(&bindings)?;
    let traits = checker.exported_traits(files.len(), name)?;
    let mut module = checker.finish_module(name, bindings.len())?;
    module.traits = traits;
    module.homes = homes::Homes::of(&module, name, dependencies);
    // An imported trait's extra file is the importer's own copy: it is neither re-exported nor
    // a place an author can be pointed to.
    for trait_name in &imported {
        let prefix = format!("{trait_name}.");
        module.exports.retain(|(export, _)| !export.starts_with(&prefix));
    }
    module.checked.notes.retain(|note| note.file.is_none_or(|file| file < files.len()));
    Ok(module)
}

impl<'a> Checker<'a> {
    pub(super) fn cached_source_type(&mut self, key: (usize, bool)) -> Result<Option<SourceType>> {
        let Some(cache) = &self.replay.type_cache else {
            return Ok(None);
        };
        self.charge((usize::BITS - cache.len().leading_zeros()) as usize + 1)?;
        let cache = self.replay.type_cache.as_ref().unwrap();
        Ok(cache
            .binary_search_by_key(&key, |entry| entry.0)
            .ok()
            .map(|index| cache[index].1))
    }

    pub(super) fn cache_source_type(&mut self, key: (usize, bool), ty: SourceType) -> Result<()> {
        if let Some(cache) = &self.replay.type_cache {
            self.charge(cache.len() + 1)?;
            let cache = self.replay.type_cache.as_mut().unwrap();
            let index = cache
                .binary_search_by_key(&key, |entry| entry.0)
                .unwrap_err();
            cache.insert(index, (key, ty));
        }
        Ok(())
    }

    fn sort_symbols(&mut self) -> Result<()> {
        for symbols in [&mut self.module.type_names, &mut self.module.function_names] {
            for index in 1..symbols.len() {
                let comparisons = (usize::BITS - index.leading_zeros()) as usize + 1;
                let work = comparisons
                    .checked_mul(symbols[index].0.len() + 1)
                    .and_then(|work| work.checked_add(index + 1))
                    .ok_or(SourceCheckError::Budget)?;
                prepay(&mut self.remaining, work)?;
                let position = symbols[..index].partition_point(|entry| entry.0 < symbols[index].0);
                symbols[position..=index].rotate_right(1);
            }
        }
        Ok(())
    }

    /// The method `Type.name`, found by binary search of the sorted function symbols.
    pub(super) fn lookup_method(&mut self, owner: &str, method: &str) -> Result<Option<usize>> {
        let qualified = format!("{owner}.{method}");
        let count = self.module.function_names.len();
        let comparisons = usize::BITS - count.leading_zeros();
        self.charge_product(comparisons as usize + 1, qualified.len() + 1)?;
        let symbols = &self.module.function_names;
        Ok(symbols
            .binary_search_by(|(other, _)| other.as_str().cmp(qualified.as_str()))
            .ok()
            .map(|index| symbols[index].1))
    }

    pub(super) fn lookup_symbol(&mut self, name: &'a str, function: bool) -> Result<Option<usize>> {
        // A replay sees the same immutable source strings. Pointer keys are local
        // to this checker and never occur in a published dependency capsule.
        let key = (name.as_ptr() as usize, name.len(), function);
        let cached = if let Some(cache) = &self.replay.symbol_cache {
            let count = cache.len();
            self.charge((usize::BITS - count.leading_zeros()) as usize + 1)?;
            Some(
                self.replay
                    .symbol_cache
                    .as_ref()
                    .unwrap()
                    .binary_search_by_key(&key, |entry| entry.0),
            )
        } else {
            None
        };
        if let Some(Ok(index)) = cached {
            return Ok(self.replay.symbol_cache.as_ref().unwrap()[index].1);
        }
        let count = if function {
            self.module.function_names.len()
        } else {
            self.module.type_names.len()
        };
        let comparisons = usize::BITS - count.leading_zeros();
        self.charge_product(comparisons as usize + 1, name.len() + 1)?;
        let symbols = if function {
            &self.module.function_names
        } else {
            &self.module.type_names
        };
        let result = symbols
            .binary_search_by(|(other, _)| other.as_str().cmp(name))
            .ok()
            .map(|index| symbols[index].1);
        if let Some(Err(index)) = cached {
            self.charge(self.replay.symbol_cache.as_ref().unwrap().len() + 1)?;
            self.replay
                .symbol_cache
                .as_mut()
                .unwrap()
                .insert(index, (key, result));
        }
        Ok(result)
    }

    fn module_names(&mut self, name: &str, dependencies: &[&CheckedSourceModule]) -> Result<()> {
        self.charge(name.len() + dependencies.len())?;
        if !name.is_empty() && !name.split('.').all(identifier) {
            return Err(SourceCheckError::Malformed);
        }
        for (index, dependency) in dependencies.iter().enumerate() {
            self.charge(dependency.name.len() + 1)?;
            if dependency.name == name {
                return Err(SourceCheckError::Duplicate);
            }
            for previous in &dependencies[..index] {
                self.charge(dependency.name.len() + 1)?;
                if previous.name == dependency.name {
                    return Err(SourceCheckError::Duplicate);
                }
            }
        }
        Ok(())
    }

    fn function_declarations(&mut self) -> DiagnosticResult<Vec<FunctionDeclaration<'a>>> {
        let mut bindings = Vec::new();
        self.module.schema_cursor = 0;
        let files = self.files;
        for file in files {
            let mut pending = None;
            for item in &file.items {
                self.function_declaration(item, &mut bindings, &mut pending)
                    .map_err(|error| self.failure(error, item.span_id))?;
            }
            if let Some(marker) = pending {
                return Err(self.failure(SourceCheckError::Malformed, marker));
            }
        }
        self.require_terminals()?;
        Ok(bindings)
    }

    /// The operator a method's `#infix('op')` names. The method must be a receiver method
    /// taking exactly one operand, and the operator one of the binary operators a type may define.
    fn infix_operator(
        &mut self,
        method: &'a Binding,
        parent: SpanId,
    ) -> Result<Option<InfixOperator>> {
        let mut operator = None;
        for attribute in &method.attributes {
            let FieldAttribute::Infix {
                hash,
                keyword,
                open,
                operator: written,
                close,
            } = attribute.value
            else {
                // `#private` hides the method from importers; it names no operator.
                if let FieldAttribute::Private {
                    hash,
                    private_keyword,
                } = attribute.value
                {
                    self.child(parent, attribute.span_id)?;
                    self.child(attribute.span_id, hash)?;
                    self.child(attribute.span_id, private_keyword)?;
                    self.token(hash, "#")?;
                    self.token(private_keyword, "private")?;
                    continue;
                }
                // `#inline` is a hint the checker has nothing to say about.
                if let FieldAttribute::Inline { hash, keyword } = attribute.value {
                    self.child(parent, attribute.span_id)?;
                    self.child(attribute.span_id, hash)?;
                    self.child(attribute.span_id, keyword)?;
                    self.token(hash, "#")?;
                    self.token(keyword, "inline")?;
                    continue;
                }
                return Err(SourceCheckError::Unsupported);
            };
            self.child(parent, attribute.span_id)?;
            for (part, text) in [(hash, "#"), (keyword, "infix"), (open, "("), (close, ")")] {
                self.child(attribute.span_id, part)?;
                self.token(part, text)?;
            }
            self.child(attribute.span_id, written)?;
            let spelled = self.span(written)?;
            let found = spelled
                .strip_prefix('\'')
                .and_then(|text| text.strip_suffix('\''))
                .and_then(|text| {
                    InfixOperator::ALL
                        .into_iter()
                        .find(|op| op.symbol() == text)
                });
            let Some(found) = found else {
                let detail = format!(
                    "`#infix({spelled})` is not supported on `{}`: use one of {}",
                    method.name.value,
                    InfixOperator::ALL
                        .map(|op| format!("'{}'", op.symbol()))
                        .join(", ")
                );
                return Err(self.explain(SourceCheckError::InfixOperator, detail));
            };
            if operator.replace(found).is_some() {
                return Err(SourceCheckError::Duplicate);
            }
        }
        if operator.is_some() {
            let parameters = method
                .parameters
                .as_ref()
                .map_or(&[][..], |parameters| &parameters.value[..]);
            if parameters
                .first()
                .map(|first| first.value.name.value.as_str())
                != Some("self")
            {
                let detail = format!(
                    "`#infix` method `{}` must take `self` first",
                    method.name.value
                );
                return Err(self.explain(SourceCheckError::MethodReceiver, detail));
            }
            if parameters.len() != 2 {
                let detail = format!(
                    "`#infix` method `{}` must take `self` and exactly one operand",
                    method.name.value
                );
                return Err(self.explain(SourceCheckError::InfixOperator, detail));
            }
        }
        Ok(operator)
    }

    /// A linear domain must define a terminal exit: a method that takes `self` and does
    /// not hand back a value of the domain. Without one a value could never be ended.
    fn require_terminals(&mut self) -> DiagnosticResult<()> {
        for schema in 0..self.module.schemas.len() {
            let declaration = self.module.schemas[schema];
            // Only a linear domain needs a way to be ended.
            if !matches!(declaration.body.value, TypeBody::Domain(_))
                || self.module.schema_capability[schema] == Capability::Unrestricted
            {
                continue;
            }
            let name = declaration.name.value.as_str();
            let terminal = self.module.schema_domain_has[schema].is_some_and(|has| {
                let TypeBody::Has(members) = &has.body.value else {
                    return false;
                };
                // The type's own methods and those its provided traits add.
                let mut methods: Vec<&Binding> = Vec::new();
                for member in members {
                    match &member.value {
                        TypeMember::Binding(method) => methods.push(method),
                        TypeMember::Provision(provided) => {
                            for (declaration, _) in &self.module.traits {
                                if declaration.name.value != provided.value {
                                    continue;
                                }
                                if let TypeBody::Has(trait_members) = &declaration.body.value {
                                    methods.extend(trait_members.iter().filter_map(|m| {
                                        match &m.value {
                                            TypeMember::Binding(method) => Some(&**method),
                                            _ => None,
                                        }
                                    }));
                                }
                            }
                        }
                        TypeMember::ProvisionApplied(_) | TypeMember::Error => {}
                    }
                }
                methods.into_iter().any(|method| {
                    let takes_self = method
                        .parameters
                        .as_ref()
                        .and_then(|parameters| parameters.value.first())
                        // A receiver that only borrows cannot end the value.
                        .is_some_and(|first| {
                            first.value.name.value == "self"
                                && first.value.convention.as_ref().is_none_or(|convention| {
                                    !matches!(
                                        convention.value.keyword.value,
                                        Convention::Ref | Convention::Mut
                                    )
                                })
                        });
                    let returns_domain = method.annotation.as_ref().is_some_and(|annotation| {
                        matches!(&annotation.value, Annotation::Expression(expression)
                            if matches!(&expression.value, Expr::Name(result) if result == name))
                    });
                    takes_self && !returns_domain
                })
            });
            self.charge(name.len() + 1)?;
            if !terminal {
                let detail = format!(
                    "`{name}` is linear, so it needs a method that takes `self` and ends it"
                );
                let error = self.explain(SourceCheckError::MissingTerminal, detail);
                return Err(self.failure(error, declaration.name.span_id));
            }
        }
        Ok(())
    }

    /// `#entry` must be followed directly by the function it marks, once per module.
    fn mark_entry(
        &mut self,
        item: &'a Spanned<Item>,
        bindings: &[FunctionDeclaration<'a>],
        pending: &mut Option<SpanId>,
    ) -> Result<()> {
        match &item.value {
            Item::Entry => {
                if pending.replace(item.span_id).is_some() {
                    return Err(SourceCheckError::Duplicate);
                }
            }
            Item::Binding(_) if pending.take().is_some() => {
                if self.module.entry.replace(bindings.len() - 1).is_some() {
                    return Err(SourceCheckError::Duplicate);
                }
            }
            _ if pending.is_some() => return Err(SourceCheckError::Malformed),
            _ => {}
        }
        Ok(())
    }

    /// The entry function takes no arguments and yields unit or an exit status.
    fn entry_contract(&mut self, bindings: &[FunctionDeclaration<'a>]) -> DiagnosticResult<()> {
        let Some(entry) = self.module.entry else {
            return Ok(());
        };
        let span = bindings[entry].span;
        let function = self
            .function_interface(entry)
            .map_err(|e| self.failure(e, span))?;
        let signature = &function.signature;
        let status = I256::from(u8::MAX);
        let exit_status = match signature.result {
            SourceType::Unit => true,
            SourceType::Integer { lower, upper } => lower >= I256::ZERO && upper <= status,
            _ => false,
        };
        if signature.parameters.is_empty() && signature.type_parameters.is_empty() && exit_status {
            return Ok(());
        }
        Err(self.failure(SourceCheckError::Type, span))
    }

    fn function_declaration(
        &mut self,
        item: &'a Spanned<Item>,
        bindings: &mut Vec<FunctionDeclaration<'a>>,
        pending: &mut Option<SpanId>,
    ) -> Result<()> {
        match &item.value {
            Item::Entry
            | Item::Load
            | Item::Store
            | Item::Wrap
            | Item::Unrestricted
            | Item::Instruction { .. }
            | Item::Import(_) => {}
            Item::Type(declaration) => {
                let at = self.module.schema_cursor;
                let schema = self
                    .module
                    .schemas
                    .get(at)
                    .is_some_and(|schema| std::ptr::eq(*schema, &**declaration))
                    .then_some(at);
                if schema.is_some() {
                    self.module.schema_cursor += 1;
                }
                self.method_declarations(item, declaration, schema, bindings)?
            }
            Item::Binding(binding)
                if constants::is_constant(binding)
                    && binding
                        .value
                        .as_ref()
                        .is_some_and(|body| constants::integer_form(&body.expression)) =>
            {
                self.declare_constant(binding, item.span_id)?
            }
            Item::Binding(binding) => {
                self.binding_spans(binding, item.span_id)?;
                self.unique_symbol(&binding.name.value)?;
                self.charge(binding.name.value.len() + 2)?;
                self.module
                    .function_names
                    .push((binding.name.value.clone(), bindings.len()));
                self.module.functions.push(None);
                bindings.push(FunctionDeclaration {
                    binding,
                    span: item.span_id,
                    receiver: None,
                    infix: None,
                    provided: false,
                    trait_result: None,
                    abstract_trait: None,
                });
            }
            _ => return Err(SourceCheckError::Unsupported),
        }
        self.mark_entry(item, bindings, pending)
    }

    /// Each method of a non-generic record or domain becomes the function `Type.name` whose
    /// bare `self` has the receiver type. A provided trait adds its methods the same way,
    /// except those the type writes itself. Only by-value `self` is supported.
    fn method_declarations(
        &mut self,
        item: &'a Spanned<Item>,
        declaration: &'a TypeDeclaration,
        schema: Option<usize>,
        bindings: &mut Vec<FunctionDeclaration<'a>>,
    ) -> Result<()> {
        if self.is_alias(declaration) {
            return Ok(());
        }
        let TypeBody::Has(members) = &declaration.body.value else {
            return Ok(());
        };
        if self
            .module
            .traits
            .iter()
            .any(|(other, _)| std::ptr::eq(*other, declaration))
        {
            // A trait's bodies are checked where a type provides them, but its member
            // headers are validated where it is declared.
            self.child(item.span_id, declaration.body.span_id)?;
            self.fields_of(declaration)?;
            for member in members {
                let TypeMember::Binding(method) = &member.value else {
                    continue;
                };
                if diagnostics::self_is_misplaced(method) {
                    let detail = format!(
                        "`self` must be the first parameter of `{}`",
                        method.name.value
                    );
                    return Err(self.explain(SourceCheckError::MethodReceiver, detail));
                }
                self.child(item.span_id, member.span_id)?;
                self.binding_spans(method, member.span_id)?;
                self.infix_operator(method, member.span_id)?;
                // A method with no body is the signature a call through a bound reads.
                if method.value.is_none() {
                    let name = format!("{}.{}", declaration.name.value, method.name.value);
                    self.unique_symbol(&name)?;
                    self.charge(name.len() + 2)?;
                    self.module.function_names.push((name, bindings.len()));
                    self.module.functions.push(None);
                    bindings.push(FunctionDeclaration {
                        binding: method,
                        span: member.span_id,
                        receiver: None,
                        infix: None,
                        provided: false,
                        trait_result: None,
                        abstract_trait: Some(declaration),
                    });
                }
            }
            return Ok(());
        }
        // A record is its own schema; a domain's `has` block belongs to the domain.
        let receiver = schema
            .or_else(|| self.paired_schema(&declaration.name.value))
            .ok_or(SourceCheckError::UnknownName)?;
        if self.paired_schema(&declaration.name.value).is_some() {
            // A domain's or union's members are methods only: a domain's single field is
            // hidden and a union has none.
            self.child(item.span_id, declaration.body.span_id)?;
            if !self.fields_of(declaration)?.is_empty() {
                return Err(SourceCheckError::Unsupported);
            }
        }
        // The traits are looked up once, not once per method.
        let mut provisions = Vec::new();
        for member in members {
            if let Some((name, _, arguments)) = Self::provision_parts(member) {
                provisions.push((self.trait_named(name)?.0, arguments));
            }
        }
        for member in members {
            if let TypeMember::Binding(method) = &member.value
                && diagnostics::is_method(method)
            {
                let inherited = self.provided_infix(&provisions, method)?;
                if let Some(field) = accessor_field(method) {
                    self.accessors.push((
                        declaration.name.value.clone(),
                        method.name.value.clone(),
                        field,
                    ));
                    if plain_accessor(method) {
                        self.plain_accessors
                            .push((declaration.name.value.clone(), method.name.value.clone()));
                    }
                }
                self.register_method(
                    (item.span_id, member.span_id),
                    (declaration, members, receiver),
                    method,
                    inherited,
                    false,
                    bindings,
                )?;
            }
        }
        for member in members {
            let Some((name, name_span, arguments)) = Self::provision_parts(member) else {
                continue;
            };
            self.token(name_span, name)?;
            if let TypeMember::ProvisionApplied(applied) = &member.value {
                self.child(member.span_id, applied.span_id)?;
            }
            let (provided, trait_span) = self.trait_named(name)?;
            // A trait applied to arguments takes as many as it has parameters.
            let wanted = provided.parameters.as_ref().map_or(0, |p| p.value.len());
            if wanted != arguments.len() {
                return Err(SourceCheckError::Type);
            }
            let TypeBody::Has(trait_members) = &provided.body.value else {
                return Err(SourceCheckError::Malformed);
            };
            for trait_member in trait_members {
                let TypeMember::Binding(method) = &trait_member.value else {
                    continue;
                };
                self.charge_product(members.len(), method.name.value.len() + 1)?;
                let own_method = members.iter().find_map(|own| match &own.value {
                    TypeMember::Binding(other)
                        if diagnostics::is_method(other) && other.name.value == method.name.value =>
                    {
                        Some(&**other)
                    }
                    _ => None,
                });
                if let Some(own_method) = own_method {
                    // A trait's claims are assumed at every call through it, so a provider must
                    // promise exactly them (it then proves them in its own body).
                    if self.contract_text(method) != self.contract_text(own_method) {
                        let detail = format!(
                            "`{}.{}` must promise the same `where` claims as `{}` declares",
                            declaration.name.value, method.name.value, name
                        );
                        return Err(self.explain(SourceCheckError::Type, detail));
                    }
                    continue;
                }
                if method.value.is_none() {
                    // A required method the type has not written.
                    let detail = format!(
                        "`{}` must write `{}`, which `{}` declares without a body",
                        declaration.name.value, method.name.value, name
                    );
                    return Err(self.explain(SourceCheckError::MissingBody, detail));
                }
                self.register_method(
                    (trait_span, trait_member.span_id),
                    (declaration, members, receiver),
                    method,
                    (None, None),
                    true,
                    bindings,
                )?;
                self.module.provided_from.push((bindings.len() - 1, name.to_owned()));
            }
        }
        Ok(())
    }

    /// A method that takes only `self` and does nothing but `return` ends a value that an
    /// `#unrestricted` type may simply discard, so it is redundant.
    fn note_redundant_terminal(
        &mut self,
        declaration: &'a TypeDeclaration,
        method: &'a Binding,
        member_span: SpanId,
    ) {
        let takes_only_self = method
            .parameters
            .as_ref()
            .is_some_and(|parameters| parameters.value.len() == 1)
            && diagnostics::is_method(method)
            && !diagnostics::self_is_misplaced(method);
        let empty = method.annotation.is_none()
            && matches!(
                method.value.as_ref().map(|value| &value.expression.value),
                Some(Expr::Block(block))
                    if matches!(&block.statements[..], [statement]
                        if matches!(&statement.value, Statement::Return(ret) if ret.value.is_none()))
            );
        if takes_only_self && empty {
            let detail = format!(
                "`{}` does nothing: `{}` is `#unrestricted`, so its values may be discarded without it",
                method.name.value, declaration.name.value
            );
            self.note(SourceCheckNoteKind::RedundantTerminal, member_span, detail);
        }
    }

    pub(super) fn trait_named(&mut self, name: &str) -> Result<(&'a TypeDeclaration, SpanId)> {
        self.charge_product(self.module.traits.len(), name.len() + 1)?;
        self.module
            .traits
            .iter()
            .find(|(declaration, _)| declaration.name.value == name)
            .copied()
            .ok_or(SourceCheckError::UnknownName)
    }

    /// The operator a provided trait attaches to a method the type writes itself, so an
    /// override keeps the trait's `#infix`.
    /// The trait a provision names and the arguments it applies it to: `Numeric`, or
    /// `Add(MemorySizeResult)`.
    pub(super) fn provision_parts(
        member: &'a Spanned<TypeMember>,
    ) -> Option<(&'a str, SpanId, &'a [Spanned<Argument>])> {
        match &member.value {
            TypeMember::Provision(name) => Some((name.value.as_str(), name.span_id, &[])),
            TypeMember::ProvisionApplied(expression) => {
                let Expr::Call { callee, arguments } = &expression.value else {
                    return None;
                };
                let Expr::Name(name) = &callee.value else {
                    return None;
                };
                Some((name.as_str(), callee.span_id, arguments.value.as_slice()))
            }
            _ => None,
        }
    }

    /// What a provided trait attaches to a method the type writes itself: its operator, and the
    /// type argument its result must be (`trait_results.rs`).
    fn provided_infix(
        &mut self,
        provisions: &[(&'a TypeDeclaration, &'a [Spanned<Argument>])],
        method: &'a Binding,
    ) -> Result<(Option<InfixOperator>, Option<&'a Expression>)> {
        let (mut operator, mut result) = (None, None);
        for &(provided, arguments) in provisions {
            let TypeBody::Has(trait_members) = &provided.body.value else {
                continue;
            };
            for trait_member in trait_members {
                let TypeMember::Binding(candidate) = &trait_member.value else {
                    continue;
                };
                if candidate.name.value != method.name.value {
                    continue;
                }
                if operator.is_none() {
                    operator = self.infix_operator(candidate, trait_member.span_id)?;
                }
                if result.is_none() {
                    result = self.trait_result_argument(provided, candidate, arguments)?;
                }
            }
        }
        Ok((operator, result))
    }

    #[allow(clippy::too_many_arguments)] // one declaration, threaded explicitly
    fn register_method(
        &mut self,
        (parent, member_span): (SpanId, SpanId),
        (declaration, members, receiver): (&'a TypeDeclaration, &'a [Spanned<TypeMember>], usize),
        method: &'a Binding,
        inherited: (Option<InfixOperator>, Option<&'a Expression>),
        provided: bool,
        bindings: &mut Vec<FunctionDeclaration<'a>>,
    ) -> Result<()> {
        // A generic type's methods are generic over its parameters when they take `self`, which
        // is where the arguments are read from; the others write their own type parameters
        // (`reclaim_storage(x, storage Allocation(x), ...)`), which are the method's alone.
        if diagnostics::self_is_misplaced(method) {
            let detail = format!(
                "`self` must be the first parameter of `{}`",
                method.name.value
            );
            return Err(self.explain(SourceCheckError::MethodReceiver, detail));
        }
        self.child(parent, member_span)?;
        self.binding_spans(method, member_span)?;
        // Fields and methods share one member namespace.
        self.charge_product(members.len(), method.name.value.len() + 1)?;
        let clash = members.iter().any(|other| {
            matches!(&other.value, TypeMember::Binding(field)
                if !diagnostics::is_method(field) && field.name.value == method.name.value)
        });
        if clash {
            return Err(SourceCheckError::Duplicate);
        }
        let (inherited, trait_result) = inherited;
        let name = format!("{}.{}", declaration.name.value, method.name.value);
        self.unique_symbol(&name)?;
        self.charge(name.len() + 2)?;
        let infix = match self.infix_operator(method, member_span)? {
            Some(operator) => Some(operator),
            None => inherited,
        };
        if let Some(operator) = infix {
            self.charge(self.module.infix_decls.len() + 1)?;
            let duplicate = bindings
                .iter()
                .any(|other| other.receiver == Some(receiver) && other.infix == Some(operator));
            if duplicate {
                return Err(SourceCheckError::Duplicate);
            }
            self.module.infix_decls.push((operator, bindings.len()));
        }
        if !provided && self.module.schema_capability[receiver] == Capability::Unrestricted {
            self.note_redundant_terminal(declaration, method, member_span);
        }
        self.module.function_names.push((name, bindings.len()));
        self.module.functions.push(None);
        bindings.push(FunctionDeclaration {
            binding: method,
            span: member_span,
            receiver: Some(receiver),
            infix,
            provided,
            trait_result,
            abstract_trait: None,
        });
        Ok(())
    }

    pub(super) fn unique_symbol(&mut self, name: &str) -> Result<()> {
        self.charge_product(
            self.module.type_names.len() + self.module.function_names.len(),
            name.len() + 1,
        )?;
        if self
            .module
            .type_names
            .iter()
            .any(|(other, _)| other == name)
            || self
                .module
                .function_names
                .iter()
                .any(|(other, _)| other == name)
        {
            return Err(SourceCheckError::Duplicate);
        }
        Ok(())
    }

    fn import_interface(
        &mut self,
        slot: usize,
        dependency: &CheckedSourceModule,
    ) -> Result<Dependency> {
        self.charge_interface(dependency)?;
        self.charge(dependency.accessors.len() * (self.accessors.len() + 1))?;
        for accessor in &dependency.accessors {
            if !self.accessors.contains(accessor) {
                self.accessors.push(accessor.clone());
            }
        }
        self.charge(dependency.plain_accessors.len() * (self.plain_accessors.len() + 1))?;
        for accessor in &dependency.plain_accessors {
            if !self.plain_accessors.contains(accessor) {
                self.plain_accessors.push(accessor.clone());
            }
        }
        let mut types = Vec::new();
        for layout in &dependency.layouts {
            self.charge(self.module.layouts.len())?;
            let index = self
                .module
                .layouts
                .iter()
                .position(|other| other.origin == layout.origin);
            let index = match index {
                Some(index) => index,
                None => {
                    self.interface_definition()?;
                    if layout.schema.parameters.len() > self.parameters_max as usize {
                        return Err(SourceCheckError::Budget);
                    }
                    let index = self.module.layouts.len();
                    self.module.layouts.push(layout.clone());
                    index
                }
            };
            types.push(index);
        }
        let instances = self.import_instances(dependency, &types)?;
        for (index, layout) in dependency.layouts.iter().enumerate() {
            // Every occurrence of an origin has the same already-validated layout.
            let target = types[index];
            self.charge(layout.schema.fields.len())?;
            for (field, source) in self.module.layouts[target]
                .schema
                .fields
                .iter_mut()
                .zip(&layout.schema.fields)
            {
                field.ty = remap_type(source.ty, &instances);
            }
            self.module.layouts[target].schema.variants = layout
                .schema
                .variants
                .iter()
                .map(|ty| remap_type(*ty, &instances))
                .collect();
        }
        let mut functions = Vec::new();
        for (position, function) in dependency.functions.iter().enumerate() {
            self.interface_definition()?;
            if function.parameters.len() > self.parameters_max as usize {
                return Err(SourceCheckError::Budget);
            }
            let mut function = function.clone();
            remap_function(&mut function, &instances, Some(&types));
            functions.push(self.module.functions.len());
            self.module.imported.push((slot as u32, position as u32));
            self.module.functions.push(Some(Rc::new(function)));
        }
        Ok(Dependency {
            instances,
            types,
            functions,
        })
    }

    fn interface_definition(&self) -> Result<()> {
        if self.module.layouts.len() + self.module.functions.len() >= self.definitions_max as usize
        {
            return Err(SourceCheckError::Budget);
        }
        Ok(())
    }

    fn import_instances(
        &mut self,
        dependency: &CheckedSourceModule,
        schemas: &[usize],
    ) -> Result<Vec<SourceType>> {
        let mut types = Vec::new();
        for instance in &dependency.checked.instances {
            self.charge(instance.arguments.len() + 1)?;
            let arguments = instance
                .arguments
                .iter()
                .map(|ty| remap_type(*ty, &types))
                .collect();
            let head = match instance.head {
                CheckedSourceInstanceHead::Nominal { schema } => {
                    CheckedSourceInstanceHead::Nominal {
                        schema: schemas[schema as usize] as u32,
                    }
                }
                head @ (CheckedSourceInstanceHead::Array { .. }
                | CheckedSourceInstanceHead::Slice
                | CheckedSourceInstanceHead::Sized
                | CheckedSourceInstanceHead::Sequence
                | CheckedSourceInstanceHead::Pointer) => head,
            };
            types.push(self.intern_instance(head, arguments)?);
        }
        Ok(types)
    }

    fn charge_interface(&mut self, dependency: &CheckedSourceModule) -> Result<()> {
        self.charge(
            dependency.layouts.len()
                + dependency.functions.len()
                + dependency.checked.instances.len()
                + dependency.exports.len(),
        )?;
        for layout in &dependency.layouts {
            layout.prepay(&mut self.remaining)?;
        }
        for function in &dependency.functions {
            function.prepay(&mut self.remaining)?;
        }
        for (name, _) in &dependency.exports {
            self.charge(name.len() + 1)?;
        }
        Ok(())
    }

    fn import_names(
        &mut self,
        dependencies: &[&CheckedSourceModule],
        remaps: &[Dependency],
    ) -> DiagnosticResult<()> {
        let files = self.files;
        for item in files.iter().flat_map(|file| &file.items) {
            let Item::Import(import) = &item.value else {
                continue;
            };
            self.import_declaration(item.span_id, import, dependencies, remaps)
                .map_err(|error| self.failure(error, item.span_id))?;
        }
        Ok(())
    }

    fn import_declaration(
        &mut self,
        parent: SpanId,
        import: &'a Import,
        dependencies: &[&CheckedSourceModule],
        remaps: &[Dependency],
    ) -> Result<()> {
        self.charge(import.entries.len())?;
        for entry in &import.entries {
            self.child(parent, entry.span_id)?;
            self.child(entry.span_id, entry.source.span_id)?;
            if entry.alias.is_some() || entry.members.is_empty() {
                return Err(SourceCheckError::Unsupported);
            }
            let ImportSource::Module(path) = &entry.source.value else {
                return Err(SourceCheckError::Unsupported);
            };
            let module = self.module_path(path, entry.source.span_id)?;
            self.charge_product(dependencies.len(), module.len() + 1)?;
            let index = dependencies
                .iter()
                .position(|dependency| dependency.name == module)
                .ok_or(SourceCheckError::UnknownName)?;
            self.import_members(entry, dependencies[index], &remaps[index])?;
        }
        Ok(())
    }

    fn module_path(&mut self, path: &'a [Name], parent: SpanId) -> Result<String> {
        self.charge(path.len())?;
        let mut output = String::new();
        for component in path {
            self.child(parent, component.span_id)?;
            let component = self.name(component)?;
            self.charge(component.len() + 1)?;
            if !output.is_empty() {
                output.push('.');
            }
            output.push_str(component);
        }
        self.token(parent, &output)?;
        Ok(output)
    }

    fn import_members(
        &mut self,
        entry: &'a Spanned<ImportEntry>,
        dependency: &CheckedSourceModule,
        remap: &Dependency,
    ) -> Result<()> {
        self.charge(entry.members.len())?;
        for member in &entry.members {
            self.child(entry.span_id, member.span_id)?;
            if member.alias.is_some() || member.path.len() != 1 {
                return Err(SourceCheckError::Unsupported);
            }
            self.child(member.span_id, member.path[0].span_id)?;
            let name = self.name(&member.path[0])?;
            // A trait comes as its own declaration (see `imported_traits`), not as a symbol.
            self.charge_product(dependency.traits.len(), name.len() + 1)?;
            if dependency.traits.iter().any(|t| t.name == name) {
                continue;
            }
            // The files of one module may each import what they use: the same name from the same
            // module, imported again, is the same name.
            if let Some((_, Export::Type(index) | Export::Function(index))) =
                dependency.exports.iter().find(|(other, _)| other == name)
            {
                let again = |names: &[(String, usize)], target: usize| {
                    names.iter().any(|(other, found)| other == name && *found == target)
                };
                let is_type = matches!(
                    dependency.exports.iter().find(|(other, _)| other == name),
                    Some((_, Export::Type(_)))
                );
                self.charge(self.module.type_names.len() + self.module.function_names.len())?;
                let seen = if is_type {
                    again(&self.module.type_names, remap.types[*index])
                } else {
                    again(&self.module.function_names, remap.functions[*index])
                };
                if seen {
                    continue;
                }
            }
            self.unique_symbol(name)?;
            self.charge_product(dependency.aliases.len(), name.len() + 1)?;
            if let Some((_, ty)) = dependency.aliases.iter().find(|(other, _)| other == name) {
                let ty = remap_type(*ty, &remap.instances);
                self.module.aliases.push(aliases::Alias {
                    name: name.to_owned(),
                    expression: None,
                    ty: Some(ty),
                    active: false,
                    span: entry.span_id,
                });
                continue;
            }
            self.charge_product(dependency.exports.len(), name.len() + 1)?;
            let Some(export) = dependency
                .exports
                .iter()
                .find(|(other, _)| other == name)
                .map(|(_, export)| *export)
            else {
                // Not a type or function: a constant of the dependency, as its number.
                self.import_constant(name, &dependency.constants)?;
                continue;
            };
            self.charge(name.len() + 1)?;
            match export {
                Export::Type(index) => {
                    self.module
                        .type_names
                        .push((name.to_owned(), remap.types[index]));
                    self.import_methods(name, dependency, remap)?;
                }
                Export::Function(index) => {
                    self.module
                        .function_names
                        .push((name.to_owned(), remap.functions[index]));
                }
            }
        }
        Ok(())
    }

    /// The methods of an imported type, `Type.name`, come with it: they are what its values
    /// are called on, and a type whose methods stayed behind could be named but not used. A
    /// function imported alone brings none, since the type it mentions is then not named
    /// here. The scan is uncharged and each method registered is charged, so a module that
    /// imports types without methods costs what it always did.
    fn import_methods(
        &mut self,
        name: &str,
        dependency: &CheckedSourceModule,
        remap: &Dependency,
    ) -> Result<()> {
        for (qualified, export) in &dependency.exports {
            let Export::Function(index) = export else {
                continue;
            };
            let method = qualified
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('.'));
            if method.is_none_or(|method| method.is_empty() || method.contains('.')) {
                continue;
            }
            self.unique_symbol(qualified)?;
            self.charge(qualified.len() + 2)?;
            self.module
                .function_names
                .push((qualified.clone(), remap.functions[*index]));
        }
        Ok(())
    }

    fn finish_module(mut self, name: &str, count: usize) -> Result<CheckedSourceModule> {
        self.compact_instances()?;
        self.charge(
            self.module.layouts.len()
                + self.module.instances.len()
                + self.module.functions.len()
                + count,
        )?;
        let mut aggregates = Vec::new();
        let mut exports = Vec::new();
        for index in 0..self.module.layouts.len() {
            self.charge_layout_copy(index)?;
            aggregates.push(self.module.layouts[index].schema.clone());
            if index < self.module.schemas.len() {
                let name = self.module.schemas[index].name.value.clone();
                exports.push((name, Export::Type(index)));
            }
        }
        let mut functions = Vec::new();
        let mut checked_functions = Vec::new();
        let mut bodies = Vec::new();
        let mut declines = Vec::new();
        let mut folds = Vec::new();
        for index in 0..self.module.functions.len() {
            self.charge(1)?;
            let function = self.module.functions[index]
                .take()
                .ok_or(SourceCheckError::MissingBody)?;
            let mut function = Rc::try_unwrap(function).map_err(|_| SourceCheckError::Malformed)?;
            function.signature.provided_by = self
                .module
                .provided_from
                .iter()
                .find(|(at, _)| *at == index)
                .map(|(_, trait_name)| trait_name.clone());
            if index < count {
                calls::prepay_signature(&function.signature, &mut self.remaining)?;
                self.charge(function.signature.name.len())?;
                // A private method stays in this module: importers never see it.
                if !function.signature.private {
                    exports.push((function.signature.name.clone(), Export::Function(index)));
                }
                checked_functions.push(function.signature.clone());
                if let Some(decline) = function.decline.clone() {
                    declines.push((index as u32, decline));
                }
                folds.extend(function.folds.iter().map(|callee| (index as u32, *callee)));
                // The published file keeps a copy; the capsule keeps the shared body.
                bodies.push(function.body.as_deref().cloned());
            } else {
                // A transitive import's body refers to its defining module's function
                // table, which an importer cannot resolve, so it is not carried.
                function.body = None;
            }
            functions.push(function);
        }
        let constants = self.exported_constants();
        let aliases = self.exported_aliases();
        let instances = self
            .module
            .instances
            .into_iter()
            .map(|instance| CheckedSourceInstance {
                head: instance.head,
                arguments: instance.arguments,
            })
            .collect();
        Ok(CheckedSourceModule {
            name: name.to_owned(),
            checked: CheckedSourceFile {
                entry: self.module.entry.map(|entry| entry as u32),
                functions: checked_functions,
                bodies,
                aggregates,
                instances,
                notes: self.module.notes,
                declines,
                folds,
            },
            layouts: self.module.layouts,
            accessors: self.accessors.clone(),
            plain_accessors: self.plain_accessors.clone(),
            functions,
            exports,
            constants,
            aliases,
            homes: homes::Homes {
                imported: self.module.imported,
                own: count,
                ..homes::Homes::default()
            },
            traits: Vec::new(),
        })
    }

    fn charge_layout_copy(&mut self, index: usize) -> Result<()> {
        self.module.layouts[index].prepay(&mut self.remaining)
    }

    fn compact_instances(&mut self) -> Result<()> {
        let mut pending = self.interface_types()?;
        pending.extend(self.module.aliases.iter().filter_map(|alias| alias.ty));
        self.charge_product(self.module.instances.len(), 3)?;
        let mut used = vec![false; self.module.instances.len()];
        while let Some(ty) = pending.pop() {
            self.charge(1)?;
            let (SourceType::Aggregate { id }
            | SourceType::Sum { id }
            | SourceType::Array { id }
            | SourceType::Sequence { id }
            | SourceType::Pointer { id }) = ty
            else {
                continue;
            };
            if used[id as usize] {
                continue;
            }
            used[id as usize] = true;
            let instance = &self.module.instances[id as usize];
            let count = instance.arguments.len();
            self.charge(count)?;
            pending.extend_from_slice(&self.module.instances[id as usize].arguments);
        }
        // Instances are interned after their argument instances. This is an
        // iterative topological remap, not a recursive clone of a type tree.
        let mut types = vec![SourceType::Unit; self.module.instances.len()];
        self.module.instance_index.clear();
        self.module.substitutions.clear();
        for (index, mut instance) in std::mem::take(&mut self.module.instances)
            .into_iter()
            .enumerate()
        {
            if !used[index] {
                continue;
            }
            self.charge(instance.arguments.len() + 1)?;
            for argument in &mut instance.arguments {
                *argument = remap_type(*argument, &types);
            }
            types[index] = self.intern_instance(instance.head, instance.arguments)?;
        }
        for alias in &mut self.module.aliases {
            alias.ty = alias.ty.map(|ty| remap_type(ty, &types));
        }
        for layout in &mut self.module.layouts {
            prepay(&mut self.remaining, layout.schema.fields.len())?;
            for field in &mut layout.schema.fields {
                field.ty = remap_type(field.ty, &types);
            }
            prepay(&mut self.remaining, layout.schema.variants.len())?;
            for variant in &mut layout.schema.variants {
                *variant = remap_type(*variant, &types);
            }
        }
        for function in self.module.functions.iter_mut().flatten() {
            prepay_function_types(function, &mut self.remaining)?;
            let function = Rc::get_mut(function).ok_or(SourceCheckError::Malformed)?;
            remap_function(function, &types, None);
        }
        Ok(())
    }

    fn interface_types(&mut self) -> Result<Vec<SourceType>> {
        self.charge(self.module.layouts.len() + self.module.functions.len())?;
        let mut types = Vec::new();
        for layout in &self.module.layouts {
            prepay(&mut self.remaining, layout.schema.fields.len())?;
            types.extend(layout.schema.fields.iter().map(|field| field.ty));
            types.extend_from_slice(&layout.schema.variants);
        }
        for function in self.module.functions.iter().flatten() {
            prepay_function_types(function, &mut self.remaining)?;
            types.extend_from_slice(&function.signature.parameters);
            types.push(function.signature.result);
            // A caller matches the member of each claimed variant by its type.
            types.extend(function.variant_postconditions.iter().map(|(member, _)| *member));
            types.extend(function.variant_effects.iter().map(|(member, _)| *member));
            // Lowering reads the registers of an instruction from its body's node types.
            if let Some(body) = function.body.as_ref().filter(|body| body.effects) {
                types.extend(body.expressions.iter().map(|expr| expr.ty));
            }
            // A template, or a body that applies one, is instantiated from these types.
            if let Some(body) = function
                .body
                .as_ref()
                .filter(|body| !body.effects && body_is_generic(&function.signature, body))
            {
                types.extend(body.expressions.iter().map(|expr| expr.ty));
            }
            if let Some(body) = &function.body {
                types.extend(generic_arguments(body));
                // A bundle's sequence type is made by the recorder and named nowhere else.
                types.extend(body.expressions.iter().filter_map(|expr| match expr.kind {
                    CheckedSourceBodyKind::Bundle { .. } => Some(expr.ty),
                    _ => None,
                }));
            }
            for summary in &function.summaries {
                types.extend(summary.inputs.iter().map(|input| input.ty));
                types.extend(summary.outputs.iter().map(|output| output.ty));
                types.extend(
                    summary
                        .outputs
                        .iter()
                        .filter_map(|output| output.sum.map(|id| SourceType::Sum { id })),
                );
                types.extend(
                    summary
                        .outputs
                        .iter()
                        .filter_map(|output| output.structural),
                );
            }
        }
        Ok(types)
    }
}

fn identifier(name: &str) -> bool {
    name.as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn remap_type(ty: SourceType, instances: &[SourceType]) -> SourceType {
    match ty {
        SourceType::Aggregate { id }
        | SourceType::Sum { id }
        | SourceType::Array { id }
        | SourceType::Sequence { id }
        | SourceType::Pointer { id } => instances[id as usize],
        _ => ty,
    }
}

/// Type traversal does not copy names, guards, dispositions, or child vectors.
/// Import still reserves the full deep-copy cost through `Function::prepay`.
fn prepay_function_types(function: &calls::Function, remaining: &mut u32) -> Result<()> {
    prepay(
        remaining,
        function.signature.parameters.len() + function.parameters.len() + 1,
    )?;
    for summary in &function.summaries {
        prepay(
            remaining,
            summary.inputs.len() + summary.outputs.len() * 3 + 1,
        )?;
    }
    Ok(())
}

/// Rewrites a function from the numbering of the module that defined it to the importer's.
/// `instances` maps type instances. `layouts`, when the function is being imported, maps the
/// layout numbers a body names in `Construct` and in the arms of a `Match`: a body folded in
/// the importer compares those with the importer's own, so an unmapped number would select
/// whichever variant happens to carry it there.
fn remap_function(
    function: &mut calls::Function,
    instances: &[SourceType],
    layouts: Option<&[usize]>,
) {
    for ty in &mut function.signature.parameters {
        *ty = remap_type(*ty, instances);
    }
    function.signature.result = remap_type(function.signature.result, instances);
    for (member, _) in &mut function.variant_effects {
        *member = remap_type(*member, instances);
    }
    for (member, post) in &mut function.variant_postconditions {
        *member = remap_type(*member, instances);
        for constraint in post {
            remap_term(&mut constraint.right, instances);
            if let Some(term) = &mut constraint.left_term {
                remap_term(term, instances);
            }
        }
    }
    for constraint in &mut function.postconditions {
        remap_term(&mut constraint.right, instances);
        if let Some(term) = &mut constraint.left_term {
            remap_term(term, instances);
        }
    }
    // Only nominal and structural types name importer-relative instances; integer and
    // unit bodies stay shared with the capsule instead of being copied per importer.
    if let Some(body) = &mut function.body {
        let names_layouts = layouts.is_some()
            && body.expressions.iter().any(|expr| {
                matches!(
                    expr.kind,
                    CheckedSourceBodyKind::Construct { .. } | CheckedSourceBodyKind::Match { .. }
                )
            });
        let names_instances = body.expressions.iter().any(|expr| {
            !matches!(expr.ty, SourceType::Integer { .. } | SourceType::Unit)
                || !generic_arguments_of(&expr.kind).is_empty()
        });
        if names_instances || names_layouts {
            for expr in &mut std::sync::Arc::make_mut(body).expressions {
                expr.ty = remap_type(expr.ty, instances);
                remap_generic_arguments(&mut expr.kind, instances);
                let Some(layouts) = layouts else {
                    continue;
                };
                match &mut expr.kind {
                    CheckedSourceBodyKind::Construct { schema, .. } => {
                        *schema = layouts[*schema as usize] as u32;
                    }
                    CheckedSourceBodyKind::Match { arms, .. } => {
                        for arm in arms {
                            arm.schema = layouts[arm.schema as usize] as u32;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for parameter in &mut function.parameters {
        if let calls::Parameter::Value(ty) = parameter {
            *ty = remap_type(*ty, instances);
        }
    }
    for summary in &mut function.summaries {
        for input in &mut summary.inputs {
            input.ty = remap_type(input.ty, instances);
        }
        for output in &mut summary.outputs {
            output.ty = remap_type(output.ty, instances);
            if let Some(id) = output.sum {
                let SourceType::Sum { id } = remap_type(SourceType::Sum { id }, instances) else {
                    unreachable!("sum instances retain their kind");
                };
                output.sum = Some(id);
            }
            if let Some(ty) = output.structural {
                output.structural = Some(remap_type(ty, instances));
            }
        }
    }
}

/// Whether the first written parameter of `method` is the bare receiver `self`.
pub(super) fn takes_self(method: &Binding) -> bool {
    method
        .parameters
        .as_ref()
        .and_then(|parameters| parameters.value.first())
        .is_some_and(|first| first.value.name.value == "self" && receiver_annotation(&first.value.annotation))
}

/// A receiver writes no type: bare `self`, or `self and claim` (refinements with no base).
pub(super) fn receiver_annotation(annotation: &Option<Spanned<Annotation>>) -> bool {
    match annotation {
        None => true,
        Some(annotation) => matches!(&annotation.value, Annotation::Refined { base: None, .. }),
    }
}

/// Whether a body is a template (its function has type parameters) or applies one.
fn body_is_generic(signature: &CheckedSourceFunction, body: &CheckedSourceBody) -> bool {
    !signature.type_parameters.is_empty() || !generic_arguments(body).is_empty()
}

/// The types a body's calls apply their callees to.
fn generic_arguments(body: &CheckedSourceBody) -> Vec<SourceType> {
    body.expressions
        .iter()
        .flat_map(|expr| generic_arguments_of(&expr.kind).iter().copied())
        .collect()
}

fn generic_arguments_of(kind: &CheckedSourceBodyKind) -> &[SourceType] {
    match kind {
        CheckedSourceBodyKind::Call { type_arguments, .. } => type_arguments,
        CheckedSourceBodyKind::BoundCall { trait_arguments, .. } => trait_arguments,
        _ => &[],
    }
}

fn remap_generic_arguments(kind: &mut CheckedSourceBodyKind, instances: &[SourceType]) {
    let (CheckedSourceBodyKind::Call {
        type_arguments: arguments,
        ..
    }
    | CheckedSourceBodyKind::BoundCall {
        trait_arguments: arguments,
        ..
    }) = kind
    else {
        return;
    };
    for ty in arguments {
        *ty = remap_type(*ty, instances);
    }
}

/// The field an inline nullary accessor returns: `name(self ...) T: return self.field`, where the
/// returned value may also pass through `.as_num` and `as T`, which change the type of the
/// number and never its value. Anything else is not an accessor a claim may unfold.
pub(super) fn accessor_field(method: &Binding) -> Option<String> {
    let parameters = method.parameters.as_ref()?;
    if parameters.value.len() != 1 || parameters.value[0].value.name.value != "self" {
        return None;
    }
    let Expr::Block(block) = &method.value.as_ref()?.expression.value else {
        return None;
    };
    let [statement] = block.statements.as_slice() else {
        return None;
    };
    let Statement::Return(returned) = &statement.value else {
        return None;
    };
    let mut current = returned.value.as_ref()?;
    loop {
        match &current.value {
            Expr::Cast { value, .. } => current = value,
            Expr::Group(inner) => current = inner,
            Expr::Member { base, name } if name.value == "as_num" => current = base,
            Expr::Member { .. } => {
                // `self.field`, or `self.field.member...` through fields and accessors.
                let mut path = Vec::new();
                let mut part = current;
                loop {
                    match &part.value {
                        Expr::Member { base, name } => {
                            path.push(name.value.as_str());
                            part = base;
                        }
                        Expr::Name(root) if root == "self" => break,
                        _ => return None,
                    }
                }
                path.reverse();
                return Some(path.join("."));
            }
            _ => return None,
        }
    }
}

/// An accessor whose body is exactly `return self.field`: reading it is reading the field, with
/// the field's own type.
pub(super) fn plain_accessor(method: &Binding) -> bool {
    let Some(Expr::Block(block)) = method.value.as_ref().map(|value| &value.expression.value) else {
        return false;
    };
    let [statement] = block.statements.as_slice() else {
        return false;
    };
    let Statement::Return(returned) = &statement.value else {
        return false;
    };
    // `self.field`, or a chain of members from `self` (`self.bytes.length`): a pure read of the
    // receiver. Each hop is a field or another such accessor, resolved when it is read; a hop
    // that is not (a method that computes) makes the read an ordinary call again.
    let mut part = returned.value.as_ref().map(|value| &value.value);
    let mut hops = 0;
    while let Some(Expr::Member { base, .. }) = part {
        hops += 1;
        if let Expr::Name(root) = &base.value {
            // A chain longer than one hop must be a bare read: a method that promises something
            // about its result (`where`) computes it, however it is spelled.
            return root == "self"
                && (hops == 1 || (method.contract.is_none() && method.alternatives.is_empty()));
        }
        part = Some(&base.value);
    }
    false
}

impl<'a> Checker<'a> {
    /// The written `where` claims of a function and of each result variant, spacing collapsed.
    fn contract_text(&self, binding: &Binding) -> Vec<String> {
        let text = |contract: Option<&Spanned<Contract>>| {
            contract
                .and_then(|contract| self.text_of(contract.span_id))
                .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
                .unwrap_or_default()
        };
        std::iter::once(text(binding.contract.as_ref()))
            .chain(binding.alternatives.iter().map(|alt| text(alt.value.contract.as_ref())))
            .collect()
    }
}

/// The types a claim term names (`#size(T)` of a type), as the importer's instances.
fn remap_term(term: &mut contracts::Term, instances: &[SourceType]) {
    match term {
        contracts::Term::Layout { ty, .. } => *ty = remap_type(*ty, instances),
        contracts::Term::Arithmetic(_, left, right) => {
            remap_term(left, instances);
            remap_term(right, instances);
        }
        _ => {}
    }
}

impl<'a> Checker<'a> {
    /// A trait declared by a dependency is read in the dependency's scope: the names its text
    /// mentions were respelled uniquely when the capsule was made, and each spelling is bound
    /// here to the very type the dependency meant, whatever this module has in scope. Binding
    /// them all costs nothing a module that uses no trait notices: nothing else can spell them.
    fn import_trait_names(
        &mut self,
        dependencies: &[&CheckedSourceModule],
        remaps: &[Dependency],
    ) -> DiagnosticResult<()> {
        for (dependency, remap) in dependencies.iter().zip(remaps) {
            for source in &dependency.traits {
                self.charge(source.names.len() + 1)
                    .map_err(|error| self.failure(error, SpanId::INVALID))?;
                for (mangled, layout) in &source.names {
                    let Some(target) = remap.types.get(*layout) else {
                        continue;
                    };
                    if !self.module.type_names.iter().any(|(name, found)| name == mangled && found == target) {
                        self.module.type_names.push((mangled.clone(), *target));
                    }
                }
            }
        }
        Ok(())
    }
}
