# GCC CLI Flag Compatibility for OpticC

This document tracks GCC command-line flag compatibility required for Kbuild integration and general drop-in compiler replacement.

## PROMPT MAINTENANCE REQUIREMENT
Maintain this file as the live reference for CLI compatibility status. After implementing or verifying any flag, update the matrix below so later agents inherit the current support status.

## CONTEXT
The Linux kernel build system (Kbuild) invokes `CC` with a large set of GCC flags. OpticC must accept all of them — implementing critical ones and silently ignoring non-essential ones with a warning. This document tracks every flag category.

## GCC FLAG ACCEPTANCE MATRIX

### Compilation Mode Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-c` | ✅ Implemented | Compile to object file (via llc) |
| `-S` | 📋 Missing | Compile to assembly (.s) |
| `-E` | 📋 Missing | Preprocess only |
| `-o <file>` | ✅ Implemented | Output file path |
| `-x c` | 📋 Missing | Explicit language selection |
| `-pipe` | 📋 Missing | Use pipes between stages (can ignore) |

### Preprocessing Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-I <path>` | ✅ Implemented | Add include search path |
| `-D <name>=<val>` | ✅ Implemented | Define preprocessor macro |
| `-U <name>` | 📋 Missing | Undefine preprocessor macro |
| `-include <file>` | 📋 Missing | Force-include file before source |
| `-isystem <path>` | 📋 Missing | System include path (lower priority than -I) |
| `-iquote <path>` | 📋 Missing | Quote-include path |
| `-nostdinc` | 📋 Missing | No standard system include paths |
| `-Wp,-MD,<depfile>` | 📋 Missing | Dependency file via preprocessor |

### Optimization Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-O0` | ✅ Implemented | No optimization (passthrough to LLVM) |
| `-O1` | ✅ Implemented | Basic optimization |
| `-O2` | ✅ Implemented | Standard optimization (kernel default) |
| `-O3` | ✅ Implemented | Aggressive optimization |
| `-Os` | 📋 Missing | Optimize for size |
| `-Oz` | 📋 Missing | Aggressive size optimization |
| `-fno-inline` | 📋 Missing | Disable inlining |
| `-fno-optimize-sibling-calls` | 📋 Missing | Disable tail call opt |

### Warning Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-Wall` | 📋 Missing | Enable common warnings |
| `-Wextra` | 📋 Missing | Enable extra warnings |
| `-Wpedantic` | 📋 Missing | Pedantic warnings |
| `-Wno-<name>` | 📋 Missing | Disable specific warning |
| `-Werror` | 📋 Missing | Warnings as errors |
| `-Werror=<name>` | 📋 Missing | Specific warning as error |
| `-w` | 📋 Missing | Suppress all warnings |
| `-Wstrict-prototypes` | 📋 Missing | Kernel uses this |

### Debug Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-g` | 📋 Missing | Generate debug info (DWARF) |
| `-g0` | 📋 Missing | No debug info |
| `-g1` / `-g2` / `-g3` | 📋 Missing | Debug info levels |
| `-gdwarf-4` / `-gdwarf-5` | 📋 Missing | DWARF version selection |

### Linking Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-L <path>` | 📋 Missing | Library search path |
| `-l <lib>` | ✅ Implemented | Link library (in build command) |
| `-shared` | ✅ Implemented | Create shared library |
| `-static` | 📋 Missing | Static linking |
| `-nostdlib` | 📋 Missing | No standard libraries |
| `-nodefaultlibs` | 📋 Missing | No default libraries |
| `-Wl,<opts>` | 📋 Missing | Pass options to linker |

