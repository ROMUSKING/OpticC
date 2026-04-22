use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Instant;

use rayon::{prelude::*, ThreadPoolBuilder};

use crate::arena::Arena;
use crate::backend::llvm::LlvmBackend;
use crate::db::OpticDb;
use crate::frontend::parser::Parser as CParser;
use crate::frontend::preprocessor::Preprocessor;
use crate::types::TypeSystem;

const LARGE_COMPILER_STACK_SIZE: usize = 64 * 1024 * 1024;
const LARGE_STACK_INPUT_THRESHOLD_BYTES: u64 = 512 * 1024;
const CACHE_SCHEMA_VERSION: &str = "v5-lvalue-array-index-fix";

static COMPILE_INVOCATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompilePhaseTimings {
    pub preprocess_ms: u64,
    pub parse_ms: u64,
    pub codegen_ms: u64,
    pub optimize_ms: u64,
    pub ir_write_ms: u64,
    pub llc_ms: u64,
}

impl CompilePhaseTimings {
    pub fn total_ms(&self) -> u64 {
        self.preprocess_ms
            + self.parse_ms
            + self.codegen_ms
            + self.optimize_ms
            + self.ir_write_ms
            + self.llc_ms
    }
}

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
        use sha2::{Digest, Sha256};
        let mut h1 = Sha256::new();
        h1.update(source.as_bytes());
        let source_hash: [u8; 32] = h1.finalize().into();

        let mut h2 = Sha256::new();
        for f in flags {
            h2.update(f.as_bytes());
        }
        let flags_hash: [u8; 32] = h2.finalize().into();

        CacheKey {
            source_hash,
            flags_hash,
        }
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

