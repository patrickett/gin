use std::ops::ControlFlow;

use super::super::proof_walk::write::{ProofVisitorMut, walk_term};
use super::*;
use ast::{NormalExpr, ProofTerm, Ty};
use internment::Intern;

pub(super) fn normalize(
    typed: &TypedFileAst,
    signature: &TypedBind,
    proof: &TypedPostcondition,
    returned: Option<ExprId>,
) -> TypedPostcondition {
    let mut proof = proof.clone();
    let Some(returned) = returned else {
        return proof;
    };
    if !matches!(
        typed
            .type_registry
            .resolved_definition_for_type(&signature.return_type),
        Ty::Record { .. }
    ) || typed.exprs.ty_of(returned) != Some(&signature.return_type)
    {
        return proof;
    }
    let Some(NormalExpr::Var(root)) =
        super::super::symbolic_values::normal_expr(typed, None, &Default::default(), returned)
    else {
        return proof;
    };
    if !proof.types.contains_key(&root) {
        return proof;
    }
    struct ResultRenamer {
        root: Intern<String>,
        prefix: String,
    }

    impl ProofVisitorMut for ResultRenamer {
        fn visit_term(&mut self, term: &mut ProofTerm) -> ControlFlow<()> {
            if let ProofTerm::Name(name) = term {
                if *name == self.root {
                    *name = Intern::from_ref("$result");
                } else if let Some(suffix) = name.as_str().strip_prefix(&self.prefix) {
                    *name = Intern::new(format!("$result.{suffix}"));
                }
            }
            walk_term(self, term)
        }
    }

    let _ = ResultRenamer {
        root,
        prefix: format!("{root}."),
    }
    .visit_proposition(&mut proof.proposition);
    proof.types.remove(&root);
    proof
        .types
        .insert(Intern::from_ref("$result"), signature.return_type.clone());
    proof
}
