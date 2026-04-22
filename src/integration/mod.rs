use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::build::{BuildConfig, Builder, OutputType};

#[cfg(feature = "network")]
use std::io::{Read, Write};

pub const DEFAULT_SQLITE_GITHUB_URL: &str =
    "https://github.com/abramov7613/sqlite-amalgamation-mirror/archive/refs/heads/main.zip";
pub const DEFAULT_SQLITE_GITHUB_VERSION: &str = "github-main";

pub struct IntegrationTest {
    pub test_dir: PathBuf,
    pub output_dir: PathBuf,
    pub sqlite_url: String,
    pub sqlite_version: String,
}

#[derive(Debug)]
pub struct IntegrationResult {
    pub download_success: bool,
    pub preprocess_success: bool,
    pub compile_success: bool,
    pub link_success: bool,
    pub smoke_test_success: bool,
    pub library_created: bool,
    pub library_size_bytes: u64,
    pub compile_time_ms: u64,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl IntegrationResult {
    pub fn new() -> Self {
        IntegrationResult {
            download_success: false,
            preprocess_success: false,
            compile_success: false,
            link_success: false,
            smoke_test_success: false,
            library_created: false,
            library_size_bytes: 0,
            compile_time_ms: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn all_passed(&self) -> bool {
        self.download_success
            && self.preprocess_success
            && self.compile_success
            && self.link_success
            && self.smoke_test_success
            && self.library_created
            && self.errors.is_empty()
            && self.warnings.is_empty()
    }

    pub fn add_error(&mut self, msg: &str) {
        self.errors.push(msg.to_string());
    }

    pub fn add_warning(&mut self, msg: &str) {
        self.warnings.push(msg.to_string());
    }
}

impl Default for IntegrationResult {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IntegrationResultSerializable {
    pub download_success: bool,
    pub preprocess_success: bool,
    pub compile_success: bool,
    pub link_success: bool,
    pub smoke_test_success: bool,
    pub library_created: bool,
    pub library_size_bytes: u64,
    pub compile_time_ms: u64,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl From<&IntegrationResult> for IntegrationResultSerializable {
    fn from(result: &IntegrationResult) -> Self {
        IntegrationResultSerializable {
            download_success: result.download_success,
            preprocess_success: result.preprocess_success,
            compile_success: result.compile_success,
            link_success: result.link_success,
            smoke_test_success: result.smoke_test_success,
            library_created: result.library_created,
            library_size_bytes: result.library_size_bytes,
            compile_time_ms: result.compile_time_ms,
            errors: result.errors.clone(),
            warnings: result.warnings.clone(),
        }
    }
}

impl IntegrationTest {
    pub fn new(test_dir: PathBuf, output_dir: PathBuf, sqlite_url: String) -> Self {
        let version = Self::extract_version_from_url(&sqlite_url);
        IntegrationTest {
            test_dir,
            output_dir,
            sqlite_url,
            sqlite_version: version,
        }
    }

    pub fn with_defaults() -> Self {
        IntegrationTest::new(
            PathBuf::from("/tmp/optic_integration"),
            PathBuf::from("/tmp/optic_integration/output"),
            DEFAULT_SQLITE_GITHUB_URL.to_string(),
        )
    }

    fn extract_version_from_url(url: &str) -> String {
        if url == DEFAULT_SQLITE_GITHUB_URL {
            return DEFAULT_SQLITE_GITHUB_VERSION.to_string();
        }
        if let Some(pos) = url.rfind("sqlite-amalgamation-") {
            let rest = &url[pos + "sqlite-amalgamation-".len()..];
            if let Some(end) = rest.find(".zip") {
                return rest[..end].to_string();
            }
        }
        "unknown".to_string()
    }

    pub fn validate_url(url: &str) -> bool {
        url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("file://")
            || Path::new(url).exists()
    }

    pub fn download_sqlite(&self) -> Result<PathBuf, String> {
        let zip_path = self.test_dir.join("sqlite-amalgamation.zip");

        if let Some(local_path) = self.sqlite_url.strip_prefix("file://") {
            let path = PathBuf::from(local_path);
            if path.exists() {
                return Ok(path);
            }
            return Err(format!("Local SQLite path does not exist: {}", path.display()));
        }

        let direct_path = PathBuf::from(&self.sqlite_url);
        if direct_path.exists() {
            return Ok(direct_path);
        }

        fs::create_dir_all(&self.test_dir)
            .map_err(|e| format!("Failed to create test directory: {}", e))?;

        #[cfg(feature = "network")]
        {
            match self.download_sqlite_with_network_feature(&zip_path) {
                Ok(path) => return Ok(path),
                Err(network_error) => {
                    return self
                        .download_sqlite_with_system_tool(&zip_path)
                        .map_err(|system_error| {
                            format!(
                                "Failed to download SQLite with network feature ({}) or system downloader ({}).",
                                network_error, system_error
                            )
                        });
                }
            }
        }

        #[cfg(not(feature = "network"))]
        {
            return self.download_sqlite_with_system_tool(&zip_path);
        }
    }

    #[cfg(feature = "network")]
    fn download_sqlite_with_network_feature(&self, zip_path: &Path) -> Result<PathBuf, String> {
        let response = ureq::get(&self.sqlite_url).call().map_err(|e| {
            format!(
                "Failed to download SQLite: {}. This may be an environment limitation.",
                e
            )
        })?;

        let mut file = fs::File::create(zip_path)
            .map_err(|e| format!("Failed to create zip file: {}", e))?;

        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Failed to read response: {}", e))?;

        file.write_all(&bytes)
            .map_err(|e| format!("Failed to write zip file: {}", e))?;

        Ok(zip_path.to_path_buf())
    }

    fn download_sqlite_with_system_tool(&self, zip_path: &Path) -> Result<PathBuf, String> {
        let curl = Command::new("curl")
            .arg("-L")
            .arg("--fail")
            .arg("--output")
            .arg(zip_path)
            .arg(&self.sqlite_url)
            .output();

        if let Ok(output) = curl {
            if output.status.success() {
                return Ok(zip_path.to_path_buf());
            }
        }

        let wget = Command::new("wget")
            .arg("-O")
            .arg(zip_path)
            .arg(&self.sqlite_url)
            .output();

        if let Ok(output) = wget {
            if output.status.success() {
                return Ok(zip_path.to_path_buf());
            }
        }

        Err("No working downloader found; tried curl and wget".to_string())
    }

    pub fn download_sqlite_mock(&self) -> Result<PathBuf, String> {
        let zip_path = self.test_dir.join("sqlite-amalgamation.zip");
        fs::create_dir_all(&self.test_dir)
            .map_err(|e| format!("Failed to create test directory: {}", e))?;
        fs::write(&zip_path, "mock zip content")
            .map_err(|e| format!("Failed to write mock zip: {}", e))?;
        Ok(zip_path)
    }

    pub fn extract_sqlite(&self, zip_path: &Path) -> Result<PathBuf, String> {
        if zip_path.is_dir() {
            return self.find_sqlite3_c(zip_path);
        }

        if zip_path.file_name().and_then(|n| n.to_str()) == Some("sqlite3.c") {
            return Ok(zip_path.to_path_buf());
        }

        let extract_dir = self.test_dir.join("sqlite-extracted");
        fs::create_dir_all(&extract_dir)
            .map_err(|e| format!("Failed to create extract directory: {}", e))?;

        let file =
            fs::File::open(zip_path).map_err(|e| format!("Failed to open zip file: {}", e))?;

        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Failed to read zip archive: {}", e))?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| format!("Failed to access file in archive: {}", e))?;

            let outpath = match file.enclosed_name() {
                Some(path) => extract_dir.join(path),
                None => continue,
            };

            if file.name().ends_with('/') {
                fs::create_dir_all(&outpath)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            } else {
                if let Some(p) = outpath.parent() {
                    fs::create_dir_all(p)
                        .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                }
                let mut outfile = fs::File::create(&outpath)
                    .map_err(|e| format!("Failed to create file: {}", e))?;
                io::copy(&mut file, &mut outfile)
                    .map_err(|e| format!("Failed to copy file content: {}", e))?;
            }
        }

        let sqlite_c = self.find_sqlite3_c(&extract_dir)?;
        Ok(sqlite_c)
    }

    pub fn extract_sqlite_mock(&self) -> Result<PathBuf, String> {
        let extract_dir = self.test_dir.join("sqlite-extracted");
        fs::create_dir_all(&extract_dir)
            .map_err(|e| format!("Failed to create extract directory: {}", e))?;

        let sqlite_c_path = extract_dir.join("sqlite3.c");
        let mock_sqlite_c = self.generate_mock_sqlite_c();
        fs::write(&sqlite_c_path, &mock_sqlite_c)
            .map_err(|e| format!("Failed to write mock sqlite3.c: {}", e))?;

        let header_path = extract_dir.join("sqlite3.h");
        fs::write(&header_path, "/* mock sqlite3.h */")
            .map_err(|e| format!("Failed to write mock sqlite3.h: {}", e))?;

        Ok(sqlite_c_path)
    }

    fn find_sqlite3_c(&self, dir: &Path) -> Result<PathBuf, String> {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Ok(found) = self.find_sqlite3_c(&path) {
                        return Ok(found);
                    }
                }
                if path.file_name().and_then(|n| n.to_str()) == Some("sqlite3.c") {
                    return Ok(path);
                }
            }
        }
        Err("sqlite3.c not found in extracted archive".to_string())
    }

    fn generate_mock_sqlite_c(&self) -> String {
        r#"/* Mock SQLite amalgamation for testing */
#include "sqlite3.h"

int sqlite3_open(const char *filename, void **ppDb) {
    (void)filename;
    (void)ppDb;
    return 0;
}

int sqlite3_close(void *db) {
    (void)db;
    return 0;
}

int sqlite3_exec(void *db, const char *sql, void *callback, void *arg, char **errmsg) {
    (void)db;
    (void)sql;
    (void)callback;
    (void)arg;
    (void)errmsg;
    return 0;
}

const char *sqlite3_libversion(void) {
    return "3.49.2";
}

const char *sqlite3_sourceid(void) {
    return "mock-2026-01-01";
}
"#
        .to_string()
    }

    pub fn preprocess_sqlite(&self, sqlite_c: &Path) -> Result<PathBuf, String> {
        let preprocessed = self.output_dir.join("sqlite3_preprocessed.c");
        fs::create_dir_all(&self.output_dir)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;

        let gcc = self.find_tool(&["gcc", "clang"]);
        match gcc {
            Ok(compiler) => {
                let output = Command::new(&compiler)
                    .arg("-E")
                    .arg("-P")
                    .arg("-o")
                    .arg(&preprocessed)
                    .arg(sqlite_c)
                    .output()
                    .map_err(|e| format!("Failed to run preprocessor: {}", e))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("Preprocessor failed: {}", stderr));
                }
            }
            Err(_) => {
                fs::copy(sqlite_c, &preprocessed)
                    .map_err(|e| format!("Failed to copy source file: {}", e))?;
            }
        }

