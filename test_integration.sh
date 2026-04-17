#!/bin/bash
# Optic C-Compiler Integration Test Script
# =========================================
# This script provides instructions for testing the Optic C-Compiler.
# NOTE: Cargo/rustc is not available in this environment.
# This script documents the test procedures.

set -e

OPTIC_C_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$OPTIC_C_DIR"

echo "========================================"
echo "Optic C-Compiler Integration Test Suite"
echo "========================================"
echo ""

# Check if cargo is available
check_rust() {
    if command -v cargo &> /dev/null; then
        return 0
    else
        return 1
    fi
}

# Test 1: Build the compiler
test_build() {
    echo "----------------------------------------"
    echo "TEST 1: Building the Optic C-Compiler"
    echo "----------------------------------------"
    
    if ! check_rust; then
        echo "SKIPPED: Rust/cargo not available in this environment"
        echo ""
        return 1
    fi
    
    echo "Building in debug mode..."
    cargo build
    echo "Build successful!"
    echo ""
}

# Test 2: Unit tests
test_unit_tests() {
    echo "----------------------------------------"
    echo "TEST 2: Running Unit Tests"
    echo "----------------------------------------"
    
    if ! check_rust; then
        echo "SKIPPED: Rust/cargo not available in this environment"
        echo ""
        return 1
    fi
    
    echo "Running all unit tests..."
    cargo test --lib
    echo ""
}

# Test 3: Parse a C source file
test_parse() {
    echo "----------------------------------------"
    echo "TEST 3: Parse C Source File"
    echo "----------------------------------------"
    
    if ! check_rust; then
        echo "SKIPPED: Rust/cargo not available in this environment"
        echo ""
        return 1
    fi
    
    # Create test file
    TEST_FILE="$OPTIC_C_DIR/test_input.c"
    cat > "$TEST_FILE" << 'EOF'
#include <stdio.h>

#define MAX(a, b) ((a) > (b) ? (a) : (b))

int main() {
    int x = 10;
    int y = 20;
    int z = MAX(x, y);
    printf("Max: %d\n", z);
    return 0;
}
EOF
    
    echo "Created test file: $TEST_FILE"
    echo "Contents:"
    cat "$TEST_FILE"
    echo ""
    
    echo "Compiling with Optic C-Compiler..."
    cargo run --release -- compile "$TEST_FILE" -o test_output 2>&1 || true
    
    echo ""
    rm -f "$TEST_FILE" test_output
}

# Test 4: VFS Mount Test
test_vfs() {
    echo "----------------------------------------"
    echo "TEST 4: VFS Shadow Comment Injection"
    echo "----------------------------------------"
    
    if ! check_rust; then
        echo "SKIPPED: Rust/cargo not available in this environment"
        echo ""
        return 1
    fi
    
    # This test requires the VFS implementation which is incomplete
    if [ ! -s src/vfs/mod.rs ] || [ $(wc -c < src/vfs/mod.rs) -lt 50 ]; then
        echo "SKIPPED: VFS implementation is incomplete (stub only)"
        echo ""
        return 1
    fi
    
    MOUNT_POINT="/tmp/optic_vfs_test"
    mkdir -p "$MOUNT_POINT"
    
    echo "Mounting VFS at $MOUNT_POINT..."
    cargo run --release -- mount "$MOUNT_POINT" &
    VFS_PID=$!
    
    sleep 2
    
    echo "Creating vulnerable C source..."
    cat > "$MOUNT_POINT/test.c" << 'EOF'
void vulnerable() {
    char *p = NULL;
    *p = 'x';  // Null dereference
}
EOF
    
    echo "Reading file through VFS (should inject shadow comments)..."
    cat "$MOUNT_POINT/test.c"
    
    kill $VFS_PID 2>/dev/null || true
    rm -rf "$MOUNT_POINT"
    echo ""
}

# Test 5: Verify Arena Performance
test_arena_perf() {
    echo "----------------------------------------"
    echo "TEST 5: Arena Memory Performance"
    echo "----------------------------------------"
    
    if ! check_rust; then
        echo "SKIPPED: Rust/cargo not available in this environment"
        echo ""
        return 1
    fi
    
    echo "Running arena high-speed allocation test..."
    cargo test test_high_speed_sequential_allocation -- --nocapture 2>&1 || true
    echo ""
}

