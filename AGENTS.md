# AGENTS.md — Valerie Engineering & Verification Rules

This document outlines the mandatory operational principles, architectural invariants, and workflow guidelines for all AI agents working within the Valerie ecosystem. Compliance with these rules is mandatory.

---

## 1. Core Verification & Compilation Philosophy

- **Zero-Bypass Verification**: Never disable, mock, stub, or bypass verification proofs or compiler pipelines.
- **No Dummy Code or Fake Stubs**: Never emit dummy instructions, synthetic `ret` blocks, or mock return values to make a test appear green. Every procedure, transformation, and lowering must be semantically authentic and fully realized in target machine instructions.
- **Root-Cause Resolution**: Fix bugs at their origin in the architecture or compiler specifications. Never patch symptomatically at the edges or introduce ad-hoc shortcuts.

---

## 2. Artifact Hygiene & Source of Truth

- **Sources Over Generated Artifacts**:
  - Always make modifications in formal specification files (`src/spec/`, `src/compiler/`, `src/proof/`) or native driver crates (`crates/`).
  - **Never directly edit extracted or generated artifacts** (e.g., `dist/ocaml/`, extracted C files, or build caches).
  - All artifacts in `dist/` must remain 100% reproducible through formal extraction and native compilation tasks.
- **Cache Integrity**:
  - Respect verification caches (`.cache/`, `hints/`). When modifying interfaces or AST definitions, ensure dependent caches are cleanly refreshed and re-verified.
- **Version Control Invariant**:
  - **Never execute raw Git commands** (`git add`, `git commit`, `git checkout`, `git reset`, `git status`, etc.) unless explicitly instructed by the user.

---

## 3. Multi-Architecture Parity & Execution Workflow

- **Universal Target Parity**:
  - Valerie supports multiple distinct target architectures (AArch64, x86_64, RISC-V 64, Arm32).
  - Architectural changes, instruction definitions, lowering passes, and emitter rules must maintain parity across all supported ISAs.
  - An architecture must not be left half-implemented or stubbed out.
- **Pure Assembly Emission**:
  - `valerie emit` must output 100% clean, pure, target-compliant assembly code.
  - Primitive hardware instructions defined as instruction procedures must be lowered to actual machine instructions and never emitted as standalone functions with dummy stubs.
  - Assembly output must remain clean of concatenated test harnesses or ad-hoc entrypoints; target test harnesses must be synthesized dynamically by the driver at execution time.
- **Rigorous Multi-Engine Testing**:
  - Validate code execution on both Native hardware and QEMU cross-architecture emulation.
  - An implementation is only complete when it builds, verifies, links, and runs to completion (exit code 0) across all targeted architectures.

---

## 4. Driver & Task Orchestration Workflow

- **Workspace-Driven Tasks**:
  - Always utilize the defined workspace manifests (`valerie.toml`, `Cargo.toml`) and official CLI workflows (`valerie build`, `valerie emit`, `valerie run`, `valerie test`, and defined `[tasks.*]`).
  - Keep task configurations declarative and reproducible across environments.
- **Compiler Construction Pipeline**:
  - Follow the canonical compilation lifecycle:
    1. Parse source specifications into AST.
    2. Typecheck and verify proof obligations.
    3. Lower AST to target-specific machine semantics.
    4. Emit verified assembly conforming to target ABI.
    5. Assemble, link, and validate binary execution.

---

## 5. Cryptographic Safety & Constant-Time Guarantees

- **Constant-Time Invariant**:
  - Cryptographic routines must execute in constant time.
- **Zero Compiler Mutations**:
  - Verified assembly sequences must match formal models bit-for-bit without compiler-induced instruction inflation, unexpected reordering, or register spilling bugs.

---

## 6. Agent Conduct & Pair-Programming Standards

- **Proactive & Concrete Execution**:
  - Formulate rigorous plans, inspect ground truth directly from the filesystem, and execute changes decisively.
  - Do not ask for user intervention on tasks you have the tools and capabilities to solve.
- **Verification Evidence**:
  - Always corroborate success with concrete command output (build logs, compilation statuses, test results, disassembly inspections).
- **Preserve Documentation & Context**:
  - Maintain the integrity of existing comments, architectural notes, and docstrings unless explicitly asked to modify them.
