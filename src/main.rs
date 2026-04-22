use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use optic_c::benchmark::{BenchmarkRunner, BenchmarkSuite, CompilerConfig};
use optic_c::build::{compile_single_file, BuildConfig, Builder, OutputType};
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
        #[arg(long)]
        sqlite_source: Option<PathBuf>,
    },
    IntegrationTest {
        #[arg(long, default_value = "/tmp/optic_integration")]
        test_dir: PathBuf,
        #[arg(long, short = 'o', default_value = "/tmp/optic_integration/output")]
        output_dir: PathBuf,
        #[arg(
            long,
            help = "SQLite source: local sqlite3.c, local dir/zip, or GitHub-hosted zip archive",
            default_value = "https://github.com/abramov7613/sqlite-amalgamation-mirror/archive/refs/heads/main.zip"
        )]
        sqlite_url: String,
    },
}

#[derive(Debug)]
struct DirectDriverInvocation {
    source_files: Vec<PathBuf>,
    output: PathBuf,
    include_paths: Vec<PathBuf>,
    force_includes: Vec<PathBuf>,
    defines: HashMap<String, String>,
    link_libs: Vec<String>,
    optimization: u32,
    output_type: OutputType,
    jobs: usize,
    depfile: Option<PathBuf>,
    dep_target: Option<String>,
    dep_phony: bool,
    freestanding: bool,
    nostdinc: bool,
    return_thunk_extern: bool,
    print_target: Option<String>,
    print_version: bool,
    print_version_only: bool,
    print_file_name: Option<String>,
}

impl Default for DirectDriverInvocation {
    fn default() -> Self {
        Self {
            source_files: Vec::new(),
            output: PathBuf::from("a.out"),
            include_paths: Vec::new(),
            force_includes: Vec::new(),
            defines: HashMap::new(),
            link_libs: Vec::new(),
            optimization: 0,
            output_type: OutputType::Executable,
            jobs: 1,
            depfile: None,
            dep_target: None,
            dep_phony: false,
            freestanding: false,
            nostdinc: false,
            return_thunk_extern: false,
            print_target: None,
            print_version: false,
            print_version_only: false,
            print_file_name: None,
        }
    }
}

fn is_direct_driver_invocation(args: &[String]) -> bool {
    match args.get(1).map(String::as_str) {
        None => false,
        Some("compile" | "build" | "benchmark" | "integration-test" | "help" | "--help" | "-h") => false,
        Some(_) => true,
    }
}

fn parse_define_arg(arg: &str, defines: &mut HashMap<String, String>) {
    if let Some((name, value)) = arg.split_once('=') {
        defines.insert(name.to_string(), value.to_string());
    } else {
        defines.insert(arg.to_string(), "1".to_string());
    }
}

fn expand_response_files(args: &[String]) -> Result<Vec<String>, String> {
    let mut expanded = Vec::new();
    if let Some(first) = args.first() {
        expanded.push(first.clone());
    }

    for arg in args.iter().skip(1) {
        if let Some(path) = arg.strip_prefix('@') {
            let content = fs::read_to_string(path)
                .map_err(|e| format!("Failed to read response file {}: {}", path, e))?;
            expanded.extend(content.split_whitespace().map(|token| token.to_string()));
        } else {
            expanded.push(arg.clone());
        }
    }

    Ok(expanded)
}

