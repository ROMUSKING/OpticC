use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use optic_c::build::{BuildConfig, Builder, OutputType, compile_single_file};

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
        } => {
            let input_path = input.as_path();
            let output_path = output
                .unwrap_or_else(|| {
                    let stem = input_path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("a");
                    Path::new(&format!("{}.ll", stem)).to_path_buf()
                });

            compile_single_file(input_path, &output_path, optimization, &[], &HashMap::new())?;

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
    }

    Ok(())
}
