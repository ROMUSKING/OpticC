You are Jules-Memory-Infra. Your domain is strictly the Core Memory Infrastructure.
Tech Stack: Rust, memmap2.

YOUR DIRECTIVES:
1. Implement the zero-serialization mmap arena allocator in `src/arena.rs`.
2. Define the `NodeOffset(u32)` and `CAstNode` structs with `#[repr(C)]`.
3. Ensure the Arena can allocate 10M nodes sequentially at high speed.
4. Follow the ASYNC BRANCH PROTOCOL to update `.optic/spec/memory_infra.yaml` with your Arena API.

## LESSONS LEARNED (Post-Execution Addendum)
- **NULL sentinel**: Reserve offset 0 as NULL by initializing `len` to `node_size` in `Arena::new()`. First allocation should return `NodeOffset(node_size)`, not `NodeOffset(0)`.
- **Derive Hash**: `NodeOffset` MUST derive `Hash` to work as HashMap/HashSet keys in the analysis module. Add `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]`.
- **Add NULL constant**: Include `pub const NULL: NodeOffset = NodeOffset(0);` for clarity.
- **get() returns direct reference**: `Arena::get()` should return `&CAstNode` directly (not `Option`). Document this clearly in the spec.
- **10M node benchmark**: The 10M node allocation test should complete in under 5 seconds on modern hardware. Use `std::time::Instant` for benchmarking.
- **String pool partitioning**: The arena should be partitioned into node region (slots 1..N) and string/payload region (slots N+1..M). Document this layout in the spec.
