use std::hint::black_box;
use std::mem::size_of;
use std::path::Path;
use std::time::{Duration, Instant};

use ast::{Bind, Declare, Expr, Typed};
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedExpr, TypedTag};

const SAMPLES: usize = 30;
const WARMUPS: usize = 3;

struct Workload {
    name: &'static str,
    source: String,
}

fn expression_heavy_source() -> String {
    let mut source = String::from("#default(IntegerLiteral)\nInt is in 0...10000\nvalues := [");
    for value in 0..500 {
        if value != 0 {
            source.push_str(", ");
        }
        source.push_str(&value.to_string());
    }
    source.push_str("]\n");
    source
}

fn bind_heavy_source() -> String {
    let mut source = String::from("#default(IntegerLiteral)\nInt is in 0...10000\n");
    for value in 0..200 {
        source.push_str(&format!("value{value} := {value}\n"));
    }
    for index in 0..100 {
        source.push_str(&format!("identity{index}(x Int) Int := x\n"));
    }
    source
}

fn declaration_heavy_source() -> String {
    let mut source = String::new();
    for index in 0..200 {
        source.push_str(&format!("Range{index} is in {index}...{}\n", index + 1000));
    }
    for index in 0..100 {
        source.push_str(&format!(
            "Choice{index}(x) is Left{index}(x) or Right{index}(x)\n"
        ));
    }
    source
}

fn range_declarations(count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        source.push_str(&format!("Range{index} is in {index}...{}\n", index + 1000));
    }
    source
}

fn union_declarations(count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
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

fn gin_sources(path: &Path, sources: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            gin_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "gin")
            && let Ok(source) = std::fs::read_to_string(path)
        {
            sources.push(source);
        }
    }
}

#[test]
#[ignore = "manual declaration attribute frequency report"]
fn report_declare_attribute_frequency() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sources = Vec::new();
    gin_sources(&workspace.join("modules"), &mut sources);
    gin_sources(&workspace.join("crates"), &mut sources);

    let mut declarations = 0;
    let mut attributed = 0;
    let mut raw_items = 0;
    for source in &sources {
        let parsed = TokenCursor::parse_source(source);
        for declaration in parsed.tags.values() {
            declarations += 1;
            if let Some(items) = &declaration.attributes.raw_attributes {
                attributed += 1;
                raw_items += items.len();
            }
        }
    }

    println!(
        "files={} declarations={} attributed={} default={} raw_items={} declare_B={} attrs_B={} typed_tag_B={}",
        sources.len(),
        declarations,
        attributed,
        declarations - attributed,
        raw_items,
        size_of::<Declare>(),
        size_of::<ast::DeclareAttributes>(),
        size_of::<TypedTag>(),
    );
}

#[test]
#[ignore = "manual declaration stage scaling report"]
fn report_declaration_stage_scaling() {
    println!("shape count stage_declare_p50_us");
    for (shape, generate) in [
        ("range", range_declarations as fn(usize) -> String),
        ("union", union_declarations as fn(usize) -> String),
    ] {
        for count in [25, 50, 100, 200, 400] {
            let parsed = TokenCursor::parse_source(&generate(count));
            let samples = measure(|| {
                black_box(typecheck::transform::stage_declare(
                    black_box(&parsed),
                    FileId(0),
                    black_box(&TransformCtx::new()),
                ));
            });
            println!(
                "{shape:<5} {count:>5} {:>20}",
                percentile(&samples, 1, 2).as_micros()
            );
        }
    }
}

#[test]
#[ignore = "manual AST locality baseline"]
fn report_ast_locality_baseline() {
    let workloads = [
        Workload {
            name: "expression-heavy",
            source: expression_heavy_source(),
        },
        Workload {
            name: "bind-heavy",
            source: bind_heavy_source(),
        },
        Workload {
            name: "declaration-heavy",
            source: declaration_heavy_source(),
        },
    ];

    println!(
        "workload             bytes tags defs roots typed_exprs flaws parse_inline_B typed_inline_B parse_p10_us parse_p50_us parse_p90_us parse_declare_p50_us e2e_p10_us e2e_p50_us e2e_p90_us"
    );

    for workload in workloads {
        let parsed = TokenCursor::parse_source(&workload.source);
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        let typed_exprs = typed.exprs.len();
        let flaws = typed.exprs.flaws.iter().map(Vec::len).sum::<usize>()
            + typed.warnings.len()
            + typed.declaration_flaws.len()
            + typed.parse_warnings.len();
        let parse_inline_bytes = parsed.tags.len() * size_of::<Declare>()
            + parsed.defs.len() * size_of::<Bind>()
            + parsed.exprs.len() * size_of::<Expr>()
            + typed_exprs * size_of::<Typed<Expr>>();
        let typed_inline_bytes = typed_exprs * size_of::<TypedExpr>();

        let parse_samples = measure(|| {
            black_box(TokenCursor::parse_source(black_box(&workload.source)));
        });
        let declare_samples = measure(|| {
            let parsed = TokenCursor::parse_source(black_box(&workload.source));
            black_box(typecheck::transform::stage_declare(
                black_box(&parsed),
                FileId(0),
                black_box(&TransformCtx::new()),
            ));
        });
        let e2e_samples = measure(|| {
            let parsed = TokenCursor::parse_source(black_box(&workload.source));
            black_box(transform(
                black_box(&parsed),
                FileId(0),
                black_box(&TransformCtx::new()),
            ));
        });

        println!(
            "{:<20} {:>5} {:>4} {:>4} {:>5} {:>11} {:>5} {:>14} {:>14} {:>12} {:>12} {:>12} {:>14} {:>10} {:>10} {:>10}",
            workload.name,
            workload.source.len(),
            parsed.tags.len(),
            parsed.defs.len(),
            parsed.exprs.len(),
            typed_exprs,
            flaws,
            parse_inline_bytes,
            typed_inline_bytes,
            percentile(&parse_samples, 1, 10).as_micros(),
            percentile(&parse_samples, 1, 2).as_micros(),
            percentile(&parse_samples, 9, 10).as_micros(),
            percentile(&declare_samples, 1, 2).as_micros(),
            percentile(&e2e_samples, 1, 10).as_micros(),
            percentile(&e2e_samples, 1, 2).as_micros(),
            percentile(&e2e_samples, 9, 10).as_micros(),
        );
    }
}
