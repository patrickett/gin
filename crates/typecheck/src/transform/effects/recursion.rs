use super::*;

pub(super) fn recursive_calls<'a>(
    files: impl IntoIterator<Item = &'a TypedFileAst>,
) -> HashMap<DefId, HashSet<DefId>> {
    let mut calls = HashMap::new();
    for typed in files {
        for (id, bind) in &typed.defs {
            let mut targets = HashSet::new();
            let mut pending = bind_body_exprs(bind, typed);
            let mut visited = HashSet::new();
            while let Some(expression) = pending.pop() {
                if !visited.insert(expression) {
                    continue;
                }
                let Some(kind) = typed.exprs.kind_of(expression) else {
                    continue;
                };
                if let TypedExprKind::FnCall { target, args, .. } = kind
                    && (args.is_some() || typed.exprs.place_of(expression).flatten().is_none())
                {
                    targets.insert(*target);
                }
                let _: ControlFlow<()> = walk_expr_children(kind, &mut |child| {
                    pending.push(child);
                    ControlFlow::Continue(())
                });
            }
            calls.insert(*id, targets);
        }
    }
    let ids = calls.keys().copied().collect::<Vec<_>>();
    let indices = ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect::<HashMap<_, _>>();
    let edges = ids
        .iter()
        .map(|id| {
            calls[id]
                .iter()
                .filter_map(|target| indices.get(target).copied())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut reverse = vec![Vec::new(); ids.len()];
    for (source, targets) in edges.iter().enumerate() {
        for target in targets {
            reverse[*target].push(source);
        }
    }
    let mut visited = vec![false; ids.len()];
    let mut order = Vec::with_capacity(ids.len());
    for start in 0..ids.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, edge)) = stack.last_mut() {
            if *edge < edges[*node].len() {
                let target = edges[*node][*edge];
                *edge += 1;
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
    let mut components = vec![None; ids.len()];
    for start in order.into_iter().rev() {
        if components[start].is_some() {
            continue;
        }
        components[start] = Some(start);
        let mut pending = vec![start];
        while let Some(node) = pending.pop() {
            for predecessor in &reverse[node] {
                if components[*predecessor].is_none() {
                    components[*predecessor] = Some(start);
                    pending.push(*predecessor);
                }
            }
        }
    }
    ids.iter()
        .enumerate()
        .filter_map(|(source, id)| {
            let targets = edges[source]
                .iter()
                .filter(|target| components[source] == components[**target])
                .map(|target| ids[*target])
                .collect::<HashSet<_>>();
            (!targets.is_empty()).then_some((*id, targets))
        })
        .collect()
}
