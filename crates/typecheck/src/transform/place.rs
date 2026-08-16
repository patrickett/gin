use ast::integer::{CanonicalIntegerExpr, IntegerDomain, IntegerKnowledge, IntegerValidity};
use diagnostic::Diagnostic;

use crate::ty::Ty;
use crate::typed::{
    PlaceId, PlaceVersionComponent, PlaceVersionId, PlaceVersionOrigin, TypedFileAst,
};

fn knowledge_domain(knowledge: &IntegerKnowledge) -> Option<IntegerDomain> {
    match knowledge {
        IntegerKnowledge::Exact(CanonicalIntegerExpr::Value(value)) => {
            IntegerDomain::bounded(*value, *value)
        }
        IntegerKnowledge::Domain(domain) => Some(domain.clone()),
        IntegerKnowledge::Exact(CanonicalIntegerExpr::Symbolic(_))
        | IntegerKnowledge::Unknown
        | IntegerKnowledge::Poison => None,
    }
}

fn version_predecessors(origin: &PlaceVersionOrigin) -> Vec<PlaceVersionId> {
    match origin {
        PlaceVersionOrigin::Parameter
        | PlaceVersionOrigin::Declared
        | PlaceVersionOrigin::Initializer(_) => Vec::new(),
        PlaceVersionOrigin::Projection { base } => vec![*base],
        PlaceVersionOrigin::Rebind { predecessor, .. } => predecessor.iter().copied().collect(),
        PlaceVersionOrigin::Join(predecessors) => predecessors.clone(),
        PlaceVersionOrigin::LoopPhi {
            incoming,
            backedges,
        } => std::iter::once(*incoming)
            .chain(backedges.iter().copied())
            .collect(),
    }
}

fn place_version_components(typed: &TypedFileAst) -> Vec<PlaceVersionComponent> {
    let count = typed.place_versions.len();
    let edges = typed
        .place_versions
        .iter()
        .map(|version| {
            version_predecessors(&version.origin)
                .into_iter()
                .map(|version| version.0 as usize)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut reverse = vec![Vec::new(); count];
    for (source, targets) in edges.iter().enumerate() {
        for target in targets {
            reverse[*target].push(source);
        }
    }
    let mut visited = vec![false; count];
    let mut order = Vec::with_capacity(count);
    for start in 0..count {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, edge_index)) = stack.last_mut() {
            if *edge_index < edges[*node].len() {
                let target = edges[*node][*edge_index];
                *edge_index += 1;
                if !visited[target] {
                    visited[target] = true;
                    stack.push((target, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }
    visited.fill(false);
    let mut components = Vec::new();
    for start in order.into_iter().rev() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut pending = vec![start];
        let mut versions = Vec::new();
        while let Some(node) = pending.pop() {
            versions.push(PlaceVersionId(node as u32));
            for predecessor in &reverse[node] {
                if !visited[*predecessor] {
                    visited[*predecessor] = true;
                    pending.push(*predecessor);
                }
            }
        }
        versions.sort_by_key(|version| version.0);
        let cyclic =
            versions.len() > 1 || edges[versions[0].0 as usize].contains(&(versions[0].0 as usize));
        components.push(PlaceVersionComponent { versions, cyclic });
    }
    components.sort_by_key(|component| component.versions[0].0);
    components
}

fn cyclic_write_expression(
    typed: &TypedFileAst,
    component: &PlaceVersionComponent,
) -> Option<crate::ExprId> {
    component.versions.iter().find_map(|version| {
        match &typed.place_versions[version.0 as usize].origin {
            PlaceVersionOrigin::Rebind { value, .. } => Some(*value),
            _ => None,
        }
    })
}

pub(crate) fn stage_freeze_place_contracts(typed: &mut TypedFileAst) {
    typed.place_version_components = place_version_components(typed);
    for place_index in 0..typed.places.len() {
        let place = PlaceId(place_index as u32);
        if typed.places[place_index].explicit_contract
            || !matches!(typed.places[place_index].ty, Ty::AnonymousInteger { .. })
        {
            continue;
        }
        let cyclic_component = typed.place_version_components.iter().find(|component| {
            component.cyclic
                && component
                    .versions
                    .iter()
                    .any(|version| typed.place_versions[version.0 as usize].place == place)
        });
        if let Some(write) =
            cyclic_component.and_then(|component| cyclic_write_expression(typed, component))
        {
            let name = typed.places[place_index].name;
            typed.exprs.flaws[write.as_usize()].push(
                Diagnostic::new(
                    "type-cyclic-place-requires-explicit-contract",
                    format!(
                        "loop-carried place `{}` requires an explicit finite contract",
                        name.as_str()
                    ),
                )
                .with_arg("name", name.as_str().to_string()),
            );
            continue;
        }
        let domains = typed
            .place_versions
            .iter()
            .filter(|version| version.place == place)
            .filter_map(|version| version.integer_knowledge.as_ref())
            .filter_map(knowledge_domain)
            .collect::<Vec<_>>();
        if domains.is_empty() {
            continue;
        }
        let contract = Ty::AnonymousInteger {
            validity: IntegerValidity::new(IntegerDomain::union(domains)),
        };
        typed.places[place_index].ty = contract.clone();
        for version in typed
            .place_versions
            .iter_mut()
            .filter(|version| version.place == place)
        {
            version.ty = contract.clone();
        }
        for (expr_index, resolved_place) in typed.exprs.place.iter().enumerate() {
            if *resolved_place == Some(place)
                && matches!(typed.exprs.ty[expr_index], Ty::AnonymousInteger { .. })
            {
                typed.exprs.ty[expr_index] = contract.clone();
            }
        }
    }
}
