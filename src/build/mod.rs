use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use rayon::prelude::*;

use crate::arena::Arena;
use crate::backend::llvm::LlvmBackend;
use crate::db::OpticDb;
use crate::frontend::parser::Parser as CParser;
use crate::frontend::preprocessor::Preprocessor;
use crate::types::TypeSystem;

#[derive(Debug)]
pub enum BuildError {
    CompileError(String, String),
    LinkError(String),
    IoError(std::io::Error),
    NoSourceFiles,
    ExternalToolError { tool: String, message: String },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::CompileError(file, msg) => write!(f, "compile error in {}: {}", file, msg),
            BuildError::LinkError(msg) => write!(f, "link error: {}", msg),
            BuildError::IoError(e) => write!(f, "I/O error: {}", e),
            BuildError::NoSourceFiles => write!(f, "no source files found"),
            BuildError::ExternalToolError { tool, message } => {
                write!(f, "external tool '{}' error: {}", tool, message)
            }
        }
    }
}

impl std::error::Error for BuildError {}

impl From<std::io::Error> for BuildError {
    fn from(e: std::io::Error) -> Self {
        BuildError::IoError(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputType {
    Object,
    StaticLib,
    SharedLib,
    Executable,
}

impl OutputType {
    pub fn from_extension(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("o") => OutputType::Object,
            Some("a") => OutputType::StaticLib,
            Some("so") => OutputType::SharedLib,
            _ => OutputType::Executable,
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "object" | "obj" => Some(OutputType::Object),
            "static" | "staticlib" | "a" => Some(OutputType::StaticLib),
            "shared" | "sharedlib" | "so" => Some(OutputType::SharedLib),
            "executable" | "exe" | "bin" => Some(OutputType::Executable),
            "auto" => None,
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheKey {
    pub source_hash: [u8; 32],
    pub flags_hash: [u8; 32],
}

impl CacheKey {
    pub fn new(source: &str, flags: &[String]) -> Self {
        use sha2::{Sha256, Digest};
        let mut h1 = Sha256::new();
        h1.update(source.as_bytes());
        let source_hash: [u8; 32] = h1.finalize().into();

        let mut h2 = Sha256::new();
        for f in flags {
            h2.update(f.as_bytes());
        }
        let flags_hash: [u8; 32] = h2.finalize().into();

        CacheKey { source_hash, flags_hash }
    }
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.source_hash == other.source_hash && self.flags_hash == other.flags_hash
    }
}

impl Eq for CacheKey {}

impl std::hash::Hash for CacheKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.source_hash.hash(state);
        self.flags_hash.hash(state);
    }
}

pub struct BuildConfig {
    pub source_files: Vec<PathBuf>,
    pub output: PathBuf,
    pub include_paths: Vec<PathBuf>,
    pub defines: HashMap<String, String>,
    pub link_libs: Vec<String>,
    pub jobs: usize,
    pub optimization: u32,
    pub output_type: OutputType,
}

impl BuildConfig {
    pub fn new() -> Self {
        BuildConfig {
            source_files: Vec::new(),
            output: PathBuf::from("a.out"),
            include_paths: Vec::new(),
            defines: HashMap::new(),
            link_libs: Vec::new(),
            jobs: 1,
            optimization: 0,
            output_type: OutputType::Executable,
        }
    }

    pub fn with_source_files(mut self, files: Vec<PathBuf>) -> Self {
        self.source_files = files;
        self
    }

    pub fn with_output(mut self, output: PathBuf) -> Self {
        self.output = output;
        self
    }

    pub fn with_output_type(mut self, output_type: OutputType) -> Self {
        self.output_type = output_type;
        self
    }

    pub fn with_jobs(mut self, jobs: usize) -> Self {
        self.jobs = jobs.max(1);
        self
    }

    pub fn with_optimization(mut self, opt: u32) -> Self {
        self.optimization = opt;
        self
    }

    pub fn with_include_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.include_paths = paths;
        self
    }

    pub fn with_defines(mut self, defines: HashMap<String, String>) -> Self {
        self.defines = defines;
        self
    }

    pub fn with_link_libs(mut self, libs: Vec<String>) -> Self {
        self.link_libs = libs;
        self
    }

    pub fn discover_source_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext == "c" {
                        files.push(path);
                    }
                }
            }
        }
        files.sort();
        files
    }

    pub fn validate(&self) -> Result<(), BuildError> {
        if self.source_files.is_empty() {
            return Err(BuildError::NoSourceFiles);
        }
        for file in &self.source_files {
            if !file.exists() {
                return Err(BuildError::CompileError(
                    file.display().to_string(),
                    "file does not exist".to_string(),
                ));
            }
        }
        Ok(())
    }

    pub fn compiler_flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        for path in &self.include_paths {
            flags.push(format!("-I{}", path.display()));
        }
        for (name, value) in &self.defines {
            flags.push(format!("-D{}={}", name, value));
        }
        flags
    }
}

