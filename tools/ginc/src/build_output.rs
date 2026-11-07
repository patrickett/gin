use crate::cli::Args;
use crate::compile::CompileResult;
use flask::FlaskConfig;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(crate) struct CompilationTimings {
    enabled: bool,
    total_start: std::time::Instant,
    phases: Vec<(&'static str, std::time::Duration)>,
    json_path: Option<PathBuf>,
    pub(crate) object_cache_hit: Option<bool>,
}

impl CompilationTimings {
    pub(crate) fn new(enabled: bool, json_path: Option<PathBuf>) -> Self {
        Self {
            enabled,
            total_start: std::time::Instant::now(),
            phases: Vec::new(),
            json_path,
            object_cache_hit: None,
        }
    }

    pub(crate) fn enabled() -> bool {
        crate::compile::env_flag_enabled("GINC_TIMINGS")
    }

    pub(crate) fn run<T>(&mut self, name: &'static str, action: impl FnOnce() -> T) -> T {
        let phase = match name {
            "discover-sources" => compiler_profile::Phase::Discover,
            "read-sources" => compiler_profile::Phase::Read,
            "parse" => compiler_profile::Phase::Parse,
            "front-end" => compiler_profile::Phase::FrontEnd,
            "typecheck-diagnostics" => compiler_profile::Phase::Diagnostics,
            "mlir-lowering" => compiler_profile::Phase::Mlir,
            "artifact-key" | "toolchain-identity" => compiler_profile::Phase::ArtifactKey,
            "object-cache-lookup" => compiler_profile::Phase::ObjectCache,
            "linking" | "link" => compiler_profile::Phase::Linking,
            "load" => compiler_profile::Phase::Read,
            "check" => compiler_profile::Phase::FrontEnd,
            "lower" => compiler_profile::Phase::Mlir,
            "object" => compiler_profile::Phase::NativeEmission,
            _ => compiler_profile::Phase::Other,
        };
        let _profile = compiler_profile::span(phase);
        if !self.enabled && self.json_path.is_none() {
            return action();
        }
        let start = std::time::Instant::now();
        let value = action();
        if self.enabled || self.json_path.is_some() {
            self.phases.push((name, start.elapsed()));
        }
        value
    }

    fn report(&self) {
        let elapsed = self.total_start.elapsed();
        if let Some(path) = &self.json_path {
            let phases: Vec<_> = self.phases.iter().map(|(name, duration)| {
                serde_json::json!({"name": name, "seconds": duration.as_secs_f64()})
            }).collect();
            let report = serde_json::json!({
                "schema_version": 1,
                "total_seconds": elapsed.as_secs_f64(),
                "object_cache_hit": self.object_cache_hit,
                "phases": phases,
            });
            if let Err(error) = std::fs::write(path, report.to_string()) {
                eprintln!("could not write timings to {}: {error}", path.display());
            }
        }
        if !self.enabled {
            return;
        }

        crate::build_output::timings(&self.phases, elapsed, self.object_cache_hit);
    }

    pub(crate) fn record(&mut self, name: &'static str, duration: std::time::Duration) {
        if self.enabled || self.json_path.is_some() {
            self.phases.push((name, duration));
        }
    }
}

impl Drop for CompilationTimings {
    fn drop(&mut self) {
        self.report();
    }
}

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
        if let CompileResult::Failed(_) = result {
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
