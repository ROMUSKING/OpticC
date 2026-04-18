use crate::build::{compile_source_to_object_with_stats, CompilePhaseTimings};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub name: String,
    pub compiler: String,
    pub version: String,
    pub optimization: String,
    pub metrics: BenchmarkMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BenchmarkMetrics {
    pub compile_time_ms: u64,
    pub output_size_bytes: u64,
    pub peak_memory_kb: u64,
    pub preprocess_time_ms: u64,
    pub parse_time_ms: u64,
    pub codegen_time_ms: u64,
    pub optimize_time_ms: u64,
    pub ir_write_time_ms: u64,
    pub llc_time_ms: u64,
    pub correctness: String,
    pub test_total: u64,
    pub test_passed: u64,
    pub test_failed: u64,
}

impl BenchmarkMetrics {
    pub fn new() -> Self {
        BenchmarkMetrics {
            compile_time_ms: 0,
            output_size_bytes: 0,
            peak_memory_kb: 0,
            preprocess_time_ms: 0,
            parse_time_ms: 0,
            codegen_time_ms: 0,
            optimize_time_ms: 0,
            ir_write_time_ms: 0,
            llc_time_ms: 0,
            correctness: "error".to_string(),
            test_total: 0,
            test_passed: 0,
            test_failed: 0,
        }
    }

    pub fn pass(tests: u64) -> Self {
        BenchmarkMetrics {
            compile_time_ms: 0,
            output_size_bytes: 0,
            peak_memory_kb: 0,
            preprocess_time_ms: 0,
            parse_time_ms: 0,
            codegen_time_ms: 0,
            optimize_time_ms: 0,
            ir_write_time_ms: 0,
            llc_time_ms: 0,
            correctness: "pass".to_string(),
            test_total: tests,
            test_passed: tests,
            test_failed: 0,
        }
    }

    pub fn fail(tests: u64, failed: u64) -> Self {
        BenchmarkMetrics {
            compile_time_ms: 0,
            output_size_bytes: 0,
            peak_memory_kb: 0,
            preprocess_time_ms: 0,
            parse_time_ms: 0,
            codegen_time_ms: 0,
            optimize_time_ms: 0,
            ir_write_time_ms: 0,
            llc_time_ms: 0,
            correctness: "fail".to_string(),
            test_total: tests,
            test_passed: tests.saturating_sub(failed),
            test_failed: failed,
        }
    }

    pub fn skipped() -> Self {
        BenchmarkMetrics {
            correctness: "skipped".to_string(),
            ..BenchmarkMetrics::new()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BenchmarkSuite {
    Micro,
    Coreutils,
    Synthetic,
}

impl std::fmt::Display for BenchmarkSuite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchmarkSuite::Micro => write!(f, "micro"),
            BenchmarkSuite::Coreutils => write!(f, "coreutils"),
            BenchmarkSuite::Synthetic => write!(f, "synthetic"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompilerConfig {
    pub name: String,
    pub command: String,
    pub compile_args: Vec<String>,
    pub link_args: Vec<String>,
    pub compile_only: bool,
    pub internal: bool,
}

impl CompilerConfig {
    pub fn new(name: &str, command: &str) -> Self {
        CompilerConfig {
            name: name.to_string(),
            command: command.to_string(),
            compile_args: Vec::new(),
            link_args: Vec::new(),
            compile_only: false,
            internal: false,
        }
    }

    pub fn with_compile_args(mut self, args: Vec<String>) -> Self {
        self.compile_only = args.iter().any(|arg| arg == "-c");
        self.compile_args = args;
        self
    }

    pub fn with_link_args(mut self, args: Vec<String>) -> Self {
        self.link_args = args;
        self
    }

    pub fn with_compile_only(mut self, compile_only: bool) -> Self {
        self.compile_only = compile_only;
        self
    }

    pub fn opticc() -> Self {
        CompilerConfig {
            name: "opticc".to_string(),
            command: "opticc".to_string(),
            compile_args: Vec::new(),
            link_args: Vec::new(),
            compile_only: true,
            internal: true,
        }
    }

    pub fn is_available(&self) -> bool {
        if self.internal {
            return true;
        }
        Command::new(&self.command)
            .arg("--version")
            .output()
            .is_ok()
    }

    pub fn get_version(&self) -> String {
        if self.internal {
            return env!("CARGO_PKG_VERSION").to_string();
        }
        match Command::new(&self.command).arg("--version").output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().next().unwrap_or("unknown").to_string()
            }
            Err(_) => "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CompileMeasurement {
    pub total_ms: u64,
    pub peak_memory_kb: u64,
    pub phase_timings: CompilePhaseTimings,
}

#[derive(Debug)]
pub enum BenchmarkError {
    CompileError(String, String),
    RuntimeError(String),
    CorrectnessError(String, String, String),
    IoError(std::io::Error),
    CompilerNotAvailable(String),
}

impl std::fmt::Display for BenchmarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchmarkError::CompileError(file, msg) => {
                write!(f, "compile error in {}: {}", file, msg)
            }
            BenchmarkError::RuntimeError(msg) => write!(f, "runtime error: {}", msg),
            BenchmarkError::CorrectnessError(bench, compiler, msg) => {
                write!(f, "correctness error in {} ({}): {}", bench, compiler, msg)
            }
            BenchmarkError::IoError(e) => write!(f, "I/O error: {}", e),
            BenchmarkError::CompilerNotAvailable(name) => {
                write!(f, "compiler '{}' not available", name)
            }
        }
    }
}

impl std::error::Error for BenchmarkError {}

impl From<std::io::Error> for BenchmarkError {
    fn from(e: std::io::Error) -> Self {
        BenchmarkError::IoError(e)
    }
}

pub struct BenchmarkRunner {
    pub suites: Vec<BenchmarkSuite>,
    pub compilers: Vec<CompilerConfig>,
    pub optimization_levels: Vec<String>,
    pub results_dir: PathBuf,
    pub runs_per_benchmark: usize,
}

impl BenchmarkRunner {
    pub fn new() -> Self {
        let mut compilers = Vec::new();
        compilers.push(CompilerConfig::opticc());

        let gcc = CompilerConfig::new("gcc", "gcc")
            .with_compile_args(vec!["-c".to_string()])
            .with_link_args(vec![]);
        if gcc.is_available() {
            compilers.push(gcc);
        }

        let clang = CompilerConfig::new("clang", "clang")
            .with_compile_args(vec!["-c".to_string()])
            .with_link_args(vec![]);
        if clang.is_available() {
            compilers.push(clang);
        }

        BenchmarkRunner {
            suites: vec![BenchmarkSuite::Micro],
            compilers,
            optimization_levels: vec!["O0".to_string()],
            results_dir: PathBuf::from("benchmarks/results"),
            runs_per_benchmark: 5,
        }
    }

