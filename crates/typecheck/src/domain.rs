use crate::tag_declarations::TagDeclarations;
use diagnostic::code::{DomainCode, PatternCode};
use std::collections::{HashMap, HashSet};

use ast::folder::{Folder, ParameterSubstitution};
use ast::integer::IntegerDomain;
use ast::{ConstValue, Declare, DeclareValue, Expr, HasMember, ParameterKind, Typed};
use diagnostic::Diagnostic;
use internment::Intern;

use crate::analysis::TypeEnv;

pub(crate) struct Domains<'a> {
    types: TypeEnv<'a>,
    declarations: TagDeclarations<'a>,
    values: Option<&'a HashMap<Intern<String>, ConstValue>>,
    file: Option<&'a ast::FileAst>,
    definitions: Option<&'a ast::DefMap>,
    reads: Option<&'a crate::transform::implementation_reads::ImplementationReadRecorder>,
}

impl<'a> Domains<'a> {
    pub fn new(types: TypeEnv<'a>, declarations: impl Into<TagDeclarations<'a>>) -> Self {
        Self {
            types,
            declarations: declarations.into(),
            values: None,
            file: None,
            definitions: None,
            reads: None,
        }
    }

    pub fn with_values(mut self, values: &'a HashMap<Intern<String>, ConstValue>) -> Self {
        self.values = Some(values);
        self
    }

    pub fn with_file(mut self, file: &'a ast::FileAst) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_definitions(mut self, definitions: &'a ast::DefMap) -> Self {
        self.definitions = Some(definitions);
        self
    }

    pub fn with_reads(
        mut self,
        reads: Option<&'a crate::transform::implementation_reads::ImplementationReadRecorder>,
    ) -> Self {
        self.reads = reads;
        self
    }

    fn binding(
        &self,
        name: Intern<String>,
        seen: &mut HashSet<Intern<String>>,
        depth: usize,
    ) -> Result<Option<ConstValue>, Diagnostic> {
        if let Some(value) = self.values.and_then(|values| values.get(&name)) {
            return Ok(Some(value.clone()));
        }
        let definition = self
            .definitions
            .and_then(|defs| defs.get(&name))
            .or_else(|| {
                if let Some(reads) = self.reads {
                    reads.binding(name);
                }
                self.file.and_then(|file| file.defs.get(&name))
            })
            .filter(|bind| bind.params.as_ref().is_none_or(|params| params.is_empty()));
        if let Some(ast::BindValue::Expr(body)) = definition.map(|b| &b.value) {
            if !seen.insert(name) {
                return Ok(None);
            }
            let value = self.eval(&body.value, seen, depth + 1);
            seen.remove(&name);
            return value;
        }
        self.alias(name, &[], seen, depth + 1)
    }

    pub fn value(&self, expr: &Expr) -> Result<Option<ConstValue>, Diagnostic> {
        self.eval(expr, &mut HashSet::new(), 0)
    }

    pub fn projection(&self, expr: &Expr) -> Result<Option<Expr>, Diagnostic> {
        if let Expr::RecordGet { base, field } = expr {
            self.property(&base.value, *field, &mut HashSet::new(), 0)
        } else {
            Ok(None)
        }
    }

    fn eval(
        &self,
        expr: &Expr,
        seen: &mut HashSet<Intern<String>>,
        depth: usize,
    ) -> Result<Option<ConstValue>, Diagnostic> {
        if depth >= 128 {
            // type-domain-recursion
            return Err(Diagnostic::new(
                DomainCode::Recursion,
                "domain evaluation did not terminate",
            ));
        }
        match expr {
            Expr::DomainValue(domain) => Ok(Some(ConstValue::Domain(domain.clone()))),
            Expr::DomainOf(operand) => {
                let local_name = match &operand.value {
                    Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => {
                        Some(call.path.root)
                    }
                    Expr::SelfRef => Some(Intern::from_ref("self")),
                    _ => None,
                };
                let ty = local_name
                    .and_then(|name| self.types.tag_types.get(&name).cloned())
                    .unwrap_or_else(|| self.types.resolve_expr(&operand.value));
                let validity = self
                    .types
                    .registry
                    .and_then(|r| r.integer_validity_for_type(ty.without_reference_wrappers()))
                    .or_else(|| ty.without_reference_wrappers().anonymous_integer_validity());
                let Some(validity) = validity else {
                    // type-domain-operand
                    return Err(Diagnostic::new(
                        DomainCode::Operand,
                        "`domain of` requires an integer type or a value with a declared integer domain",
                    ));
                };
                Ok(Some(ConstValue::Domain(validity.domain().clone().into())))
            }
            Expr::Range(range) => {
                let (Some(ConstValue::Int(start)), Some(ConstValue::Int(end))) = (
                    self.eval(&range.start.value, seen, depth + 1)?,
                    self.eval(&range.end.value, seen, depth + 1)?,
                ) else {
                    return Ok(None);
                };
                let domain =
                    IntegerDomain::bounded(start, end).unwrap_or_else(IntegerDomain::empty);
                Ok(Some(ConstValue::Domain(domain.into())))
            }
            Expr::Lit(ast::Literal::Int(value)) => Ok(Some(ConstValue::Int(*value))),
            Expr::Negate(inner) => Ok(match self.eval(&inner.value, seen, depth + 1)? {
                Some(ConstValue::Int(value)) => Some(ConstValue::Int(-value)),
                _ => None,
            }),
            Expr::Binary(binary) => Ok(
                match (
                    self.eval(&binary.lhs.value, seen, depth + 1)?,
                    self.eval(&binary.rhs.value, seen, depth + 1)?,
                ) {
                    (Some(left), Some(right)) => left.eval_binop(&binary.op, &right),
                    _ => None,
                },
            ),
            Expr::RecordGet { base, field } => {
                if let Some(body) = self.property(&base.value, *field, seen, depth + 1)? {
                    self.eval(&body, seen, depth + 1)
                } else {
                    Ok(None)
                }
            }
            Expr::AnonymousTag(name) => self.binding(*name, seen, depth + 1),
            Expr::TagCall(call) => self.alias(call.name, &call.args, seen, depth + 1),
            Expr::FnCall(call) if call.args.is_none() => {
                if let Some((field, prefix)) = call.path.segments.split_last() {
                    let mut parts = vec![call.path.root.as_str()];
                    parts.extend(prefix.iter().map(|p| p.as_str()));
                    let base = Expr::AnonymousTag(Intern::new(parts.join(".")));
                    if let Some(body) = self.property(&base, *field, seen, depth + 1)? {
                        return self.eval(&body, seen, depth + 1);
                    }
                    parts.push(field.as_str());
                    return self.binding(Intern::new(parts.join(".")), seen, depth + 1);
                }
                self.binding(call.path.root, seen, depth + 1)
            }
            _ => Ok(None),
        }
    }

