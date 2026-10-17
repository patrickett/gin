//! Compile-time expression evaluation — fold expressions to [`ConstValue`]s.
//!
//! Handles literal folding, tag constructors, binary ops, bind references,
//! and compile-time bind validation.

use derive_more::{Deref, DerefMut, From};
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;

use internment::Intern;

use crate::analysis::pattern::collect_pattern_bindings;
use crate::assembly::target_const_record;
use crate::prepare_target::eval_type_static_member;

use ast::expr::{Expr, Literal, Typed};

use ast::{Bind, BindValue, FileAst, ModPath, WhenArm};
use ast::{ConstValue, HashFloat};

const MAX_COMPILE_TIME_DEPTH: usize = 512;

fn target_query_operand_ty(reference: &ast::TypeReference) -> Option<ast::Ty> {
    match reference {
        ast::TypeReference::Unit => Some(ast::Ty::Unit),
        ast::TypeReference::Pointer(inner) => Some(ast::Ty::Ptr {
            inner: Box::new(target_query_operand_ty(inner)?),
        }),
        ast::TypeReference::Nominal(_) | ast::TypeReference::Application { .. } => None,
    }
}

#[derive(Clone, Default, Deref, DerefMut, From)]
pub struct ConstEnv(HashMap<Intern<String>, Option<ConstValue>>);

impl FromIterator<(Intern<String>, Option<ConstValue>)> for ConstEnv {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = (Intern<String>, Option<ConstValue>)>,
    {
        Self(iter.into_iter().collect())
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ComptimeCall {
    name: Intern<String>,
    args: Box<[ConstValue]>,
}

enum ConstEvalFailure {
    ArityMismatch,
    Recursion,
    DepthExhaustion,
}

pub struct CompTimeEvaluator<'a> {
    const_binds: &'a ConstEnv,
    ast: &'a FileAst,
    target_layout: Option<crate::layout::TargetLayout>,
    target: Option<flask::TargetTriple>,
}

impl<'a> CompTimeEvaluator<'a> {
    pub fn new(const_binds: &'a ConstEnv, ast: &'a FileAst) -> Self {
        Self {
            const_binds,
            ast,
            target_layout: None,
            target: None,
        }
    }

    pub fn for_target(
        const_binds: &'a ConstEnv,
        ast: &'a FileAst,
        target: &flask::CompileTarget,
    ) -> Self {
        Self {
            const_binds,
            ast,
            target_layout: crate::layout::TargetLayout::from_compile_target(target).ok(),
            target: match target {
                flask::CompileTarget::Concrete(triple) => Some(triple.clone()),
                flask::CompileTarget::Library => None,
            },
        }
    }

    pub(crate) fn for_layout(
        const_binds: &'a ConstEnv,
        ast: &'a FileAst,
        target_layout: Option<crate::layout::TargetLayout>,
    ) -> Self {
        Self {
            const_binds,
            ast,
            target_layout,
            target: None,
        }
    }

    pub fn eval(&self, expr: &Expr) -> Option<ConstValue> {
        let mut call_stack = HashSet::new();
        EvalFrame::new(
            self.const_binds,
            self.ast,
            self.target_layout,
            self.target.clone(),
            &mut call_stack,
        )
        .eval(expr)
    }

