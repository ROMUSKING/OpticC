use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use optic_c::benchmark::{BenchmarkRunner, BenchmarkSuite, CompilerConfig};
use optic_c::build::{BuildConfig, Builder, OutputType, compile_single_file};
use optic_c::integration::IntegrationTest;

#[derive(Parser)]
#[command(name = "optic_c")]
#[command(version = "0.1.0")]
#[command(about = "Optic C Compiler", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Compile {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(short = 'O', long, default_value = "0")]
        optimization: u32,
        #[arg(long, short = 'I')]
        include_paths: Vec<PathBuf>,
    },
    Build {
        #[arg(long)]
        src_dir: Option<PathBuf>,
        #[arg(long, short)]
        output: PathBuf,
        #[arg(long, short = 'j', default_value = "1")]
        jobs: usize,
        #[arg(long, short = 'I')]
        include_paths: Vec<PathBuf>,
        #[arg(long, short = 'D')]
        defines: Vec<String>,
        #[arg(long)]
        link_libs: Vec<String>,
        #[arg(long, short = 't', default_value = "auto")]
        output_type: String,
        #[arg(long)]
        source_files: Vec<PathBuf>,
    },
    Benchmark {
        #[arg(long, short = 's', default_value = "all")]
        suite: String,
        #[arg(long, short = 'c', default_value = "all")]
        compilers: String,
        #[arg(long, short = 'o', default_value = "output")]
        output_dir: PathBuf,
        #[arg(long, default_value = "5")]
        runs: usize,
    },
    IntegrationTest {
        #[arg(long, default_value = "/tmp/optic_integration")]
        test_dir: PathBuf,
        #[arg(long, short = 'o', default_value = "/tmp/optic_integration/output")]
        output_dir: PathBuf,
        #[arg(long, default_value = "https://www.sqlite.org/2026/sqlite-amalgamation-3490200.zip")]
        sqlite_url: String,
    },
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compile {
            input,
            output,
            optimization,
            include_paths,
        } => {
            let input_path = input.as_path();
            let output_path = output
                .unwrap_or_else(|| {
                    let stem = input_path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("a");
                    Path::new(&format!("{}.ll", stem)).to_path_buf()
                });

            // Add system include paths by default
            let mut sys_includes = include_paths;
            sys_includes.push(PathBuf::from("/usr/include"));
            sys_includes.push(PathBuf::from("/usr/lib/llvm-14/lib/clang/14.0.0/include"));
            sys_includes.push(PathBuf::from("/usr/lib/gcc/x86_64-linux-gnu/11/include"));

            compile_single_file(input_path, &output_path, optimization, &sys_includes, &HashMap::new())?;

            println!("Compiled {} -> {}", input_path.display(), output_path.display());
        }
        Commands::Build {
            src_dir,
            output,
            jobs,
            include_paths,
            defines,
            link_libs,
            output_type,
            source_files,
        } => {
            let mut source_files = source_files;

            if let Some(dir) = src_dir {
                let discovered = BuildConfig::discover_source_files(&dir);
                if discovered.is_empty() {
                    eprintln!("Warning: no .c files found in {}", dir.display());
                }
                source_files.extend(discovered);
            }

            if source_files.is_empty() {
                eprintln!("Error: no source files specified");
                std::process::exit(1);
            }

            let output_type = if output_type == "auto" {
                OutputType::from_extension(&output)
            } else {
                OutputType::from_str(&output_type).unwrap_or(OutputType::Executable)
            };

            let mut defines_map = HashMap::new();
            for def in defines {
                if let Some((name, value)) = def.split_once('=') {
                    defines_map.insert(name.to_string(), value.to_string());
                } else {
                    defines_map.insert(def.clone(), "1".to_string());
                }
            }

            let config = BuildConfig::new()
                .with_source_files(source_files)
                .with_output(output)
                .with_output_type(output_type)
                .with_jobs(jobs)
                .with_include_paths(include_paths)
                .with_defines(defines_map)
                .with_link_libs(link_libs);

            let mut builder = Builder::new(config);
            builder.build()?;

            println!("Build completed successfully");
        }
        Commands::Benchmark {
            suite,
            compilers,
            output_dir,
            runs,
        } => {
            let mut runner = BenchmarkRunner::new()
                .with_results_dir(output_dir.clone())
                .with_runs_per_benchmark(runs);

            match suite.as_str() {
                "all" => {
                    runner.add_suite(BenchmarkSuite::Micro);
                    runner.add_suite(BenchmarkSuite::Coreutils);
                    runner.add_suite(BenchmarkSuite::Synthetic);
                }
                "micro" => {
                    runner.add_suite(BenchmarkSuite::Micro);
                }
                "coreutils" => {
                    runner.add_suite(BenchmarkSuite::Coreutils);
                }
                "synthetic" => {
                    runner.add_suite(BenchmarkSuite::Synthetic);
                }
                _ => {
                    eprintln!("Unknown suite: {}", suite);
                    eprintln!("Available: all, micro, coreutils, synthetic");
                    std::process::exit(1);
                }
            }

            if compilers != "all" {
                runner.compilers.clear();
                for name in compilers.split(',') {
                    let name = name.trim();
                    let config = match name {
                        "gcc" => CompilerConfig::new("gcc", "gcc"),
                        "clang" => CompilerConfig::new("clang", "clang"),
                        _ => {
                            eprintln!("Unknown compiler: {}", name);
                            continue;
                        }
                    };
                    if config.is_available() {
                        runner.add_compiler(config);
                    } else {
                        eprintln!("Warning: {} not available, skipping", name);
                    }
                }
            }

            if runner.compilers.is_empty() {
                eprintln!("Error: no compilers available");
                std::process::exit(1);
            }

            let results = runner.run()?;

            let md_report = optic_c::benchmark::generate_markdown_report(&results);
            let report_path = output_dir.join("report.md");
            std::fs::write(&report_path, &md_report)?;

            println!("Benchmark completed: {} results", results.len());
            println!("Report written to: {}", report_path.display());
        }
        Commands::IntegrationTest {
            test_dir,
            output_dir,
            sqlite_url,
        } => {
            let test = IntegrationTest::new(test_dir.clone(), output_dir.clone(), sqlite_url);

            println!("Running SQLite integration test...");
            println!("  SQLite URL: {}", test.sqlite_url);
            println!("  Version: {}", test.sqlite_version);
            println!("  Test dir: {}", test_dir.display());
            println!("  Output dir: {}", output_dir.display());
            println!();

            let result = test.run();

            let report = test.generate_report(&result);
            let report_path = output_dir.join("integration_report.md");
            std::fs::write(&report_path, &report)?;

            println!();
            println!("Integration test completed.");
            println!("  Download: {}", if result.download_success { "SUCCESS" } else { "FAILED" });
            println!("  Preprocess: {}", if result.preprocess_success { "SUCCESS" } else { "FAILED" });
            println!("  Compile: {}", if result.compile_success { "SUCCESS" } else { "FAILED" });
            println!("  Link: {}", if result.link_success { "SUCCESS" } else { "FAILED" });
            println!("  Library: {}", if result.library_created { "CREATED" } else { "NOT CREATED" });
            println!("  Size: {} bytes", result.library_size_bytes);
            println!("  Time: {} ms", result.compile_time_ms);

            if !result.errors.is_empty() {
                println!();
                println!("Errors:");
                for error in &result.errors {
                    println!("  - {}", error);
                }
            }

            if !result.warnings.is_empty() {
                println!();
                println!("Warnings:");
                for warning in &result.warnings {
                    println!("  - {}", warning);
                }
            }

            println!();
            println!("Report written to: {}", report_path.display());

            if !result.all_passed() {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
