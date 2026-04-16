You are Jules-Memory-Infra. Your domain is strictly the Core Memory Infrastructure.
Tech Stack: Rust, memmap2.

YOUR DIRECTIVES:
1. Implement the zero-serialization mmap arena allocator in `src/arena.rs`.
2. Define the `NodeOffset(u32)` and `CAstNode` structs with `#[repr(C)]`.
3. Ensure the Arena can allocate 10M nodes sequentially at high speed.
4. Follow the ASYNC BRANCH PROTOCOL to update `.optic/spec/memory_infra.yaml` with your Arena API.

---

## COMPLETION STATUS: DONE

### What was implemented:
- `src/arena.rs` (551 lines) — Complete mmap arena allocator
- `CAstNode` — 64-byte `#[repr(C)]` struct with tree links (parent, first_child, last_child, next_sibling, prev_sibling), source location, data field, payload support
- `NodeOffset(u32)` — Newtype with `NULL` sentinel constant
- `SourceLocation` — Source position metadata (file_id, line, column, length)
- `NodeFlags` — Bitflags: IS_VALID, HAS_ERROR, IS_SYNTHETIC, HAS_PAYLOAD
- `Arena` — Bump allocator with `new()`, `alloc()`, `get()`, `get_mut()`, `store_string()`, `get_string()`, `store_payload()`, `get_payload()`, `flush()`
- `ArenaError` — Error enum: ZeroCapacity, AllocationFull, StringPoolFull, InvalidOffset, IoError
- 10 unit tests including 10M node allocation benchmark

### Lessons Learned:
- **NULL sentinel conflict**: Arena initially allocated from offset 0, but analysis code treats `NodeOffset(0)` as NULL. Fix: reserve offset 0 by initializing `len` to `node_size` in `Arena::new()`.
- **Hash derive needed**: `NodeOffset` must derive `Hash` to be used as HashMap/HashSet keys in the analysis module.
- **API contract clarity**: The spec file (`memory_infra.yaml`) must explicitly document that `get()` returns `&CAstNode` directly (not `Option<&CAstNode>`) and that offset 0 is reserved as NULL.

### Bugs reported to inbox:
- `.optic/tasks/inbox_arena/null_sentinel_conflict.md` — Offset 0 allocation conflicted with NULL sentinel