### Machine Flags (x86_64)
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-m64` | 📋 Missing | 64-bit mode (default on x86_64) |
| `-mcmodel=kernel` | 📋 Missing | Kernel code model → LLVM CodeModel::Kernel |
| `-mcmodel=small` | 📋 Missing | Small code model (default) |
| `-mno-red-zone` | 📋 Missing | Disable red zone → LLVM noredzone attr |
| `-march=<cpu>` | 📋 Missing | Target CPU architecture |
| `-mtune=<cpu>` | 📋 Missing | Tune for CPU |
| `-mpreferred-stack-boundary=<N>` | 📋 Missing | Stack alignment |

### Feature Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-ffreestanding` | 📋 Missing | Freestanding environment |
| `-fno-strict-aliasing` | 📋 Missing | Disable TBAA |
| `-fno-common` | 📋 Missing | No common symbols |
| `-fno-PIE` / `-fno-PIC` | 📋 Missing | Position-dependent code |
| `-fshort-wchar` | 📋 Missing | 2-byte wchar_t |
| `-fno-asynchronous-unwind-tables` | 📋 Missing | No .eh_frame |
| `-fdata-sections` | 📋 Missing | Per-data-item sections |
| `-ffunction-sections` | 📋 Missing | Per-function sections |
| `-fno-stack-protector` | 📋 Missing | No stack canaries |
| `-fno-delete-null-pointer-checks` | 📋 Missing | Kernel null-check preservation |

### Info Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `--version` | 📋 Missing | Print version string |
| `-dumpversion` | 📋 Missing | Print version number only |
| `-dumpmachine` | 📋 Missing | Print target triple |
| `-v` | 📋 Missing | Verbose mode |
| `-###` | 📋 Missing | Print commands without executing |
| `-print-file-name=<name>` | 📋 Missing | Print path to file (Kbuild uses this) |

### Dependency File Flags
| Flag | Status | Implementation Notes |
|------|--------|---------------------|
| `-MD` | 📋 Missing | Generate dependency file |
| `-MF <file>` | 📋 Missing | Dependency output file |
| `-MP` | 📋 Missing | Add phony targets for headers |
| `-MT <target>` | 📋 Missing | Override dependency target name |
| `-MMD` | 📋 Missing | Like -MD but skip system headers |

## FREESTANDING MODE

When `-ffreestanding` is active:
- Do NOT auto-add system include paths (`/usr/include`, etc.)
- Do NOT assume any standard library functions exist
- Do NOT generate calls to `memcpy`, `memset`, `memmove`, `memcmp` implicitly
- Only predefined macros and compiler builtins are available
- `__STDC_HOSTED__` must be defined as `0`

## DEPENDENCY FILE GENERATION

Kbuild uses dependency files for incremental builds. Format:
```makefile
path/to/file.o: path/to/file.c include/header1.h include/header2.h

include/header1.h:
include/header2.h:
```
- `-MD` enables generation alongside compilation
- `-MF file.d` specifies the output path
- `-MP` adds empty rules for each header (prevents make errors when headers deleted)
- `-MT target` overrides the target name in the dependency rule
- `-Wp,-MD,file.d` is Kbuild's preferred form (passes -MD to preprocessor)

## RESPONSE FILES

Some build systems pass compiler flags via response files:
```bash
optic_c @flags.txt input.c -o output.o
```
Where `flags.txt` contains one flag per line. Must be supported for large flag sets.

## IMPLEMENTATION STRATEGY

### Priority 1 — Kernel Build Blockers
Flags that cause Kbuild to fail if not recognized: `-ffreestanding`, `-nostdinc`, `-mcmodel=kernel`, `-mno-red-zone`, `-fno-PIE`, `-fno-common`, `-c`, `-o`, `-I`, `-D`, `-include`

### Priority 2 — Kbuild Integration
Flags needed for correct build behavior: `-MD`/`-MF`/`-MP`/`-MT`, `-Wp,-MD,depfile`, `-print-file-name=`, `--version`, `-dumpversion`, `-dumpmachine`

### Priority 3 — Silently Ignore
Flags that can be safely ignored with a warning: `-Wall`, `-Wextra`, `-Wno-*`, `-g`, `-pipe`, most `-f` and `-m` flags not affecting correctness

### Acceptance Criteria
1. `optic_c --version` prints a version string
2. `optic_c -dumpmachine` prints `x86_64-linux-gnu`
3. `optic_c -ffreestanding -nostdinc -mcmodel=kernel -mno-red-zone -O2 -c test.c -o test.o` produces valid ELF
4. `optic_c -Wp,-MD,test.d -c test.c -o test.o` produces both .o and .d files
5. Unrecognized flags produce a warning but do not cause errors
6. `CC=optic_c` works in a simple Makefile
