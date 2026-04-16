use clap::{Parser, Subcommand};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use optic_c::arena::Arena;
use optic_c::backend::llvm::LlvmBackend;
use optic_c::frontend::parser::Parser as CParser;

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

            compile_file(input_path, &output_path, optimization)?;
        }
    }

    Ok(())
}

fn compile_file(
    input_path: &Path,
    output_path: &Path,
    opt_level: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(input_path)
        .map_err(|e| format!("Failed to read input file '{}': {}", input_path.display(), e))?;

    let arena = Arena::new("/tmp/optic_c_arena.bin", 1024 * 1024)
        .map_err(|e| format!("Failed to create AST arena: {}", e))?;

    let mut parser = CParser::new(arena);
    let ast_root = parser.parse(&source)
        .map_err(|e| format!("Parse error at line {}, column {}: {}", e.line, e.column, e.message))?;

    let context = inkwell::context::Context::create();
    let module_name = input_path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("input");
    let mut backend = LlvmBackend::new(&context, module_name);

    backend.compile(&parser.arena, ast_root)
        .map_err(|e| format!("Backend compilation error: {}", e))?;

    if opt_level > 0 {
        backend.optimize(opt_level)
            .map_err(|e| format!("Optimization error: {}", e))?;
    }

    backend.verify()
        .map_err(|e| format!("LLVM verification error: {}", e))?;

    let ir = backend.dump_ir();
    
    let mut file = fs::File::create(output_path)
        .map_err(|e| format!("Failed to create output file '{}': {}", output_path.display(), e))?;
    
    file.write_all(ir.as_bytes())
        .map_err(|e| format!("Failed to write output file: {}", e))?;

    println!("Compiled {} -> {}", input_path.display(), output_path.display());

    Ok(())
}
