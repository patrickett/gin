use diagnostic::code::ImportCode;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use ast::folder::{Folder, Visitor, walk_bind_mut, walk_expr, walk_expr_mut};
use ast::{Bind, BindValue, ConstValue, DefinitionId, Expr, FileAst, FnCall, Import, ImportSource};
use ast::{HasSpanId, Literal, ModPath, SpanId, Typed, WhenArm, WhenExpr};
use diagnostic::Diagnostic;
use internment::Intern;
use typecheck::analysis::{CompTimeEvaluator, ConstEnv};

use crate::conditional_import_arguments::{argument_value, bind_arguments};

const SPECIALIZATIONS_MAX: usize = 1024;
const VISITS_MAX: usize = 100_000;

type SpecializationKey = (DefinitionId, Vec<Option<ConstValue>>);

pub(crate) struct StageFile {
    pub path: PathBuf,
    pub ast: FileAst,
}

pub(crate) fn contains_imports(ast: &FileAst) -> bool {
    let mut finder = ImportFinder(false);
    let _ = finder.visit_file_ast(ast);
    finder.0
}

pub(crate) fn select_imports(
    files: &mut [StageFile],
    target: &flask::CompileTarget,
) -> Vec<(PathBuf, Diagnostic)> {
    let mut stage = ImportStage::new(files, target);
    for (index, file) in files.iter_mut().enumerate() {
        stage.current = index;
        file.ast
            .defs
            .retain(|id, _| !stage.templates.contains_key(id));
        let _ = stage.visit_file_ast(&mut file.ast);
    }
    for _ in 0..SPECIALIZATIONS_MAX {
        let Some((owner, mut bind, environment)) = stage.pending.pop() else {
            break;
        };
        stage.environment = environment;
        stage.current = owner;
        stage.aliases.clear();
        stage.unresolved_condition = None;
        let _ = walk_bind_mut(&mut stage, &mut bind);
        files[owner].ast.defs.insert(bind.name, bind);
    }
    if !stage.pending.is_empty() {
        // use-specialization-limit
        stage.diagnose(Diagnostic::new(
            ImportCode::SpecializationLimit,
            "conditional imports exceeded the function specialization limit",
        ));
    }
    for (file, imports) in files.iter_mut().zip(stage.imports) {
        file.ast.uses.extend(imports);
    }
    stage
        .diagnostics
        .into_iter()
        .map(|(owner, diagnostic)| (files[owner].path.clone(), diagnostic))
        .collect()
}

fn import_templates(ast: &FileAst) -> HashMap<DefinitionId, Bind> {
    let mut names = HashSet::new();
    for _ in 0..=ast.defs.len() {
        let previous = names.len();
        for (id, bind) in &ast.defs {
            if bind.params.is_some() && !bind.attributes.entry {
                let mut finder = TemplateFinder {
                    names: &names,
                    found: false,
                };
                let _ = finder.visit_bind(bind);
                if finder.found {
                    names.insert(*id);
                }
            }
        }
        if names.len() == previous {
            break;
        }
    }
    ast.defs
        .iter()
        .filter(|(id, _)| names.contains(id))
        .map(|(id, bind)| (*id, bind.clone()))
        .collect()
}

struct ImportFinder(bool);

impl Visitor for ImportFinder {
    fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        if matches!(expr, Expr::Import(_)) {
            self.0 = true;
            return ControlFlow::Break(());
        }
        walk_expr(self, expr)
    }
}

struct TemplateFinder<'a> {
    names: &'a HashSet<DefinitionId>,
    found: bool,
}

impl Visitor for TemplateFinder<'_> {
    fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        let template_call = match expr {
            Expr::FnCall(call) => self
                .names
                .contains(&DefinitionId::named(call.mangled_name())),
            _ => false,
        };
        if matches!(expr, Expr::Import(_)) || template_call {
            self.found = true;
            return ControlFlow::Break(());
        }
        walk_expr(self, expr)
    }
}

struct ImportStage<'a> {
    evaluation_ast: FileAst,
    target: &'a flask::CompileTarget,
    contexts: Vec<StageFile>,
    current: usize,
    owners: HashMap<DefinitionId, (usize, bool)>,
    templates: HashMap<DefinitionId, Bind>,
    template_names: HashSet<DefinitionId>,
    specializations: HashMap<SpecializationKey, Intern<String>>,
    pending: Vec<(usize, Bind, ConstEnv)>,
    imports: Vec<Vec<Import>>,
    diagnostics: Vec<(usize, Diagnostic)>,
    environment: ConstEnv,
    aliases: HashMap<Intern<String>, Intern<String>>,
    unresolved_condition: Option<SpanId>,
    visits: usize,
}

