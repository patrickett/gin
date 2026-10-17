use std::collections::HashSet;
use std::mem;
use std::ops::ControlFlow;

use internment::Intern;

use crate::{DefMap, FileAst, FnCall, folder::*, path::ModPath};

impl FileAst {
    /// Prefix every top-level def with `module_qual.` (e.g. `io.print`), rewrite
    /// same-file references, and keep [`Bind::name`](Bind) in sync with the def map key.
    pub fn qualify_module_defs(mut self, module_qual: &str) -> FileAst {
        let module_qual = module_qual.trim_matches('.');
        if module_qual.is_empty() {
            return self;
        }

        let qual_parts: Vec<Intern<String>> = module_qual
            .split('.')
            .filter(|s| !s.is_empty())
            .map(Intern::<String>::from_ref)
            .collect();
        if qual_parts.is_empty() {
            return self;
        }

        let qual_prefix = qual_parts
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>()
            .join(".");

        let old_names: HashSet<Intern<String>> = self.defs.keys().map(|id| id.0).collect();

        let mut new_defs = DefMap::new();
        let old_defs = mem::take(&mut self.defs);
        let mut folder = ModuleQualifyFolder {
            old_names,
            qual_parts,
            qual_prefix,
        };
        for (name, mut bind) in old_defs {
            let new_name =
                Intern::<String>::new(format!("{}.{}", folder.qualified_prefix(), name.0.as_str()));
            let _ = folder.visit_bind(&mut bind);
            let bind = bind.remap_module_symbol(new_name);
            new_defs.insert(name.with_name(new_name), bind);
        }
        self.defs = new_defs;
        for declaration in self.tags.values_mut() {
            let shadowed: Vec<_> = declaration
                .params
                .iter()
                .flat_map(|params| params.keys())
                .copied()
                .filter(|name| folder.old_names.remove(name))
                .collect();
            if let crate::DeclareValue::Refinement(predicate) = &mut declaration.value {
                folder.qualify_predicate(predicate);
            }
            if let crate::DeclareValue::Has(members) = &mut declaration.value {
                for member in members {
                    if let crate::HasMember::Property(property) = member {
                        if let Some(surface) = &mut property.ty {
                            let _ = folder.visit_expr(&mut surface.value);
                        }
                        if let Some(predicate) = &mut property.refinement {
                            folder.qualify_predicate(predicate);
                        }
                    }
                }
            }
            if let Some(bits) = &mut declaration.attributes.bits {
                let _ = folder.visit_expr(&mut bits.value);
            }
            folder.old_names.extend(shadowed);
        }

        let mut new_private = HashSet::new();
        for name in self.private_defs.drain() {
            new_private.insert(Intern::<String>::new(format!(
                "{}.{}",
                folder.qualified_prefix(),
                name.as_str()
            )));
        }
        self.private_defs = new_private;

        self
    }
}

struct ModuleQualifyFolder {
    old_names: HashSet<Intern<String>>,
    qual_parts: Vec<Intern<String>>,
    qual_prefix: String,
}

impl ModuleQualifyFolder {
    fn qualified_prefix(&self) -> &str {
        &self.qual_prefix
    }

    fn qualify_predicate(&self, predicate: &mut crate::PredicateExpr) {
        let prefix = self.qualified_prefix();
        crate::ty::walk_predicate_names(predicate, &mut |name| {
            if self.old_names.contains(name) {
                *name = Intern::new(format!("{prefix}.{name}"));
            }
        });
    }

    fn qualify_proposition(&self, proposition: &mut crate::ProofProposition) {
        let mut predicate = crate::PredicateExpr::Proposition(Box::new(proposition.clone()));
        self.qualify_predicate(&mut predicate);
        let crate::PredicateExpr::Proposition(value) = predicate else {
            unreachable!()
        };
        *proposition = *value;
    }
}

impl Folder for ModuleQualifyFolder {
    fn visit_return(&mut self, ret: &mut crate::Return) -> ControlFlow<()> {
        if let Some(proof) = &mut ret.postcondition {
            self.qualify_proposition(&mut proof.value);
        }
        crate::folder::walk_return_mut(self, ret)
    }

    fn visit_bind(&mut self, bind: &mut crate::Bind) -> ControlFlow<()> {
        let shadowed: Vec<_> = bind
            .params
            .iter()
            .flat_map(|params| params.keys())
            .copied()
            .filter(|name| self.old_names.remove(name))
            .collect();
        for predicate in bind.param_refinements.values_mut() {
            self.qualify_predicate(predicate);
        }
        if let Some(predicate) = &mut bind.return_refinement {
            self.qualify_predicate(predicate);
        }
        for proof in &mut bind.payload_postconditions {
            let shadowed_result = self.old_names.remove(&proof.name);
            self.qualify_proposition(&mut proof.proposition);
            if shadowed_result {
                self.old_names.insert(proof.name);
            }
        }
        let result = walk_bind_mut(self, bind);
        self.old_names.extend(shadowed);
        result
    }

    fn visit_fn_call(&mut self, call: &mut FnCall) -> ControlFlow<()> {
        if let Some(id) = call.definition
            && self.old_names.contains(&id.0)
        {
            let prefix = self.qualified_prefix();
            call.definition = Some(id.with_name(Intern::new(format!("{prefix}.{}", id.0))));
        }
        call.path
            .rewrite_module_path(&self.old_names, &self.qual_parts);
        walk_fn_call_mut(self, call)
    }
}

impl ModPath {
    /// Rewrite this path's root to a qualified module path when it references a local definition.
    fn rewrite_module_path(
        &mut self,
        old_names: &HashSet<Intern<String>>,
        qual_parts: &[Intern<String>],
    ) {
        let local_name = if self.segments.is_empty() {
            self.root
        } else {
            Intern::new(self.to_qualified_string())
        };
        if !old_names.contains(&local_name) {
            return;
        }
        let old_root = self.root;
        let mut segments = Vec::with_capacity(qual_parts.len() + self.segments.len());
        segments.extend_from_slice(&qual_parts[1..]);
        segments.push(old_root);
        segments.append(&mut self.segments);
        self.root = qual_parts[0];
        self.segments = segments;
    }
}
