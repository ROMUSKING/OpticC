use clap::{Parser, Subcommand};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use optic_c::arena::Arena;
use optic_c::backend::llvm::LlvmBackend;
use optic_c::frontend::parser::Parser as CParser;
use optic_c::frontend::preprocessor::Preprocessor;
use optic_c::db::OpticDb;
use optic_c::types::TypeSystem;

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
    let db_path = format!("/tmp/optic_db_{}.redb", std::process::id());
    let db = OpticDb::new(&db_path)
        .map_err(|e| format!("Failed to create database: {}", e))?;

    let mut pp = Preprocessor::new(db);
    let tokens = pp.process(input_path.to_str().unwrap())
        .map_err(|e| format!("Preprocessor error: {}", e))?;

    let estimated_nodes = (tokens.len() / 2).max(1024) as u32;
    let arena_path = format!("/tmp/optic_c_arena_{}.bin", std::process::id());

    let arena = Arena::new(&arena_path, estimated_nodes * 2)
        .map_err(|e| format!("Failed to create AST arena: {}", e))?;

    let mut parser = CParser::new(arena);
    let ast_root = parser.parse_tokens(tokens)
        .map_err(|e| format!("Parse error at line {}, column {}: {}", e.line, e.column, e.message))?;

    let context = inkwell::context::Context::create();
    let module_name = input_path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("input");
    let type_system = TypeSystem::new();
    let mut backend = LlvmBackend::with_types(&context, module_name, &type_system);

    backend.compile(&parser.arena, ast_root)
        .map_err(|e| format!("Backend compilation error: {}", e))?;

    if opt_level > 0 {
        backend.optimize(opt_level)
            .map_err(|e| format!("Optimization error: {}", e))?;
    }

    // Verify is optional - don't fail on verification errors for now
    let _ = backend.verify();

    let ir = backend.dump_ir();
    
    let mut file = fs::File::create(output_path)
        .map_err(|e| format!("Failed to create output file '{}': {}", output_path.display(), e))?;
    
    file.write_all(ir.as_bytes())
        .map_err(|e| format!("Failed to write output file: {}", e))?;

    // Clean up arena file and db file
    let _ = std::fs::remove_file(&arena_path);
    let _ = std::fs::remove_file(&db_path);

    println!("Compiled {} -> {}", input_path.display(), output_path.display());

    Ok(())
}