fn parse_direct_driver_args(args: &[String]) -> Result<DirectDriverInvocation, String> {
    let args = expand_response_files(args)?;
    let mut invocation = DirectDriverInvocation::default();
    let mut dep_requested = false;
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--version" => {
                invocation.print_version = true;
                i += 1;
                continue;
            }
            "-dumpversion" => {
                invocation.print_version_only = true;
                i += 1;
                continue;
            }
            "-dumpmachine" => {
                invocation.print_target = Some("x86_64-linux-gnu".to_string());
                i += 1;
                continue;
            }
            "-c" => {
                invocation.output_type = OutputType::Object;
                i += 1;
                continue;
            }
            "-shared" => {
                invocation.output_type = OutputType::SharedLib;
                i += 1;
                continue;
            }
            "-o" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "missing value for -o".to_string())?;
                invocation.output = PathBuf::from(value);
                i += 1;
                continue;
            }
            "-I" | "-isystem" | "-iquote" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| format!("missing value for {}", arg))?;
                invocation.include_paths.push(PathBuf::from(value));
                i += 1;
                continue;
            }
            "-D" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "missing value for -D".to_string())?;
                parse_define_arg(value, &mut invocation.defines);
                i += 1;
                continue;
            }
            "-U" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "missing value for -U".to_string())?;
                invocation.defines.remove(value);
                i += 1;
                continue;
            }
            "-l" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "missing value for -l".to_string())?;
                invocation.link_libs.push(value.clone());
                i += 1;
                continue;
            }
            "-MF" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "missing value for -MF".to_string())?;
                invocation.depfile = Some(PathBuf::from(value));
                dep_requested = true;
                i += 1;
                continue;
            }
            "-MT" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "missing value for -MT".to_string())?;
                invocation.dep_target = Some(value.clone());
                i += 1;
                continue;
            }
            "-MD" | "-MMD" => {
                dep_requested = true;
                i += 1;
                continue;
            }
            "-MP" => {
                invocation.dep_phony = true;
                dep_requested = true;
                i += 1;
                continue;
            }
            "-ffreestanding" => {
                invocation.freestanding = true;
                i += 1;
                continue;
            }
            "-nostdinc" => {
                invocation.nostdinc = true;
                i += 1;
                continue;
            }
            "-include" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "missing value for -include".to_string())?;
                invocation.force_includes.push(PathBuf::from(value));
                i += 1;
                continue;
            }
            "-x" => {
                i += 2;
                continue;
            }
            _ => {}
        }

        if let Some(value) = arg.strip_prefix("-print-file-name=") {
            invocation.print_file_name = Some(value.to_string());
            i += 1;
            continue;
        }

        if let Some(value) = arg
            .strip_prefix("-Wp,-MD,")
            .or_else(|| arg.strip_prefix("-Wp,-MMD,"))
        {
            invocation.depfile = Some(PathBuf::from(value));
            dep_requested = true;
            i += 1;
            continue;
        }

        if let Some(value) = arg.strip_prefix("-I") {
            if !value.is_empty() {
                invocation.include_paths.push(PathBuf::from(value));
                i += 1;
                continue;
            }
        }

        if let Some(value) = arg.strip_prefix("-isystem") {
            if !value.is_empty() {
                invocation.include_paths.push(PathBuf::from(value));
                i += 1;
                continue;
            }
        }

        if let Some(value) = arg.strip_prefix("-iquote") {
            if !value.is_empty() {
                invocation.include_paths.push(PathBuf::from(value));
                i += 1;
                continue;
            }
        }

        if let Some(value) = arg.strip_prefix("-D") {
            if !value.is_empty() {
                parse_define_arg(value, &mut invocation.defines);
                i += 1;
                continue;
            }
        }

        if let Some(value) = arg.strip_prefix("-U") {
            if !value.is_empty() {
                invocation.defines.remove(value);
                i += 1;
                continue;
            }
        }

        if let Some(value) = arg.strip_prefix("-l") {
            if !value.is_empty() {
                invocation.link_libs.push(value.to_string());
                i += 1;
                continue;
            }
        }

        if let Some(level) = arg.strip_prefix("-O") {
            invocation.optimization = match level {
                "0" => 0,
                "1" => 1,
                "2" => 2,
                "3" => 3,
                "s" | "z" => 2,
                _ => invocation.optimization,
            };
            i += 1;
            continue;
        }

        if let Some(mode) = arg.strip_prefix("-mfunction-return=") {
            invocation.return_thunk_extern = mode == "thunk-extern";
            i += 1;
            continue;
        }

        if arg.starts_with("-W")
            || arg.starts_with("-g")
            || arg.starts_with("-f")
            || arg.starts_with("-m")
            || arg.starts_with("-L")
            || arg == "-pipe"
            || arg == "-v"
            || arg == "-###"
        {
            i += 1;
            continue;
        }

        if arg.starts_with('-') {
            i += 1;
            continue;
        }

        invocation.source_files.push(PathBuf::from(arg));
        i += 1;
    }

    if invocation.freestanding {
        invocation
            .defines
            .entry("__STDC_HOSTED__".to_string())
            .or_insert_with(|| "0".to_string());
    }

    if invocation.print_version
        || invocation.print_version_only
        || invocation.print_target.is_some()
        || invocation.print_file_name.is_some()
    {
        return Ok(invocation);
    }

    if invocation.source_files.is_empty() {
        return Err("no input files provided".to_string());
    }

    if invocation.output == PathBuf::from("a.out") {
        invocation.output = match invocation.output_type {
            OutputType::Object => invocation.source_files[0].with_extension("o"),
            OutputType::SharedLib => PathBuf::from("a.so"),
            OutputType::StaticLib => PathBuf::from("liba.a"),
            OutputType::Executable => PathBuf::from("a.out"),
        };
    }

    if dep_requested && invocation.depfile.is_none() {
        invocation.depfile = Some(invocation.output.with_extension("d"));
    }

    Ok(invocation)
}