    pub fn add_suite(&mut self, suite: BenchmarkSuite) {
        if !self.suites.contains(&suite) {
            self.suites.push(suite);
        }
    }

    pub fn add_compiler(&mut self, config: CompilerConfig) {
        if config.is_available() && !self.compilers.iter().any(|c| c.name == config.name) {
            self.compilers.push(config);
        }
    }

    pub fn with_optimization_levels(mut self, levels: Vec<String>) -> Self {
        self.optimization_levels = levels;
        self
    }

    pub fn with_results_dir(mut self, dir: PathBuf) -> Self {
        self.results_dir = dir;
        self
    }

    pub fn with_runs_per_benchmark(mut self, runs: usize) -> Self {
        self.runs_per_benchmark = runs.max(1);
        self
    }

    pub fn run(&self) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
        let mut all_results = Vec::new();

        fs::create_dir_all(&self.results_dir)?;

        for suite in &self.suites {
            let suite_results = match suite {
                BenchmarkSuite::Micro => self.run_micro_benchmarks()?,
                BenchmarkSuite::Coreutils => self.run_coreutils_benchmarks()?,
                BenchmarkSuite::Synthetic => self.run_synthetic_benchmarks()?,
            };
            all_results.extend(suite_results);
        }

        self.save_results(&all_results)?;

        Ok(all_results)
    }

    pub fn run_micro_benchmarks(&self) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
        let mut results = Vec::new();

        let benchmarks = vec![
            ("loop_sum", MICRO_LOOP_SUM),
            ("function_call", MICRO_FUNC_CALL),
            ("arithmetic", MICRO_ARITHMETIC),
            ("pointer_ops", MICRO_POINTER_OPS),
        ];

        for (name, source) in benchmarks {
            for compiler in &self.compilers {
                for opt in &self.optimization_levels {
                    let result = self.run_single_benchmark(name, source, compiler, opt)?;
                    results.push(result);
                }
            }
        }

