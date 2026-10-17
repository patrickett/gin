use crate::cli::Args;
use crate::compile::{CompileFailure, CompileResult};
use flask::FlaskConfig;
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

pub(crate) struct BuildOutput {
    start: Instant,
    quiet: bool,
}

impl BuildOutput {
    pub(crate) fn new(args: &Args) -> Self {
        let output = Self {
            start: Instant::now(),
            quiet: args.quiet || args.timings_json.is_some(),
        };
        if !output.quiet {
            let package = FlaskConfig::find_package_config(&args.input)
                .map(|(config, root)| {
                    format!("{} v{} ({})", config.name, config.version, root.display())
                })
                .unwrap_or_else(|| args.input.display().to_string());
            status("Compiling", &package, false);
        }
        output
    }

    pub(crate) fn finish(self, result: &CompileResult, args: &Args) {
        if let CompileResult::Failed(error) = result {
            eprintln!("error: {error}");
            if let CompileFailure::EmissionFailed { diagnostics, .. } = error
                && diagnostics.is_empty()
            {
                eprintln!("error: no diagnostics were produced");
            }
            if !self.quiet {
                status(
                    "Failed",
                    &format!("after {:.2}s", self.start.elapsed().as_secs_f64()),
                    true,
                );
            }
            return;
        }
        if self.quiet {
            return;
        }
        let optimization = match args.profile {
            crate::cli::Profile::Debug => "unoptimized",
            crate::cli::Profile::Release
                if args.fast_llvm || crate::compile::GinCompiler::fast_llvm_enabled() =>
            {
                "fast LLVM"
            }
            crate::cli::Profile::Release => "optimized",
        };
        status(
            "Finished",
            &format!(
                "{} [{optimization}] in {:.2}s",
                args.profile,
                self.start.elapsed().as_secs_f64()
            ),
            false,
        );
        if let CompileResult::Success { output: Some(path) } = result {
            status("Artifact", &path.display().to_string(), false);
        }
    }
}

fn status(label: &str, message: &str, failed: bool) {
    if std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        let color = if failed { 31 } else { 32 };
        eprintln!("\x1b[1;{color}m{label:>12}\x1b[0m {message}");
    } else {
        eprintln!("{label:>12} {message}");
    }
}

pub(crate) fn timings(phases: &[(&str, Duration)], total: Duration, cache: Option<bool>) {
    let _ = write_timings(&mut std::io::stderr().lock(), phases, total, cache);
}

fn write_timings(
    out: &mut impl Write,
    phases: &[(&str, Duration)],
    total: Duration,
    cache: Option<bool>,
) -> std::io::Result<()> {
    writeln!(
        out,
        "     Timings {:.2} ms total",
        total.as_secs_f64() * 1000.0
    )?;
    writeln!(
        out,
        "             {:<24} {:>11} {:>7}",
        "Phase", "Time", "Share"
    )?;
    let measured: Duration = phases.iter().map(|(_, duration)| *duration).sum();
    for (name, duration) in phases
        .iter()
        .copied()
        .chain(std::iter::once(("other", total.saturating_sub(measured))))
    {
        let share = if total.is_zero() {
            0.0
        } else {
            duration.as_secs_f64() / total.as_secs_f64()
        };
        let bar = "█".repeat((share.clamp(0.0, 1.0) * 20.0).round() as usize);
        writeln!(
            out,
            "             {name:<24} {:>8.2} ms {:>6.1}% {bar}",
            duration.as_secs_f64() * 1000.0,
            share * 100.0
        )?;
    }
    if let Some(hit) = cache {
        writeln!(
            out,
            "             Object cache: {}",
            if hit { "hit" } else { "miss or disabled" }
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_table_accounts_for_unmeasured_time_and_cache_hits() {
        let mut output = Vec::new();
        write_timings(
            &mut output,
            &[("front-end", Duration::from_millis(75))],
            Duration::from_millis(100),
            Some(true),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("100.00 ms total"));
        assert!(output.contains("75.0%"));
        assert!(output.contains("25.0%"));
        assert!(output.contains("Object cache: hit"));
    }
}
