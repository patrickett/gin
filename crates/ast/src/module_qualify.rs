use std::collections::HashSet;
use std::mem;
use std::ops::ControlFlow;

use internment::Intern;

use crate::{
    folder::*,
    path::ModPath,
    AsmExpr, DefMap, Expr, FileAst, FnCall,
};

use ControlFlow::Continue;

/// Prefix every top-level def in `ast` with `module_qual.` (e.g. `io.print`), rewrite
/// same-file references, and keep [`Bind::name`](Bind) in sync with the def map key.
pub fn qualify_module_defs(mut ast: FileAst, module_qual: &str) -> FileAst {
    let module_qual = module_qual.trim_matches('.');
    if module_qual.is_empty() {
        return ast;
    }

    let qual_parts: Vec<Intern<String>> = module_qual
        .split('.')
        .filter(|s| !s.is_empty())
        .map(Intern::<String>::from_ref)
        .collect();
    if qual_parts.is_empty() {
        return ast;
    }

    let old_names: HashSet<Intern<String>> = ast.defs.keys().copied().collect();

    let mut new_defs = DefMap::new();
    let old_defs = mem::take(&mut ast.defs);
    let mut folder = ModuleQualifyFolder {
        old_names,
        qual_parts,
    };
    for (name, mut bind) in old_defs {
        let new_name = Intern::<String>::new(format!("{module_qual}.{}", name.as_str()));
        let _ = folder.fold_bind(&mut bind);
        let bind = bind.remap_module_symbol(new_name);
        new_defs.insert(new_name, bind);
    }
    ast.defs = new_defs;

    let mut new_private = HashSet::new();
    for name in ast.private_defs.drain() {
        new_private.insert(Intern::<String>::new(format!(
            "{module_qual}.{}",
            name.as_str()
        )));
    }
    ast.private_defs = new_private;

    ast
}

struct ModuleQualifyFolder {
    old_names: HashSet<Intern<String>>,
    qual_parts: Vec<Intern<String>>,
}

impl Folder for ModuleQualifyFolder {
    fn fold_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        match expr {
            Expr::TypeQualified(path) => {
                maybe_rewrite_fn_path(path, &self.old_names, &self.qual_parts);
                Continue(())
            }
            _ => walk_expr_mut(self, expr),
        }
    }

    fn fold_fn_call(&mut self, call: &mut FnCall) -> ControlFlow<()> {
        maybe_rewrite_fn_path(&mut call.path, &self.old_names, &self.qual_parts);
        walk_fn_call_mut(self, call)
    }

    fn fold_asm_expr(&mut self, a: &mut AsmExpr) -> ControlFlow<()> {
        for o in &mut a.operands {
            self.fold_expr(o)?;
        }
        Continue(())
    }
}

fn maybe_rewrite_fn_path(
    path: &mut ModPath,
    old_names: &HashSet<Intern<String>>,
    qual_parts: &[Intern<String>],
) {
    if !path.segments.is_empty() {
        return;
    }
    if !old_names.contains(&path.root) {
        return;
    }
    let span = path.span;
    let old_root = path.root;
    let mut segments: Vec<Intern<String>> = qual_parts[1..].to_vec();
    segments.push(old_root);
    path.root = qual_parts[0];
    path.segments = segments;
    path.span = span;
}
