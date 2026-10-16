# Valerie

Valerie is a build system and compiler toolchain for formally verified cryptographic assembly. It drives the compilation, machine lowering, and formal verification of assembly specifications across **AArch64**, **x86_64**, **Arm32**, and **RISC-V 64**.

At the core of the project is **`valerie-core`**, a complete rewrite of the Vale verified assembly framework in modern **F\*** and **Pulse separation logic**, replacing the legacy C#/.NET toolchain with a self-contained, native compiler and formal semantics.

---

## The Core Engine: `valerie-core`

[`valerie-core`](valerie-core) is the formal verification and compilation backend:

- **Self-Contained in F\* & Native OCaml**: Replaces legacy Mono/C# compiler dependencies with an authentic verified AST pipeline, desugarer, and native lexer/parser (`src/parser/`, `src/compiler/`) written in modern F\* and OCaml.
- **First-Class Pulse Separation Logic (`src/pulse/`)**: Implements machine registers and heap memory using **Pulse** (`#lang-pulse`). Replaces legacy $O(N^2)$ pairwise disjointness conditions with spatial separating conjunction (`**`), proving linear heap and register framing directly.
- **Formal Refinement Proofs**: Proves full simulation lemmas establishing that physical Pulse separation logic machine steps simulate the mathematical operational semantics 1:1.
- **Universal Multi-Architecture Semantics (`src/spec/`)**: Complete formal machine definitions, instruction decoders, Weakest Precondition (WP) calculi, and calling convention proofs across **AArch64**, **x86_64**, **Arm32**, and **RISC-V 64**.
- **Verified Compiler Executable (`valy` / `valerie-compiler`)**: Extracted and compiled to a native binary that emits clean target assembly, F\* proof obligations, Pulse separation logic, or IR AST dumps.

---

## What Valerie Can Do

- **Compiles Vale specifications (`.vaf`)**: Lowers verified procedure ASTs to clean assembly (`.s`), F\* (`.fst`/`.fsti`), Pulse separation logic, or IR AST dumps without compiler mutations.
- **Verifies formal proofs**: Orchestrates F\*, Pulse, and Vale verification tasks through a parallel dependency DAG with topological wave scheduling.
- **Builds target objects and libraries**: Compiles emitted assembly to object files (`.o`), static libraries (`.a`), and binaries using Clang or LLVM backends.
- **Executes and tests code**: Runs native execution tests directly or emulates cross-architecture targets under QEMU (`qemu-aarch64`, `qemu-arm`, `qemu-x86_64`, `qemu-riscv64`).
- **Audits constant-time safety**: Statically audits assembly opcodes for secret-dependent branches, variable-latency instructions (division/remainder), and ARMv8.4-A Data Independent Timing (`DIT`) enforcement.
- **Generates C/C++ ABI headers**: Produces clean C header prototypes (`.h`) directly from procedure signatures.
- **Manages declarative workspaces**: Drives verification targets, caches, and build rules through `valerie.toml` manifests.
- **Installs the compiler**: Auto-compiles and installs the verified backend compiler to `~/.local/bin/valy` (with `valerie-compiler` and `vale` aliases) via `valerie vale --install`.

---

## Architecture Matrix

| Architecture | Word Size | ABI | Extensions & Features |
| :--- | :---: | :--- | :--- |
| **AArch64** | 64-bit | AAPCS64 (Linux, macOS, Windows) | Crypto (AES, SHA-2/3, SM3/4), PMULL, DIT |
| **x86_64** | 64-bit | System V AMD64, Windows x64 | AES-NI, VAES, PCLMULQDQ, SHA-NI |
| **RISC-V 64** | 64-bit | lp64d | Zk scalar cryptography, Zbb |
| **Arm32** | 32-bit | AAPCS32 Hard/Soft-Float | Core ALU, DSP bitops, VFP/NEON |

---

## Commands

| Command | Shorthand | What it does |
| :--- | :---: | :--- |
| `valerie build [file]` | `valerie b` | Compiles source into an object file (`.o`) or workspace static library (`.a`) |
| `valerie emit <file>` | | Emits assembly (`.s`), F\* (`.fst`), Pulse (`--pulse`), or IR |
| `valerie verify [file]` | `valerie v` | Verifies F\*, Pulse, or Vale formal proofs |
| `valerie test [filter]` | `valerie t` | Runs test suite natively or under QEMU (`--qemu`) |
| `valerie run <file>` | `valerie r` | Runs an assembly or executable file (Native or QEMU) |
| `valerie audit <file>` | | Checks assembly instructions for constant-time safety and DIT state |
| `valerie header <file>` | | Generates C ABI header declarations (`.h`) |
| `valerie bundle <file>` | | Packages compiled artifacts (`.s`, `.o`, `.h`, `.a`) into an output directory |
| `valerie check [file]` | `valerie c` | Checks AST and typing without code emission |
| `valerie diff <file>` | | Validates assembly parity against compiler output |
| `valerie clean` | | Clears build artifacts and verification caches |
| `valerie init [name]` | | Scaffolds a new project with a `valerie.toml` manifest |
| `valerie completions <shell>` | | Generates shell completions (supports `--install`) |
| `valerie vale [args...]` | | Direct pass-through to verified compiler (`--install` to build and install `valy`) |

---

## Setup & Installation

### Build Driver
```bash
cargo build --release
cp target/release/valerie ~/.local/bin/valerie
```

### Install Backend Compiler (`valy`)
Compiles the verified `valerie-core` backend and installs it to `~/.local/bin/`:
```bash
valerie vale --install
```

---

## Repository Structure

- **[`valerie-core`](valerie-core)**: The verified Vale compiler and formal semantics in F\* and Pulse (`src/spec/`, `src/pulse/`, `src/proof/`, `src/compiler/`, `src/parser/`).
- **[`crates/valerie-driver`](crates/valerie-driver)**: Unified CLI driver (`valerie`), build system, test runner, constant-time auditor, and F\*/Pulse verification bridge.
- **[`crates/valerie-manifest`](crates/valerie-manifest)**: Declarative `valerie.toml` manifest parsing, task dependency scheduler, and toolchain resolution.
- **[`crates/valerie-target`](crates/valerie-target)**: Target architecture models, register files, and calling convention definitions for AArch64, x86_64, Arm32, and RISC-V 64.
- **[`crates/valerie-depgraph`](crates/valerie-depgraph)**: Parallel verification DAG scanner and topological dependency wave scheduler.

---

## Codebase Metrics

<!-- LOC_STATS_START -->
```text
+----------------------+-------+-------+---------+-------+---------+
| Language             | Files | Lines |    Code | Comm. |   Blank |
+----------------------+-------+-------+---------+-------+---------+
| F* / Pulse           |    45 |  9339 |    8470 |   226 |     643 |
| Rust                 |    33 |  9470 |    8059 |   544 |     867 |
| Vale (.vaf)          |    29 |  3801 |    3165 |   115 |     521 |
| OCaml                |     7 |   793 |     715 |     8 |      70 |
| Menhir Parser        |     1 |   555 |     501 |     4 |      50 |
| OCamllex             |     1 |   310 |     280 |     1 |      29 |
| TOML                 |     7 |   201 |     178 |     1 |      22 |
| Python               |     1 |    82 |      68 |     1 |      13 |
+----------------------+-------+-------+---------+-------+---------+
| Total                |   124 | 24551 |   21436 |   900 |    2215 |
+----------------------+-------+-------+---------+-------+---------+
```
<!-- LOC_STATS_END -->

## License

Licensed under the GNU Affero General Public License v3.0. See [LICENSE](LICENSE).
