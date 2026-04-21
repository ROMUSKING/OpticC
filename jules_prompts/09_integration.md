# Jules-Integration

Your job is to keep OpticC's verification path accurate and current.

## Current verified state (2026-04-21)
### Baseline
- `cargo build` ✅
- `cargo test` ✅
- `cargo test -- --list | grep -c ': test'` → **405** discovered tests
- `cargo test test_full_pipeline_local_fixture -- --nocapture` ✅

### Real SQLite truth gate
A real GitHub-hosted SQLite amalgamation archive has now been verified through the integration harness up to the smoke-test boundary:
- download ✅
- extraction ✅
- preprocessing ✅
- OpticC compile ✅
- shared-library link ✅
- smoke test ❌

Current smoke failure:
- undefined reference to `u8`
- undefined reference to `vtabCallConstructor`

## Standard verification commands
```bash
cargo build
cargo test
cargo test test_full_pipeline_local_fixture -- --nocapture

# default remote target now points to a GitHub SQLite archive
cargo run -- integration-test \
  --test-dir /tmp/optic_sqlite_github/test \
  --output-dir /tmp/optic_sqlite_github/out
```

## Notes on acquisition/setup
- The integration harness now defaults to a GitHub-hosted SQLite amalgamation archive.
- Remote archive download now works in the normal build setup via `curl`/`wget` fallback.
- You no longer need to rebuild with Cargo's `network` feature just to fetch the remote archive.
- The local fixture remains useful for fast regression checks, but it is not the full truth gate.

## Definition of done for SQLite
The SQLite truth gate is only green when the real-archive integration run passes all of:
1. download
2. extract
3. preprocess
4. compile
5. shared-library link
6. smoke test

## Current next action
Hand off the unresolved-symbol smoke failure to backend/type-system owners. Integration is no longer the blocker for download/setup.