impl<'a> ImportStage<'a> {
    fn new(files: &[StageFile], target: &'a flask::CompileTarget) -> Self {
        let mut evaluation_ast = FileAst::default();
        let mut owners = HashMap::new();
        for (index, file) in files.iter().enumerate() {
            for id in file.ast.defs.keys() {
                owners.insert(*id, (index, file.ast.private_defs.contains(&id.0)));
            }
            evaluation_ast.merge_from(file.ast.clone());
        }
        let templates = import_templates(&evaluation_ast);
        let template_names = templates.keys().copied().collect();
        Self {
            evaluation_ast,
            target,
            contexts: files
                .iter()
                .map(|file| StageFile {
                    path: file.path.clone(),
                    ast: file.ast.clone(),
                })
                .collect(),
            current: 0,
            owners,
            templates,
            template_names,
            specializations: HashMap::new(),
            pending: Vec::new(),
            imports: vec![Vec::new(); files.len()],
            diagnostics: Vec::new(),
            environment: ConstEnv::default(),
            aliases: HashMap::new(),
            unresolved_condition: None,
            visits: 0,
        }
    }
    fn diagnose(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push((self.current, diagnostic));
    }

    fn context(&self) -> &StageFile {
        &self.contexts[self.current]
    }

    fn normalize(&self, expr: Expr) -> Expr {
        let mut expression = FileAst::default();
        expression.uses = self.context().ast.uses.clone();
        expression.uses.extend(self.imports[self.current].clone());
        expression.symbol_aliases = self.context().ast.symbol_aliases.clone();
        expression.exprs.push((expr, SpanId::INVALID));
        expression.apply_symbol_aliases();
        expression.exprs.pop().unwrap().0
    }

    fn evaluate(&self, expr: &Expr) -> Option<ConstValue> {
        let mut evaluation = self.evaluation_ast.clone();
        evaluation.defs.retain(|id, _| {
            let root = id.0.as_str().split('.').next().unwrap();
            let name = id.0.as_str().rsplit('.').next().unwrap();
            self.environment.get(&Intern::from_ref(root)) != Some(&None)
                && self.environment.get(&Intern::from_ref(name)) != Some(&None)
        });
        CompTimeEvaluator::for_target(&self.environment, &evaluation, self.target)
            .eval(&self.normalize(expr.clone()))
    }

    fn selected_arm(&self, when: &WhenExpr) -> Option<SelectedArm> {
        let mut probe = when.clone();
        for (index, arm) in probe.arms.iter_mut().enumerate() {
            let bindings = arm_binding_names(arm);
            let mut fields = vec![(
                Intern::from_ref("__gin_arm"),
                Typed::infer(
                    Expr::Lit(Literal::Int((index as i128).into())),
                    SpanId::INVALID,
                ),
            )];
            fields.extend(bindings.into_iter().map(|name| {
                (
                    name,
                    Typed::infer(
                        Expr::FnCall(FnCall {
                            syntax: ast::CallSyntax::Path,
                            definition: None,
                            path: ast::Spanned::new(
                                ModPath::new(name, Vec::new()),
                                SpanId::INVALID,
                            ),
                            args: None,
                        }),
                        SpanId::INVALID,
                    ),
                )
            }));
            arm_body_mut(arm).value = Expr::RecordLit(fields);
        }
        let ConstValue::Record { fields } = self.evaluate(&Expr::When(probe))? else {
            return None;
        };
        let Some(((_, ConstValue::Int(index)), bindings)) = fields.split_first() else {
            return Some(SelectedArm {
                index: when.arms.len(),
                bindings: Vec::new(),
            });
        };
        Some(SelectedArm {
            index: ast::integer::to_usize(*index)?,
            bindings: bindings.to_vec(),
        })
    }

