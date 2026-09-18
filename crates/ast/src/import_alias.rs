use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use internment::Intern;

use crate::{
    Bind, Expr, FileAst, FnCall, ImportSource, Pattern, TagCall, WhenArm,
    folder::*,
    path::ModPath,
    span::{SpanId, Spanned},
};

use ControlFlow::Continue;

type AliasMap = HashMap<Intern<String>, Spanned<ModPath>>;

impl FileAst {
    /// Rewrite expressions so imported symbols can be referenced by their bare names.
    pub fn apply_symbol_aliases(&mut self) {
        if self.symbol_aliases.is_empty() {
            return;
        }
        let alias_map = self.build_alias_map();
        let namespaces = self
            .uses
            .iter()
            .flat_map(|import| &import.0)
            .filter(|module| {
                matches!(
                    module.source,
                    ImportSource::Package(_)
                        | ImportSource::Local(..)
                        | ImportSource::LocalFile(..)
                )
            })
            .map(|module| {
                module
                    .alias
                    .unwrap_or_else(|| Intern::new(module.effective_name()))
            })
            .collect();
        let mut folder = ImportAliasFolder {
            alias_map,
            namespaces,
            alias_spans: Vec::new(),
        };
        let _ = walk_file_ast_mut(&mut folder, self);
        self.symbol_alias_spans = folder.alias_spans;
    }
}

impl FileAst {
    fn build_alias_map(&self) -> AliasMap {
        let mut map = HashMap::new();
        for alias in &self.symbol_aliases {
            map.insert(alias.alias, alias.target.clone());
        }
        map
    }
}

struct ImportAliasFolder {
    alias_map: AliasMap,
    namespaces: HashSet<Intern<String>>,
    alias_spans: Vec<SpanId>,
}

impl Expr {
    fn apply_alias(&mut self, alias_map: &AliasMap) {
        match self {
            Expr::FnCall(call) => {
                call.path.apply_alias(alias_map);
                if let Some(args) = &mut call.args {
                    for arg in args {
                        arg.value.apply_alias(alias_map);
                    }
                }
            }
            Expr::TagCall(call) => {
                if let Some(path) = &mut call.qual_path {
                    path.value.apply_alias(alias_map);
                }
                for arg in &mut call.args {
                    arg.value.apply_alias(alias_map);
                }
            }
            Expr::Ref { inner, .. }
            | Expr::TakePtr(inner)
            | Expr::Deref(inner)
            | Expr::Negate(inner) => inner.value.apply_alias(alias_map),
            Expr::ArrayLit(values) | Expr::List(values) => {
                for value in values {
                    value.value.apply_alias(alias_map);
                }
            }
            _ => {}
        }
    }
}

impl Folder for ImportAliasFolder {
    fn visit_bind(&mut self, bind: &mut Bind) -> ControlFlow<()> {
        for attribute in bind.attributes.raw_attributes.iter_mut().flatten() {
            if let crate::AttributeItem::Call { name, args, .. } = attribute
                && matches!(name.as_str(), "machine")
            {
                for arg in args {
                    self.visit_expr(arg)?;
                }
            }
        }
        let mut aliases = Vec::new();
        let mut namespaces = Vec::new();
        if let Some(params) = &bind.params {
            for name in params.keys() {
                if let Some(target) = self.alias_map.remove(name) {
                    aliases.push((*name, target));
                }
                if self.namespaces.remove(name) {
                    namespaces.push(*name);
                }
            }
        }
        let result = walk_bind_mut(self, bind);
        self.alias_map.extend(aliases);
        self.namespaces.extend(namespaces);
        result
    }