fn cache_key_hex(key: &CacheKey) -> String {
    key.source_hash
        .iter()
        .chain(key.flags_hash.iter())
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

fn opticc_cache_dir() -> PathBuf {
    std::env::temp_dir().join("opticc-cache")
}

fn build_cache_key(
    source: &Path,
    include_paths: &[PathBuf],
    force_includes: &[PathBuf],
    defines: &HashMap<String, String>,
    optimization: u32,
    nostdinc: bool,
    return_thunk_extern: bool,
) -> Result<CacheKey, BuildError> {
    let source_text = fs::read_to_string(source)
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;

    let mut flags: Vec<String> = include_paths
        .iter()
        .map(|path| format!("-I{}", path.display()))
        .collect();
    flags.extend(
        force_includes
            .iter()
            .map(|path| format!("-include{}", path.display())),
    );

    let mut define_entries: Vec<_> = defines.iter().collect();
    define_entries.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in define_entries {
        flags.push(format!("-D{}={}", name, value));
    }
    flags.push(format!("-O{}", optimization));
    flags.push(format!("--cache-schema={}", CACHE_SCHEMA_VERSION));
    if nostdinc {
        flags.push("-nostdinc".to_string());
    }
    if return_thunk_extern {
        flags.push("-mfunction-return=thunk-extern".to_string());
    }

    Ok(CacheKey::new(&source_text, &flags))
}

fn rewrite_return_thunks_for_kernel(asm: &str) -> String {
    let mut rewritten = String::with_capacity(asm.len() + 64);
    for line in asm.lines() {
        let trimmed = line.trim();
        if matches!(trimmed, "ret" | "retq" | "retl") {
            let indent_len = line.len() - line.trim_start().len();
            rewritten.push_str(&line[..indent_len]);
            rewritten.push_str("jmp\t__x86_return_thunk\n");
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    rewritten
}

fn restore_cached_object(cache_key: &CacheKey, obj_path: &Path) -> Result<bool, BuildError> {
    let cached_object = opticc_cache_dir().join(format!("{}.o", cache_key_hex(cache_key)));
    if !cached_object.exists() {
        return Ok(false);
    }

    if let Some(parent) = obj_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&cached_object, obj_path)?;
    Ok(true)
}

fn store_cached_object(cache_key: &CacheKey, obj_path: &Path) -> Result<(), BuildError> {
    let cache_dir = opticc_cache_dir();
    fs::create_dir_all(&cache_dir)?;
    let cached_object = cache_dir.join(format!("{}.o", cache_key_hex(cache_key)));
    fs::copy(obj_path, cached_object)?;
    Ok(())
}

pub struct BuildConfig {
    pub source_files: Vec<PathBuf>,
    pub output: PathBuf,
    pub include_paths: Vec<PathBuf>,
    pub force_includes: Vec<PathBuf>,
    pub defines: HashMap<String, String>,
    pub link_libs: Vec<String>,
    pub jobs: usize,
    pub optimization: u32,
    pub output_type: OutputType,
    pub nostdinc: bool,
    pub return_thunk_extern: bool,
}

impl BuildConfig {
    pub fn new() -> Self {
        BuildConfig {
            source_files: Vec::new(),
            output: PathBuf::from("a.out"),
            include_paths: Vec::new(),
            force_includes: Vec::new(),
            defines: HashMap::new(),
            link_libs: Vec::new(),
            jobs: 1,
            optimization: 0,
            output_type: OutputType::Executable,
            nostdinc: false,
            return_thunk_extern: false,
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

    pub fn with_force_includes(mut self, headers: Vec<PathBuf>) -> Self {
        self.force_includes = headers;
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

    pub fn with_return_thunk_extern(mut self, enabled: bool) -> Self {
        self.return_thunk_extern = enabled;
        self
    }

    pub fn with_nostdinc(mut self, enabled: bool) -> Self {
        self.nostdinc = enabled;
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
        if self.return_thunk_extern {
            flags.push("-mfunction-return=thunk-extern".to_string());
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
        let build_id = COMPILE_INVOCATION_ID.fetch_add(1, Ordering::Relaxed);
        let temp_dir = PathBuf::from(format!(
            "/tmp/opticc_build_{}_{}",
            std::process::id(),
            build_id
        ));
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
        let force_includes = self.config.force_includes.clone();
        let defines = self.config.defines.clone();
        let optimization = self.config.optimization;
        let nostdinc = self.config.nostdinc;
        let return_thunk_extern = self.config.return_thunk_extern;
        let source_files = self.config.source_files.clone();

        fs::create_dir_all(&temp_dir)?;

        let pool = ThreadPoolBuilder::new()
            .num_threads(self.config.jobs)
            .stack_size(LARGE_COMPILER_STACK_SIZE)
            .build()
            .map_err(|e| BuildError::ExternalToolError {
                tool: "rayon".to_string(),
                message: e.to_string(),
            })?;

        let results: Vec<Result<PathBuf, BuildError>> = pool.install(|| {
            source_files
                .par_iter()
                .map(|source| {
                    compile_file_to_object_impl(
                        source,
                        &temp_dir,
                        &include_paths,
                        &force_includes,
                        &defines,
                        optimization,
                        nostdinc,
                        return_thunk_extern,
                    )
                })
                .collect()
        });

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
            &self.config.force_includes,
            &self.config.defines,
            self.config.optimization,
            self.config.nostdinc,
            self.config.return_thunk_extern,
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
            return Err(BuildError::LinkError(
                "no object files to archive".to_string(),
            ));
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
    force_includes: &[PathBuf],
    defines: &HashMap<String, String>,
    optimization: u32,
    nostdinc: bool,
    return_thunk_extern: bool,
) -> Result<PathBuf, BuildError> {
    if !should_use_large_stack(source) {
        return compile_file_to_object_impl(
            source,
            temp_dir,
            include_paths,
            force_includes,
            defines,
            optimization,
            nostdinc,
            return_thunk_extern,
        );
    }

    let source = source.to_path_buf();
    let temp_dir = temp_dir.to_path_buf();
    let include_paths = include_paths.to_vec();
    let force_includes = force_includes.to_vec();
    let defines = defines.clone();
    let source_name = source.display().to_string();

    thread::Builder::new()
        .name(format!(
            "optic-compile-{}",
            source
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file")
        ))
        .stack_size(LARGE_COMPILER_STACK_SIZE)
        .spawn(move || {
            compile_file_to_object_impl(
                &source,
                &temp_dir,
                &include_paths,
                &force_includes,
                &defines,
                optimization,
                nostdinc,
                return_thunk_extern,
            )
        })
        .map_err(BuildError::IoError)?
        .join()
        .unwrap_or_else(|_| {
            Err(BuildError::CompileError(
                source_name,
                "compiler worker panicked".to_string(),
            ))
        })
}

fn compile_file_to_object_impl(
    source: &Path,
    temp_dir: &Path,
    include_paths: &[PathBuf],
    force_includes: &[PathBuf],
    defines: &HashMap<String, String>,
    optimization: u32,
    nostdinc: bool,
    return_thunk_extern: bool,
) -> Result<PathBuf, BuildError> {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let ll_path = temp_dir.join(format!("{}.ll", stem));
    let obj_path = temp_dir.join(format!("{}.o", stem));

    compile_source_to_object_with_stats_impl(
        source,
        &ll_path,
        &obj_path,
        include_paths,
        force_includes,
        defines,
        optimization,
        nostdinc,
        return_thunk_extern,
    )?;

    Ok(obj_path)
}

pub fn compile_single_file(
    input_path: &Path,
    output_path: &Path,
    opt_level: u32,
    include_paths: &[PathBuf],
    defines: &HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !should_use_large_stack(input_path) {
        return compile_single_file_impl(
            input_path,
            output_path,
            opt_level,
            include_paths,
            defines,
        )
        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(std::io::Error::other(e)) });
    }

    let input_path = input_path.to_path_buf();
    let output_path = output_path.to_path_buf();
    let include_paths = include_paths.to_vec();
    let defines = defines.clone();
    let input_name = input_path.display().to_string();

    thread::Builder::new()
        .name(format!(
            "optic-single-{}",
            input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("input")
        ))
        .stack_size(LARGE_COMPILER_STACK_SIZE)
        .spawn(move || {
            compile_single_file_impl(
                &input_path,
                &output_path,
                opt_level,
                &include_paths,
                &defines,
            )
        })
        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?
        .join()
        .unwrap_or_else(|_| {
            Err(format!(
                "Compilation worker panicked while compiling {}",
                input_name
            ))
        })
        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(std::io::Error::other(e)) })
}

fn compile_single_file_impl(
    input_path: &Path,
    output_path: &Path,
    opt_level: u32,
    include_paths: &[PathBuf],
    defines: &HashMap<String, String>,
) -> Result<(), String> {
    let compile_id = next_compile_invocation_id();
    let artifacts = compile_to_ir_artifacts(
        input_path,
        opt_level,
        include_paths,
        &[],
        defines,
        &format!("/tmp/optic_db_{}_{}.redb", std::process::id(), compile_id),
        &format!(
            "/tmp/optic_c_arena_{}_{}.bin",
            std::process::id(),
            compile_id
        ),
        false,
    )?;

    let start = Instant::now();
    let mut file = fs::File::create(output_path).map_err(|e| {
        format!(
            "Failed to create output file '{}': {}",
            output_path.display(),
            e
        )
    })?;

    file.write_all(artifacts.ir.as_bytes())
        .map_err(|e| format!("Failed to write output file: {}", e))?;
    let _ = start.elapsed();

    Ok(())
}

pub fn clear_compile_cache() -> Result<(), BuildError> {
    let cache_dir = opticc_cache_dir();
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

pub fn compile_source_to_object_with_stats(
    input_path: &Path,
    output_path: &Path,
    opt_level: u32,
    include_paths: &[PathBuf],
    defines: &HashMap<String, String>,
) -> Result<CompilePhaseTimings, BuildError> {
    let compile_id = next_compile_invocation_id();
    let ll_path = std::env::temp_dir().join(format!(
        "opticc_bench_{}_{}.ll",
        std::process::id(),
        compile_id
    ));

    if !should_use_large_stack(input_path) {
        return compile_source_to_object_with_stats_impl(
            input_path,
            &ll_path,
            output_path,
            include_paths,
            &[],
            defines,
            opt_level,
            false,
            false,
        );
    }

    let input_path = input_path.to_path_buf();
    let output_path = output_path.to_path_buf();
    let ll_path_clone = ll_path.clone();
    let include_paths = include_paths.to_vec();
    let defines = defines.clone();
    let input_name = input_path.display().to_string();

    thread::Builder::new()
        .name(format!(
            "optic-object-{}",
            input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("input")
        ))
        .stack_size(LARGE_COMPILER_STACK_SIZE)
        .spawn(move || {
            compile_source_to_object_with_stats_impl(
                &input_path,
                &ll_path_clone,
                &output_path,
                &include_paths,
                &[],
                &defines,
                opt_level,
                false,
                false,
            )
        })
        .map_err(BuildError::IoError)?
        .join()
        .unwrap_or_else(|_| {
            Err(BuildError::CompileError(
                input_name,
                "compiler worker panicked".to_string(),
            ))
        })
}

struct CompileArtifacts {
    ir: String,
    timings: CompilePhaseTimings,
}

fn compile_source_to_object_with_stats_impl(
    source: &Path,
    ll_path: &Path,
    obj_path: &Path,
    include_paths: &[PathBuf],
    force_includes: &[PathBuf],
    defines: &HashMap<String, String>,
    optimization: u32,
    nostdinc: bool,
    return_thunk_extern: bool,
) -> Result<CompilePhaseTimings, BuildError> {
    let cache_key = build_cache_key(
        source,
        include_paths,
        force_includes,
        defines,
        optimization,
        nostdinc,
        return_thunk_extern,
    )?;
    if restore_cached_object(&cache_key, obj_path)? {
        return Ok(CompilePhaseTimings::default());
    }

    let compile_id = next_compile_invocation_id();
    let artifacts = compile_to_ir_artifacts(
        source,
        optimization,
        include_paths,
        force_includes,
        defines,
        &format!(
            "/tmp/optic_db_build_{}_{}.redb",
            std::process::id(),
            compile_id
        ),
        &format!(
            "/tmp/optic_c_arena_build_{}_{}.bin",
            std::process::id(),
            compile_id
        ),
        nostdinc,
    )
    .map_err(|e| BuildError::CompileError(source.display().to_string(), e))?;
    let mut timings = artifacts.timings;

    let ir_start = Instant::now();
    let mut file = fs::File::create(ll_path)
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;
    file.write_all(artifacts.ir.as_bytes())
        .map_err(|e| BuildError::CompileError(source.display().to_string(), e.to_string()))?;
    timings.ir_write_ms = ir_start.elapsed().as_millis() as u64;

    // Debug: always save a copy of the IR for inspection
    let _ = fs::copy(ll_path, "/tmp/optic_last_ir.ll");

    let llc = find_tool(&["llc-18", "llc", "llc-17", "llc-16"])?;
    let llc_start = Instant::now();
    let llc_output = if return_thunk_extern {
        let asm_path = obj_path.with_extension("s");
        let asm_output = Command::new(&llc)
            .arg("-relocation-model=pic")
            .arg("-filetype=asm")
            .arg("-o")
            .arg(&asm_path)
            .arg(ll_path)
            .output()
            .map_err(BuildError::IoError)?;

        if asm_output.status.success() {
            let asm = fs::read_to_string(&asm_path).map_err(BuildError::IoError)?;
            // Debug: save pre-rewrite assembly
            if let Ok(debug_asm) = std::env::var("OPTIC_DEBUG_ASM") {
                let _ = fs::write(&debug_asm, &asm);
            }
            let rewritten = rewrite_return_thunks_for_kernel(&asm);
            // Debug: save post-rewrite assembly
            if let Ok(debug_asm_post) = std::env::var("OPTIC_DEBUG_ASM_POST") {
                let _ = fs::write(&debug_asm_post, &rewritten);
            }
            fs::write(&asm_path, rewritten).map_err(BuildError::IoError)?;
            let cc = find_tool(&["clang", "gcc"])?;
            let assembled = Command::new(&cc)
                .arg("-c")
                .arg("-x")
                .arg("assembler")
                .arg("-o")
                .arg(obj_path)
                .arg(&asm_path)
                .output()
                .map_err(BuildError::IoError)?;
            let _ = fs::remove_file(&asm_path);
            assembled
        } else {
            asm_output
        }
    } else {
        Command::new(&llc)
            .arg("-relocation-model=pic")
            .arg("-filetype=obj")
            .arg("-o")
            .arg(obj_path)
            .arg(ll_path)
            .output()
            .map_err(BuildError::IoError)?
    };
    timings.llc_ms = llc_start.elapsed().as_millis() as u64;

    let _ = fs::remove_file(ll_path);

    if !llc_output.status.success() {
        let stderr = String::from_utf8_lossy(&llc_output.stderr);
        return Err(BuildError::ExternalToolError {
            tool: "llc".to_string(),
            message: stderr.to_string(),
        });
    }

    store_cached_object(&cache_key, obj_path)?;

    Ok(timings)
}

fn resolve_force_include_path(
    input_path: &Path,
    header: &Path,
    include_paths: &[PathBuf],
) -> PathBuf {
    if header.is_absolute() {
        return header.to_path_buf();
    }

    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join(header);
        if candidate.exists() {
            return candidate;
        }
    }

    if let Some(parent) = input_path.parent() {
        let candidate = parent.join(header);
        if candidate.exists() {
            return candidate;
        }
    }

    for base in include_paths {
        let candidate = base.join(header);
        if candidate.exists() {
            return candidate;
        }
    }

    header.to_path_buf()
}

fn compile_to_ir_artifacts(
    input_path: &Path,
    opt_level: u32,
    include_paths: &[PathBuf],
    force_includes: &[PathBuf],
    defines: &HashMap<String, String>,
    db_path: &str,
    arena_path: &str,
    nostdinc: bool,
) -> Result<CompileArtifacts, String> {
    let result = (|| {
        let db = OpticDb::new(db_path).map_err(|e| format!("Failed to create database: {}", e))?;

        let mut pp = Preprocessor::new(db);
        if nostdinc {
            pp.disable_default_include_paths();
        }

        for path in include_paths {
            pp.add_include_path(path.to_str().unwrap_or(""));
        }
        for (name, value) in defines {
            pp.define_macro(name, value);
        }

        let preprocess_start = Instant::now();
        let tokens = if force_includes.is_empty() {
            pp.process(input_path.to_str().unwrap())
                .map_err(|e| format!("Preprocessor error: {}", e))?
        } else {
            let source_text = fs::read_to_string(input_path)
                .map_err(|e| format!("Failed to read source file '{}': {}", input_path.display(), e))?;
            let mut prefixed_source = String::new();
            for header in force_includes {
                let resolved = resolve_force_include_path(input_path, header, include_paths);
                prefixed_source.push_str(&format!("#include \"{}\"\n", resolved.display()));
            }
            prefixed_source.push_str(&source_text);
            pp.process_source(&prefixed_source, input_path.to_str().unwrap())
                .map_err(|e| format!("Preprocessor error: {}", e))?
        };
        let preprocess_ms = preprocess_start.elapsed().as_millis() as u64;

        // Debug: save preprocessed tokens for inspection
        {
            let token_dump: String = tokens.iter().map(|t| format!("{} ", t.text)).collect();
            let _ = fs::write("/tmp/optic_last_tokens.txt", &token_dump);
        }

        let estimated_nodes = (tokens.len() / 2).max(1024) as u32;

        let arena = Arena::new(arena_path, estimated_nodes * 2)
            .map_err(|e| format!("Failed to create AST arena: {}", e))?;

        let parse_start = Instant::now();
        let mut parser = CParser::new(arena);
        let ast_root = parser.parse_tokens(tokens).map_err(|e| {
            format!(
                "Parse error at line {}, column {}: {}",
                e.line, e.column, e.message
            )
        })?;
        let parse_ms = parse_start.elapsed().as_millis() as u64;

        let context = inkwell::context::Context::create();
        let module_name = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("input");
        let type_system = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, module_name, &type_system);

        let codegen_start = Instant::now();
        backend
            .compile(&parser.arena, ast_root)
            .map_err(|e| format!("Backend compilation error: {}", e))?;
        let codegen_ms = codegen_start.elapsed().as_millis() as u64;

        let optimize_start = Instant::now();
        if opt_level > 0 {
            backend
                .optimize(opt_level)
                .map_err(|e| format!("Optimization error: {}", e))?;
        }
        let optimize_ms = optimize_start.elapsed().as_millis() as u64;

        let _ = backend.verify();

        Ok(CompileArtifacts {
            ir: backend.dump_ir(),
            timings: CompilePhaseTimings {
                preprocess_ms,
                parse_ms,
                codegen_ms,
                optimize_ms,
                ir_write_ms: 0,
                llc_ms: 0,
            },
        })
    })();

    let _ = fs::remove_file(arena_path);
    let _ = fs::remove_file(db_path);

    result
}

fn should_use_large_stack(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.len() >= LARGE_STACK_INPUT_THRESHOLD_BYTES)
        .unwrap_or(true)
}

fn next_compile_invocation_id() -> u64 {
    COMPILE_INVOCATION_ID.fetch_add(1, Ordering::Relaxed)
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
            .with_force_includes(vec![PathBuf::from("force.h")])
            .with_defines(defines.clone())
            .with_link_libs(vec!["m".to_string()]);

        assert_eq!(config.source_files.len(), 1);
        assert_eq!(config.output, PathBuf::from("test.o"));
        assert_eq!(config.output_type, OutputType::Object);
        assert_eq!(config.jobs, 4);
        assert_eq!(config.optimization, 2);
        assert_eq!(config.include_paths.len(), 1);
        assert_eq!(config.force_includes, vec![PathBuf::from("force.h")]);
        assert_eq!(config.defines.get("DEBUG"), Some(&"1".to_string()));
        assert_eq!(config.link_libs, vec!["m".to_string()]);
    }

    #[test]
    fn test_output_type_from_extension() {
        assert_eq!(
            OutputType::from_extension(Path::new("foo.o")),
            OutputType::Object
        );
        assert_eq!(
            OutputType::from_extension(Path::new("foo.a")),
            OutputType::StaticLib
        );
        assert_eq!(
            OutputType::from_extension(Path::new("foo.so")),
            OutputType::SharedLib
        );
        assert_eq!(
            OutputType::from_extension(Path::new("foo")),
            OutputType::Executable
        );
        assert_eq!(
            OutputType::from_extension(Path::new("foo.exe")),
            OutputType::Executable
        );
    }

    #[test]
    fn test_output_type_from_str() {
        assert_eq!(OutputType::from_str("object"), Some(OutputType::Object));
        assert_eq!(OutputType::from_str("static"), Some(OutputType::StaticLib));
        assert_eq!(OutputType::from_str("shared"), Some(OutputType::SharedLib));
        assert_eq!(
            OutputType::from_str("executable"),
            Some(OutputType::Executable)
        );
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
        let config =
            BuildConfig::new().with_source_files(vec![PathBuf::from("/nonexistent/file.c")]);
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            BuildError::CompileError(_, _)
        ));
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
    fn test_rewrite_return_thunks_for_kernel_rethunk() {
        let input = "hello:\n\txorl\t%eax, %eax\n\tretq\nworld:\n\tret\n";
        let rewritten = rewrite_return_thunks_for_kernel(input);
        assert!(!rewritten.contains("\tretq\n"), "bare retq should be removed: {rewritten}");
        assert!(!rewritten.contains("\tret\n"), "bare ret should be removed: {rewritten}");
        assert!(rewritten.contains("\tjmp\t__x86_return_thunk\n"), "rethunk jump missing: {rewritten}");
    }

    #[test]
    fn test_build_config_return_thunk_builder() {
        let config = BuildConfig::new().with_return_thunk_extern(true);
        assert!(config.return_thunk_extern);
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
    fn test_force_include_header_is_applied_before_source() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let source_path = temp_dir.path().join("main.c");
        let header_path = temp_dir.path().join("force.h");
        let missing_db = temp_dir.path().join("missing.redb");
        let missing_arena = temp_dir.path().join("missing.bin");
        let ok_db = temp_dir.path().join("ok.redb");
        let ok_arena = temp_dir.path().join("ok.bin");

        fs::write(&header_path, "#define FORCE_MAGIC 42\n").unwrap();
        fs::write(
            &source_path,
            "#ifndef FORCE_MAGIC\n#error missing_force_include\n#endif\nint value(void) { return FORCE_MAGIC; }\n",
        )
        .unwrap();

        let without_force = compile_to_ir_artifacts(
            &source_path,
            0,
            &[temp_dir.path().to_path_buf()],
            &[],
            &HashMap::new(),
            missing_db.to_str().unwrap(),
            missing_arena.to_str().unwrap(),
            false,
        );
        assert!(without_force.is_err());
        let without_force_err = without_force.err().unwrap();
        assert!(without_force_err.contains("missing_force_include"));

        let with_force = compile_to_ir_artifacts(
            &source_path,
            0,
            &[temp_dir.path().to_path_buf()],
            &[PathBuf::from("force.h")],
            &HashMap::new(),
            ok_db.to_str().unwrap(),
            ok_arena.to_str().unwrap(),
            false,
        );
        assert!(with_force.is_ok(), "force-include compilation should succeed");
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

    // ===== Multi-Translation-Unit (multi-TU) compilation tests =====

    /// Helper: compile C source string to LLVM IR via the same pipeline used by the build system.
    /// Returns (ir_string, timings).
    fn compile_source_string_to_ir(
        source: &str,
        module_name: &str,
    ) -> Result<String, String> {
        use crate::arena::Arena;
        use crate::backend::llvm::LlvmBackend;
        use crate::frontend::parser::Parser as CParser;
        use crate::types::TypeSystem;

        let arena_path = std::env::temp_dir().join(format!(
            "optic_test_arena_{}_{}_{}.bin",
            std::process::id(),
            module_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let arena = Arena::new(&arena_path, 65536)
            .map_err(|e| format!("Arena creation failed: {}", e))?;
        let mut parser = CParser::new(arena);
        let root = parser
            .parse(source)
            .map_err(|e| format!("Parse error: {} (line {}, col {})", e.message, e.line, e.column))?;

        let context = inkwell::context::Context::create();
        let ts = TypeSystem::new();
        let mut backend = LlvmBackend::with_types(&context, module_name, &ts);
        backend
            .compile(&parser.arena, root)
            .map_err(|e| format!("Backend error: {}", e))?;
        backend
            .verify()
            .map_err(|e| format!("LLVM verification failed: {}", e))?;

        let ir = backend.dump_ir();
        let _ = fs::remove_file(&arena_path);
        Ok(ir)
    }

    #[test]
    fn test_multi_tu_ir_generation_helper() {
        // Compile helper.c (defines add function) and verify IR
        let ir = compile_source_string_to_ir(
            "int add(int a, int b) { return a + b; }",
            "helper",
        )
        .expect("helper.c compilation failed");

        assert!(
            ir.contains("define i32 @add("),
            "Helper TU should define add:\n{}",
            ir
        );
        assert!(
            !ir.contains("declare i32 @add"),
            "Helper TU should NOT declare add (it defines it):\n{}",
            ir
        );
    }

    #[test]
    fn test_multi_tu_ir_generation_main() {
        // Compile main.c (extern decl + call) and verify IR
        let ir = compile_source_string_to_ir(
            "extern int add(int a, int b); \
             int main(void) { return add(3, 4); }",
            "main",
        )
        .expect("main.c compilation failed");

        assert!(
            ir.contains("declare i32 @add("),
            "Main TU should declare add (extern):\n{}",
            ir
        );
        assert!(
            ir.contains("define i32 @main"),
            "Main TU should define main:\n{}",
            ir
        );
        assert!(
            !ir.contains("define i32 @add"),
            "Main TU should NOT define add:\n{}",
            ir
        );
        assert!(
            ir.contains("call i32 @add("),
            "Main TU should call add:\n{}",
            ir
        );
    }

    #[test]
    fn test_multi_tu_both_ir_verify() {
        // Verify that both TUs independently produce valid LLVM IR
        let helper_ir = compile_source_string_to_ir(
            "int add(int a, int b) { return a + b; }",
            "helper",
        );
        assert!(helper_ir.is_ok(), "Helper TU should compile & verify: {:?}", helper_ir.err());

        let main_ir = compile_source_string_to_ir(
            "extern int add(int a, int b); \
             int main(void) { return add(3, 4); }",
            "main",
        );
        assert!(main_ir.is_ok(), "Main TU should compile & verify: {:?}", main_ir.err());
    }

    #[test]
    fn test_multi_tu_file_based_ir_compilation() {
        // Test the file-based compile_to_ir_artifacts function with two real files
        let test_id = format!(
            "multi_tu_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let temp_dir = std::env::temp_dir().join(&test_id);
        fs::create_dir_all(&temp_dir).unwrap();

        // Write helper.c
        let helper_path = temp_dir.join("helper.c");
        fs::write(
            &helper_path,
            "int add(int a, int b) { return a + b; }\n",
        )
        .unwrap();

        // Write main.c
        let main_path = temp_dir.join("main.c");
        fs::write(
            &main_path,
            "extern int add(int a, int b);\nint main(void) { return add(3, 4); }\n",
        )
        .unwrap();

        // Compile each file through the IR artifact pipeline
        let db_path1 = temp_dir.join("helper.redb").display().to_string();
        let arena_path1 = temp_dir.join("helper_arena.bin").display().to_string();
        let helper_result = compile_to_ir_artifacts(
            &helper_path,
            0,
            &[],
            &[],
            &HashMap::new(),
            &db_path1,
            &arena_path1,
            false,
        );
        assert!(
            helper_result.is_ok(),
            "helper.c compile_to_ir_artifacts failed: {:?}",
            helper_result.err()
        );
        let helper_ir = helper_result.unwrap().ir;
        assert!(
            helper_ir.contains("define i32 @add("),
            "helper.c IR should define add:\n{}",
            helper_ir
        );

        let db_path2 = temp_dir.join("main.redb").display().to_string();
        let arena_path2 = temp_dir.join("main_arena.bin").display().to_string();
        let main_result = compile_to_ir_artifacts(
            &main_path,
            0,
            &[],
            &[],
            &HashMap::new(),
            &db_path2,
            &arena_path2,
            false,
        );
        assert!(
            main_result.is_ok(),
            "main.c compile_to_ir_artifacts failed: {:?}",
            main_result.err()
        );
        let main_ir = main_result.unwrap().ir;
        assert!(
            main_ir.contains("declare i32 @add("),
            "main.c IR should declare add:\n{}",
            main_ir
        );
        assert!(
            main_ir.contains("define i32 @main"),
            "main.c IR should define main:\n{}",
            main_ir
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_multi_tu_build_config_with_multiple_sources() {
        // Test BuildConfig with multiple source files validates correctly
        let test_id = format!(
            "multi_tu_cfg_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let temp_dir = std::env::temp_dir().join(&test_id);
        fs::create_dir_all(&temp_dir).unwrap();

        let helper_path = temp_dir.join("helper.c");
        fs::write(&helper_path, "int add(int a, int b) { return a + b; }\n").unwrap();

        let main_path = temp_dir.join("main.c");
        fs::write(
            &main_path,
            "extern int add(int a, int b);\nint main(void) { return add(3, 4); }\n",
        )
        .unwrap();

        // Validate the config accepts both files
        let config = BuildConfig::new()
            .with_source_files(vec![helper_path.clone(), main_path.clone()])
            .with_output(temp_dir.join("test_program"));
        assert!(config.validate().is_ok(), "Config validation should pass for existing files");

        // Verify source discovery finds both
        let discovered = BuildConfig::discover_source_files(&temp_dir);
        assert_eq!(discovered.len(), 2, "Should discover 2 .c files");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_multi_tu_object_compilation() {
        // Test compiling two C files to object files using compile_file_to_object_impl
        // This exercises the full frontend → LLVM → LLC pipeline per file
        let test_id = format!(
            "multi_tu_obj_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let temp_dir = std::env::temp_dir().join(&test_id);
        fs::create_dir_all(&temp_dir).unwrap();

        let helper_path = temp_dir.join("helper.c");
        fs::write(&helper_path, "int add(int a, int b) { return a + b; }\n").unwrap();

        let main_path = temp_dir.join("main.c");
        fs::write(
            &main_path,
            "extern int add(int a, int b);\nint main(void) { return add(3, 4); }\n",
        )
        .unwrap();

        // Skip if llc is not available
        if find_tool(&["llc-18", "llc", "llc-17", "llc-16"]).is_err() {
            eprintln!("Skipping object compilation test: llc not found");
            let _ = fs::remove_dir_all(&temp_dir);
            return;
        }

        let obj_dir = temp_dir.join("obj");
        fs::create_dir_all(&obj_dir).unwrap();

        // Compile helper.c → helper.o
        let helper_obj = compile_file_to_object_impl(
            &helper_path,
            &obj_dir,
            &[],
            &[],
            &HashMap::new(),
            0,
            false,
            false,
        );
        assert!(
            helper_obj.is_ok(),
            "helper.c object compilation failed: {:?}",
            helper_obj.err()
        );
        let helper_obj_path = helper_obj.unwrap();
        assert!(helper_obj_path.exists(), "helper.o should exist");
        assert!(
            fs::metadata(&helper_obj_path).unwrap().len() > 0,
            "helper.o should be non-empty"
        );

        // Compile main.c → main.o
        let main_obj = compile_file_to_object_impl(
            &main_path,
            &obj_dir,
            &[],
            &[],
            &HashMap::new(),
            0,
            false,
            false,
        );
        assert!(
            main_obj.is_ok(),
            "main.c object compilation failed: {:?}",
            main_obj.err()
        );
        let main_obj_path = main_obj.unwrap();
        assert!(main_obj_path.exists(), "main.o should exist");
        assert!(
            fs::metadata(&main_obj_path).unwrap().len() > 0,
            "main.o should be non-empty"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_multi_tu_full_build_pipeline() {
        // End-to-end: compile two C files and link into an executable using Builder
        let test_id = format!(
            "multi_tu_e2e_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let temp_dir = std::env::temp_dir().join(&test_id);
        fs::create_dir_all(&temp_dir).unwrap();

        let helper_path = temp_dir.join("helper.c");
        fs::write(&helper_path, "int add(int a, int b) { return a + b; }\n").unwrap();

        let main_path = temp_dir.join("main.c");
        fs::write(
            &main_path,
            "extern int add(int a, int b);\nint main(void) { return add(3, 4); }\n",
        )
        .unwrap();

        // Skip if required tools are not available
        if find_tool(&["llc-18", "llc", "llc-17", "llc-16"]).is_err() {
            eprintln!("Skipping full build pipeline test: llc not found");
            let _ = fs::remove_dir_all(&temp_dir);
            return;
        }
        if find_tool(&["clang", "gcc"]).is_err() {
            eprintln!("Skipping full build pipeline test: linker not found");
            let _ = fs::remove_dir_all(&temp_dir);
            return;
        }

        let output_path = temp_dir.join("test_program");
        let config = BuildConfig::new()
            .with_source_files(vec![helper_path, main_path])
            .with_output(output_path.clone())
            .with_output_type(OutputType::Executable);

        let mut builder = Builder::new(config);
        let result = builder.build();
        assert!(
            result.is_ok(),
            "Multi-TU build should succeed: {:?}",
            result.err()
        );
        assert!(output_path.exists(), "Linked executable should exist");
        assert!(
            fs::metadata(&output_path).unwrap().len() > 0,
            "Linked executable should be non-empty"
        );

        // Run the executable and verify exit code (add(3,4) = 7)
        let run_result = std::process::Command::new(&output_path).output();
        if let Ok(output) = run_result {
            assert_eq!(
                output.status.code(),
                Some(7),
                "Program should exit with add(3,4)=7, got {:?}",
                output.status.code()
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_multi_tu_three_files() {
        // Three-TU scenario: math.c, utils.c, main.c
        let math_ir = compile_source_string_to_ir(
            "int add(int a, int b) { return a + b; } \
             int sub(int a, int b) { return a - b; }",
            "math",
        )
        .expect("math.c compilation failed");

        let utils_ir = compile_source_string_to_ir(
            "extern int add(int a, int b); \
             int double_add(int a, int b) { return add(a, b) + add(a, b); }",
            "utils",
        )
        .expect("utils.c compilation failed");

        let main_ir = compile_source_string_to_ir(
            "extern int double_add(int a, int b); \
             extern int sub(int a, int b); \
             int main(void) { return double_add(2, 3) - sub(10, 5); }",
            "main",
        )
        .expect("main.c compilation failed");

        // math.c: defines add and sub
        assert!(math_ir.contains("define i32 @add("), "math.c should define add:\n{}", math_ir);
        assert!(math_ir.contains("define i32 @sub("), "math.c should define sub:\n{}", math_ir);

        // utils.c: declares add, defines double_add
        assert!(utils_ir.contains("declare i32 @add("), "utils.c should declare add:\n{}", utils_ir);
        assert!(
            utils_ir.contains("define i32 @double_add("),
            "utils.c should define double_add:\n{}",
            utils_ir
        );

        // main.c: declares double_add and sub, defines main
        assert!(
            main_ir.contains("declare i32 @double_add("),
            "main.c should declare double_add:\n{}",
            main_ir
        );
        assert!(
            main_ir.contains("declare i32 @sub("),
            "main.c should declare sub:\n{}",
            main_ir
        );
        assert!(
            main_ir.contains("define i32 @main"),
            "main.c should define main:\n{}",
            main_ir
        );
    }

    #[test]
    fn test_multi_tu_parallel_compilation() {
        // Verify compile_all works with parallel jobs for multiple TUs
        let test_id = format!(
            "multi_tu_par_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let temp_dir = std::env::temp_dir().join(&test_id);
        fs::create_dir_all(&temp_dir).unwrap();

        // Skip if llc is not available
        if find_tool(&["llc-18", "llc", "llc-17", "llc-16"]).is_err() {
            eprintln!("Skipping parallel compilation test: llc not found");
            let _ = fs::remove_dir_all(&temp_dir);
            return;
        }

        // Create multiple source files
        let helper_path = temp_dir.join("helper.c");
        fs::write(&helper_path, "int add(int a, int b) { return a + b; }\n").unwrap();

        let util_path = temp_dir.join("util.c");
        fs::write(
            &util_path,
            "extern int add(int a, int b);\nint triple(int x) { return add(x, add(x, x)); }\n",
        )
        .unwrap();

        let main_path = temp_dir.join("main.c");
        fs::write(
            &main_path,
            "extern int triple(int x);\nint main(void) { return triple(3); }\n",
        )
        .unwrap();

        let output_path = temp_dir.join("test_parallel");
        let config = BuildConfig::new()
            .with_source_files(vec![helper_path, util_path, main_path])
            .with_output(output_path.clone())
            .with_output_type(OutputType::Executable)
            .with_jobs(2); // Parallel compilation

        let mut builder = Builder::new(config);
        let result = builder.build();
        assert!(
            result.is_ok(),
            "Parallel multi-TU build should succeed: {:?}",
            result.err()
        );

        // Verify all object files were produced
        // (Builder cleans up temp_dir, but if build succeeded, the output exists)
        assert!(output_path.exists(), "Linked executable should exist");

        // Run and verify: triple(3) = add(3, add(3, 3)) = add(3, 6) = 9
        let run_result = std::process::Command::new(&output_path).output();
        if let Ok(output) = run_result {
            assert_eq!(
                output.status.code(),
                Some(9),
                "Program should exit with triple(3)=9, got {:?}",
                output.status.code()
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