    fn import(&mut self, import: &mut Import) {
        if let Some(span) = self.unresolved_condition {
            self.diagnose(
                // use-condition-not-compile-time
                Diagnostic::new(
                    ImportCode::ConditionNotCompileTime,
                    "conditional import requires a condition known at compile time",
                )
                .with_help("supply compile-time-known inputs to the enclosing function")
                .at_span(self.context().ast.span_table.get(span)),
            );
            return;
        }
        for module in &mut import.0 {
            match &mut module.source {
                ImportSource::LocalBundle(bundle) => {
                    for member in &mut bundle.members {
                        let name = member.flat_name();
                        let alias = self.scoped_name(name, member.span);
                        member.alias = Some(alias);
                    }
                }
                ImportSource::CurrentModule { member } => {
                    let alias = self.scoped_name(member.flat_name(), member.span);
                    member.alias = Some(alias);
                }
                ImportSource::LocalFile(path, _) => {
                    let name = module.alias.expect("file imports have a namespace alias");
                    let directory = self.context().path.parent().unwrap_or(Path::new(""));
                    let path = directory.join(path);
                    let path = path.canonicalize().unwrap_or(path);
                    let encoded: String = path
                        .as_os_str()
                        .as_encoded_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    let alias = Intern::new(format!("$file${encoded}"));
                    self.aliases.insert(name, alias);
                    module.alias = Some(alias);
                }
                _ => {
                    let name = module
                        .alias
                        .unwrap_or_else(|| Intern::new(module.effective_name()));
                    let alias = self.scoped_name(name, module.span_id());
                    module.alias = Some(alias);
                }
            }
        }
        if !self.imports[self.current].contains(import) {
            self.imports[self.current].push(import.clone());
        }
    }

    fn scoped_name(&mut self, name: Intern<String>, span: SpanId) -> Intern<String> {
        let alias = Intern::new(format!("$use${name}${}", span.into_inner()));
        self.aliases.insert(name, alias);
        alias
    }

    fn specialize_values(&self, template: &Bind, args: &[&Typed<Expr>]) -> Vec<Option<ConstValue>> {
        let mut mutations = ParameterMutations {
            evaluation_ast: &self.evaluation_ast,
            parameters: template
                .params
                .iter()
                .flat_map(|params| params.keys())
                .copied()
                .collect(),
            mutated: template
                .param_conventions
                .iter()
                .filter_map(|(name, convention)| {
                    (*convention == ast::ParamConvention::Mutate).then_some(*name)
                })
                .collect(),
            mutating: false,
        };
        let _ = mutations.visit_bind(template);
        template
            .params
            .iter()
            .flat_map(|params| params.keys())
            .zip(args)
            .map(|(name, arg)| {
                if mutations.mutated.contains(name) {
                    None
                } else {
                    self.evaluate(&arg.value)
                }
            })
            .collect()
    }

    fn specialize(&mut self, call: &mut FnCall) {
        let id = call
            .definition
            .unwrap_or_else(|| DefinitionId::named(call.mangled_name()));
        let Some(template) = self.templates.get(&id) else {
            return;
        };
        let (owner, private) = self.owners[&id];
        if private && owner != self.current {
            return;
        }
        let Some(args) = &call.args else {
            // use-condition-not-compile-time
            self.diagnose(Diagnostic::new(
                ImportCode::ConditionNotCompileTime,
                "a function with conditional imports must be called with known import conditions",
            ).at_span(self.context().ast.span_table.get(call.path.span_id)));
            return;
        };
        let ordered = match bind_arguments(template, args) {
            Ok(ordered) => ordered,
            Err((code, message)) => {
                self.diagnose(
                    Diagnostic::new(code, message)
                        .at_span(self.context().ast.span_table.get(call.path.span_id)),
                );
                return;
            }
        };
        let values = self.specialize_values(template, &ordered);
        let key = (id, values.clone());
        if let Some(name) = self.specializations.get(&key) {
            call.path.value = ModPath::new(*name, Vec::new());
            call.definition = None;
            return;
        }
        if self.specializations.len() == SPECIALIZATIONS_MAX {
            self.diagnose(
                // use-specialization-limit
                Diagnostic::new(
                    ImportCode::SpecializationLimit,
                    "conditional imports exceeded the specialization limit",
                )
                .at_span(self.context().ast.span_table.get(call.path.span_id)),
            );
            return;
        }
        let name = Intern::new(format!(
            "{}$imports${}",
            template.name,
            self.specializations.len()
        ));
        let (bind, environment) = specialize_bind(template, name, values);
        self.specializations.insert(key, name);
        self.pending.push((owner, bind, environment));
        call.path.value = ModPath::new(name, Vec::new());
        call.definition = None;
    }
}

