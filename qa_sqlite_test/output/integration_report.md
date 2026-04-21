# OpticC SQLite Integration Test Report

## Configuration

- **SQLite URL:** https://www.sqlite.org/2026/sqlite-amalgamation-3490200.zip
- **SQLite Version:** 3490200
- **Test Directory:** /home/runner/work/OpticC/OpticC/qa_sqlite_test
- **Output Directory:** /home/runner/work/OpticC/OpticC/qa_sqlite_test/output

## Results Summary

- Overall Status: FAIL
- Download: FAILED
- Preprocess: SUCCESS
- Compile: SUCCESS
- Link: SUCCESS
- Library Created: SUCCESS
- Library Size: 15448 bytes
- Compile Time: 60 ms

## Errors

- Download failed: Network downloads require the 'network' feature. This is an environment limitation.
- Extraction failed: Failed to read zip archive: invalid Zip archive: Could not find EOCD

## Warnings

- Attempting to use mock SQLite for testing

## JSON Summary

```json
{
  "download_success": false,
  "preprocess_success": true,
  "compile_success": true,
  "link_success": true,
  "library_created": true,
  "library_size_bytes": 15448,
  "compile_time_ms": 60,
  "errors": [
    "Download failed: Network downloads require the 'network' feature. This is an environment limitation.",
    "Extraction failed: Failed to read zip archive: invalid Zip archive: Could not find EOCD"
  ],
  "warnings": [
    "Attempting to use mock SQLite for testing"
  ]
}
```