    pub fn eval_bind_call(&self, bind: &Bind, args: &[ConstValue]) -> Option<ConstValue> {
        let mut call_stack = HashSet::new();
        EvalFrame::new(
            self.const_binds,
            self.ast,
            self.target_layout,
            self.target.clone(),
            &mut call_stack,
        )
        .eval_bind_call(bind, args, 0)
    }
}

/// A stateful compile-time evaluator for constant expressions.
struct EvalFrame<'a, 'b> {
    const_binds: &'a ConstEnv,
    ast: &'a FileAst,
    target_layout: Option<crate::layout::TargetLayout>,
    target: Option<flask::TargetTriple>,
    depth: usize,
    call_stack: &'b mut HashSet<ComptimeCall>,
    returned: std::rc::Rc<std::cell::RefCell<Option<ConstValue>>>,
    condition_bindings: ConstEnv,
    when_subjects: HashMap<ast::SpanId, ConstValue>,
}

impl<'a, 'b> EvalFrame<'a, 'b> {
    pub fn new(
        const_binds: &'a ConstEnv,
        ast: &'a FileAst,
        target_layout: Option<crate::layout::TargetLayout>,
        target: Option<flask::TargetTriple>,
        call_stack: &'b mut HashSet<ComptimeCall>,
    ) -> Self {
        Self {
            const_binds,
            ast,
            target_layout,
            target,
            depth: 0,
            call_stack,
            returned: Default::default(),
            condition_bindings: ConstEnv::default(),
            when_subjects: HashMap::new(),
        }
    }

    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    pub fn eval(&mut self, expr: &Expr) -> Option<ConstValue> {
        self.check_depth().ok()?;
        if let Some(value) = self.returned.borrow().clone() {
            return Some(value);
        }
        match expr {
            Expr::Block(items) => self.eval_sequence(items),
            Expr::Return(ret) => {
                let values = ret
                    .values
                    .iter()
                    .map(|value| self.eval(&value.value))
                    .collect::<Option<Vec<_>>>()?;
                let value = match values.as_slice() {
                    [] => ConstValue::Record {
                        fields: Vec::new().into(),
                    },
                    [value] => value.clone(),
                    _ => ConstValue::ResultRow(values.into()),
                };
                self.returned.replace(Some(value.clone()));
                Some(value)
            }
            Expr::Target => Some(target_const_record(self.target.as_ref()?)),
            Expr::Lit(lit) => match lit {
                Literal::Int(n) => Some(ConstValue::Int(*n)),
                Literal::Float(HashFloat(f)) => Some(ConstValue::Float(HashFloat(*f))),
                Literal::String(s) => Some(ConstValue::Text(s.clone())),
            },
            Expr::FormatString(_) => None,
            Expr::ResultValues(items) => items
                .iter()
                .map(|item| self.eval(&item.value))
                .collect::<Option<Vec<_>>>()
                .map(|items| ConstValue::ResultRow(items.into())),
            Expr::AnonymousTag(name) => self
                .const_binds
                .get(name)
                .and_then(|cv| cv.clone())
                .or_else(|| {
                    Some(ConstValue::Tag {
                        name: *name,
                        qual_path: None,
                        args: Vec::new().into(),
                    })
                }),
            Expr::RecordLit(fields) => {
                let pairs: Vec<(Intern<String>, ConstValue)> = fields
                    .iter()
                    .filter_map(|(name, expr)| {
                        let cv = self.eval(&expr.value)?;
                        Some((*name, cv))
                    })
                    .collect();
                if pairs.len() == fields.len() {
                    Some(ConstValue::Record {
                        fields: pairs.into(),
                    })
                } else {
                    None
                }
            }
            Expr::TagCall(call) => {
                let args = self.eval_args(Some(&call.args))?;
                Some(ConstValue::Tag {
                    name: call.name,
                    qual_path: call.qualifier(),
                    args: args.into(),
                })
            }
            Expr::RecordSet { .. } | Expr::Destructure { .. } => None,
            Expr::RecordGet { base, field } => match self.eval(&base.value)? {
                ConstValue::Text(text) if field.as_str() == "len" => {
                    Some(ConstValue::Int((text.len() as u64).into()))
                }
                ConstValue::Record { fields } => fields
                    .iter()
                    .find(|(n, _)| n == field)
                    .map(|(_, v)| v.clone()),
                ConstValue::Tag { name, args, .. } => {
                    let declaration = self.ast.tags.get(&name).or_else(|| {
                        self.ast.tags.values().find(|decl| {
                            decl.name.as_str().rsplit(['.', ':']).next() == Some(name.as_str())
                        })
                    });
                    let idx = declaration
                        .and_then(|decl| decl.params.as_ref())
                        .and_then(|params| params.keys().position(|param_name| param_name == field))
                        .or_else(|| {
                            declaration.and_then(|decl| match &decl.value {
                                ast::DeclareValue::Has(members) => members
                                    .iter()
                                    .filter_map(|member| match member {
                                        ast::HasMember::Property(property)
                                            if property.qualifier.as_ref().is_none_or(
                                                |qualifier| qualifier.name == decl.name,
                                            ) =>
                                        {
                                            Some(property)
                                        }
                                        _ => None,
                                    })
                                    .position(|property| property.name == *field),
                                _ => None,
                            })
                        })
                        .or_else(|| match name.as_str() {
                            "Target" => ["arch", "vendor", "os"]
                                .iter()
                                .position(|field_name| *field_name == field.as_str()),
                            _ => None,
                        })?;

                    args.get(idx).cloned()
                }
                _ => None,
            },
            Expr::Bind(b) if b.params.is_none() => {
                if let Some(Some(cv)) = self.const_binds.get(&b.name) {
                    return Some(cv.clone());
                }
                if let BindValue::Expr(expr) = &b.value
                    && let Some(cv) = self.eval(&expr.value)
                {
                    return Some(cv);
                }
                let bind = self.resolve_bind(b.name)?.clone();
                if !bind.is_constant() && !bind.attributes.inline_always {
                    return None;
                }
                self.eval_bind_call(&bind, &[], self.depth + 1)
            }
            Expr::ArrayLit(elems) => {
                let items: Vec<ConstValue> =
                    elems.iter().filter_map(|e| self.eval(&e.value)).collect();
                (items.len() == elems.len()).then(|| ConstValue::FixedArray(items.into()))
            }
            Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elems)) => {
                let items = elems
                    .iter()
                    .map(|elem| self.eval(&elem.value))
                    .collect::<Option<Vec<_>>>()?;
                if elems.iter().any(|elem| elem.name.is_some()) {
                    Some(ConstValue::sequence(
                        items,
                        elems.iter().map(|elem| elem.name),
                    ))
                } else {
                    Some(ConstValue::Sequence(items.into()))
                }
            }
            Expr::List(elems) => {
                let items: Vec<ConstValue> =
                    elems.iter().filter_map(|e| self.eval(&e.value)).collect();
                (items.len() == elems.len()).then(|| ConstValue::Sequence(items.into()))
            }
            Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { seed, count }) => {
                Some(ConstValue::SequenceRepeat {
                    seed: Box::new(self.eval(&seed.value)?),
                    count: Box::new(sequence_count_expr(&count.value)?),
                })
            }
            Expr::Negate(inner) => {
                let inner_value = self.eval(&inner.value)?;
                match inner_value {
                    ConstValue::Int(value) => Some(ConstValue::Int(-value)),
                    _ => None,
                }
            }
            Expr::Binary(bin) => {
                let lhs = self.eval(&bin.lhs.value)?;
                let rhs = self.eval(&bin.rhs.value)?;
                if matches!(bin.op, ast::BinOp::Add)
                    && let (ConstValue::Text(lhs), ConstValue::Text(rhs)) = (&lhs, &rhs)
                {
                    return Some(ConstValue::Text(format!("{lhs}{rhs}")));
                }
                lhs.eval_binop(&bin.op, &rhs)
            }
            Expr::TargetQuery { kind, operand } => {
                let operand = target_query_operand_ty(operand)?;
                let value = self.target_layout?.query(*kind, &operand, None).ok()?;
                Some(ConstValue::Int(i256::I256::from(value)))
            }
            Expr::When(when) => {
                let has_patterns = when
                    .arms
                    .iter()
                    .any(|arm| matches!(arm, WhenArm::Is { .. }));
                if has_patterns && when.subject.is_none() {
                    return None;
                }
                let subject = match &when.subject {
                    Some(subject) => self.eval(&subject.value)?,
                    None => ConstValue::ZERO,
                };
                let previous = when
                    .subject
                    .as_ref()
                    .and_then(|source| self.when_subjects.insert(source.span_id, subject.clone()));
                let result = self.eval_matching_when_arm(&when.arms, &subject);
                if let Some(source) = &when.subject {
                    self.when_subjects.remove(&source.span_id);
                    if let Some(previous) = previous {
                        self.when_subjects.insert(source.span_id, previous);
                    }
                }
                result
            }
            Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty() => {
                let name = call.path.value.root;
                if let Some(cv) = self
                    .condition_bindings
                    .get(&name)
                    .or_else(|| self.const_binds.get(&name))
                    .and_then(|cv| cv.clone())
                {
                    return Some(cv);
                }
                let final_name = call
                    .path
                    .value
                    .segments
                    .last()
                    .unwrap_or(&call.path.value.root);
                let bind = self.resolve_bind_by_parts(&name, final_name)?;
                if !bind.is_constant() && !bind.attributes.inline_always {
                    return None;
                }
                self.eval_bind_call(bind, &[], self.depth + 1)
            }
            Expr::FnCall(call) if call.args.is_none() && !call.path.value.segments.is_empty() => {
                let fq_name = self.qualified_name(&call.path.value);
                if let Some(cv) = self.const_binds.get(&fq_name).and_then(|cv| cv.clone()) {
                    return Some(cv);
                }
                let bind = self.resolve_bind(fq_name);
                if let Some(bind) = bind {
                    return self.eval_bind_call(bind, &[], self.depth + 1);
                }
                if call.path.value.segments.len() != 1 {
                    return None;
                }
                let base = call.path.value.root;
                let field = call.path.value.segments[0];
                if let Some(cv) = eval_type_static_member(base, field, self.ast, self.const_binds) {
                    return Some(cv);
                }
                if let Some(Some(cv)) = self.const_binds.get(&base)
                    && let ConstValue::Record { fields } = cv
                {
                    return fields
                        .iter()
                        .find(|(n, _)| n == &field)
                        .map(|(_, v)| v.clone());
                }
                None
            }
            Expr::FnCall(call) if call.path.value.segments.is_empty() => {
                let name = call.path.value.root;
                let args = self.eval_args(call.args.as_deref())?;
                if args.is_empty()
                    && let Some(Some(cv)) = self.const_binds.get(&name)
                {
                    return Some(cv.clone());
                }
                let bind = self.resolve_bind(name)?.clone();
                self.eval_bind_call(&bind, &args, self.depth + 1)
            }
            Expr::FnCall(call) => {
                let name = self.qualified_name(&call.path.value);
                let args = self.eval_args(call.args.as_deref())?;
                if args.is_empty()
                    && let Some(Some(cv)) = self.const_binds.get(&name)
                {
                    return Some(cv.clone());
                }
                let final_name = call
                    .path
                    .value
                    .segments
                    .last()
                    .unwrap_or(&call.path.value.root);
                let bind = self.resolve_bind_by_parts(&name, final_name)?;
                self.eval_bind_call(bind, &args, self.depth + 1)
            }
            _ => None,
        }
    }

    fn eval_in_env(&mut self, env: &ConstEnv, depth: usize, expr: &Expr) -> Option<ConstValue> {
        let mut frame = EvalFrame::new(
            env,
            self.ast,
            self.target_layout,
            self.target.clone(),
            self.call_stack,
        )
        .with_depth(depth);
        frame.returned = self.returned.clone();
        frame.condition_bindings = self.condition_bindings.clone();
        frame.when_subjects = self.when_subjects.clone();
        frame.eval(expr)
    }

    fn eval_subject(&mut self, subject: &Typed<Expr>) -> Option<ConstValue> {
        if let Some(value) = self.when_subjects.get(&subject.span_id) {
            return Some(value.clone());
        }
        self.eval(&subject.value)
    }

    fn eval_sequence(&mut self, items: &[Typed<Expr>]) -> Option<ConstValue> {
        let mut env = self.const_binds.clone();
        let mut value = ConstValue::Record {
            fields: Vec::new().into(),
        };
        for item in items {
            if let Expr::Bind(bind) = &item.value {
                if bind.params.is_some() {
                    continue;
                }
                if matches!(
                    bind.operator,
                    ast::BindOperator::Rebind | ast::BindOperator::Compound(_)
                ) && self.const_binds.contains_key(&bind.name)
                {
                    return None;
                }
                if let BindValue::Expr(initializer) = &bind.value {
                    value = self.eval_in_env(&env, self.depth, &initializer.value)?;
                    if let ast::BindOperator::Compound(operator) = &bind.operator {
                        value = env
                            .get(&bind.name)?
                            .as_ref()?
                            .eval_binop(operator, &value)?;
                    }
                    env.insert(bind.name, Some(value.clone()));
                } else {
                    return None;
                }
            } else {
                value = self.eval_in_env(&env, self.depth, &item.value)?;
            }
            if let Some(returned) = self.returned.borrow().clone() {
                return Some(returned);
            }
        }
        Some(value)
    }

    fn eval_args(&mut self, args: Option<&[Typed<Expr>]>) -> Option<Vec<ConstValue>> {
        args.map_or_else(
            || Some(Vec::new()),
            |args| args.iter().map(|arg| self.eval(&arg.value)).collect(),
        )
    }

    fn eval_matching_when_arm(
        &mut self,
        arms: &[WhenArm],
        subject: &ConstValue,
    ) -> Option<ConstValue> {
        for arm in arms {
            match arm {
                WhenArm::Cond {
                    condition, body, ..
                } => {
                    let bindings = self.condition_bindings.clone();
                    let matches = self.eval_condition(condition);
                    let result = matches.and_then(|matches| {
                        if matches {
                            self.eval(&body.value)
                        } else {
                            None
                        }
                    });
                    self.condition_bindings = bindings;
                    if matches? {
                        return result;
                    }
                }
                WhenArm::Is { pattern, body, .. } => {
                    let matches = crate::analysis::pattern::pattern_matches_with_integer_validity(
                        &pattern.value,
                        subject,
                        None,
                    )?;
                    if matches {
                        let mut arm_env = self.const_binds.clone();
                        let mut flat = HashMap::new();
                        collect_pattern_bindings(&pattern.value, subject, &mut flat);
                        for (k, v) in flat {
                            arm_env.insert(k, Some(v));
                        }
                        let result = self.eval_in_env(&arm_env, self.depth, &body.value);
                        return result;
                    }
                }
                WhenArm::Else(body, _) => return self.eval(&body.value),
            }
        }
        Some(ConstValue::Record {
            fields: Vec::new().into(),
        })
    }

    fn eval_condition(&mut self, condition: &ast::Condition) -> Option<bool> {
        match condition {
            ast::Condition::Proposition { subject } => {
                let ConstValue::Proposition(holds) = self.eval_subject(subject)? else {
                    return None;
                };
                Some(holds)
            }
            ast::Condition::Compare {
                left,
                relation,
                right,
            } => {
                let ConstValue::Int(left) = self.eval_subject(left)? else {
                    return None;
                };
                let ConstValue::Int(right) = self.eval_subject(right)? else {
                    return None;
                };
                Some(relation.holds(left, right))
            }
            ast::Condition::Is { subject, pattern } => {
                let value = self.eval_subject(subject)?;
                let matches = crate::analysis::pattern::pattern_matches_with_integer_validity(
                    &pattern.value,
                    &value,
                    None,
                )?;
                if matches {
                    let mut bindings = HashMap::new();
                    collect_pattern_bindings(&pattern.value, &value, &mut bindings);
                    self.condition_bindings.extend(
                        bindings
                            .into_iter()
                            .map(|(name, value)| (name, Some(value))),
                    );
                }
                Some(matches)
            }
            ast::Condition::Not(inner) => Some(!self.eval_condition(inner)?),
            ast::Condition::And(left, right) => {
                Some(self.eval_condition(left)? && self.eval_condition(right)?)
            }
            ast::Condition::Or(left, right) => {
                Some(self.eval_condition(left)? || self.eval_condition(right)?)
            }
        }
    }

    pub(crate) fn eval_bind_call(
        &mut self,
        bind: &Bind,
        args: &[ConstValue],
        depth: usize,
    ) -> Option<ConstValue> {
        self.check_arity(bind, args).ok()?;
        let call_key = ComptimeCall {
            name: bind.name,
            args: args.into(),
        };
        self.enter_call(call_key.clone()).ok()?;

        let mut env = self.const_binds.clone();
        if let Some(params) = &bind.params {
            for (name, value) in params.keys().zip(args) {
                env.insert(*name, Some(value.clone()));
            }
        }
        let caller_return = std::mem::take(&mut self.returned);
        let result = (|| match &bind.value {
            BindValue::Expr(expr) => self.eval_in_env(&env, depth, &expr.value),
            BindValue::Body { exprs, ret } => {
                for expr in exprs {
                    if let Expr::Bind(local) = &expr.value
                        && local.params.is_none()
                        && let BindValue::Expr(initializer) = &local.value
                    {
                        let value = self.eval_in_env(&env, depth, &initializer.value)?;
                        let value = if let ast::BindOperator::Compound(operator) = &local.operator {
                            env.get(&local.name)?
                                .as_ref()?
                                .eval_binop(operator, &value)?
                        } else {
                            value
                        };
                        env.insert(local.name, Some(value));
                    } else {
                        self.eval_in_env(&env, depth, &expr.value)?;
                    }
                }
                if let Some(value) = self.returned.borrow().clone() {
                    return Some(value);
                }
                ret.values
                    .iter()
                    .map(|ret| self.eval_in_env(&env, depth, &ret.value))
                    .collect::<Option<Vec<_>>>()
                    .and_then(|values| match values.as_slice() {
                        [] => None,
                        [value] => Some(value.clone()),
                        _ => Some(ConstValue::ResultRow(values.into())),
                    })
            }
            BindValue::Unassigned => None,
        })();
        self.returned = caller_return;
        self.call_stack.remove(&call_key);
        result
    }

    fn check_arity(&self, bind: &Bind, args: &[ConstValue]) -> Result<(), ConstEvalFailure> {
        let parameter_count = bind.params.as_ref().map_or(0, |params| params.len());
        if parameter_count == args.len() {
            Ok(())
        } else {
            Err(ConstEvalFailure::ArityMismatch)
        }
    }

    fn check_depth(&self) -> Result<(), ConstEvalFailure> {
        if self.depth > MAX_COMPILE_TIME_DEPTH {
            Err(ConstEvalFailure::DepthExhaustion)
        } else {
            Ok(())
        }
    }

    fn enter_call(&mut self, call: ComptimeCall) -> Result<(), ConstEvalFailure> {
        if self.call_stack.insert(call) {
            Ok(())
        } else {
            Err(ConstEvalFailure::Recursion)
        }
    }

    fn qualified_name(&self, path: &ModPath) -> Intern<String> {
        let mut parts = vec![path.root.as_str()];
        parts.extend(path.segments.iter().map(|segment| segment.as_str()));
        Intern::new(parts.join("."))
    }

    fn resolve_bind(&self, name: Intern<String>) -> Option<&'a Bind> {
        let final_name = name.as_str().rsplit(['.', ':']).next()?;
        self.ast
            .defs
            .get(&name)
            .or_else(|| self.ast.defs.get(&Intern::from_ref(final_name)))
            .or_else(|| {
                self.ast
                    .defs
                    .values()
                    .find(|bind| bind.name.as_str().rsplit(['.', ':']).next() == Some(final_name))
            })
    }

    fn resolve_bind_by_parts(
        &self,
        name: &Intern<String>,
        final_name: &Intern<String>,
    ) -> Option<&'a Bind> {
        self.ast
            .defs
            .get(name)
            .or_else(|| self.ast.defs.get(final_name))
            .or_else(|| {
                self.ast.defs.values().find(|bind| {
                    bind.name.as_str().rsplit(['.', ':']).next() == Some(final_name.as_str())
                })
            })
    }
}