impl Folder for ImportStage<'_> {
    fn visit_file_ast(&mut self, ast: &mut FileAst) -> ControlFlow<()> {
        for bind in ast.defs.values_mut() {
            self.environment.clear();
            self.aliases.clear();
            self.visit_bind(bind)?;
        }
        self.environment.clear();
        self.aliases.clear();
        for (expr, _) in &mut ast.exprs {
            self.visit_expr(expr)?;
        }
        ControlFlow::Continue(())
    }

    fn visit_bind(&mut self, bind: &mut Bind) -> ControlFlow<()> {
        let previous_environment = self.environment.clone();
        let previous_aliases = self.aliases.clone();
        if let Some(params) = &bind.params {
            for name in params.keys() {
                self.environment.insert(*name, None);
                self.aliases.remove(name);
            }
        }
        let result = walk_bind_mut(self, bind);
        let value = match &bind.value {
            BindValue::Expr(expr) if bind.params.is_none() && bind.is_constant() => {
                self.evaluate(&expr.value)
            }
            _ => None,
        };
        self.environment = previous_environment;
        self.aliases = previous_aliases;
        if bind.params.is_none() {
            self.environment.insert(bind.name, value);
            self.aliases.remove(&bind.name);
        }
        result
    }

    fn visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        self.visits += 1;
        if self.visits > VISITS_MAX {
            // use-staging-limit
            self.diagnose(Diagnostic::new(
                ImportCode::StagingLimit,
                "conditional import evaluation exceeded its traversal limit",
            ));
            return ControlFlow::Break(());
        }
        if let Expr::FnCall(call) = expr {
            if call.args.is_none() && call.path.segments.is_empty() {
                if let Some(Some(value)) = self.environment.get(&call.path.root) {
                    *expr = typecheck::const_value::const_value_to_expr(value);
                    return ControlFlow::Continue(());
                }
            }
        }
        if let Expr::Import(import) = expr {
            self.import(import);
            *expr = Expr::Block(Vec::new());
            return ControlFlow::Continue(());
        }
        if let Expr::When(when) = expr {
            let mut finder = TemplateFinder {
                names: &self.template_names,
                found: false,
            };
            let _ = finder.visit_expr(&Expr::When(when.clone()));
            if finder.found {
                let previous = self.unresolved_condition;
                if let Some(selected) = self.selected_arm(when) {
                    *expr = selected_body(when, selected);
                    self.unresolved_condition = None;
                    let result = self.visit_expr(expr);
                    self.unresolved_condition = previous;
                    return result;
                }
                self.unresolved_condition = Some(when.body_span.into_inner());
                let result = walk_expr_mut(self, expr);
                self.unresolved_condition = previous;
                return result;
            }
        }
        if matches!(expr, Expr::Block(_)) {
            let environment = self.environment.clone();
            let aliases = self.aliases.clone();
            let result = walk_expr_mut(self, expr);
            self.environment = environment;
            self.aliases = aliases;
            return result;
        }
        walk_expr_mut(self, expr)
    }

    fn visit_fn_call(&mut self, call: &mut FnCall) -> ControlFlow<()> {
        if let Some(args) = &mut call.args {
            for argument in args {
                if let Expr::Bind(bind) = &mut argument.value {
                    if let BindValue::Expr(value) = &mut bind.value {
                        self.visit_expr(value)?;
                    }
                } else {
                    self.visit_expr(argument)?;
                }
            }
        }
        if let Some(alias) = self.aliases.get(&call.path.root) {
            call.path.root = *alias;
        }
        if call.args.is_some() || !call.path.segments.is_empty() {
            if let Expr::FnCall(normalized) = self.normalize(Expr::FnCall(call.clone())) {
                *call = normalized;
            }
        }
        self.specialize(call);
        ControlFlow::Continue(())
    }
}

fn specialize_bind(
    template: &Bind,
    name: Intern<String>,
    values: Vec<Option<ConstValue>>,
) -> (Bind, ConstEnv) {
    let parameters = template
        .params
        .as_ref()
        .expect("import templates have parameters");
    assert_eq!(parameters.len(), values.len());
    let environment: ConstEnv = parameters.keys().copied().zip(values).collect();
    let mut bind = template.clone();
    bind.name = name;
    (bind, environment)
}

struct SelectedArm {
    index: usize,
    bindings: Vec<(Intern<String>, ConstValue)>,
}

