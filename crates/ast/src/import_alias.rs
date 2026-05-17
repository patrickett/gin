use std::collections::HashMap;
use std::ops::ControlFlow;

use internment::Intern;

use crate::{
    Expr, FileAst, FnCall, SymbolAlias, TagCall,
    folder::*,
    path::ModPath,
    span::{SpanId, Spanned},
};

use ControlFlow::Continue;

type AliasMap = HashMap<Intern<String>, Spanned<ModPath>>;

/// Rewrite expressions so imported symbols can be referenced by their bare names.
pub fn apply_symbol_aliases(ast: &mut FileAst) {
    if ast.symbol_aliases.is_empty() {
        return;
    }
    let alias_map = build_alias_map(&ast.symbol_aliases);
    let mut folder = ImportAliasFolder {
        alias_map,
        alias_spans: Vec::new(),
    };
    let _ = walk_file_ast_mut(&mut folder, ast);
    ast.symbol_alias_spans = folder.alias_spans;
}

fn build_alias_map(aliases: &[SymbolAlias]) -> AliasMap {
    let mut map = HashMap::new();
    for alias in aliases {
        map.insert(alias.alias, alias.target.clone());
    }
    map
}

struct ImportAliasFolder {
    alias_map: AliasMap,
    alias_spans: Vec<SpanId>,
}

impl Folder for ImportAliasFolder {
    fn fold_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
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
            _ => walk_expr_mut(self, expr),
        }
    }

    fn fold_fn_call(&mut self, call: &mut FnCall) -> ControlFlow<()> {
        if let Some(span) = apply_alias_to_mod_path(&mut call.path, &self.alias_map) {
            self.alias_spans.push(span);
        }
        walk_fn_call_mut(self, call)
    }

    fn fold_tag_call(&mut self, tc: &mut TagCall) -> ControlFlow<()> {
        if let Some(path) = &mut tc.qual_path {
            apply_alias_to_mod_path(path, &self.alias_map);
        }
        walk_tag_call_mut(self, tc)
    }
}

fn apply_alias_to_mod_path(path: &mut ModPath, alias_map: &AliasMap) -> Option<SpanId> {
    if !path.segments.is_empty() {
        return None;
    }
    if let Some(target) = alias_map.get(&path.root) {
        path.root = target.root;
        path.segments = target.segments.clone();
        return Some(target.span_id);
    }
    None
}