pub struct Builder {
    config: BuildConfig,
    object_files: Vec<PathBuf>,
    temp_dir: PathBuf,
}

impl Builder {
    pub fn new(config: BuildConfig) -> Self {
        let temp_dir = PathBuf::from(format!("/tmp/opticc_build_{}", std::process::id()));
        Builder {
            config,
            object_files: Vec::new(),
            temp_dir,
        }
    }

    pub fn build(&mut self) -> Result<(), BuildError> {
        self.config.validate()?;

        fs::create_dir_all(&self.temp_dir)?;

        self.compile_all()?;

        if self.object_files.is_empty() {
            return Err(BuildError::NoSourceFiles);
        }

        match self.config.output_type {
            OutputType::Object => {
                if self.object_files.len() == 1 {
                    if let Some(obj) = self.object_files.first() {
                        fs::copy(obj, &self.config.output)?;
                    }
                } else {
                    return Err(BuildError::LinkError(
                        "multiple source files cannot produce a single object file".to_string(),
                    ));
                }
            }
            OutputType::StaticLib => self.create_static_lib()?,
            OutputType::SharedLib => self.create_shared_lib()?,
            OutputType::Executable => self.create_executable()?,
        }

        let _ = fs::remove_dir_all(&self.temp_dir);

        Ok(())
    }

    fn compile_all(&mut self) -> Result<(), BuildError> {
        let temp_dir = self.temp_dir.clone();
        let include_paths = self.config.include_paths.clone();
        let defines = self.config.defines.clone();
        let optimization = self.config.optimization;

        fs::create_dir_all(&temp_dir)?;

        let results: Vec<Result<PathBuf, BuildError>> = self
            .config
            .source_files
            .par_iter()
            .map_with(
                (temp_dir, include_paths, defines, optimization),
                |(temp_dir, include_paths, defines, optimization), source| {
                    compile_file_to_object(
                        source,
                        temp_dir,
                        include_paths,
                        defines,
                        *optimization,
                    )
                },
            )
            .collect();

        let mut objects = Vec::new();
        for result in results {
            match result {
                Ok(obj) => objects.push(obj),
                Err(e) => return Err(e),
            }
        }

        self.object_files = objects;
        Ok(())
    }

    pub fn compile_file(&self, source: &Path) -> Result<PathBuf, BuildError> {
        compile_file_to_object(
            source,
            &self.temp_dir,
            &self.config.include_paths,
            &self.config.defines,
            self.config.optimization,
        )
    }