        Ok(results)
    }

    pub fn run_coreutils_benchmarks(&self) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
        let mut results = Vec::new();

        let programs = vec![
            ("hello", COREUTILS_HELLO),
            ("cat_simple", COREUTILS_CAT),
            ("wc_simple", COREUTILS_WC),
        ];

        for (name, source) in programs {
            for compiler in &self.compilers {
                for opt in &self.optimization_levels {
                    let result = self.run_single_benchmark(name, source, compiler, opt)?;
                    results.push(result);
                }
            }
        }

        Ok(results)
    }

    pub fn run_synthetic_benchmarks(&self) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
        let mut results = Vec::new();

        let sizes = vec![1000, 5000, 10000];

        for size in sizes {
            let source = generate_synthetic_c(size);
            let name = format!("synthetic_{}loc", size);

            for compiler in &self.compilers {
                for opt in &self.optimization_levels {
                    let result = self.run_single_benchmark(&name, &source, compiler, opt)?;
                    results.push(result);
                }
            }
        }

        Ok(results)
    }

    fn run_single_benchmark(
        &self,
        name: &str,
        source: &str,
        compiler: &CompilerConfig,
        optimization: &str,
    ) -> Result<BenchmarkResult, BenchmarkError> {
        let bench_dir = self.results_dir.join(name);
        fs::create_dir_all(&bench_dir)?;

        let src_path = bench_dir.join("test.c");
        fs::write(&src_path, source)?;

        let opt_flag = format!("-{}", optimization);
        let out_path = bench_dir.join(format!("test_{}_{}", compiler.name, optimization));

        let mut measurements = Vec::new();
        let mut compile_success = false;

        for _ in 0..self.runs_per_benchmark {
            let elapsed = self.measure_compile_time(compiler, &src_path, &out_path, &opt_flag);
            if let Ok(ms) = elapsed {
                measurements.push(ms);
                compile_success = true;
            } else {
                break;
            }
        }

        if !compile_success || measurements.is_empty() {
            return Ok(BenchmarkResult {
                name: name.to_string(),
                compiler: compiler.name.clone(),
                version: compiler.get_version(),
                optimization: optimization.to_string(),
                metrics: BenchmarkMetrics {
                    correctness: "error".to_string(),
                    ..BenchmarkMetrics::new()
                },
            });
        }

        let avg_compile_time =
            measurements.iter().map(|m| m.total_ms).sum::<u64>() / measurements.len() as u64;

        let output_size = self.measure_output_size(&out_path).unwrap_or(0);

        let measurement = average_measurements(&measurements);
        let correctness = self.measure_correctness(&out_path, compiler, name);

        let metrics = BenchmarkMetrics {
            compile_time_ms: avg_compile_time,
            output_size_bytes: output_size,
            peak_memory_kb: measurement.peak_memory_kb,
            preprocess_time_ms: measurement.phase_timings.preprocess_ms,
            parse_time_ms: measurement.phase_timings.parse_ms,
            codegen_time_ms: measurement.phase_timings.codegen_ms,
            optimize_time_ms: measurement.phase_timings.optimize_ms,
            ir_write_time_ms: measurement.phase_timings.ir_write_ms,
            llc_time_ms: measurement.phase_timings.llc_ms,
            correctness: correctness.correctness.clone(),
            test_total: correctness.test_total,
            test_passed: correctness.test_passed,
            test_failed: correctness.test_failed,
        };

        Ok(BenchmarkResult {
            name: name.to_string(),
            compiler: compiler.name.clone(),
            version: compiler.get_version(),
            optimization: optimization.to_string(),
            metrics,
        })
    }

    pub fn measure_compile_time(
        &self,
        compiler: &CompilerConfig,
        src_path: &Path,
        out_path: &Path,
        opt_flag: &str,
    ) -> Result<CompileMeasurement, BenchmarkError> {
        if compiler.internal {
            let opt_level = opt_flag
                .trim_start_matches("-O")
                .parse::<u32>()
                .unwrap_or(0);
            let timings = compile_source_to_object_with_stats(
                src_path,
                out_path,
                opt_level,
                &[],
                &HashMap::new(),
            )
            .map_err(|e| BenchmarkError::CompileError(src_path.display().to_string(), e.to_string()))?;

            return Ok(CompileMeasurement {
                total_ms: timings.total_ms(),
                peak_memory_kb: measure_peak_memory().unwrap_or(0),
                phase_timings: timings,
            });
        }

        let start = Instant::now();

        let mut cmd = Command::new(&compiler.command);
        cmd.arg(opt_flag).arg("-o").arg(out_path).arg(src_path);

        for arg in &compiler.compile_args {
            cmd.arg(arg);
        }

        let output = cmd.output().map_err(BenchmarkError::IoError)?;

        let elapsed = start.elapsed().as_millis() as u64;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BenchmarkError::CompileError(
                src_path.display().to_string(),
                stderr.to_string(),
            ));
        }

        Ok(CompileMeasurement {
            total_ms: elapsed,
            peak_memory_kb: measure_peak_memory().unwrap_or(0),
            phase_timings: CompilePhaseTimings::default(),
        })
    }

    pub fn measure_output_size(&self, out_path: &Path) -> Result<u64, BenchmarkError> {
        let metadata = fs::metadata(out_path)?;
        Ok(metadata.len())
    }

    pub fn measure_correctness(
        &self,
        binary_path: &Path,
        compiler: &CompilerConfig,
        _name: &str,
    ) -> BenchmarkMetrics {
        if compiler.compile_only {
            return BenchmarkMetrics::skipped();
        }
        if !binary_path.exists() {
            return BenchmarkMetrics::fail(1, 1);
        }

        match Command::new(binary_path).output() {
            Ok(output) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if stdout.trim().is_empty() {
                        BenchmarkMetrics::pass(1)
                    } else {
                        BenchmarkMetrics::pass(1)
                    }
                } else {
                    BenchmarkMetrics::fail(1, 1)
                }
            }
            Err(_) => BenchmarkMetrics::fail(1, 1),
        }
    }

    fn save_results(&self, results: &[BenchmarkResult]) -> Result<(), BenchmarkError> {
        let json = generate_json_report(results).map_err(|e| BenchmarkError::RuntimeError(e.to_string()))?;
        let results_path = self.results_dir.join("results.json");
        fs::write(&results_path, json)?;
        Ok(())
    }

    pub fn generate_markdown_report(&self) -> Result<String, BenchmarkError> {
        let results = self.run()?;
        Ok(generate_markdown_report(&results))
    }

    pub fn generate_json_report(&self) -> Result<String, BenchmarkError> {
        let results = self.run()?;
        generate_json_report(&results).map_err(|e| BenchmarkError::RuntimeError(e.to_string()))
    }
}

impl Default for BenchmarkRunner {
    fn default() -> Self {
        Self::new()
    }
}