    fn alias(
        &self,
        name: Intern<String>,
        args: &[Typed<Expr>],
        seen: &mut HashSet<Intern<String>>,
        depth: usize,
    ) -> Result<Option<ConstValue>, Diagnostic> {
        let Some(decl) = self.declarations.get(&name) else {
            return Ok(None);
        };
        if !seen.insert(name) {
            return Ok(None);
        }
        let result = (|| {
            let Some(subst) = self.arguments(decl, args, seen, depth + 1)? else {
                return Ok(None);
            };
            match &decl.value {
                DeclareValue::Alias(body) => {
                    let mut body = body.value.clone();
                    let _ = ParameterSubstitution::new(&subst).visit_expr(&mut body);
                    self.eval(&body, seen, depth + 1)
                }
                DeclareValue::Range(lb, ub) => Ok(Some(ConstValue::Domain(
                    IntegerDomain::bounded(*lb, *ub)
                        .unwrap_or_else(IntegerDomain::empty)
                        .into(),
                ))),
                _ => Ok(None),
            }
        })();
        seen.remove(&name);
        result
    }

    fn arguments(
        &self,
        decl: &Declare,
        args: &[Typed<Expr>],
        seen: &mut HashSet<Intern<String>>,
        depth: usize,
    ) -> Result<Option<HashMap<Intern<String>, Expr>>, Diagnostic> {
        let Some(params) = &decl.params else {
            return Ok(Some(HashMap::new()));
        };
        if !params.values().any(|p| p.range_pattern.is_some()) {
            return Ok(None);
        }
        if params.len() != args.len() {
            // type-range-argument-count
            return Err(Diagnostic::new(
                DomainCode::RangeArgumentCount,
                format!(
                    "`{}` expects {} domain arguments, found {}",
                    decl.name,
                    params.len(),
                    args.len()
                ),
            ));
        }
        let mut subst = HashMap::new();
        for ((name, parameter), arg) in params.iter().zip(args) {
            if let Some(pattern) = &parameter.range_pattern {
                let value = self.eval(&arg.value, seen, depth + 1)?;
                let Some(ConstValue::Domain(domain)) = value else {
                    // type-range-pattern
                    return Err(Diagnostic::new(
                        PatternCode::Range,
                        "range parameter requires a domain consisting of one closed interval",
                    ));
                };
                let Some((lower, upper)) = domain.closed_bounds() else {
                    // type-range-pattern
                    return Err(Diagnostic::new(
                        PatternCode::Range,
                        "domain does not match a single closed interval; range matching preserves all constraints",
                    ));
                };
                subst.insert(
                    pattern.start,
                    crate::const_value::normal_expr_to_expr(&lower),
                );
                subst.insert(pattern.end, crate::const_value::normal_expr_to_expr(&upper));
            } else if matches!(parameter.kind, ParameterKind::Generic) {
                subst.insert(*name, arg.value.clone());
            }
        }
        Ok(Some(subst))
    }

    fn property(
        &self,
        base: &Expr,
        field: Intern<String>,
        seen: &mut HashSet<Intern<String>>,
        depth: usize,
    ) -> Result<Option<Expr>, Diagnostic> {
        let (name, args) = match base {
            Expr::AnonymousTag(name) => (*name, &[][..]),
            Expr::TagCall(call) => (call.name, call.args.as_slice()),
            _ => return Ok(None),
        };
        let Some(decl) = self.declarations.get(&name) else {
            return Ok(None);
        };
        if !seen.insert(name) {
            return Ok(None);
        }
        let result = (|| {
            let Some(subst) = self.arguments(decl, args, seen, depth + 1)? else {
                return Ok(None);
            };
            for declaration in std::iter::once(decl).chain(&decl.extensions) {
                let DeclareValue::Has(members) = &declaration.value else {
                    continue;
                };
                let Some(bindings) = self.arguments(declaration, args, seen, depth + 1)? else {
                    continue;
                };
                for member in members {
                    if let HasMember::Property(property) = member
                        && property.name == field
                        && let Some(expr) = property.body.as_ref().and_then(|b| b.as_expr())
                    {
                        let mut body = expr.value.clone();
                        let _ = ParameterSubstitution::new(&bindings).visit_expr(&mut body);
                        return Ok(Some(body));
                    }
                }
            }
            if let DeclareValue::Alias(alias) = &decl.value {
                let mut alias = alias.value.clone();
                let _ = ParameterSubstitution::new(&subst).visit_expr(&mut alias);
                return self.property(&alias, field, seen, depth + 1);
            }
            Ok(None)
        })();
        seen.remove(&name);
        result
    }
}
