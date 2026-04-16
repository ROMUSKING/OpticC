You are Jules-DB-Infra. Your domain is strictly the Embedded Database Infrastructure.
Tech Stack: Rust, redb.

YOUR DIRECTIVES:
1. Implement the embedded KV-store using `redb` in `src/db.rs` for header deduplication.
2. Provide a clean API for inserting and querying file hashes and macro definitions.
3. Follow the ASYNC BRANCH PROTOCOL to update `.optic/spec/db_infra.yaml` with your DB API.

---

## COMPLETION STATUS: DONE

### What was implemented:
- `src/db.rs` (264 lines) — Complete redb wrapper
- `OpticDb` — Database wrapper with two typed tables:
  - `file_hashes`: `[u8; 32]` -> `&str` (SHA-256 hash to file path)
  - `macros`: `&str` -> `&str` (macro name to definition)
- API methods: `new()`, `insert_file_hash()`, `get_file_path_by_hash()`, `contains_file_hash()`, `insert_macro()`, `get_macro()`, `remove_file_hash()`, `remove_macro()`, `file_hash_count()`, `macro_count()`
- `DbError` — Error enum with `From<redb::Error>` and `From<std::io::Error>`
- 12 unit tests covering all CRUD operations and persistence

### Lessons Learned:
- **redb 4.0 API breaking changes**: The redb 4.0 release introduced new error types (`TransactionError`, `TableError`, `StorageError`, `CommitError`) that require explicit `From` implementations. The `ReadableDatabase` trait must be imported to use `begin_read()`.
- **Unused imports**: `WriteTransaction` was imported but never used; redb handles transactions internally via the Database methods.
- **Always check redb version compatibility**: When updating dependencies, verify that trait imports and error types match the new version.

### Bugs reported to inbox:
- `.optic/tasks/inbox_db_infra/redb_api_compat.md` — redb 4.0 API incompatibility with missing error type From impls