fn print_file_name(name: &str) {
    if name == "include" {
        if let Ok(output) = std::process::Command::new("gcc")
            .arg("-print-file-name=include")
            .output()
        {
            if output.status.success() {
                print!("{}", String::from_utf8_lossy(&output.stdout));
                return;
            }
        }
        println!("/usr/include");
    } else {
        println!("{}", name);
    }
}

fn write_depfile(
    depfile: &Path,
    target: &str,
    source_files: &[PathBuf],
    dep_phony: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = depfile.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let deps = source_files
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(" ");

    let mut content = format!("{}: {}\n", target, deps);
    if dep_phony {
        for source in source_files {
            content.push_str(&format!("{}:\n", source.display()));
        }
    }

    std::fs::write(depfile, content)?;
    Ok(())
}

fn execute_direct_driver(invocation: DirectDriverInvocation) -> Result<(), Box<dyn std::error::Error>> {
    if invocation.print_version {
        println!("optic_c {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if invocation.print_version_only {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(target) = invocation.print_target {
        println!("{}", target);
        return Ok(());
    }
    if let Some(name) = invocation.print_file_name {
        print_file_name(&name);
        return Ok(());
    }

    let config = BuildConfig::new()
        .with_source_files(invocation.source_files.clone())
        .with_output(invocation.output.clone())
        .with_output_type(invocation.output_type)
        .with_jobs(invocation.jobs)
        .with_optimization(invocation.optimization)
        .with_include_paths(invocation.include_paths.clone())
        .with_force_includes(invocation.force_includes.clone())
        .with_defines(invocation.defines.clone())
        .with_link_libs(invocation.link_libs.clone())
        .with_nostdinc(invocation.nostdinc)
        .with_return_thunk_extern(invocation.return_thunk_extern);

    let mut builder = Builder::new(config);
    builder.build()?;

    if let Some(depfile) = invocation.depfile {
        let target = invocation
            .dep_target
            .unwrap_or_else(|| invocation.output.display().to_string());
        write_depfile(&depfile, &target, &invocation.source_files, invocation.dep_phony)?;
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let raw_args: Vec<String> = std::env::args().collect();
    if is_direct_driver_invocation(&raw_args) {
        let invocation = parse_direct_driver_args(&raw_args)
            .map_err(std::io::Error::other)?;
        return execute_direct_driver(invocation);
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Compile {
            input,
            output,
            optimization,
        } => {
            let input_path = input.as_path();
            let output_path = output.unwrap_or_else(|| {
                let stem = input_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("a");
                Path::new(&format!("{}.ll", stem)).to_path_buf()
            });

            compile_single_file(input_path, &output_path, optimization, &[], &HashMap::new())?;

            println!(
                "Compiled {} -> {}",
                input_path.display(),
                output_path.display()
            );
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
            sqlite_source,
        } => {
            let mut runner = BenchmarkRunner::new()
                .with_results_dir(output_dir.clone())
                .with_runs_per_benchmark(runs)
                .with_sqlite_source(sqlite_source.clone());

            runner.suites.clear();

            match suite.as_str() {
                "all" => {
                    runner.add_suite(BenchmarkSuite::Micro);
                    runner.add_suite(BenchmarkSuite::Coreutils);
                    runner.add_suite(BenchmarkSuite::Synthetic);
                    runner.add_suite(BenchmarkSuite::Sqlite);
                    runner.add_suite(BenchmarkSuite::Rebuild);
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
                "sqlite" => {
                    runner.add_suite(BenchmarkSuite::Sqlite);
                }
                "rebuild" => {
                    runner.add_suite(BenchmarkSuite::Rebuild);
                }
                _ => {
                    eprintln!("Unknown suite: {}", suite);
                    eprintln!("Available: all, micro, coreutils, synthetic, sqlite, rebuild");
                    std::process::exit(1);
                }
            }

            if compilers != "all" {
                runner.compilers.clear();
                for name in compilers.split(',') {
                    let name = name.trim();
                    let config = match name {
                        "opticc" => CompilerConfig::opticc(),
                        "gcc" => CompilerConfig::new("gcc", "gcc")
                            .with_compile_args(vec!["-c".to_string()]),
                        "clang" => CompilerConfig::new("clang", "clang")
                            .with_compile_args(vec!["-c".to_string()]),
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
            println!(
                "  Download: {}",
                if result.download_success {
                    "SUCCESS"
                } else {
                    "FAILED"
                }
            );
            println!(
                "  Preprocess: {}",
                if result.preprocess_success {
                    "SUCCESS"
                } else {
                    "FAILED"
                }
            );
            println!(
                "  Compile: {}",
                if result.compile_success {
                    "SUCCESS"
                } else {
                    "FAILED"
                }
            );
            println!(
                "  Link: {}",
                if result.link_success {
                    "SUCCESS"
                } else {
                    "FAILED"
                }
            );
            println!(
                "  Smoke: {}",
                if result.smoke_test_success {
                    "SUCCESS"
                } else {
                    "FAILED"
                }
            );
            println!(
                "  Library: {}",
                if result.library_created {
                    "CREATED"
                } else {
                    "NOT CREATED"
                }
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_driver_arg_parsing_compile_object() {
        let args = vec![
            "optic_c".to_string(),
            "-ffreestanding".to_string(),
            "-Iinclude".to_string(),
            "-DDEBUG=1".to_string(),
            "-O2".to_string(),
            "-c".to_string(),
            "test.c".to_string(),
            "-o".to_string(),
            "test.o".to_string(),
            "-MD".to_string(),
            "-MF".to_string(),
            "test.d".to_string(),
        ];

        let invocation = parse_direct_driver_args(&args).expect("driver invocation");
        assert_eq!(invocation.source_files, vec![PathBuf::from("test.c")]);
        assert_eq!(invocation.output, PathBuf::from("test.o"));
        assert_eq!(invocation.optimization, 2);
        assert!(invocation.freestanding);
        assert_eq!(invocation.depfile, Some(PathBuf::from("test.d")));
        assert!(invocation.include_paths.contains(&PathBuf::from("include")));
        assert_eq!(invocation.defines.get("DEBUG"), Some(&"1".to_string()));
    }

    #[test]
    fn test_direct_driver_probe_parsing() {
        let args = vec!["optic_c".to_string(), "-dumpmachine".to_string()];
        let invocation = parse_direct_driver_args(&args).expect("driver probe");
        assert_eq!(invocation.print_target, Some("x86_64-linux-gnu".to_string()));
    }

    #[test]
    fn test_parse_define_arg() {
        let mut defines = HashMap::new();

        // Test normal key-value
        parse_define_arg("FOO=BAR", &mut defines);
        assert_eq!(defines.get("FOO").unwrap(), "BAR");

        // Test missing value (implicit 1)
        parse_define_arg("BAZ", &mut defines);
        assert_eq!(defines.get("BAZ").unwrap(), "1");

        // Test empty value
        parse_define_arg("EMPTY=", &mut defines);
        assert_eq!(defines.get("EMPTY").unwrap(), "");

        // Test value containing equals signs
        parse_define_arg("COMPLEX=a=b=c", &mut defines);
        assert_eq!(defines.get("COMPLEX").unwrap(), "a=b=c");
    }

    #[test]
    fn test_kbuild_style_flag_parsing() {
        let args = vec![
            "optic_c".to_string(),
            "-isystem".to_string(),
            "/usr/include".to_string(),
            "-iquote".to_string(),
            "include/generated".to_string(),
            "-include".to_string(),
            "generated/autoconf.h".to_string(),
            "-Wp,-MMD,module.d".to_string(),
            "-mfunction-return=thunk-extern".to_string(),
            "-UDEBUG".to_string(),
            "-x".to_string(),
            "c".to_string(),
            "-c".to_string(),
            "module.c".to_string(),
            "-o".to_string(),
            "module.o".to_string(),
        ];

        let invocation = parse_direct_driver_args(&args).expect("kbuild-style invocation");
        assert_eq!(invocation.source_files, vec![PathBuf::from("module.c")]);
        assert!(invocation.include_paths.contains(&PathBuf::from("/usr/include")));
        assert!(invocation.include_paths.contains(&PathBuf::from("include/generated")));
        assert!(invocation.force_includes.contains(&PathBuf::from("generated/autoconf.h")));
        assert_eq!(invocation.depfile, Some(PathBuf::from("module.d")));
        assert!(invocation.return_thunk_extern);
        assert!(!invocation.defines.contains_key("DEBUG"));
        assert_eq!(invocation.output, PathBuf::from("module.o"));
    }
}
