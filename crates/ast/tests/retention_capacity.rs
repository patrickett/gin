use std::fs;
use std::hint::black_box;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ast::{Bind, Declare, Expr, FileAst};
use indexmap::IndexMap;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, stage_declare, transform, transform_finish};
use typecheck::{FileId, TypedExprKind};

const SAMPLES: usize = 30;
const WARMUPS: usize = 3;

fn representative_source() -> String {
    let mut source = String::from("#default(IntegerLiteral)\nInt is in 0...10000\n");
    for index in 0..200 {
        source.push_str(&format!(
            "value{index} := [{index}, {}, {}]\n",
            index + 1,
            index + 2
        ));
    }
    for index in 0..100 {
        source.push_str(&format!("identity{index}(x Int) Int := x\n"));
    }
    for index in 0..100 {
        source.push_str(&format!(
            "Choice{index}(x) is Left{index}(x) or Right{index}(x)\n"
        ));
    }
    source
}

fn percentile(samples: &[Duration], numerator: usize, denominator: usize) -> Duration {
    samples[(samples.len() - 1) * numerator / denominator]
}

fn measure(mut operation: impl FnMut()) -> Vec<Duration> {
    for _ in 0..WARMUPS {
        operation();
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        operation();
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    samples
}

fn slot_slack(ast: &FileAst) -> usize {
    ast.uses.capacity().saturating_sub(ast.uses.len())
        + ast.tags.capacity().saturating_sub(ast.tags.len())
        + ast.defs.capacity().saturating_sub(ast.defs.len())
        + ast
            .method_binds
            .capacity()
            .saturating_sub(ast.method_binds.len())
        + ast.exprs.capacity().saturating_sub(ast.exprs.len())
        + ast
            .symbol_aliases
            .capacity()
            .saturating_sub(ast.symbol_aliases.len())
        + ast
            .symbol_alias_spans
            .capacity()
            .saturating_sub(ast.symbol_alias_spans.len())
        + ast
            .parse_warnings
            .capacity()
            .saturating_sub(ast.parse_warnings.len())
}

fn gin_files(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            gin_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "gin") {
            files.push(path);
        }
    }
}

#[test]
#[ignore = "manual repository corpus Bind occupancy baseline"]
fn report_repository_bind_occupancy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
    let mut files = Vec::new();
    gin_files(&root, &mut files);
    files.sort();

    let mut file_count = 0;
    let mut bind_count = 0;
    let mut nonempty_counts = [0usize; 10];
    let mut total_slot_slack = 0;
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        let parsed = TokenCursor::parse_source(&source);
        file_count += 1;
        total_slot_slack += slot_slack(&parsed);
        for bind in parsed.defs.values().chain(&parsed.method_binds) {
            bind_count += 1;
            let occupied = [
                bind.params.as_ref().is_some_and(|value| !value.is_empty()),
                !bind.param_slots.is_empty(),
                !bind.param_conventions.is_empty(),
                !bind.group_params.is_empty(),
                !bind.param_groups.is_empty(),
                !bind.param_refinements.is_empty(),
                !bind.receiver_typevars.is_empty(),
                bind.attributes != Default::default(),
                bind.return_type != ast::TyState::Infer,
                bind.receiver_type.is_some(),
            ];
            for (count, occupied) in nonempty_counts.iter_mut().zip(occupied) {
                *count += usize::from(occupied);
            }
        }
    }

    println!(
        "files={} binds={} top_level_slot_slack={}",
        file_count, bind_count, total_slot_slack
    );
    println!(
        "bind_nonempty params={} param_slots={} conventions={} group_params={} param_groups={} refinements={} receiver_typevars={} attributes={} return_state={} receiver={}",
        nonempty_counts[0],
        nonempty_counts[1],
        nonempty_counts[2],
        nonempty_counts[3],
        nonempty_counts[4],
        nonempty_counts[5],
        nonempty_counts[6],
        nonempty_counts[7],
        nonempty_counts[8],
        nonempty_counts[9],
    );
}

#[test]
#[ignore = "manual parse-AST retention and collection-capacity baseline"]
fn report_retention_and_capacity() {
    let source = representative_source();
    let parsed = TokenCursor::parse_source(&source);
    let binds: Vec<&Bind> = parsed.defs.values().chain(&parsed.method_binds).collect();
    let nonempty =
        |predicate: fn(&Bind) -> bool| binds.iter().filter(|bind| predicate(bind)).count();

    println!(
        "source_bytes={} binds={} tags={}",
        source.len(),
        binds.len(),
        parsed.tags.len()
    );
    println!(
        "bind_nonempty params={} param_slots={} conventions={} group_params={} param_groups={} refinements={} receiver_typevars={} attributes={} return_state={}",
        nonempty(|bind| bind.params.as_ref().is_some_and(|value| !value.is_empty())),
        nonempty(|bind| !bind.param_slots.is_empty()),
        nonempty(|bind| !bind.param_conventions.is_empty()),
        nonempty(|bind| !bind.group_params.is_empty()),
        nonempty(|bind| !bind.param_groups.is_empty()),
        nonempty(|bind| !bind.param_refinements.is_empty()),
        nonempty(|bind| !bind.receiver_typevars.is_empty()),
        nonempty(|bind| bind.attributes != Default::default()),
        nonempty(|bind| bind.return_type != ast::TyState::Infer),
    );
    println!(
        "top_level_slot_slack={} top_level_inline_bytes={} bind_inline_bytes={} declare_inline_bytes={} param_slots_inline_bytes={} receiver_typevars_inline_bytes={} return_state_inline_bytes={}",
        slot_slack(&parsed),
        parsed.defs.len() * size_of::<Bind>()
            + parsed.tags.len() * size_of::<Declare>()
            + parsed.exprs.len() * size_of::<Expr>(),
        size_of::<Bind>(),
        size_of::<Declare>(),
        size_of::<IndexMap<Intern<String>, ast::ParamSlot>>(),
        size_of::<std::collections::HashMap<Intern<String>, ast::TyState>>(),
        size_of::<ast::TyState>(),
    );

    let typed = transform(&parsed, FileId(0), &TransformCtx::new());
    let arena_capacity = typed.exprs.kind.capacity();
    let arena_slack = arena_capacity.saturating_sub(typed.exprs.len());
    let child_vec_slack: usize = typed
        .exprs
        .kind
        .iter()
        .map(|kind| match kind {
            TypedExprKind::List(items) | TypedExprKind::TupleLit(items) => {
                items.capacity().saturating_sub(items.len())
            }
            _ => 0,
        })
        .sum();
    println!(
        "typed_exprs={} arena_capacity={} arena_slack={} child_expr_id_slack={}",
        typed.exprs.len(),
        arena_capacity,
        arena_slack,
        child_vec_slack,
    );

    let clone_samples = measure(|| {
        black_box(black_box(&parsed).clone());
    });
    let transform_samples = measure(|| {
        black_box(transform(
            black_box(&parsed),
            FileId(0),
            black_box(&TransformCtx::new()),
        ));
    });
    let no_clone_samples = measure(|| {
        let ctx = TransformCtx::new();
        let mut typed = stage_declare(black_box(&parsed), FileId(0), black_box(&ctx));
        transform_finish(&mut typed, black_box(&parsed), black_box(&ctx));
        black_box(typed);
    });
    println!(
        "clone_p50_us={} transform_p50_us={} staged_without_clone_p50_us={}",
        percentile(&clone_samples, 1, 2).as_micros(),
        percentile(&transform_samples, 1, 2).as_micros(),
        percentile(&no_clone_samples, 1, 2).as_micros(),
    );
}