    fn link_objects(&self) -> Result<(), BuildError> {
        if self.object_files.is_empty() {
            return Err(BuildError::LinkError("no object files to link".to_string()));
        }

        let clang = find_tool(&["clang", "gcc"])?;
        let mut cmd = Command::new(&clang);

        for obj in &self.object_files {
            cmd.arg(obj);
        }

        cmd.arg("-o").arg(&self.config.output);

        for lib in &self.config.link_libs {
            cmd.arg(format!("-l{}", lib));
        }

        let output = cmd.output().map_err(BuildError::IoError)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BuildError::LinkError(stderr.to_string()));
        }

        Ok(())
    }

    fn create_static_lib(&self) -> Result<(), BuildError> {
        if self.object_files.is_empty() {
            return Err(BuildError::LinkError("no object files to archive".to_string()));
        }

        let ar = find_tool(&["ar"])?;
        let mut cmd = Command::new(&ar);
        cmd.arg("rcs").arg(&self.config.output);

        for obj in &self.object_files {
            cmd.arg(obj);
        }

        let output = cmd.output().map_err(BuildError::IoError)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BuildError::ExternalToolError {
                tool: "ar".to_string(),
                message: stderr.to_string(),
            });
        }

        Ok(())
    }

    fn create_shared_lib(&self) -> Result<(), BuildError> {
        if self.object_files.is_empty() {
            return Err(BuildError::LinkError("no object files to link".to_string()));
        }

        let clang = find_tool(&["clang", "gcc"])?;
        let mut cmd = Command::new(&clang);
        cmd.arg("-shared");

        for obj in &self.object_files {
            cmd.arg(obj);
        }

        cmd.arg("-o").arg(&self.config.output);

        for lib in &self.config.link_libs {
            cmd.arg(format!("-l{}", lib));
        }

        let output = cmd.output().map_err(BuildError::IoError)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BuildError::LinkError(stderr.to_string()));
        }

        Ok(())
    }

    fn create_executable(&self) -> Result<(), BuildError> {
        self.link_objects()
    }

    pub fn run_external(&self, cmd: &str, args: &[&str]) -> Result<(), BuildError> {
        let output = Command::new(cmd)
            .args(args)
            .output()
            .map_err(BuildError::IoError)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BuildError::ExternalToolError {
                tool: cmd.to_string(),
                message: stderr.to_string(),
            });
        }

        Ok(())
    }

    pub fn object_files(&self) -> &[PathBuf] {
        &self.object_files
    }

    pub fn temp_dir(&self) -> &Path {
        &self.temp_dir
    }
}

fn find_tool(candidates: &[&str]) -> Result<String, BuildError> {
    for candidate in candidates {
        if Command::new(candidate).arg("--version").output().is_ok() {
            return Ok(candidate.to_string());
        }
    }
    Err(BuildError::ExternalToolError {
        tool: candidates.join(", "),
        message: "no suitable tool found".to_string(),
    })
}

fn compile_file_to_object(
    source: &Path,
    temp_dir: &Path,
    include_paths: &[PathBuf],
    defines: &HashMap<String, String>,
    optimization: u32,
) -> Result<PathBuf, BuildError> {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let ll_path = temp_dir.join(format!("{}.ll", stem));
    let obj_path = temp_dir.join(format!("{}.o", stem));

    let db_path = format!("/tmp/optic_db_build_{}_{}.redb", std::process::id(), stem);
    let db = OpticDb::new(&db_path)
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;

    let mut pp = Preprocessor::new(db);

    for path in include_paths {
        pp.add_include_path(path.to_str().unwrap_or(""));
    }
    for (name, value) in defines {
        pp.define_macro(name, value);
    }

    let tokens = pp
        .process(source.to_str().unwrap())
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;

    let estimated_nodes = (tokens.len() / 2).max(1024) as u32;
    let arena_path = format!("/tmp/optic_c_arena_build_{}_{}.bin", std::process::id(), stem);

    let arena = Arena::new(&arena_path, estimated_nodes * 2)
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;

    let mut parser = CParser::new(arena);
    let ast_root = parser
        .parse_tokens(tokens)
        .map_err(|e| {
            BuildError::CompileError(
                source.display().to_string(),
                format!("parse error at line {}, column {}: {}", e.line, e.column, e.message),
            )
        })?;

    let context = inkwell::context::Context::create();
    let module_name = source.file_stem().and_then(|s| s.to_str()).unwrap_or("input");
    let type_system = TypeSystem::new();
    let mut backend = LlvmBackend::with_types(&context, module_name, &type_system);

    backend
        .compile(&parser.arena, ast_root)
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;

    if optimization > 0 {
        backend
            .optimize(optimization)
            .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;
    }

    let _ = backend.verify();

    let ir = backend.dump_ir();

    let mut file = fs::File::create(&ll_path)
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;

    file.write_all(ir.as_bytes())
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;

    let llc = find_tool(&["llc", "llc-18", "llc-17", "llc-16"])?;
    let llc_output = Command::new(&llc)
        .arg("-filetype=obj")
        .arg("-o")
        .arg(&obj_path)
        .arg(&ll_path)
        .output()
        .map_err(BuildError::IoError)?;

    if !llc_output.status.success() {
        let stderr = String::from_utf8_lossy(&llc_output.stderr);
        return Err(BuildError::ExternalToolError {
            tool: "llc".to_string(),
            message: stderr.to_string(),
        });
    }

    let _ = std::fs::remove_file(&arena_path);
    let _ = std::fs::remove_file(&db_path);

    Ok(obj_path)
}