pub fn generate_markdown_report(results: &[BenchmarkResult]) -> String {
    let mut md = String::new();
    md.push_str("# OpticC Benchmark Report\n\n");
    md.push_str("## Summary\n\n");

    if results.is_empty() {
        md.push_str("No benchmark results available.\n");
        return md;
    }

    let _compilers: Vec<&str> = results
        .iter()
        .map(|r| r.compiler.as_str())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let benchmarks: Vec<&str> = results
        .iter()
        .map(|r| r.name.as_str())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    md.push_str("| Benchmark | Compiler | Optimization | Compile Time (ms) | Output Size (B) | Correctness |\n");
    md.push_str("|-----------|----------|--------------|-------------------|-----------------|-------------|\n");

    for result in results {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            result.name,
            result.compiler,
            result.optimization,
            result.metrics.compile_time_ms,
            result.metrics.output_size_bytes,
            result.metrics.correctness,
        ));
    }

    md.push_str("\n## Compiler Comparison\n\n");

    for bench in &benchmarks {
        md.push_str(&format!("### {}\n\n", bench));
        let bench_results: Vec<&BenchmarkResult> =
            results.iter().filter(|r| r.name == *bench).collect();

        if bench_results.len() >= 2 {
            let baseline = bench_results.iter().find(|r| r.compiler == "gcc");
            if let Some(base) = baseline {
                for r in &bench_results {
                    if r.compiler != base.compiler && base.metrics.compile_time_ms > 0 {
                        let ratio = r.metrics.compile_time_ms as f64
                            / base.metrics.compile_time_ms as f64;
                        md.push_str(&format!(
                            "- {} vs {}: {:.2}x compile time\n",
                            r.compiler, base.compiler, ratio
                        ));
                    }
                }
            }
        }
    }

    md.push_str("\n## Statistics\n\n");

    let total = results.len();
    let passed = results.iter().filter(|r| r.metrics.correctness == "pass").count();
    let failed = results.iter().filter(|r| r.metrics.correctness == "fail").count();
    let errors = results.iter().filter(|r| r.metrics.correctness == "error").count();
    let skipped = results.iter().filter(|r| r.metrics.correctness == "skipped").count();

    md.push_str(&format!("- Total benchmarks: {}\n", total));
    md.push_str(&format!("- Passed: {}\n", passed));
    md.push_str(&format!("- Failed: {}\n", failed));
    md.push_str(&format!("- Errors: {}\n", errors));
    md.push_str(&format!("- Skipped correctness checks: {}\n", skipped));

    let opticc_results: Vec<&BenchmarkResult> =
        results.iter().filter(|r| r.compiler == "opticc").collect();
    if !opticc_results.is_empty() {
        md.push_str("\n## OpticC Phase Breakdown\n\n");
        md.push_str("| Benchmark | Optimization | Preprocess | Parse | Codegen | Optimize | IR Write | llc |\n");
        md.push_str("|-----------|--------------|------------|-------|---------|----------|----------|-----|\n");

        for result in opticc_results {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                result.name,
                result.optimization,
                result.metrics.preprocess_time_ms,
                result.metrics.parse_time_ms,
                result.metrics.codegen_time_ms,
                result.metrics.optimize_time_ms,
                result.metrics.ir_write_time_ms,
                result.metrics.llc_time_ms,
            ));
        }
    }

    md
}

pub fn generate_json_report(results: &[BenchmarkResult]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(results)
}

pub fn calculate_averages(results: &[BenchmarkResult]) -> Vec<BenchmarkResult> {
    let mut grouped: std::collections::HashMap<(String, String, String), Vec<&BenchmarkResult>> =
        std::collections::HashMap::new();

    for r in results {
        grouped
            .entry((r.name.clone(), r.compiler.clone(), r.optimization.clone()))
            .or_default()
            .push(r);
    }

    let mut averages = Vec::new();
    for ((name, compiler, optimization), group) in grouped {
        if group.is_empty() {
            continue;
        }
        let avg_compile =
            group.iter().map(|r| r.metrics.compile_time_ms).sum::<u64>() / group.len() as u64;
        let avg_output =
            group.iter().map(|r| r.metrics.output_size_bytes).sum::<u64>() / group.len() as u64;
        let avg_memory =
            group.iter().map(|r| r.metrics.peak_memory_kb).sum::<u64>() / group.len() as u64;

        let first = group[0];
        averages.push(BenchmarkResult {
            name,
            compiler,
            optimization,
            version: first.version.clone(),
            metrics: BenchmarkMetrics {
                compile_time_ms: avg_compile,
                output_size_bytes: avg_output,
                peak_memory_kb: avg_memory,
                preprocess_time_ms: group.iter().map(|r| r.metrics.preprocess_time_ms).sum::<u64>()
                    / group.len() as u64,
                parse_time_ms: group.iter().map(|r| r.metrics.parse_time_ms).sum::<u64>()
                    / group.len() as u64,
                codegen_time_ms: group.iter().map(|r| r.metrics.codegen_time_ms).sum::<u64>()
                    / group.len() as u64,
                optimize_time_ms: group.iter().map(|r| r.metrics.optimize_time_ms).sum::<u64>()
                    / group.len() as u64,
                ir_write_time_ms: group.iter().map(|r| r.metrics.ir_write_time_ms).sum::<u64>()
                    / group.len() as u64,
                llc_time_ms: group.iter().map(|r| r.metrics.llc_time_ms).sum::<u64>()
                    / group.len() as u64,
                correctness: first.metrics.correctness.clone(),
                test_total: first.metrics.test_total,
                test_passed: first.metrics.test_passed,
                test_failed: first.metrics.test_failed,
            },
        });
    }

    averages
}

