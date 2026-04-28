use diagnostic::SpanId;

#[derive(Debug, Clone)]
pub struct ImportEdge {
    pub to: usize,
    /// Span in the *from* file for the `use ...` that introduced this edge.
    pub import_span: SpanId,
}

#[derive(Debug, Clone)]
pub struct ImportCycle {
    /// Node indices forming a closed cycle, e.g. `[a, b, c, a]`.
    pub nodes: Vec<usize>,
    /// The edge (from → to) that closed the cycle, used for diagnostic span.
    pub closing_from: usize,
    pub closing_span: SpanId,
}

/// Detect the first import cycle in a directed graph.
///
/// `adj[u]` contains outgoing edges from `u`.
//
// TODO: Report *all* import cycles (not just the first) so users with multiple independent
// cycles get a complete set of diagnostics in one compile.
pub fn detect_first_cycle(adj: &[Vec<ImportEdge>]) -> Option<ImportCycle> {
    #[derive(Copy, Clone, PartialEq, Eq)]
    enum Mark {
        Unvisited,
        Visiting,
        Visited,
    }

    let n = adj.len();
    let mut mark = vec![Mark::Unvisited; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut stack_pos: Vec<Option<usize>> = vec![None; n];

    fn dfs(
        u: usize,
        adj: &[Vec<ImportEdge>],
        mark: &mut [Mark],
        stack: &mut Vec<usize>,
        stack_pos: &mut [Option<usize>],
    ) -> Option<ImportCycle> {
        mark[u] = Mark::Visiting;
        stack_pos[u] = Some(stack.len());
        stack.push(u);

        for e in &adj[u] {
            let v = e.to;
            match mark[v] {
                Mark::Unvisited => {
                    if let Some(c) = dfs(v, adj, mark, stack, stack_pos) {
                        return Some(c);
                    }
                }
                Mark::Visiting => {
                    // Back-edge: u -> v closes a cycle. Reconstruct from v..end, then add v.
                    let start = stack_pos[v].unwrap_or(0);
                    let mut nodes = stack[start..].to_vec();
                    nodes.push(v);
                    return Some(ImportCycle {
                        nodes,
                        closing_from: u,
                        closing_span: e.import_span,
                    });
                }
                Mark::Visited => {}
            }
        }

        stack.pop();
        stack_pos[u] = None;
        mark[u] = Mark::Visited;
        None
    }

    for u in 0..n {
        if mark[u] == Mark::Unvisited {
            if let Some(c) = dfs(u, adj, &mut mark, &mut stack, &mut stack_pos) {
                return Some(c);
            }
        }
    }
    None
}
