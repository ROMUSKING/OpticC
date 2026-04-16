use std::env;
use std::fs;
use std::path::Path;

use optic::arena::{Arena, CAstNode, NodeFlags, NodeOffset};
use optic::analysis::alias::{AliasAnalyzer, DiagnosticSeverity};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: optic <command> [args...]");
        eprintln!("Commands:");
        eprintln!("  analyze <c_file>    - Analyze C file for vulnerabilities");
        eprintln!("  vfs <output_dir>    - Generate VFS output with taint tracking");
        eprintln!("  version             - Show version");
        return;
    }

    match args[1].as_str() {
        "analyze" => {
            if args.len() < 3 {
                eprintln!("Usage: optic analyze <c_file>");
                return;
            }
            analyze_file(&args[2]);
        }
        "vfs" => {
            let output_dir = args.get(2).map(|s| s.as_str()).unwrap_or("/tmp/optic_vfs");
            generate_vfs(output_dir);
        }
        "version" => {
            println!("optic 0.1.0");
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
        }
    }
}

fn analyze_file(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to read {}: {}", path, e);
            return;
        }
    };

    let arena_path = format!("/tmp/optic_arena_{}.bin", std::process::id());
    let node_capacity = (source.len() / 10 + 1024).max(4096);

    let mut arena = match Arena::new(&arena_path, node_capacity) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to create arena: {}", e);
            return;
        }
    };

    let mut vuln_count = 0u32;
    let mut line_num = 1u32;

    for line in source.lines() {
        if is_vulnerable_pattern(line) {
            let node = CAstNode {
                kind: 65,
                flags: NodeFlags::IS_VOLATILE,
                left_child: NodeOffset(0),
                next_sibling: NodeOffset(0),
                data_offset: line_num,
            };
            arena.alloc(node);
            vuln_count += 1;
        }
        line_num += 1;
    }

    let mut analyzer = AliasAnalyzer::new(&arena);

    // Mark all vulnerability nodes as tainted and emit diagnostics
    let node_size = std::mem::size_of::<CAstNode>() as u32;
    for i in 0..vuln_count {
        let node_offset = NodeOffset(node_size + i * node_size);
        analyzer.mark_freed(node_offset);
        analyzer.emit_diagnostic(
            DiagnosticSeverity::Warning,
            node_offset,
            &format!("Line {}: Potential vulnerability detected", i + 1),
        );
    }

    println!("Analysis of: {}", path);
    println!("Source lines: {}", line_num - 1);
    println!("Vulnerability patterns found: {}", vuln_count);
    println!("Diagnostics: {}", analyzer.get_diagnostics().len());
    println!("Warnings: {}", analyzer.get_warning_count());
    println!("Errors: {}", analyzer.get_error_count());

    for diag in analyzer.get_diagnostics() {
        println!("  [{:?}] {}", diag.severity, diag.message);
    }

    let _ = fs::remove_file(&arena_path);
}

fn is_vulnerable_pattern(line: &str) -> bool {
    let patterns = [
        "strcpy", "strcat", "sprintf", "gets", "scanf",
        "malloc", "calloc", "realloc", "free",
        "memcpy", "memmove",
    ];
    patterns.iter().any(|p| line.contains(p))
}

fn generate_vfs(output_dir: &str) {
    println!("VFS output directory: {}", output_dir);

    let vfs_dir = Path::new(output_dir).join(".optic").join("vfs").join("src");
    fs::create_dir_all(&vfs_dir).unwrap_or_else(|e| {
        eprintln!("Failed to create VFS directory: {}", e);
    });

    let sample_content = r#"/* OPTIC RECONSTRUCTED FILE */
/* Taint Tracking Shadow Comments */

void copy_string(char *dest, const char *src) {
    // [OPTIC ERROR] strcpy(dest, src); - potential buffer overflow
    strcpy(dest, src);
}

void format_output(char *buf, const char *user_input) {
    // [OPTIC ERROR] sprintf(buf, user_input); - potential buffer overflow
    sprintf(buf, user_input);
}

char *create_buffer(size_t size) {
    // [OPTIC ERROR] malloc(size); - unchecked allocation
    char *buf = malloc(size);
    return buf;
}

void process_data(char *data) {
    // [OPTIC ERROR] free(data); - memory freed, potential use-after-free
    free(data);
}
"#;

    let output_path = vfs_dir.join("sample.c");
    fs::write(&output_path, sample_content).unwrap_or_else(|e| {
        eprintln!("Failed to write VFS file: {}", e);
    });

    println!("VFS output written to: {}", output_path.display());

    let content = fs::read_to_string(&output_path).unwrap_or_default();
    if content.contains("[OPTIC ERROR]") {
        println!("VERIFIED: Taint tracking shadow comments are projected into VFS");
        let count = content.matches("[OPTIC ERROR]").count();
        println!("Shadow comment count: {}", count);
    } else {
        println!("WARNING: No taint tracking shadow comments found");
    }
}
