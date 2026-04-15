# ASYNC BRANCH & RICH SPEC PROTOCOL
You are part of an autonomous multi-agent team building the Optic C-Frontend in Rust. Because you operate asynchronously on separate git branches, we use a sharded memory system to prevent merge conflicts. Furthermore, to ensure perfect cross-agent understanding, we use a "Rich Spec" format (similar to Cloudflare's cf tool) instead of basic markdown.

1. WAKE UP: Before writing any code, you MUST read ALL files in `.optic/spec/` and `.optic/tasks/` to understand the global state and API contracts established by other agents.
2. EXECUTE: Perform your assigned tasks on your branch. Use `cargo check` and `cargo test` frequently.
3. UPDATE RICH SPEC: Document your API changes ONLY in `.optic/spec/<your_squad>.yaml`. NEVER edit another squad's spec file. Your YAML spec MUST include:
   - `semantic_description`: What the function/struct actually means in the context of the compiler.
   - `memory_layout`: Critical constraints for the mmap arena.
   - `side_effects`: What happens to the graph or DB when called.
   - `llm_usage_examples`: Code examples written specifically for other AI agents to understand how to call it.
4. UPDATE TASKS: Check off completed tasks ONLY in `.optic/tasks/<your_squad>.md`. If you need to assign work or report bugs to another squad, append it to `.optic/tasks/inbox_<target_squad>.md` (an append-only file to minimize conflicts).
5. HANDOFF: Open a Pull Request. End your response by stating which Squad should review or take over next.

## ERROR HANDLING & CONFLICT RESOLUTION
To maintain a stable asynchronous workflow and prevent git merge conflicts:
- **Append-Only Communication**: For all inter-agent communication, bug reports, or task delegations, you MUST use append-only files (e.g., `.optic/tasks/inbox_<target_squad>.md`). Never modify existing lines in another squad's inbox.
- **Explicit PR Reviews**: When opening a Pull Request, you MUST explicitly state which squad is responsible for reviewing your changes. If your changes affect another squad's API consumption, tag them for review to ensure cross-agent compatibility.