pub fn generate_comparison_table(results: &[BenchmarkResult]) -> String {
    let mut table = String::new();
    table.push_str("Benchmark | Compiler | Compile Time (ms) | Output Size (B) | Correctness\n");
    table.push_str("--- | --- | --- | --- | ---\n");

    for r in results {
        table.push_str(&format!(
            "{} | {} | {} | {} | {}\n",
            r.name,
            r.compiler,
            r.metrics.compile_time_ms,
            r.metrics.output_size_bytes,
            r.metrics.correctness,
        ));
    }

    table
}

fn measure_peak_memory() -> Result<u64, BenchmarkError> {
    let proc_status = Path::new("/proc/self/status");
    if proc_status.exists() {
        let content = fs::read_to_string(proc_status)?;
        for line in content.lines() {
            if line.starts_with("VmPeak:") || line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(value) = parts[1].parse::<u64>() {
                        return Ok(value);
                    }
                }
            }
        }
    }
    Ok(0)
}

fn average_measurements(measurements: &[CompileMeasurement]) -> CompileMeasurement {
    if measurements.is_empty() {
        return CompileMeasurement::default();
    }

    let len = measurements.len() as u64;
    CompileMeasurement {
        total_ms: measurements.iter().map(|m| m.total_ms).sum::<u64>() / len,
        peak_memory_kb: measurements
            .iter()
            .map(|m| m.peak_memory_kb)
            .sum::<u64>()
            / len,
        phase_timings: CompilePhaseTimings {
            preprocess_ms: measurements
                .iter()
                .map(|m| m.phase_timings.preprocess_ms)
                .sum::<u64>()
                / len,
            parse_ms: measurements
                .iter()
                .map(|m| m.phase_timings.parse_ms)
                .sum::<u64>()
                / len,
            codegen_ms: measurements
                .iter()
                .map(|m| m.phase_timings.codegen_ms)
                .sum::<u64>()
                / len,
            optimize_ms: measurements
                .iter()
                .map(|m| m.phase_timings.optimize_ms)
                .sum::<u64>()
                / len,
            ir_write_ms: measurements
                .iter()
                .map(|m| m.phase_timings.ir_write_ms)
                .sum::<u64>()
                / len,
            llc_ms: measurements
                .iter()
                .map(|m| m.phase_timings.llc_ms)
                .sum::<u64>()
                / len,
        },
    }
}

fn generate_synthetic_c(num_functions: usize) -> String {
    let mut source = String::new();
    source.push_str("#include <stdio.h>\n\n");

    for i in 0..num_functions {
        source.push_str(&format!(
            "int func_{}(int x) {{\n    return x * {} + {};\n}}\n\n",
            i,
            (i % 100) + 1,
            i
        ));
    }

    source.push_str("int main() {\n    int sum = 0;\n");
    for i in 0..num_functions.min(100) {
        source.push_str(&format!("    sum += func_{}(sum);\n", i));
    }
    source.push_str("    printf(\"%d\\n\", sum);\n    return 0;\n}\n");

    source
}

const MICRO_LOOP_SUM: &str = r#"
#include <stdio.h>

int main() {
    int sum = 0;
    for (int i = 0; i < 1000000; i++) {
        sum += i;
    }
    printf("%d\n", sum);
    return 0;
}
"#;

const MICRO_FUNC_CALL: &str = r#"
#include <stdio.h>

int add(int a, int b) {
    return a + b;
}

int main() {
    int result = 0;
    for (int i = 0; i < 100000; i++) {
        result = add(result, i);
    }
    printf("%d\n", result);
    return 0;
}
"#;

const MICRO_ARITHMETIC: &str = r#"
#include <stdio.h>

int main() {
    int a = 12345;
    int b = 67890;
    int sum = a + b;
    int diff = a - b;
    int prod = a * b;
    int quot = b / a;
    int rem = b % a;
    printf("%d %d %d %d %d\n", sum, diff, prod, quot, rem);
    return 0;
}
"#;

const MICRO_POINTER_OPS: &str = r#"
#include <stdio.h>

int main() {
    int arr[100];
    int *p = arr;
    for (int i = 0; i < 100; i++) {
        *(p + i) = i * 2;
    }
    int sum = 0;
    for (int i = 0; i < 100; i++) {
        sum += arr[i];
    }
    printf("%d\n", sum);
    return 0;
}
"#;

const COREUTILS_HELLO: &str = r#"
#include <stdio.h>

int main() {
    printf("Hello, World!\n");
    return 0;
}
"#;

const COREUTILS_CAT: &str = r#"
#include <stdio.h>

int main() {
    int c;
    while ((c = getchar()) != EOF) {
        putchar(c);
    }
    return 0;
}
"#;

const COREUTILS_WC: &str = r#"
#include <stdio.h>

