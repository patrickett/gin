use std::collections::HashSet;
use std::mem;
use std::ops::ControlFlow;

use internment::Intern;

use crate::{AsmExpr, DefMap, Expr, FileAst, FnCall, folder::*, path::ModPath};

use ControlFlow::Continue;

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

        let old_names: HashSet<Intern<String>> = self.defs.keys().copied().collect();

        let mut new_defs = DefMap::new();
        let old_defs = mem::take(&mut self.defs);
        let mut folder = ModuleQualifyFolder {
            old_names,
            qual_parts,
        };
        for (name, mut bind) in old_defs {
            let new_name = Intern::<String>::new(format!("{module_qual}.{}", name.as_str()));
            let _ = folder.visit_bind(&mut bind);
            let bind = bind.remap_module_symbol(new_name);
            new_defs.insert(new_name, bind);
        }
        self.defs = new_defs;

        let mut new_private = HashSet::new();
        for name in self.private_defs.drain() {
            new_private.insert(Intern::<String>::new(format!(
                "{module_qual}.{}",
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
}

impl Folder for ModuleQualifyFolder {
    fn visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        // Type expressions (TypeExpr) are now a separate enum and are not
        // folded through the Expr Folder. Qualification of TypeExpr nodes
        // will be handled in a separate pass.
        walk_expr_mut(self, expr)
    }

    fn visit_fn_call(&mut self, call: &mut FnCall) -> ControlFlow<()> {
        call.path
            .rewrite_module_path(&self.old_names, &self.qual_parts);
        walk_fn_call_mut(self, call)
    }

    fn visit_asm_expr(&mut self, a: &mut AsmExpr) -> ControlFlow<()> {
        if let Some(spec) = &mut a.spec_expr {
            self.visit_expr(spec)?;
        }
        for o in &mut a.operand_values {
            self.visit_expr(o)?;
        }
        Continue(())
    }
}

impl ModPath {
    /// Rewrite this path's root to a qualified module path when it references a local definition.
    fn rewrite_module_path(
        &mut self,
        old_names: &HashSet<Intern<String>>,
        qual_parts: &[Intern<String>],
    ) {
        if !self.segments.is_empty() {
            return;
        }
        if !old_names.contains(&self.root) {
            return;
        }
        let old_root = self.root;
        let mut segments: Vec<Intern<String>> = qual_parts[1..].to_vec();
        segments.push(old_root);
        self.root = qual_parts[0];
        self.segments = segments;
    }
}