struct ParameterMutations<'a> {
    evaluation_ast: &'a FileAst,
    parameters: HashSet<Intern<String>>,
    mutated: HashSet<Intern<String>>,
    mutating: bool,
}

impl ParameterMutations<'_> {
    fn mutating_argument(&self, call: &FnCall, argument: &Typed<Expr>) -> bool {
        let id = call
            .definition
            .unwrap_or_else(|| DefinitionId::named(call.mangled_name()));
        let Some(callee) = self.evaluation_ast.defs.get(&id) else {
            return true;
        };
        let Some(args) = &call.args else {
            return false;
        };
        if callee.params.is_none() {
            return !args.is_empty();
        }
        let Ok(ordered) = bind_arguments(callee, args) else {
            return true;
        };
        let index = ordered
            .iter()
            .position(|value| std::ptr::eq(*value, argument_value(argument)));
        let name = index.and_then(|index| {
            callee
                .params
                .as_ref()?
                .get_index(index)
                .map(|(name, _)| name)
        });
        name.and_then(|name| callee.param_conventions.get(name))
            == Some(&ast::ParamConvention::Mutate)
    }
}

impl Visitor for ParameterMutations<'_> {
    fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        match expr {
            Expr::Bind(bind) if self.parameters.contains(&bind.name) => {
                self.mutated.insert(bind.name);
            }
            Expr::FnCall(call) if self.mutating && self.parameters.contains(&call.path.root) => {
                self.mutated.insert(call.path.root);
            }
            Expr::Ref { mutable: true, .. } => {
                let previous = self.mutating;
                self.mutating = true;
                let result = walk_expr(self, expr);
                self.mutating = previous;
                return result;
            }
            _ => {}
        }
        walk_expr(self, expr)
    }

    fn visit_fn_call(&mut self, call: &FnCall) -> ControlFlow<()> {
        let Some(args) = &call.args else {
            return ControlFlow::Continue(());
        };
        for argument in args {
            let previous = self.mutating;
            self.mutating |= self.mutating_argument(call, argument);
            self.visit_expr(argument_value(argument))?;
            self.mutating = previous;
        }
        ControlFlow::Continue(())
    }
}

fn selected_body(when: &WhenExpr, selected: SelectedArm) -> Expr {
    let Some(arm) = when.arms.get(selected.index) else {
        return Expr::Block(Vec::new());
    };
    let body = arm_body(arm);
    if selected.bindings.is_empty() {
        return body.value.clone();
    }
    let mut expressions = Vec::new();
    for (name, value) in selected.bindings {
        let value = typecheck::const_value::const_value_to_typed_expr(&value, body.span_id);
        let mut bind = Bind::new(name, body.span_id, BindValue::Expr(Box::new(value)));
        bind.operator = ast::BindOperator::FreshImmutable;
        expressions.push(Typed::infer(Expr::Bind(Box::new(bind)), body.span_id));
    }
    expressions.push(body.clone());
    Expr::Block(expressions)
}

fn arm_binding_names(arm: &WhenArm) -> Vec<Intern<String>> {
    match arm {
        WhenArm::Is { pattern, .. } => pattern.value.binding_names(),
        WhenArm::Else(..) => Vec::new(),
        WhenArm::Cond { condition, .. } => {
            let mut conditions = vec![condition];
            let mut names = Vec::new();
            for _ in 0..VISITS_MAX {
                let Some(condition) = conditions.pop() else {
                    break;
                };
                match condition {
                    ast::Condition::Is { pattern, .. } => {
                        names.extend(pattern.value.binding_names())
                    }
                    ast::Condition::Not(inner) => conditions.push(inner),
                    ast::Condition::And(left, right) | ast::Condition::Or(left, right) => {
                        conditions.push(left);
                        conditions.push(right);
                    }
                    _ => {}
                }
            }
            names.sort();
            names.dedup();
            names
        }
    }
}

fn arm_body(arm: &WhenArm) -> &Typed<Expr> {
    match arm {
        WhenArm::Cond { body, .. } | WhenArm::Is { body, .. } | WhenArm::Else(body, _) => body,
    }
}

fn arm_body_mut(arm: &mut WhenArm) -> &mut Typed<Expr> {
    match arm {
        WhenArm::Cond { body, .. } | WhenArm::Is { body, .. } | WhenArm::Else(body, _) => body,
    }
}