int main() {
    int lines = 0, words = 0, chars = 0;
    int c;
    int in_word = 0;

    while ((c = getchar()) != EOF) {
        chars++;
        if (c == '\n') lines++;
        if (c == ' ' || c == '\t' || c == '\n') {
            in_word = 0;
        } else if (!in_word) {
            in_word = 1;
            words++;
        }
    }
    printf("%d %d %d\n", lines, words, chars);
    return 0;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_result_creation() {
        let result = BenchmarkResult {
            name: "test".to_string(),
            compiler: "gcc".to_string(),
            version: "11.0".to_string(),
            optimization: "O0".to_string(),
            metrics: BenchmarkMetrics::new(),
        };
        assert_eq!(result.name, "test");
        assert_eq!(result.compiler, "gcc");
        assert_eq!(result.optimization, "O0");
        assert_eq!(result.metrics.correctness, "error");
    }

    #[test]
    fn test_benchmark_metrics_pass() {
        let metrics = BenchmarkMetrics::pass(10);
        assert_eq!(metrics.correctness, "pass");
        assert_eq!(metrics.test_total, 10);
        assert_eq!(metrics.test_passed, 10);
        assert_eq!(metrics.test_failed, 0);
    }

    #[test]
    fn test_benchmark_metrics_fail() {
        let metrics = BenchmarkMetrics::fail(10, 3);
        assert_eq!(metrics.correctness, "fail");
        assert_eq!(metrics.test_total, 10);
        assert_eq!(metrics.test_passed, 7);
        assert_eq!(metrics.test_failed, 3);
    }

    #[test]
    fn test_benchmark_metrics_default() {
        let metrics = BenchmarkMetrics::default();
        assert_eq!(metrics.compile_time_ms, 0);
        assert_eq!(metrics.output_size_bytes, 0);
        assert_eq!(metrics.peak_memory_kb, 0);
        assert_eq!(metrics.correctness, "error");
    }

    #[test]
    fn test_benchmark_suite_variants() {
        let micro = BenchmarkSuite::Micro;
        let coreutils = BenchmarkSuite::Coreutils;
        let synthetic = BenchmarkSuite::Synthetic;

        assert_eq!(format!("{}", micro), "micro");
        assert_eq!(format!("{}", coreutils), "coreutils");
        assert_eq!(format!("{}", synthetic), "synthetic");

        assert_ne!(micro, coreutils);
        assert_ne!(coreutils, synthetic);
    }

    #[test]
    fn test_compiler_config_creation() {
        let config = CompilerConfig::new("gcc", "gcc")
            .with_compile_args(vec!["-c".to_string()])
            .with_link_args(vec!["-lm".to_string()]);

        assert_eq!(config.name, "gcc");
        assert_eq!(config.command, "gcc");
        assert_eq!(config.compile_args, vec!["-c".to_string()]);
        assert_eq!(config.link_args, vec!["-lm".to_string()]);
        assert!(config.compile_only);
    }

    #[test]
    fn test_compiler_config_availability() {
        let ls_config = CompilerConfig::new("ls", "ls");
        assert!(ls_config.is_available());

        let nonexistent = CompilerConfig::new("nonexistent_xyz_123", "nonexistent_xyz_123");
        assert!(!nonexistent.is_available());
    }

    #[test]
    fn test_compiler_config_version() {
        let ls_config = CompilerConfig::new("ls", "ls");
        let version = ls_config.get_version();
        assert!(!version.is_empty());
        assert_ne!(version, "unknown");
    }

    #[test]
    fn test_benchmark_runner_creation() {
        let runner = BenchmarkRunner::new();
        assert!(!runner.suites.is_empty());
        assert!(runner.runs_per_benchmark >= 1);
        assert_eq!(runner.results_dir, PathBuf::from("benchmarks/results"));
    }

    #[test]
    fn test_benchmark_runner_default() {
        let runner = BenchmarkRunner::default();
        assert!(!runner.suites.is_empty());
    }

    #[test]
    fn test_benchmark_runner_add_suite() {
        let mut runner = BenchmarkRunner::new();
        let initial_len = runner.suites.len();
        runner.add_suite(BenchmarkSuite::Synthetic);
        assert_eq!(runner.suites.len(), initial_len + 1);
        runner.add_suite(BenchmarkSuite::Synthetic);
        assert_eq!(runner.suites.len(), initial_len + 1);
    }

    #[test]
    fn test_benchmark_runner_builder_pattern() {
        let runner = BenchmarkRunner::new()
            .with_optimization_levels(vec!["O0".to_string(), "O2".to_string()])
            .with_results_dir(PathBuf::from("/tmp/bench"))
            .with_runs_per_benchmark(3);

        assert_eq!(runner.optimization_levels.len(), 2);
        assert_eq!(runner.results_dir, PathBuf::from("/tmp/bench"));
        assert_eq!(runner.runs_per_benchmark, 3);
    }

    #[test]
    fn test_measure_output_size() {
        // Use a test-specific suffix to avoid colliding with test_measure_correctness_pass
        // which runs in parallel and uses the same process ID.
        let temp_dir = std::env::temp_dir().join(format!("optic_bench_size_test_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();
        let test_file = temp_dir.join("test_output");
        fs::write(&test_file, "hello world").unwrap();

        let runner = BenchmarkRunner::new();
        let size = runner.measure_output_size(&test_file).unwrap();
        assert_eq!(size, 11);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_measure_correctness_missing_binary() {
        let runner = BenchmarkRunner::new();
        let metrics = runner.measure_correctness(
            Path::new("/nonexistent/binary_xyz_123"),
            &CompilerConfig::new("gcc", "gcc"),
            "test",
        );
        assert_eq!(metrics.correctness, "fail");
        assert_eq!(metrics.test_failed, 1);
    }

    #[test]
    fn test_measure_correctness_pass() {
        // Use a test-specific suffix to avoid colliding with test_measure_output_size
        // which runs in parallel and uses the same process ID.
        let temp_dir = std::env::temp_dir().join(format!("optic_bench_pass_test_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();

        #[cfg(target_os = "linux")]
        {
            let src = temp_dir.join("test.c");
            let out = temp_dir.join("test_bin");
            fs::write(&src, "int main(){return 0;}").unwrap();

            let gcc_available = CompilerConfig::new("gcc", "gcc").is_available();
            if gcc_available {
                let _ = Command::new("gcc").arg("-o").arg(&out).arg(&src).output();
                if out.exists() {
                    let runner = BenchmarkRunner::new();
                    let metrics =
                        runner.measure_correctness(&out, &CompilerConfig::new("gcc", "gcc"), "test");
                    assert_eq!(metrics.correctness, "pass");
                }
            }
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_generate_markdown_report_empty() {
        let report = generate_markdown_report(&[]);
        assert!(report.contains("# OpticC Benchmark Report"));
        assert!(report.contains("No benchmark results available"));
    }

    #[test]
    fn test_generate_markdown_report_with_results() {
        let results = vec![
            BenchmarkResult {
                name: "loop_sum".to_string(),
                compiler: "gcc".to_string(),
                version: "11.0".to_string(),
                optimization: "O0".to_string(),
                metrics: BenchmarkMetrics {
                    compile_time_ms: 100,
                    output_size_bytes: 8192,
                    peak_memory_kb: 1024,
                    ..BenchmarkMetrics::pass(1)
                },
            },
        ];

        let report = generate_markdown_report(&results);
        assert!(report.contains("loop_sum"));
        assert!(report.contains("gcc"));
        assert!(report.contains("100"));
        assert!(report.contains("8192"));
        assert!(report.contains("pass"));
    }

    #[test]
    fn test_generate_json_report() {
        let results = vec![BenchmarkResult {
            name: "test".to_string(),
            compiler: "gcc".to_string(),
            version: "11.0".to_string(),
            optimization: "O0".to_string(),
            metrics: BenchmarkMetrics::pass(1),
        }];

        let json = generate_json_report(&results).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("gcc"));
        assert!(json.contains("pass"));

        let parsed: Vec<BenchmarkResult> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "test");
    }

    #[test]
    fn test_result_aggregation() {
        let results = vec![
            BenchmarkResult {
                name: "loop".to_string(),
                compiler: "gcc".to_string(),
                version: "11.0".to_string(),
                optimization: "O0".to_string(),
                metrics: BenchmarkMetrics {
                    compile_time_ms: 100,
                    output_size_bytes: 8000,
                    peak_memory_kb: 1000,
                    ..BenchmarkMetrics::pass(1)
                },
            },
            BenchmarkResult {
                name: "loop".to_string(),
                compiler: "gcc".to_string(),
                version: "11.0".to_string(),
                optimization: "O0".to_string(),
                metrics: BenchmarkMetrics {
                    compile_time_ms: 200,
                    output_size_bytes: 8200,
                    peak_memory_kb: 1100,
                    ..BenchmarkMetrics::pass(1)
                },
            },
        ];

        let averages = calculate_averages(&results);
        assert_eq!(averages.len(), 1);
        assert_eq!(averages[0].metrics.compile_time_ms, 150);
        assert_eq!(averages[0].metrics.output_size_bytes, 8100);
        assert_eq!(averages[0].metrics.peak_memory_kb, 1050);
    }

    #[test]
    fn test_average_calculation_multiple_groups() {
        let results = vec![
            BenchmarkResult {
                name: "loop".to_string(),
                compiler: "gcc".to_string(),
                version: "11.0".to_string(),
                optimization: "O0".to_string(),
                metrics: BenchmarkMetrics {
                    compile_time_ms: 100,
                    output_size_bytes: 8000,
                    peak_memory_kb: 1000,
                    ..BenchmarkMetrics::pass(1)
                },
            },
            BenchmarkResult {
                name: "loop".to_string(),
                compiler: "clang".to_string(),
                version: "14.0".to_string(),
                optimization: "O0".to_string(),
                metrics: BenchmarkMetrics {
                    compile_time_ms: 80,
                    output_size_bytes: 7500,
                    peak_memory_kb: 900,
                    ..BenchmarkMetrics::pass(1)
                },
            },
        ];

        let averages = calculate_averages(&results);
        assert_eq!(averages.len(), 2);
    }

    #[test]
    fn test_comparison_table_generation() {
        let results = vec![
            BenchmarkResult {
                name: "loop".to_string(),
                compiler: "gcc".to_string(),
                version: "11.0".to_string(),
                optimization: "O0".to_string(),
                metrics: BenchmarkMetrics {
                    compile_time_ms: 100,
                    output_size_bytes: 8000,
                    peak_memory_kb: 1000,
                    ..BenchmarkMetrics::pass(1)
                },
            },
            BenchmarkResult {
                name: "loop".to_string(),
                compiler: "clang".to_string(),
                version: "14.0".to_string(),
                optimization: "O0".to_string(),
                metrics: BenchmarkMetrics {
                    compile_time_ms: 80,
                    output_size_bytes: 7500,
                    peak_memory_kb: 900,
                    ..BenchmarkMetrics::pass(1)
                },
            },
        ];

        let table = generate_comparison_table(&results);
        assert!(table.contains("loop"));
        assert!(table.contains("gcc"));
        assert!(table.contains("clang"));
        assert!(table.contains("100"));
        assert!(table.contains("80"));
    }

    #[test]
    fn test_error_handling_missing_compiler() {
        let config = CompilerConfig::new("nonexistent_xyz", "nonexistent_xyz_abc");
        assert!(!config.is_available());

        let version = config.get_version();
        assert_eq!(version, "unknown");
    }

    #[test]
    fn test_error_handling_failed_compilation() {
        let temp_dir = std::env::temp_dir().join(format!("optic_bench_test_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();

        let src = temp_dir.join("invalid.c");
        let out = temp_dir.join("invalid_out");
        fs::write(&src, "this is not valid C code {{{{").unwrap();

        let gcc_available = CompilerConfig::new("gcc", "gcc").is_available();
        if gcc_available {
            let runner = BenchmarkRunner::new();
            let compiler = CompilerConfig::new("gcc", "gcc");
            let result = runner.measure_compile_time(&compiler, &src, &out, "-O0");
            assert!(result.is_err());
            if let Err(BenchmarkError::CompileError(_, _)) = result {
            } else {
                panic!("Expected CompileError");
            }
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_benchmark_error_display() {
        let err = BenchmarkError::CompileError("test.c".to_string(), "syntax error".to_string());
        assert!(err.to_string().contains("test.c"));
        assert!(err.to_string().contains("syntax error"));

        let err = BenchmarkError::RuntimeError("segfault".to_string());
        assert!(err.to_string().contains("runtime error"));

        let err = BenchmarkError::CorrectnessError(
            "loop".to_string(),
            "gcc".to_string(),
            "wrong output".to_string(),
        );
        assert!(err.to_string().contains("loop"));
        assert!(err.to_string().contains("gcc"));

        let err = BenchmarkError::CompilerNotAvailable("opticc".to_string());
        assert!(err.to_string().contains("opticc"));
    }

    #[test]
    fn test_benchmark_error_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let bench_err: BenchmarkError = io_err.into();
        assert!(matches!(bench_err, BenchmarkError::IoError(_)));
    }

    #[test]
    fn test_generate_synthetic_c() {
        let source = generate_synthetic_c(10);
        assert!(source.contains("#include <stdio.h>"));
        assert!(source.contains("func_0"));
        assert!(source.contains("func_9"));
        assert!(source.contains("int main()"));
    }

    #[test]
    fn test_benchmark_result_serialization() {
        let result = BenchmarkResult {
            name: "test".to_string(),
            compiler: "gcc".to_string(),
            version: "11.0".to_string(),
            optimization: "O0".to_string(),
            metrics: BenchmarkMetrics {
                compile_time_ms: 100,
                output_size_bytes: 8192,
                peak_memory_kb: 1024,
                test_total: 5,
                test_passed: 5,
                ..BenchmarkMetrics::pass(5)
            },
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: BenchmarkResult = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, result.name);
        assert_eq!(deserialized.compiler, result.compiler);
        assert_eq!(deserialized.metrics.compile_time_ms, result.metrics.compile_time_ms);
        assert_eq!(deserialized.metrics.correctness, result.metrics.correctness);
    }

    #[test]
    fn test_micro_benchmark_sources_exist() {
        assert!(!MICRO_LOOP_SUM.is_empty());
        assert!(!MICRO_FUNC_CALL.is_empty());
        assert!(!MICRO_ARITHMETIC.is_empty());
        assert!(!MICRO_POINTER_OPS.is_empty());

        assert!(MICRO_LOOP_SUM.contains("for"));
        assert!(MICRO_FUNC_CALL.contains("add("));
        assert!(MICRO_ARITHMETIC.contains("*"));
        assert!(MICRO_POINTER_OPS.contains("*p"));
    }

    #[test]
    fn test_coreutils_benchmark_sources_exist() {
        assert!(!COREUTILS_HELLO.is_empty());
        assert!(!COREUTILS_CAT.is_empty());
        assert!(!COREUTILS_WC.is_empty());

        assert!(COREUTILS_HELLO.contains("Hello"));
        assert!(COREUTILS_CAT.contains("getchar"));
        assert!(COREUTILS_WC.contains("lines"));
    }

    #[test]
    fn test_measure_peak_memory() {
        let memory = measure_peak_memory();
        assert!(memory.is_ok());
    }

    #[test]
    fn test_runs_per_benchmark_minimum_one() {
        let runner = BenchmarkRunner::new().with_runs_per_benchmark(0);
        assert_eq!(runner.runs_per_benchmark, 1);

        let runner = BenchmarkRunner::new().with_runs_per_benchmark(10);
        assert_eq!(runner.runs_per_benchmark, 10);
    }
}
