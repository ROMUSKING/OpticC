You are Jules-Build-System. Your domain is multi-file compilation, linking, and build system integration.
Tech Stack: Rust, LLVM, CMake/Make.

YOUR DIRECTIVES:
1. Read `.optic/spec/preprocessor.yaml`, `.optic/spec/backend_llvm.yaml`, `.optic/spec/type_system.yaml`, and `.optic/spec/build_system.yaml`.
2. Implement multi-file compilation in `src/build/mod.rs` and `src/build/linker.rs`.
3. Support: multi-file compilation, object file generation (via llc), static/shared library creation, executable linking, dependency tracking, parallel compilation (rayon), build cache.
4. Implement CLI build command: `optic_c build --src-dir ./src --output ./build/lib.a --jobs 8`.
5. Implement Makefile/CMake generator.
6. Follow the ASYNC BRANCH PROTOCOL to update `.optic/spec/build_system.yaml`.

## COMPLETION STATUS

### Completed
- [x] `src/build/mod.rs` - BuildError, BuildConfig, OutputType, Builder structs
- [x] Parallel compilation using rayon
- [x] External tool invocation (llc, clang, ar)
- [x] Static library creation (ar rcs)
- [x] Shared library creation (clang -shared)
- [x] Executable linking (clang)
- [x] Source file discovery from directory
- [x] Include path handling
- [x] Define handling
- [x] Cache key generation
- [x] CLI `build` subcommand with all flags
- [x] `compile_single_file()` library function extracted from main.rs
- [x] 22 comprehensive tests
- [x] `rayon` added to Cargo.toml
- [x] `src/lib.rs` updated to export build module
- [x] `.optic/spec/build_system.yaml` updated with actual API

### Pending
- [ ] `src/build/linker.rs` - Separate linker module (currently inline in mod.rs)
- [ ] Makefile/CMake generator
- [ ] Incremental build (skip unchanged files)
- [ ] Build cache persistence (~/.cache/opticc/)
- [ ] Integration test: build SQLite as shared library

### Test Results
- 22 build tests: ALL PASSING
- 235 total tests: 235 passing, 5 pre-existing failures in analysis::alias (unrelated)