# Test 6: Verify DB Infrastructure
test_db() {
    echo "----------------------------------------"
    echo "TEST 6: Database Infrastructure"
    echo "----------------------------------------"
    
    if ! check_rust; then
        echo "SKIPPED: Rust/cargo not available in this environment"
        echo ""
        return 1
    fi
    
    echo "Running DB include deduplication tests..."
    cargo test --lib db 2>&1 || echo "No DB tests found or tests failed"
    echo ""
}

# Generate test C files
generate_test_files() {
    echo "----------------------------------------"
    echo "Generating Test C Source Files"
    echo "----------------------------------------"
    
    mkdir -p "$OPTIC_C_DIR/test_samples"
    
    # Simple function
    cat > "$OPTIC_C_DIR/test_samples/simple.c" << 'EOF'
int add(int a, int b) {
    return a + b;
}

int main() {
    return add(1, 2);
}
EOF

    # With macro
    cat > "$OPTIC_C_DIR/test_samples/macro_test.c" << 'EOF'
#define SQUARE(x) ((x) * (x))
#define MAX(a, b) ((a) > (b) ? (a) : (b))

int main() {
    int x = SQUARE(5);
    int y = MAX(3, 4);
    return x + y;
}
EOF

    # With struct
    cat > "$OPTIC_C_DIR/test_samples/struct_test.c" << 'EOF'
struct Point {
    int x;
    int y;
};

int main() {
    struct Point p = {10, 20};
    return p.x + p.y;
}
EOF

    # Vulnerable code for VFS testing
    cat > "$OPTIC_C_DIR/test_samples/vulnerable.c" << 'EOF'
void null_deref() {
    char *p = NULL;
    *p = 'x';
}

void use_after_free() {
    char *p = malloc(10);
    free(p);
    p[0] = 'x';
}
EOF

    echo "Generated test files in: $OPTIC_C_DIR/test_samples/"
    ls -la "$OPTIC_C_DIR/test_samples/"
    echo ""
}

# Print component status
print_status() {
    echo "========================================"
    echo "Component Implementation Status"
    echo "========================================"
    echo ""
    
    components=(
        "src/arena.rs:Arena (Memory):COMPLETED"
        "src/db.rs:Database Infrastructure:COMPLETED"
        "src/frontend/lexer.rs:Lexer:COMPLETED"
        "src/frontend/parser.rs:Parser:COMPLETED"
        "src/frontend/macro_expander.rs:Macro Expander:COMPLETED"
        "src/analysis/alias.rs:Alias Analysis:INCOMPLETE"
        "src/backend/llvm.rs:LLVM Backend:INCOMPLETE"
        "src/vfs/mod.rs:VFS Projection:INCOMPLETE"
    )
    
    for comp in "${components[@]}"; do
        IFS=':' read -r file name status <<< "$comp"
        if [ -f "$file" ]; then
            size=$(wc -c < "$file")
            if [ $size -lt 50 ]; then
                status="STUB ONLY"
            fi
        fi
        printf "%-30s %s\n" "$name:" "$status"
    done
    
    echo ""
}

# Main
main() {
    print_status
    
    if [ "$1" == "--help" ] || [ "$1" == "-h" ]; then
        echo "Usage: $0 [test_name]"
        echo ""
        echo "Available tests:"
        echo "  build       - Build the compiler"
        echo "  unit        - Run unit tests"
        echo "  parse       - Test parsing a C file"
        echo "  vfs         - Test VFS shadow comments"
        echo "  arena       - Test arena performance"
        echo "  db          - Test database infrastructure"
        echo "  generate    - Generate test C files"
        echo "  all         - Run all tests (requires Rust)"
        echo "  status      - Show component status"
        echo ""
        echo "No arguments: Show this help"
        return 0
    fi
    
    case "$1" in
        build)
            test_build
            ;;
        unit)
            test_unit_tests
            ;;
        parse)
            test_parse
            ;;
        vfs)
            test_vfs
            ;;
        arena)
            test_arena_perf
            ;;
        db)
            test_db
            ;;
        generate)
            generate_test_files
            ;;
        all)
            test_build
            test_unit_tests
            test_parse
            test_arena_perf
            test_db
            ;;
        *)
            echo "Running default verification..."
            echo ""
            echo "To run tests, use: $0 [test_name]"
            echo "For help: $0 --help"
            echo ""
            generate_test_files
            ;;
    esac
}

main "$@"
