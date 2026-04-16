# ASYNC BRANCH & RICH SPEC PROTOCOL
You are part of an autonomous multi-agent team building the Optic C-Frontend in Rust. Because you operate asynchronously on separate git branches, we use a sharded memory system to prevent merge conflicts. Furthermore, to ensure perfect cross-agent understanding, we use a "Rich Spec" format (similar to Cloudflare's cf tool) instead of basic markdown.

1. WAKE UP: Before writing any code, you MUST read ALL files in `.optic/spec/` and `.optic/tasks/` to understand the global state and API contracts established by other agents.
2. EXECUTE: Perform your assigned tasks on your branch. Use `cargo check` and `cargo test` frequently.
3. UPDATE RICH SPEC: Document your API changes ONLY in `.optic/spec/<your_squad>.yaml`. NEVER edit another squad's spec file. Your YAML spec MUST include:
   - `semantic_description`: What the function/struct actually means in the context of the compiler.
   - `memory_layout`: Critical constraints for the mmap arena.
   - `side_effects`: What happens to the graph or DB when called.
   - `llm_usage_examples`: Code examples written specifically for other AI agents to understand how to call it.
4. UPDATE TASKS: Check off completed tasks ONLY in `.optic/tasks/<your_squad>.md`. If you need to assign work or report bugs to another squad, create a new file at `.optic/tasks/inbox_<target_squad>/<timestamp_or_uuid>.md` (creating new files guarantees no git merge conflicts).
5. HANDOFF: Open a Pull Request. End your response by stating which Squad should review or take over next.

## ERROR HANDLING & CONFLICT RESOLUTION
To maintain a stable asynchronous workflow and prevent git merge conflicts:
- **Unique ID Communication**: For all inter-agent communication, bug reports, or task delegations, you MUST create a NEW file with a unique ID (e.g., `.optic/tasks/inbox_<target_squad>/<timestamp_or_uuid>.md`). Never modify existing files in another squad's inbox.
- **Explicit PR Reviews**: When opening a Pull Request, you MUST explicitly state which squad is responsible for reviewing your changes. If your changes affect another squad's API consumption, tag them for review to ensure cross-agent compatibility.

## LESSONS LEARNED (Post-Execution Addendum)
These lessons were discovered during the first full execution of the project. Future agents should heed these warnings:

### API Contract Pitfalls
1. **Always document return types precisely**: `Arena::get()` returns `&CAstNode` directly, NOT `Option<&CAstNode>`. Ambiguous specs caused build-breaking bugs in the analysis module.
2. **Null sentinel convention must be explicit**: `NodeOffset(0)` is reserved as NULL. The arena must skip offset 0 during allocation. Document this in BOTH the arena spec and any spec that consumes arena offsets.
3. **Derive traits for cross-module types**: `NodeOffset` needs `#[derive(Hash)]` to work as HashMap/HashSet keys. Document required derives in the spec.
4. **Field names must be consistent**: The arena uses `data` for its inline u32 field. Analysis code used `data_offset` which doesn't exist. Specs are the single source of truth for field names.

### Dependency Version Issues
5. **redb 4.0 breaking changes**: New error types (`TransactionError`, `TableError`, `StorageError`, `CommitError`) require explicit `From` implementations. `ReadableDatabase` trait must be imported.
6. **inkwell 0.9 API changes**: The pass manager API changed, making `optimize()` a no-op. Always check the inkwell changelog when updating versions.
7. **fuser version compatibility**: fuser 0.17.0 requires specific trait implementations. Verify FUSE trait compatibility before implementing.

### Architecture Issues
8. **Three tokenizers exist**: `lexer.rs` (byte-level), `macro_expander.rs` (char-level with its own Lexer), and `parser.rs` (internal lex()). These have DIFFERENT Token/TokenKind types. Specs must clearly distinguish them.
9. **Arena ownership model**: The Parser OWNS the Arena (not borrows). This simplifies lifetimes but prevents sharing during parsing.
10. **VFS module is commented out**: The VFS module is `// pub mod vfs;` in lib.rs. It needs ArenaAccess trait integration before enabling.

### Testing & Debugging
11. **Debug logging is noisy**: Extensive `eprintln!` in parser.rs and llvm.rs. Consider gating behind a feature flag.
12. **Always run `cargo test` after changes**: 9 bugs were caught only during integration testing. Unit tests in individual modules don't catch cross-module API mismatches.
13. **Provenance double-counting bug**: Adding node offsets to provenance at function entry AND in match arms caused incorrect analysis results. Be careful about where provenance is recorded.
