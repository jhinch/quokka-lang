# AGENTS.md

This file provides guidance to AI agents when working with code in this repository.

Quokka is a high-level language and compiler targeting the Game Boy (SM83/"gbz80" CPU). The compiler emits RGBDS
object files and delegates linking to `rgblink`/`rgbfix`. The language reference (mdBook) lives in `docs/src/reference`
and is the specification the spec tests check against.

## Commands

```bash
cargo build --workspace
cargo test --workspace                     # everything CI runs (also runs in --release)
cargo clippy --workspace
cargo fmt --all
cargo doc --workspace --no-deps            # CI checks rustdoc (broken intra-doc links are denied)

# individual test binaries (all but lsp_tests use libtest-mimic, harness = false)
cargo test --test spec_tests               # tests/cases/**/*.toml
cargo test --test optimize_soundness_tests # tests/soundness_cases/*.toml
cargo test --test emulator_tests           # tests/emulator_cases/*.toml (needs RGBDS + SameBoy submodule)
cargo test --test doc_tests                # every ```quokka block in docs/src
cargo test --test lsp_tests

# single test / suite: pass a name filter after --
cargo test --test spec_tests -- expr::if::
cargo test --test emulator_tests -- emulator::signed_comparison::
cargo test --test spec_tests -- --exact codegen::flags::some_case

# regenerate spec test expectations (won't flip "no errors" <-> "errors" unless FORCE_ is also set)
REGENERATE_EXPECTATIONS=1 cargo test --test spec_tests
REGENERATE_EXPECTATIONS=1 FORCE_REGENERATE_EXPECTATIONS=1 cargo test --test spec_tests

# run the compiler
cargo run -- build examples/hello_world.q          # -p/--preview <feature> to enable preview features
cargo run -- analyze file.q                         # hidden plumbing commands: lex, parse, analyze, codegen
```

Prerequisites: `rgblink`/`rgbfix` on `PATH` (memory-map spec tests, emulator tests, `quokka build`), and
`git submodule update --init` for `crates/sameboy-sys/SameBoy`. `./hooks/configure` installs the git hooks; the
pre-commit hook runs `cargo fix`/`clippy --fix`/`fmt`, then the full fmt/test/clippy/doc check.

`src/lib.rs` denies several clippy lints (`cast_possible_truncation`, `must_use_candidate`,
`return_self_not_must_use`, `manual_assert`, ...) and `unsafe_code`, so clippy failures are build-breaking in CI.

## Pipeline architecture

`src/pipeline.rs` is the typestate driver every entrypoint (CLI, LSP, test harnesses) goes through:
`Pipeline::parse → ParserOutput::analyze → AnalyzerOutput::optimize → codegen → CodegenOutput::allocate →
AllocateOutput::peephole → PeepholeOutput::encode → write_obj → link`. Each stage carries the `PipelineConfig`
(the enabled `PreviewFeatureSet`) forward. Errors are accumulated as `MultiError`/`MultiResult` (`src/multi.rs`) and
reported with `miette`, so passes generally collect several diagnostics rather than bailing on the first.

- **Front end**: `lex.rs` → `parse.rs` (error-recovering, produces `ast.rs`) → `analyze/` (type inference in
  `infer.rs`/`solver.rs`; lowering to MIR happens *concurrently* with analysis, not as a separate pass).
- **MIR** (`mir.rs`): SSA with phi nodes, per-function basic blocks, globals in a `SlotMap`. `Function::check_integrity`
  validates invariants and is run after every optimization round. `consteval.rs` interprets MIR (const decls and the
  soundness test oracle).
- **Optimizer** (`optimize/`): passes implement `Optimize` and are run in rounds until no pass reports `Modified`.
  Passes: `canonicalize/` (rule modules rewriting toward one normal form per operation), `sccp`, `adce`, `prune`,
  `tree_shake`. All are on by default and disabled via `no-*` preview features.
- **Codegen** (`codegen/`): `lower/` selects instructions from MIR into LIR (`lir.rs`) with virtual registers and does
  phi elimination; `regalloc.rs` + `interference.rs` is a graph-coloring allocator with spilling; `peephole.rs` runs
  post-allocation rewrites; `encode.rs` produces the RGBDS object (`crates/obj`). Multi-opcode operations (e.g. bulk
  copy/fill, 16-bit inc/dec) are single "macro" LIR instructions expanded only at encode time.
- **Memory map** (`memory_map.rs`): derived from MIR globals + encoded object + the linker `.sym` file; compiler-owned
  symbols use the `_@` prefix so they can't collide with source identifiers.
- **Preview features** (`feature.rs`): gate unstable language features and opt-out of optimizations. Gating is done in
  the analyzer (e.g. `Analyzer::resolve_ty_ref`), not the parser. Stabilizing a feature means removing its variant and
  its gates.

### Workspace crates

- `crates/obj` — RGBDS object file model, reader and writer (prefer round-trip tests).
- `crates/opcodes` — SM83 opcode constants/timings generated at build time from the `gb-opcodes` JSON.
- `crates/sameboy-sys` / `crates/sameboy` — FFI to SameBoy and a headless harness with runtime `Checker`s (OAM
  corruption, HALT bug/hang, PPU-blocked access, uninitialized reads) used by the emulator tests.
- `crates/slotkeyset` — dense/sparse key sets over `slotmap` keys.

## Tests

The README documents every TOML case format in detail; read it before adding cases. Key points:

- **Spec tests** (`tests/cases/<category>/<suite>.toml`, named `<category>::<suite>::<case>`) have kinds `lexer`,
  `parser`, `analyzer` (expected MIR), `codegen` (expected LIR, optionally with `allocate_registers`) and
  `memory-map`. Write the source only, fill expectations with `REGENERATE_EXPECTATIONS=1`, then review the diff.
  Pipeline behavior should be tested here rather than with `#[cfg(test)]` unit tests. Call these "spec tests" in prose,
  including those under `tests/cases/golden/`.
- **Optimize soundness tests** run each case with and without optimization (per `no-*` variant) through `consteval`
  over input ranges and require identical results; use `expect_rewrite` to prove the pass actually fired.
- **Emulator tests** build a real ROM, run it in SameBoy until `await_pc`, and assert memory. Cases usually call an
  empty `#[address = ...]` checkpoint function and write results to `#[address = 0xC000]` globals; cases touching
  VRAM/OAM must turn the LCD off first.
- Name tests for the correct behavior; record known failures in `ignored = "reason"` rather than in the name.

## Conventions

- American English spelling in code, comments and fixtures.
- Don't reference Linear issue IDs (`QUOK-NN`) in source, comments or test files; commit messages are fine.