        Ok(preprocessed)
    }

    pub fn preprocess_sqlite_mock(&self, sqlite_c: &Path) -> Result<PathBuf, String> {
        let preprocessed = self.output_dir.join("sqlite3_preprocessed.c");
        fs::create_dir_all(&self.output_dir)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;
        fs::copy(sqlite_c, &preprocessed)
            .map_err(|e| format!("Failed to copy source file: {}", e))?;
        Ok(preprocessed)
    }

    pub fn compile_sqlite(&self, source: &Path) -> Result<PathBuf, String> {
        let obj_path = self.output_dir.join("sqlite3.o");

        let start = Instant::now();

        let config = BuildConfig::new()
            .with_source_files(vec![source.to_path_buf()])
            .with_output(obj_path.clone())
            .with_output_type(OutputType::Object);

        let mut builder = Builder::new(config);
        let build_result = builder.build();

        let _elapsed = start.elapsed().as_millis() as u64;

        match build_result {
            Ok(()) => {
                if obj_path.exists() {
                    Ok(obj_path)
                } else {
                    Err("Object file not created after successful build".to_string())
                }
            }
            Err(e) => Err(format!("Compilation failed: {}", e)),
        }
    }

    pub fn compile_sqlite_mock(&self, source: &Path) -> Result<PathBuf, String> {
        let obj_path = self.output_dir.join("sqlite3.o");
        fs::create_dir_all(&self.output_dir)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;
        fs::copy(source, &obj_path)
            .map_err(|e| format!("Failed to create mock object file: {}", e))?;
        Ok(obj_path)
    }

    pub fn link_sqlite(&self, obj_path: &Path) -> Result<PathBuf, String> {
        let lib_path = self.output_dir.join("libsqlite3.so");

        let clang = self.find_tool(&["clang", "gcc"]);
        match clang {
            Ok(compiler) => {
                let output = Command::new(&compiler)
                    .arg("-shared")
                    .arg("-o")
                    .arg(&lib_path)
                    .arg(obj_path)
                    .arg("-lm")
                    .output()
                    .map_err(|e| format!("Failed to run linker: {}", e))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("Linking failed: {}", stderr));
                }
            }
            Err(e) => return Err(e),
        }

        Ok(lib_path)
    }

    pub fn link_sqlite_mock(&self, obj_path: &Path) -> Result<PathBuf, String> {
        let lib_path = self.output_dir.join("libsqlite3.so");
        fs::create_dir_all(&self.output_dir)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;
        fs::copy(obj_path, &lib_path)
            .map_err(|e| format!("Failed to create mock library: {}", e))?;
        Ok(lib_path)
    }

    fn find_tool(&self, candidates: &[&str]) -> Result<String, String> {
        for candidate in candidates {
            if Command::new(candidate).arg("--version").output().is_ok() {
                return Ok(candidate.to_string());
            }
        }
        Err(format!(
            "No suitable tool found from: {}",
            candidates.join(", ")
        ))
    }

    pub fn run(&self) -> IntegrationResult {
        let mut result = IntegrationResult::new();

        fs::create_dir_all(&self.test_dir).unwrap_or(());
        fs::create_dir_all(&self.output_dir).unwrap_or(());

        let zip_path = match self.download_sqlite() {
            Ok(path) => {
                result.download_success = true;
                path
            }
            Err(e) => {
                result.add_error(&format!("Download failed: {}", e));
                return result;
            }
        };

        let sqlite_c = match self.extract_sqlite(&zip_path) {
            Ok(path) => path,
            Err(e) => {
                result.add_error(&format!("Extraction failed: {}", e));
                return result;
            }
        };

        let preprocessed = match self.preprocess_sqlite(&sqlite_c) {
            Ok(path) => {
                result.preprocess_success = true;
                path
            }
            Err(e) => {
                result.add_error(&format!("Preprocessing failed: {}", e));
                return result;
            }
        };

        let start = Instant::now();
        let obj_path = match self.compile_sqlite(&preprocessed) {
            Ok(path) => {
                result.compile_success = true;
                path
            }
            Err(e) => {
                result.add_error(&format!("Compilation failed: {}", e));
                return result;
            }
        };
        result.compile_time_ms = start.elapsed().as_millis() as u64;

        let lib_path = match self.link_sqlite(&obj_path) {
            Ok(path) => {
                result.link_success = true;
                path
            }
            Err(e) => {
                result.add_error(&format!("Linking failed: {}", e));
                return result;
            }
        };

        if lib_path.exists() {
            if let Ok(metadata) = fs::metadata(&lib_path) {
                result.library_size_bytes = metadata.len();
                result.library_created = true;
            }
        }

        match self.run_sqlite_smoke_test(&lib_path) {
            Ok(()) => result.smoke_test_success = true,
            Err(e) => result.add_error(&format!("Smoke test failed: {}", e)),
        }

        result
    }

    fn run_sqlite_smoke_test(&self, lib_path: &Path) -> Result<(), String> {
        let compiler = self.find_tool(&["clang", "gcc", "cc"])?;
        fs::create_dir_all(&self.output_dir)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;

        let smoke_source = self.output_dir.join("sqlite_smoke.c");
        let smoke_binary = self.output_dir.join("sqlite_smoke");
        let smoke_program = r#"typedef struct sqlite3 sqlite3;
extern int sqlite3_open(const char *filename, sqlite3 **pp_db);
extern int sqlite3_exec(sqlite3 *db, const char *sql, void *callback, void *arg, char **errmsg);
extern int sqlite3_close(sqlite3 *db);

int main(void) {
    sqlite3 *db = 0;
    if (sqlite3_open(":memory:", &db) != 0 || db == 0) {
        return 1;
    }
    if (sqlite3_exec(db, "CREATE TABLE t(x);", 0, 0, 0) != 0) {
        return 2;
    }
    if (sqlite3_close(db) != 0) {
        return 3;
    }
    return 0;
}
"#;
        fs::write(&smoke_source, smoke_program)
            .map_err(|e| format!("Failed to write smoke source: {}", e))?;

        let output = Command::new(&compiler)
            .arg(&smoke_source)
            .arg("-L")
            .arg(&self.output_dir)
            .arg(format!("-Wl,-rpath,{}", self.output_dir.display()))
            .arg("-lsqlite3")
            .arg("-o")
            .arg(&smoke_binary)
            .output()
            .map_err(|e| format!("Failed to build smoke test: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "smoke build failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let output = Command::new(&smoke_binary)
            .env("LD_LIBRARY_PATH", &self.output_dir)
            .output()
            .map_err(|e| format!("Failed to run smoke test: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "smoke binary exited with status {}",
                output.status
            ));
        }

        if !lib_path.exists() {
            return Err(format!("library not found: {}", lib_path.display()));
        }

        Ok(())
    }

    pub fn generate_report(&self, result: &IntegrationResult) -> String {
        let mut report = String::new();

        report.push_str("# OpticC SQLite Integration Test Report\n\n");

        report.push_str("## Configuration\n\n");
        report.push_str(&format!("- **SQLite URL:** {}\n", self.sqlite_url));
        report.push_str(&format!("- **SQLite Version:** {}\n", self.sqlite_version));
        report.push_str(&format!(
            "- **Test Directory:** {}\n",
            self.test_dir.display()
        ));
        report.push_str(&format!(
            "- **Output Directory:** {}\n",
            self.output_dir.display()
        ));
        report.push('\n');

        report.push_str("## Results Summary\n\n");

        let status = if result.all_passed() { "PASS" } else { "FAIL" };
        report.push_str(&format!("- Overall Status: {}\n", status));
        report.push_str(&format!(
            "- Download: {}\n",
            self.bool_status(result.download_success)
        ));
        report.push_str(&format!(
            "- Preprocess: {}\n",
            self.bool_status(result.preprocess_success)
        ));
        report.push_str(&format!(
            "- Compile: {}\n",
            self.bool_status(result.compile_success)
        ));
        report.push_str(&format!(
            "- Link: {}\n",
            self.bool_status(result.link_success)
        ));
        report.push_str(&format!(
            "- Smoke Test: {}\n",
            self.bool_status(result.smoke_test_success)
        ));
        report.push_str(&format!(
            "- Library Created: {}\n",
            self.bool_status(result.library_created)
        ));
        report.push_str(&format!(
            "- Library Size: {} bytes\n",
            result.library_size_bytes
        ));
        report.push_str(&format!("- Compile Time: {} ms\n", result.compile_time_ms));
        report.push('\n');

        if !result.errors.is_empty() {
            report.push_str("## Errors\n\n");
            for error in &result.errors {
                report.push_str(&format!("- {}\n", error));
            }
            report.push('\n');
        }

        if !result.warnings.is_empty() {
            report.push_str("## Warnings\n\n");
            for warning in &result.warnings {
                report.push_str(&format!("- {}\n", warning));
            }
            report.push('\n');
        }

        report.push_str("## JSON Summary\n\n");
        let serializable = IntegrationResultSerializable::from(result);
        if let Ok(json) = serde_json::to_string_pretty(&serializable) {
            report.push_str("```json\n");
            report.push_str(&json);
            report.push_str("\n```\n");
        }

        report
    }

    fn bool_status(&self, success: bool) -> &'static str {
        if success {
            "SUCCESS"
        } else {
            "FAILED"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integration_test_creation() {
        let test = IntegrationTest::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/output"),
            DEFAULT_SQLITE_GITHUB_URL.to_string(),
        );
        assert_eq!(test.test_dir, PathBuf::from("/tmp/test"));
        assert_eq!(test.output_dir, PathBuf::from("/tmp/test/output"));
        assert_eq!(test.sqlite_url, DEFAULT_SQLITE_GITHUB_URL);
        assert_eq!(test.sqlite_version, DEFAULT_SQLITE_GITHUB_VERSION);
    }

    #[test]
    fn test_integration_test_with_defaults() {
        let test = IntegrationTest::with_defaults();
        assert_eq!(test.test_dir, PathBuf::from("/tmp/optic_integration"));
        assert_eq!(
            test.output_dir,
            PathBuf::from("/tmp/optic_integration/output")
        );
        assert!(test.sqlite_url.contains("sqlite-amalgamation"));
    }

    #[test]
    fn test_integration_result_creation() {
        let result = IntegrationResult::new();
        assert!(!result.download_success);
        assert!(!result.preprocess_success);
        assert!(!result.compile_success);
        assert!(!result.link_success);
        assert!(!result.smoke_test_success);
        assert!(!result.library_created);
        assert_eq!(result.library_size_bytes, 0);
        assert_eq!(result.compile_time_ms, 0);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_integration_result_all_passed() {
        let mut result = IntegrationResult::new();
        assert!(!result.all_passed());

        result.download_success = true;
        result.preprocess_success = true;
        result.compile_success = true;
        result.link_success = true;
        result.smoke_test_success = true;
        result.library_created = true;
        assert!(result.all_passed());
    }

    #[test]
    fn test_integration_result_add_error() {
        let mut result = IntegrationResult::new();
        result.add_error("test error");
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0], "test error");
    }

    #[test]
    fn test_integration_result_add_warning() {
        let mut result = IntegrationResult::new();
        result.add_warning("test warning");
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0], "test warning");
    }

    #[test]
    fn test_url_validation() {
        assert!(IntegrationTest::validate_url(
            "https://example.com/file.zip"
        ));
        assert!(IntegrationTest::validate_url("http://example.com/file.zip"));
        assert!(!IntegrationTest::validate_url("ftp://example.com/file.zip"));
        assert!(!IntegrationTest::validate_url("/local/path/file.zip"));
        assert!(!IntegrationTest::validate_url(""));
    }

    #[test]
    fn test_version_extraction_from_url() {
        let test = IntegrationTest::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/output"),
            "https://www.sqlite.org/2026/sqlite-amalgamation-3490200.zip".to_string(),
        );
        assert_eq!(test.sqlite_version, "3490200");

        let test2 = IntegrationTest::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/output"),
            "https://www.sqlite.org/2024/sqlite-amalgamation-3450300.zip".to_string(),
        );
        assert_eq!(test2.sqlite_version, "3450300");

        let test3 = IntegrationTest::new(
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/output"),
            "https://example.com/unknown.zip".to_string(),
        );
        assert_eq!(test3.sqlite_version, "unknown");
    }

    #[test]
    fn test_path_handling() {
        let test = IntegrationTest::new(
            PathBuf::from("/tmp/test_integration"),
            PathBuf::from("/tmp/test_integration/output"),
            DEFAULT_SQLITE_GITHUB_URL.to_string(),
        );
        assert!(test.test_dir.is_absolute());
        assert!(test.output_dir.is_absolute());
        assert!(test.output_dir.starts_with(&test.test_dir));
    }

    #[test]
    fn test_error_reporting() {
        let mut result = IntegrationResult::new();
        result.add_error("Download failed: connection timeout");
        result.add_error("Extraction failed: invalid zip");
        result.add_warning("Using mock data");

        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.errors[0].contains("Download failed"));
        assert!(result.errors[1].contains("Extraction failed"));
        assert!(result.warnings[0].contains("mock"));
    }

    #[test]
    fn test_report_generation_markdown() {
        let test = IntegrationTest::with_defaults();
        let mut result = IntegrationResult::new();
        result.download_success = true;
        result.preprocess_success = true;
        result.compile_success = true;
        result.link_success = true;
        result.smoke_test_success = true;
        result.library_created = true;
        result.library_size_bytes = 1234567;
        result.compile_time_ms = 5000;

        let report = test.generate_report(&result);

        assert!(report.contains("# OpticC SQLite Integration Test Report"));
        assert!(report.contains("## Configuration"));
        assert!(report.contains("## Results Summary"));
        assert!(report.contains("Overall Status: PASS"));
        assert!(report.contains("Download: SUCCESS"));
        assert!(report.contains("Preprocess: SUCCESS"));
        assert!(report.contains("Compile: SUCCESS"));
        assert!(report.contains("Link: SUCCESS"));
        assert!(report.contains("Smoke Test: SUCCESS"));
        assert!(report.contains("Library Created: SUCCESS"));
        assert!(report.contains("1234567"));
        assert!(report.contains("5000"));
    }

    #[test]
    fn test_report_generation_with_errors() {
        let test = IntegrationTest::with_defaults();
        let mut result = IntegrationResult::new();
        result.add_error("Test error");
        result.add_warning("Test warning");

        let report = test.generate_report(&result);

        assert!(report.contains("Overall Status: FAIL"));
        assert!(report.contains("## Errors"));
        assert!(report.contains("Test error"));
        assert!(report.contains("## Warnings"));
        assert!(report.contains("Test warning"));
    }

    #[test]
    fn test_result_serialization() {
        let mut result = IntegrationResult::new();
        result.download_success = true;
        result.compile_success = true;
        result.smoke_test_success = true;
        result.library_size_bytes = 1024;
        result.compile_time_ms = 100;
        result.add_error("test error");

        let serializable = IntegrationResultSerializable::from(&result);
        let json = serde_json::to_string(&serializable).unwrap();

        assert!(json.contains("download_success"));
        assert!(json.contains("true"));
        assert!(json.contains("1024"));
        assert!(json.contains("100"));
        assert!(json.contains("test error"));

        let deserialized: IntegrationResultSerializable = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.download_success, true);
        assert_eq!(deserialized.compile_success, true);
        assert_eq!(deserialized.smoke_test_success, true);
        assert_eq!(deserialized.library_size_bytes, 1024);
        assert_eq!(deserialized.compile_time_ms, 100);
        assert_eq!(deserialized.errors.len(), 1);
    }

    #[test]
    fn test_download_mock() {
        let temp_dir =
            std::env::temp_dir().join(format!("optic_integration_test_dl_{}", std::process::id()));
        let output_dir = temp_dir.join("output");

        let test = IntegrationTest::new(
            temp_dir.clone(),
            output_dir.clone(),
            DEFAULT_SQLITE_GITHUB_URL.to_string(),
        );

        let result = test.download_sqlite_mock();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_preprocess_mock() {
        let temp_dir =
            std::env::temp_dir().join(format!("optic_integration_test_pp_{}", std::process::id()));
        let output_dir = temp_dir.join("output");
        fs::create_dir_all(&temp_dir).unwrap();

        let source = temp_dir.join("sqlite3.c");
        fs::write(&source, "int main() { return 0; }").unwrap();

        let test = IntegrationTest::new(
            temp_dir.clone(),
            output_dir.clone(),
            DEFAULT_SQLITE_GITHUB_URL.to_string(),
        );

        let result = test.preprocess_sqlite_mock(&source);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_compile_mock() {
        let temp_dir =
            std::env::temp_dir().join(format!("optic_integration_test_cc_{}", std::process::id()));
        let output_dir = temp_dir.join("output");
        fs::create_dir_all(&temp_dir).unwrap();

        let source = temp_dir.join("sqlite3.c");
        fs::write(&source, "int main() { return 0; }").unwrap();

        let test = IntegrationTest::new(
            temp_dir.clone(),
            output_dir.clone(),
            DEFAULT_SQLITE_GITHUB_URL.to_string(),
        );

        let result = test.compile_sqlite_mock(&source);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_link_mock() {
        let temp_dir =
            std::env::temp_dir().join(format!("optic_integration_test_ln_{}", std::process::id()));
        let output_dir = temp_dir.join("output");
        fs::create_dir_all(&temp_dir).unwrap();

        let obj = temp_dir.join("sqlite3.o");
        fs::write(&obj, "mock object").unwrap();

        let test = IntegrationTest::new(
            temp_dir.clone(),
            output_dir.clone(),
            DEFAULT_SQLITE_GITHUB_URL.to_string(),
        );

        let result = test.link_sqlite_mock(&obj);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "libsqlite3.so");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_mock() {
        let temp_dir =
            std::env::temp_dir().join(format!("optic_integration_test_ex_{}", std::process::id()));
        let output_dir = temp_dir.join("output");
        fs::create_dir_all(&temp_dir).unwrap();

        let test = IntegrationTest::new(
            temp_dir.clone(),
            output_dir.clone(),
            DEFAULT_SQLITE_GITHUB_URL.to_string(),
        );

        let result = test.extract_sqlite_mock();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "sqlite3.c");

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("sqlite3_open"));
        assert!(content.contains("sqlite3_close"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_full_pipeline_local_fixture() {
        let temp_dir =
            std::env::temp_dir().join(format!("optic_integration_test_fp_{}", std::process::id()));
        let output_dir = temp_dir.join("output");
        fs::create_dir_all(&output_dir).unwrap();

        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sqlite_smoke/sqlite3.c");

        let test = IntegrationTest::new(
            temp_dir.clone(),
            output_dir.clone(),
            fixture.to_string_lossy().to_string(),
        );

        let result = test.run();

        assert!(result.all_passed(), "errors: {:?}", result.errors);
        assert!(result.download_success);
        assert!(result.preprocess_success);
        assert!(result.compile_success);
        assert!(result.link_success);
        assert!(result.smoke_test_success);

        let report = test.generate_report(&result);
        assert!(report.contains("OpticC SQLite Integration Test Report"));
        assert!(report.contains("Overall Status: PASS"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_run_fails_without_fallbacks() {
        let temp_dir =
            std::env::temp_dir().join(format!("optic_integration_test_fail_{}", std::process::id()));
        let output_dir = temp_dir.join("output");

        let test = IntegrationTest::new(
            temp_dir.clone(),
            output_dir,
            "/definitely/missing/sqlite3.c".to_string(),
        );

        let result = test.run();
        assert!(!result.all_passed());
        assert!(!result.download_success);
        assert!(!result.smoke_test_success);
        assert!(!result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_default_result() {
        let result = IntegrationResult::default();
        assert!(!result.all_passed());
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }
}
