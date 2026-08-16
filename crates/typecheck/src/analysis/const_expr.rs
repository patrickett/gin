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
}

impl<'a> CompTimeEvaluator<'a> {
    pub fn new(const_binds: &'a ConstEnv, ast: &'a FileAst) -> Self {
        Self {
            const_binds,
            ast,
            target_layout: None,
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
        }
    }

    pub fn eval(&self, expr: &Expr) -> Option<ConstValue> {
        let mut call_stack = HashSet::new();
        EvalFrame::new(
            self.const_binds,
            self.ast,
            self.target_layout,
            &mut call_stack,
        )
        .eval(expr)
    }
}

/// A stateful compile-time evaluator for constant expressions.
struct EvalFrame<'a, 'b> {
    const_binds: &'a ConstEnv,
    ast: &'a FileAst,
    target_layout: Option<crate::layout::TargetLayout>,
    depth: usize,
    call_stack: &'b mut HashSet<ComptimeCall>,
}

impl<'a, 'b> EvalFrame<'a, 'b> {
    pub fn new(
        const_binds: &'a ConstEnv,
        ast: &'a FileAst,
        target_layout: Option<crate::layout::TargetLayout>,
        call_stack: &'b mut HashSet<ComptimeCall>,
    ) -> Self {
        Self {
            const_binds,
            ast,
            target_layout,
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
            Expr::Lit(lit) => match lit {
                Literal::Int(n) => Some(ConstValue::Int(*n)),
                Literal::Float(HashFloat(f)) => Some(ConstValue::Float(HashFloat(*f))),
                Literal::String(s) => Some(ConstValue::String(s.clone())),
                Literal::Number(n) => Some(ConstValue::Int((*n as u128).into())),
            },
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
                ConstValue::Record { fields } => fields
                    .iter()
                    .find(|(n, _)| n == field)
                    .map(|(_, v)| v.clone()),
                ConstValue::Tag { name, args, .. } => {
                    let idx = self
                        .ast
                        .tags
                        .get(&name)
                        .and_then(|decl| decl.params.as_ref())
                        .and_then(|params| params.keys().position(|param_name| param_name == field))
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
                None
            }
            Expr::TupleLit(elems) | Expr::List(elems) => {
                let items: Vec<ConstValue> =
                    elems.iter().filter_map(|e| self.eval(&e.value)).collect();
                if items.len() == elems.len() {
                    Some(ConstValue::List(items.into()))
                } else {
                    None
                }
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
                lhs.eval_binop(&bin.op, &rhs)
            }
            Expr::TargetQuery { kind, operand } => {
                let operand = target_query_operand_ty(operand)?;
                let value = self.target_layout?.query(*kind, &operand, None).ok()?;
                Some(ConstValue::Int(i256::I256::from(value)))
            }
            Expr::When(when) => {
                let subject = when.subject.as_ref().and_then(|s| self.eval(&s.value))?;
                self.eval_matching_when_arm(&when.arms, &subject)
            }
            Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty() => {
                self.const_binds
                    .get(&call.path.value.root)
                    .and_then(|cv| cv.clone())
            }
            Expr::FnCall(call) if call.args.is_none() && !call.path.value.segments.is_empty() => {
                let fq_name = self.qualified_name(&call.path.value);
                if let Some(cv) = self.const_binds.get(&fq_name).and_then(|cv| cv.clone()) {
                    return Some(cv);
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
                let bind = self.ast.defs.get(&name)?;
                if !bind.is_constant() {
                    return None;
                }
                self.check_arity(bind, &args).ok()?;
                self.eval_bind_call(bind, &args, self.depth + 1)
            }
            Expr::FnCall(_) => None,
            _ => None,
        }
    }

    fn eval_in_env(&mut self, env: &ConstEnv, depth: usize, expr: &Expr) -> Option<ConstValue> {
        EvalFrame::new(env, self.ast, self.target_layout, self.call_stack)
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
                WhenArm::Is { pattern, body, .. } => {
                    if crate::analysis::pattern::pattern_matches_public(&pattern.value, subject) {
                        let mut arm_env = self.const_binds.clone();
                        let mut flat = HashMap::new();
                        collect_pattern_bindings(&pattern.value, subject, &mut flat);
                        for (k, v) in flat {
                            arm_env.insert(k, Some(v));
                        }
                        return self.eval_in_env(&arm_env, self.depth, &body.value);
                    }
                }
                WhenArm::Else(body, _) => return self.eval(&body.value),
                WhenArm::Cond { .. } => return None,
            }
        }
        None
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
                ret.value
                    .as_ref()
                    .and_then(|ret| self.eval_in_env(&env, depth, &ret.value))
            }
            BindValue::Extern | BindValue::Unassigned => None,
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
}
