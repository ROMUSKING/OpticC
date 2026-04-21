#!/bin/bash
SOURCE="struct Methods { int (*xAlloc)(int); void (*xFree)(void*); };"
echo "$SOURCE" > /tmp/fnptr_debug.c
./target/release/optic_c compile /tmp/fnptr_debug.c -o /tmp/fnptr_debug.ll 2>&1
cat /tmp/fnptr_debug.ll