    fn visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        if let Expr::RecordGet { base, field } = expr
            && let Expr::FnCall(call) = &base.value
            && call.args.is_none()
            && call.path.segments.is_empty()
            && self.namespaces.contains(&call.path.root)
        {
            let mut call = call.clone();
            call.path.segments.push(*field);
            *expr = Expr::FnCall(call);
        }
        match expr {
            Expr::AnonymousTag(name) => {
                if let Some(_target) = self.alias_map.get(name) {
                    // Span removed from AnonymousTag; alias tracking for bare tags
                    // is handled through the type-expr pass instead.
                    // self.alias_spans.push(*span);
                    // Encode the target as the path of a TagCall or similar
                    // expression position. Type expressions are handled in a
                    // separate pass.
                }
                Continue(())
            }
            Expr::Destructure { value, .. } => {
                self.visit_expr(&mut value.value)?;
                Continue(())
            }
            Expr::When(when) => {
                if let Some(subject) = &mut when.subject {
                    self.visit_expr(&mut subject.value)?;
                }
                for arm in &mut when.arms {
                    self.visit_when_arm(arm)?;
                }
                Continue(())
            }
            _ => walk_expr_mut(self, expr),
        }
    }

    fn visit_when_arm(&mut self, arm: &mut WhenArm) -> ControlFlow<()> {
        match arm {
            WhenArm::Cond {
                condition, body, ..
            } => {
                self.visit_condition(condition)?;
                self.visit_expr(&mut body.value)
            }
            WhenArm::Is { pattern, body, .. } => {
                pattern.value.apply_alias(&self.alias_map);
                self.visit_expr(&mut body.value)
            }
            WhenArm::Else(body, _) => self.visit_expr(&mut body.value),
        }
    }

    fn visit_condition(&mut self, condition: &mut crate::Condition) -> ControlFlow<()> {
        match condition {
            crate::Condition::Is { subject, pattern } => {
                self.visit_expr(&mut subject.value)?;
                pattern.value.apply_alias(&self.alias_map);
                Continue(())
            }
            crate::Condition::Not(inner) => self.visit_condition(inner),
            crate::Condition::And(left, right) | crate::Condition::Or(left, right) => {
                self.visit_condition(left)?;
                self.visit_condition(right)
            }
        }
    }

    fn visit_fn_call(&mut self, call: &mut FnCall) -> ControlFlow<()> {
        if let Some(span) = call.path.apply_alias(&self.alias_map) {
            self.alias_spans.push(span);
        }
        walk_fn_call_mut(self, call)
    }

    fn visit_tag_call(&mut self, tc: &mut TagCall) -> ControlFlow<()> {
        if let Some(path) = &mut tc.qual_path {
            path.apply_alias(&self.alias_map);
        }
        walk_tag_call_mut(self, tc)
    }
}

impl Pattern {
    fn apply_alias(&mut self, alias_map: &AliasMap) {
        match self {
            Pattern::Nominal(name, span) => {
                if name
                    .as_str()
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_uppercase())
                    && let Some(target) = alias_map.get(name)
                {
                    *self = Pattern::Qualified(Spanned::new(target.value.clone(), *span));
                }
            }
            Pattern::Qualified(path) => {
                path.value.apply_alias(alias_map);
            }
            Pattern::Generic { params, .. } => {
                for (_, kind) in params {
                    match kind {
                        crate::ParameterKind::Tagged(sp) => sp.value.apply_alias(alias_map),
                        crate::ParameterKind::ValueParam { ty }
                        | crate::ParameterKind::Inferred { ty } => ty.value.apply_alias(alias_map),
                        _ => {}
                    }
                }
            }
            Pattern::Pointer(inner) | Pattern::Ref { inner, .. } => {
                inner.value.apply_alias(alias_map);
            }
            Pattern::ListCons { head, tail } => {
                head.value.apply_alias(alias_map);
                tail.value.apply_alias(alias_map);
            }
            Pattern::Invalid
            | Pattern::Literal(..)
            | Pattern::Unit
            | Pattern::ListEmpty
            | Pattern::InRange { .. } => {}
        }
    }
}

impl ModPath {
    /// Apply a symbol alias to the root of this mod path.
    fn apply_alias(&mut self, alias_map: &AliasMap) -> Option<SpanId> {
        if let Some(target) = alias_map.get(&self.root) {
            self.root = target.root;
            let mut segments = target.segments.clone();
            segments.append(&mut self.segments);
            self.segments = segments;
            return Some(target.span_id);
        }
        None
    }
}