pub fn compile_single_file(
    input_path: &Path,
    output_path: &Path,
    opt_level: u32,
    include_paths: &[PathBuf],
    defines: &HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = format!("/tmp/optic_db_{}.redb", std::process::id());
    let db = OpticDb::new(&db_path)
        .map_err(|e| format!("Failed to create database: {}", e))?;

    let mut pp = Preprocessor::new(db);

    for path in include_paths {
        pp.add_include_path(path.to_str().unwrap_or(""));
    }
    for (name, value) in defines {
        pp.define_macro(name, value);
    }

    let tokens = pp
        .process(input_path.to_str().unwrap())
        .map_err(|e| format!("Preprocessor error: {}", e))?;

    let estimated_nodes = (tokens.len() / 2).max(1024) as u32;
    let arena_path = format!("/tmp/optic_c_arena_{}.bin", std::process::id());

    let arena = Arena::new(&arena_path, estimated_nodes * 2)
        .map_err(|e| format!("Failed to create AST arena: {}", e))?;

    let mut parser = CParser::new(arena);
    let ast_root = parser
        .parse_tokens(tokens)
        .map_err(|e| format!("Parse error at line {}, column {}: {}", e.line, e.column, e.message))?;

    let context = inkwell::context::Context::create();
    let module_name = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("input");
    let type_system = TypeSystem::new();
    let mut backend = LlvmBackend::with_types(&context, module_name, &type_system);

    backend
        .compile(&parser.arena, ast_root)
        .map_err(|e| format!("Backend compilation error: {}", e))?;

    if opt_level > 0 {
        backend
            .optimize(opt_level)
            .map_err(|e| format!("Optimization error: {}", e))?;
    }

    let _ = backend.verify();

    let ir = backend.dump_ir();

    let mut file = fs::File::create(output_path)
        .map_err(|e| format!("Failed to create output file '{}': {}", output_path.display(), e))?;

    file.write_all(ir.as_bytes())
        .map_err(|e| format!("Failed to write output file: {}", e))?;

    let _ = std::fs::remove_file(&arena_path);
    let _ = std::fs::remove_file(&db_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_config_creation() {
        let config = BuildConfig::new();
        assert!(config.source_files.is_empty());
        assert_eq!(config.output, PathBuf::from("a.out"));
        assert_eq!(config.jobs, 1);
        assert_eq!(config.optimization, 0);
        assert!(config.include_paths.is_empty());
        assert!(config.defines.is_empty());
        assert!(config.link_libs.is_empty());
    }

    #[test]
    fn test_build_config_builder_pattern() {
        let mut defines = HashMap::new();
        defines.insert("DEBUG".to_string(), "1".to_string());

        let config = BuildConfig::new()
            .with_source_files(vec![PathBuf::from("test.c")])
            .with_output(PathBuf::from("test.o"))
            .with_output_type(OutputType::Object)
            .with_jobs(4)
            .with_optimization(2)
            .with_include_paths(vec![PathBuf::from("/usr/include")])
            .with_defines(defines.clone())
            .with_link_libs(vec!["m".to_string()]);

        assert_eq!(config.source_files.len(), 1);
        assert_eq!(config.output, PathBuf::from("test.o"));
        assert_eq!(config.output_type, OutputType::Object);
        assert_eq!(config.jobs, 4);
        assert_eq!(config.optimization, 2);
        assert_eq!(config.include_paths.len(), 1);
        assert_eq!(config.defines.get("DEBUG"), Some(&"1".to_string()));
        assert_eq!(config.link_libs, vec!["m".to_string()]);
    }

    #[test]
    fn test_output_type_from_extension() {
        assert_eq!(OutputType::from_extension(Path::new("foo.o")), OutputType::Object);
        assert_eq!(OutputType::from_extension(Path::new("foo.a")), OutputType::StaticLib);
        assert_eq!(OutputType::from_extension(Path::new("foo.so")), OutputType::SharedLib);
        assert_eq!(OutputType::from_extension(Path::new("foo")), OutputType::Executable);
        assert_eq!(OutputType::from_extension(Path::new("foo.exe")), OutputType::Executable);
    }

    #[test]
    fn test_output_type_from_str() {
        assert_eq!(OutputType::from_str("object"), Some(OutputType::Object));
        assert_eq!(OutputType::from_str("static"), Some(OutputType::StaticLib));
        assert_eq!(OutputType::from_str("shared"), Some(OutputType::SharedLib));
        assert_eq!(OutputType::from_str("executable"), Some(OutputType::Executable));
        assert_eq!(OutputType::from_str("auto"), None);
        assert_eq!(OutputType::from_str("unknown"), None);
    }

    #[test]
    fn test_source_file_discovery() {
        let temp_dir = std::env::temp_dir().join(format!("optic_test_{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();

        fs::write(temp_dir.join("a.c"), "int a() { return 1; }").unwrap();
        fs::write(temp_dir.join("b.c"), "int b() { return 2; }").unwrap();
        fs::write(temp_dir.join("header.h"), "#ifndef H\n#define H\n#endif").unwrap();
        fs::write(temp_dir.join("readme.txt"), "not a c file").unwrap();

        let files = BuildConfig::discover_source_files(&temp_dir);
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.file_name().unwrap() == "a.c"));
        assert!(files.iter().any(|f| f.file_name().unwrap() == "b.c"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_config_validate_no_source_files() {
        let config = BuildConfig::new();
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BuildError::NoSourceFiles));
    }

    #[test]
    fn test_config_validate_missing_file() {
        let config = BuildConfig::new()
            .with_source_files(vec![PathBuf::from("/nonexistent/file.c")]);
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BuildError::CompileError(_, _)));
    }

    #[test]
    fn test_compiler_flags_generation() {
        let mut defines = HashMap::new();
        defines.insert("DEBUG".to_string(), "1".to_string());
        defines.insert("VERSION".to_string(), "2".to_string());

        let config = BuildConfig::new()
            .with_include_paths(vec![
                PathBuf::from("/usr/include"),
                PathBuf::from("./include"),
            ])
            .with_defines(defines);

        let flags = config.compiler_flags();
        assert!(flags.contains(&"-I/usr/include".to_string()));
        assert!(flags.contains(&"-I./include".to_string()));
        assert!(flags.iter().any(|f| f.starts_with("-DDEBUG=")));
        assert!(flags.iter().any(|f| f.starts_with("-DVERSION=")));
    }

    #[test]
    fn test_cache_key_generation() {
        let key1 = CacheKey::new("int main() { return 0; }", &["-O1".to_string()]);
        let key2 = CacheKey::new("int main() { return 0; }", &["-O1".to_string()]);
        let key3 = CacheKey::new("int main() { return 1; }", &["-O1".to_string()]);
        let key4 = CacheKey::new("int main() { return 0; }", &["-O2".to_string()]);

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);
    }

    #[test]
    fn test_cache_key_hash_consistency() {
        use std::collections::HashSet;
        let key = CacheKey::new("source", &["flag1".to_string(), "flag2".to_string()]);
        let mut set = HashSet::new();
        set.insert(key.clone());
        assert!(set.contains(&key));
    }

    #[test]
    fn test_build_error_display() {
        let err = BuildError::CompileError("test.c".to_string(), "syntax error".to_string());
        assert!(err.to_string().contains("test.c"));
        assert!(err.to_string().contains("syntax error"));

        let err = BuildError::LinkError("undefined reference".to_string());
        assert!(err.to_string().contains("link error"));

        let err = BuildError::NoSourceFiles;
        assert!(err.to_string().contains("no source files"));

        let err = BuildError::ExternalToolError {
            tool: "llc".to_string(),
            message: "not found".to_string(),
        };
        assert!(err.to_string().contains("llc"));
    }

    #[test]
    fn test_build_error_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let build_err: BuildError = io_err.into();
        assert!(matches!(build_err, BuildError::IoError(_)));
    }

    #[test]
    fn test_builder_creation() {
        let config = BuildConfig::new();
        let builder = Builder::new(config);
        assert!(builder.object_files().is_empty());
        assert!(builder.temp_dir().exists() || builder.temp_dir().parent().is_some());
    }

    #[test]
    fn test_builder_build_fails_without_source_files() {
        let config = BuildConfig::new();
        let mut builder = Builder::new(config);
        let result = builder.build();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BuildError::NoSourceFiles));
    }

    #[test]
    fn test_builder_build_fails_with_nonexistent_file() {
        let config = BuildConfig::new()
            .with_source_files(vec![PathBuf::from("/nonexistent/test.c")])
            .with_output(PathBuf::from("/tmp/test_output"));
        let mut builder = Builder::new(config);
        let result = builder.build();
        assert!(result.is_err());
    }

    #[test]
    fn test_find_tool() {
        let result = find_tool(&["ls", "cat"]);
        assert!(result.is_ok());

        let result = find_tool(&["nonexistent_tool_xyz_123"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_external_success() {
        let config = BuildConfig::new();
        let builder = Builder::new(config);
        let result = builder.run_external("echo", &["hello"]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_external_failure() {
        let config = BuildConfig::new();
        let builder = Builder::new(config);
        let result = builder.run_external("false", &[]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BuildError::ExternalToolError { .. }));
    }

    #[test]
    fn test_parallel_job_count() {
        let config = BuildConfig::new().with_jobs(8);
        assert_eq!(config.jobs, 8);

        let config = BuildConfig::new().with_jobs(0);
        assert_eq!(config.jobs, 1);
    }

    #[test]
    fn test_define_handling() {
        let mut defines = HashMap::new();
        defines.insert("FOO".to_string(), "bar".to_string());
        defines.insert("NUM".to_string(), "42".to_string());

        let config = BuildConfig::new().with_defines(defines);
        let flags = config.compiler_flags();

        assert!(flags.contains(&"-DFOO=bar".to_string()));
        assert!(flags.contains(&"-DNUM=42".to_string()));
    }

    #[test]
    fn test_include_path_handling() {
        let config = BuildConfig::new().with_include_paths(vec![
            PathBuf::from("/opt/include"),
            PathBuf::from("./headers"),
        ]);

        let flags = config.compiler_flags();
        assert!(flags.contains(&"-I/opt/include".to_string()));
        assert!(flags.contains(&"-I./headers".to_string()));
    }

    #[test]
    fn test_output_type_auto_detection() {
        let output = PathBuf::from("build/libfoo.so");
        assert_eq!(OutputType::from_extension(&output), OutputType::SharedLib);

        let output = PathBuf::from("build/libfoo.a");
        assert_eq!(OutputType::from_extension(&output), OutputType::StaticLib);

        let output = PathBuf::from("build/myapp");
        assert_eq!(OutputType::from_extension(&output), OutputType::Executable);
    }
}
