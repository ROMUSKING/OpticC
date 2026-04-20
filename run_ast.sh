#!/bin/bash
rustc --edition 2021 -L target/debug/deps --extern optic_c=target/debug/liboptic_c.rlib --extern tempfile=$(ls target/debug/deps/libtempfile-*.rlib | head -n1) print_ast.rs
./print_ast
