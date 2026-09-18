//! Compile-time expression evaluation — fold expressions to [`ConstValue`]s.
//!
//! Handles literal folding, tag constructors, binary ops, bind references,
//! and compile-time bind validation.

use derive_more::{Deref, DerefMut, From};
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;

use internment::Intern;

use crate::analysis::pattern::collect_pattern_bindings;
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
        }
    }

    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    pub fn eval(&mut self, expr: &Expr) -> Option<ConstValue> {
        self.check_depth().ok()?;
        match expr {
            Expr::Target => {
                let target = self.target.as_ref()?;
                Some(ConstValue::Record {
                    fields: vec![
                        (
                            Intern::from_ref("arch"),
                            ConstValue::Text(target.arch.as_str().into()),
                        ),
                        (
                            Intern::from_ref("vendor"),
                            ConstValue::Text(target.vendor.clone()),
                        ),
                        (
                            Intern::from_ref("os"),
                            ConstValue::Text(target.os.as_str().into()),
                        ),
                        (
                            Intern::from_ref("triple"),
                            ConstValue::Text(target.raw.clone()),
                        ),
                    ]
                    .into(),
                })
            }
            Expr::Lit(lit) => match lit {
                Literal::Int(n) => Some(ConstValue::Int(*n)),
                Literal::Float(HashFloat(f)) => Some(ConstValue::Float(HashFloat(*f))),
                Literal::String(s) => Some(ConstValue::Text(s.clone())),
                Literal::Number(n) => Some(ConstValue::Int((*n as u128).into())),
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
                // `qual_path` is the prefix of the path, NOT including the last
                // segment which is already captured in `name`. Skipping the last
                // segment prevents double-appending in `to_hover_string`.
                let qual_path = call.qual_path.as_ref().map(|p| {
                    let use_segments =
                        if p.segments.last().map(|s| s.as_str()) == Some(call.name.as_str()) {
                            &p.segments[..p.segments.len().saturating_sub(1)]
                        } else {
                            &p.segments[..]
                        };
                    let mut s = p.root.as_str().to_string();
                    for seg in use_segments {
                        s.push('.');
                        s.push_str(seg.as_str());
                    }
                    s
                });
                Some(ConstValue::Tag {
                    name: call.name,
                    qual_path,
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
                                ast::DeclareValue::Has(members) => members.iter().position(|member| {
                                    matches!(member, ast::HasMember::Property(property) if property.name == *field)
                                }),
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
                self.check_arity(&bind, &[]).ok()?;
                self.eval_bind_call(&bind, &[], self.depth + 1)
            }
            Expr::ArrayLit(elems) => {
                let items: Vec<ConstValue> =
                    elems.iter().filter_map(|e| self.eval(&e.value)).collect();
                (items.len() == elems.len()).then(|| ConstValue::FixedArray(items.into()))
            }
            Expr::List(elems) | Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elems)) => {
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
                let subject = when
                    .subject
                    .as_ref()
                    .and_then(|s| self.eval(&s.value))
                    .unwrap_or(ConstValue::ZERO);
                self.eval_matching_when_arm(&when.arms, &subject)
            }
            Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty() => {
                let name = call.path.value.root;
                if let Some(cv) = self.const_binds.get(&name).and_then(|cv| cv.clone()) {
                    return Some(cv);
                }
                let final_name = call
                    .path
                    .value
                    .segments
                    .last()
                    .unwrap_or(&call.path.value.root);
                let bind = self
                    .ast
                    .defs
                    .get(&name)
                    .or_else(|| self.ast.defs.get(final_name))
                    .or_else(|| {
                        self.ast.defs.values().find(|bind| {
                            bind.name.as_str().rsplit(['.', ':']).next()
                                == Some(final_name.as_str())
                        })
                    })?;
                if !bind.is_constant() && !bind.attributes.inline_always {
                    return None;
                }
                self.check_arity(bind, &[]).ok()?;
                self.eval_bind_call(bind, &[], self.depth + 1)
            }
            Expr::FnCall(call) if call.args.is_none() && !call.path.value.segments.is_empty() => {
                let fq_name = self.qualified_name(&call.path.value);
                if let Some(cv) = self.const_binds.get(&fq_name).and_then(|cv| cv.clone()) {
                    return Some(cv);
                }
                if call.path.value.root.as_str() == "target"
                    && let Some(target) = &self.target
                {
                    let field = call.path.value.segments.last()?.as_str();
                    let value = match field {
                        "arch" => Some(ConstValue::Text(target.arch.as_str().into())),
                        "vendor" => Some(ConstValue::Text(target.vendor.clone())),
                        "os" => Some(ConstValue::Text(target.os.as_str().into())),
                        "triple" => Some(ConstValue::Text(target.raw.clone())),
                        _ => None,
                    };
                    if value.is_some() {
                        return value;
                    }
                }
                let final_name = call.path.value.segments.last().expect("qualified call");
                let bind = self
                    .ast
                    .defs
                    .get(&fq_name)
                    .or_else(|| self.ast.defs.get(final_name))
                    .or_else(|| {
                        self.ast.defs.values().find(|bind| {
                            bind.name.as_str().rsplit(['.', ':']).next()
                                == Some(final_name.as_str())
                        })
                    });
                if let Some(bind) = bind {
                    self.check_arity(bind, &[]).ok()?;
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
                if let Some(Some(cv)) = self.const_binds.get(&name) {
                    return Some(cv.clone());
                }
                if let Some(cv) = self.eval_helper(name.as_str(), &args) {
                    return Some(cv);
                }
                let bind = self.resolve_bind(name)?.clone();
                self.check_arity(&bind, &args).ok()?;
                self.eval_bind_call(&bind, &args, self.depth + 1)
            }
            Expr::FnCall(call) => {
                let name = self.qualified_name(&call.path.value);
                let args = self.eval_args(call.args.as_deref())?;
                if let Some(Some(cv)) = self.const_binds.get(&name) {
                    return Some(cv.clone());
                }
                let final_name = call
                    .path
                    .value
                    .segments
                    .last()
                    .unwrap_or(&call.path.value.root);
                let bind = self
                    .ast
                    .defs
                    .get(&name)
                    .or_else(|| self.ast.defs.get(final_name))
                    .or_else(|| {
                        self.ast.defs.values().find(|bind| {
                            bind.name.as_str().rsplit(['.', ':']).next()
                                == Some(final_name.as_str())
                        })
                    })?;
                self.check_arity(bind, &args).ok()?;
                self.eval_bind_call(bind, &args, self.depth + 1)
            }
            _ => None,
        }
    }

    fn eval_in_env(&mut self, env: &ConstEnv, depth: usize, expr: &Expr) -> Option<ConstValue> {
        EvalFrame::new(
            env,
            self.ast,
            self.target_layout,
            self.target.clone(),
            self.call_stack,
        )
        .with_depth(depth)
        .eval(expr)
    }

    fn eval_args(&mut self, args: Option<&[Typed<Expr>]>) -> Option<Vec<ConstValue>> {
        args.map_or_else(
            || Some(Vec::new()),
            |args| args.iter().map(|arg| self.eval(&arg.value)).collect(),
        )
    }

    fn eval_helper(&self, name: &str, args: &[ConstValue]) -> Option<ConstValue> {
        match (name, args) {
            // Size helpers
            ("add", [a, b]) => match (a.as_const_size_int(), b.as_const_size_int()) {
                (Some(x), Some(y)) => Some(ConstValue::const_size_tag(x + y)),
                _ => Some(ConstValue::dynamic_size_tag()),
            },
            ("max_size", [a, b]) => match (a.as_const_size_int(), b.as_const_size_int()) {
                (Some(x), Some(y)) => Some(ConstValue::const_size_tag(if x >= y { x } else { y })),
                _ => Some(ConstValue::dynamic_size_tag()),
            },
            _ => None,
        }
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
                    let matches = self.eval_condition(condition)?;
                    if matches {
                        return self.eval(&body.value);
                    }
                }
                WhenArm::Is { pattern, body, .. } => {
                    let matches =
                        crate::analysis::pattern::pattern_matches_public(&pattern.value, subject);
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
        None
    }

    fn eval_condition(&mut self, condition: &ast::Condition) -> Option<bool> {
        match condition {
            ast::Condition::Is { subject, pattern } => {
                let value = self.eval(&subject.value)?;
                let result =
                    crate::analysis::pattern::pattern_matches_public(&pattern.value, &value);
                Some(result)
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
        let call_key = ComptimeCall {
            name: bind.name,
            args: args.into(),
        };
        self.enter_call(call_key.clone()).ok()?;

        let mut env = self.const_binds.clone();
        if let Some(params) = &bind.params {
            let param_names: Vec<_> = params.keys().copied().collect();
            for (name, cv) in param_names.into_iter().zip(args.iter().cloned()) {
                env.insert(name, Some(cv));
            }
        }
        let result = match &bind.value {
            BindValue::Expr(expr) => self.eval_in_env(&env, depth, &expr.value),
            BindValue::Body { exprs, ret } => {
                for expr in exprs {
                    if let Expr::Bind(local) = &expr.value
                        && local.params.is_none()
                        && let BindValue::Expr(initializer) = &local.value
                    {
                        let value = self.eval_in_env(&env, depth, &initializer.value);
                        if let Some(value) = value {
                            env.insert(local.name, Some(value));
                        }
                    } else {
                        let _ = self.eval_in_env(&env, depth, &expr.value);
                    }
                }
                if !ret.values.is_empty() {
                    let values = ret
                        .values
                        .iter()
                        .map(|ret| self.eval_in_env(&env, depth, &ret.value))
                        .collect::<Option<Vec<_>>>()?;
                    match values.as_slice() {
                        [] => None,
                        [value] => Some(value.clone()),
                        _ => Some(ConstValue::ResultRow(values.into())),
                    }
                } else {
                    ret.values
                        .first()
                        .and_then(|ret| self.eval_in_env(&env, depth, &ret.value))
                }
            }
            BindValue::Unassigned => None,
        };
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

    fn resolve_bind(&self, name: Intern<String>) -> Option<&Bind> {
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
}

fn sequence_count_expr(expr: &Expr) -> Option<ast::NormalExpr> {
    match expr {
        Expr::Lit(Literal::Int(value)) => Some(ast::NormalExpr::from(*value)),
        Expr::Lit(Literal::Number(value)) => Some(ast::NormalExpr::from(*value as i128)),
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