fn sequence_count_expr(expr: &Expr) -> Option<ast::NormalExpr> {
    match expr {
        Expr::Lit(Literal::Int(value)) => Some(ast::NormalExpr::from(*value)),
        Expr::FnCall(call) if call.args.is_none() => {
            Some(ast::NormalExpr::Var(call.path.value.root))
        }
        Expr::Binary(binary) => {
            let lhs = Box::new(sequence_count_expr(&binary.lhs.value)?);
            let rhs = Box::new(sequence_count_expr(&binary.rhs.value)?);
            match binary.op {
                ast::BinOp::Add => Some(ast::NormalExpr::Add(lhs, rhs)),
                ast::BinOp::Subtract => Some(ast::NormalExpr::Sub(lhs, rhs)),
                ast::BinOp::Multiply => Some(ast::NormalExpr::Mul(lhs, rhs)),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::NormalExpr;
    use parser::cursor::TokenCursor;

    fn evaluate(source: &str) -> ConstValue {
        let parsed = TokenCursor::parse_source(&format!("value: {source}\n"));
        let bind = parsed.defs.get(&Intern::from_ref("value")).expect("value");
        let BindValue::Expr(expr) = &bind.value else {
            panic!("value must have an expression body");
        };
        CompTimeEvaluator::new(&ConstEnv::default(), &parsed)
            .eval(&expr.value)
            .expect("constant value")
    }

    #[test]
    fn aggregate_constants_preserve_semantic_literal_category() {
        assert!(matches!(
            evaluate("[1, 2]"),
            ConstValue::Sequence(items) if items.len() == 2
        ));
        assert!(matches!(
            evaluate("[1; count]"),
            ConstValue::SequenceRepeat { seed, count }
                if seed == Box::new(ConstValue::Int(1.into()))
                    && count.as_ref() == &NormalExpr::Var(Intern::from_ref("count"))
        ));
    }
}

#[cfg(test)]
#[path = "const_expr/call_tests.rs"]
mod call_tests;

#[cfg(test)]
#[path = "const_expr/field_tests.rs"]
mod field_tests;
