[![CI](https://github.com/jhinch/quokka-lang/actions/workflows/main.yml/badge.svg)](https://github.com/jhinch/quokka-lang/actions/workflows/main.yml)

# Quokka Lang

A high level language designed for developing Game Boy games.

WARNING: This is in the early stages of development and large portions
of the compiler are not written yet

# Internals

## Compiler structure

The compilation goes through following phases:

- Lexing (Lexical analysis)
- Parsing (Syntactic analysis)
- Analysis (Semantic analysis / type checking)
- Lowering (lowering AST to MIR / conversion to static single assignment form. Happens concurrently with analysis)
- Optimization
- Code gen (lowering MIR to LIR)
- Phi elimination (Happens concurrently with code gen)
- Register allocation
- Object file creation
- Linking (Currently delegates to RGBDS)

## Spec Tests

The conformity to the language specification defined in the [Quokka Lang Reference](docs/src/reference) is validated
via the spec test suites.

```bash
cargo test --test spec_tests
```

Test suites are defined in TOML files grouped by category within [tests/cases](tests/cases) directory. The test names
take the form `<category>::<suite>::<case>`, where the category is the directory and the suite is the file name
without its extension.

### TOML file format

The TOML file specifies one or more test cases. A file can contain cases of different kinds as specified
below. Regardless of the kind of test, the name of the test must be unique within the file, which the suite's
`_lint_cases` test enforces. Unknown fields in a case are rejected.

Each entry of `expected_errors` takes the form `<code>: <message> @ [<start>:<end>]`, where the code is omitted for
diagnostics without one and the byte offsets are those of the diagnostic's primary label (omitted if it has none).

#### Lexer test

```toml
[[case]]
# name (required) of the test case
name = "case_name"
# kind (required) always "lexer" for lexer test
kind = "lexer"
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program. Alternatively, `source.file = "filename.q"` for an external file,
# resolved relative to the directory of the TOML file
source = """
fn main() {
}
"""
# expected_tokens (optional) one `<kind>:<text>` line per token in order. Lexing stops at the first error, so the
# tokens before it are still expected. defaults to empty
expected_tokens = """
...
"""
# expected_errors (optional) array of strings for each of the errors expected. defaults to empty
expected_errors = []

# More than one case can be defined per test suite file
[[case]]
# ...
```

#### Parser test

```toml
[[case]]
# name (required) of the test case
name = "case_name"
# kind (required) always "parser" for parser test
kind = "parser"
# entrypoint (optional) the parsing entrypoint: "root", "item", "statement" or "expr". Defaults to "root"
entrypoint = "root"
# preview_features (optional) which preview features to activate for this test. defaults to empty
preview_features = []
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program. Alternatively, `source.file = "filename.q"` for an external file,
# resolved relative to the directory of the TOML file
source = """
fn main() {
}
"""
# expected_ast (optional) a string for the RON representation of the AST. The parser recovers from errors, so a case
# can expect both an AST and errors. defaults to empty
expected_ast = """
...
"""
# expected_errors (optional) array of strings for each of the errors expected. defaults to empty
expected_errors = []

# More than one case can be defined per test suite file
[[case]]
# ...
```

#### Analyzer test

```toml
[[case]]
# name (required) of the test case
name = "case_name"
# kind (required) always "analyzer" for analyzer test
kind = "analyzer"
# preview_features (optional) which preview features to activate for this test. defaults to empty
preview_features = []
# optimize (optional) enables optimization on MIR. Defaults to false.
# Individual optimizations can be opted in to (if off by default) or opted out of (if on by default) via
# preview_features, e.g. "no-canonicalize", "no-adce", "no-sccp", "no-prune" or "no-tree-shake"
optimize = true
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program. Alternatively, `source.file = "filename.q"` for an external file,
# resolved relative to the directory of the TOML file
source = """
fn main() {
}
"""
# either expected_mir or expected_errors is required
# expected_mir a string for the generated MIR
expected_mir = """
...
"""
# expected_errors array of strings for each of the errors expected from analysis or optimization. The source must
# parse: a syntax error fails the test rather than being compared
expected_errors = []

# More than one case can be defined per test suite file
[[case]]
# ...
```

#### Codegen test

```toml
[[case]]
# name (required) of the test case
name = "case_name"
# kind (required) always "codegen" for codegen test
kind = "codegen"
# preview_features (optional) which preview features to activate for this test. defaults to empty
preview_features = []
# optimize (optional) enables optimization on MIR. Defaults to false.
# Individual optimizations can be opted in to (if off by default) or opted out of (if on by default) via
# preview_features, e.g. "no-canonicalize", "no-adce", "no-sccp", "no-prune" or "no-tree-shake"
optimize = true
# allocate_registers (optional) enables register allocation followed by peephole optimization on LIR.
# Defaults to false.
allocate_registers = true
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program. Alternatively, `source.file = "filename.q"` for an external file,
# resolved relative to the directory of the TOML file
source = """
fn main() {
}
"""
# either expected_lir or expected_errors is required
# expected_lir a string for the generated LIR
expected_lir = """
...
"""
# expected_errors array of strings for each of the errors expected from code generation. The source must parse,
# analyze and optimize: an error from those phases fails the test rather than being compared
expected_errors = []

# More than one case can be defined per test suite file
[[case]]
# ...
```

### Regenerate expectations

It is common that the expectations in test suites no longer match the compiler's internal and that the expectations
need to be updated rather than it being a bug in the compiler. This can be achieved by running the tests using
the `REGENERATE_EXPECTATIONS` environment variable.

```bash
REGENERATE_EXPECTATIONS=1 cargo test --test spec_tests
```

`REGENERATE_EXPECTATIONS` will generate the expectations when none exist and will adjust existing the values
within existing expectations. It will *not*, however, remove an existing. For example, it will not change
a test asserting no errors into one that asserts errors (or vice versa). Doing that silently would let a real
regression masquerade as an updated expectation.

Expectations can be forced to be updated using the `FORCE_REGENERATE_EXPECTATIONS` environment variable:

```bash
REGENERATE_EXPECTATIONS=1 FORCE_REGENERATE_EXPECTATIONS=1 cargo test --test spec_tests
```

Only reach for `FORCE_REGENERATE_EXPECTATIONS` once you've confirmed the change in behaviour is intentional.

## Optimization Soundness Tests

The MIR optimizer is validated via differential execution rather than by comparing against known output. Each case
compiles a small program twice, once without and once with optimization, interprets both resulting MIRs using the
compile-time constant evaluator across a range of input values and asserts that the results are byte-identical.

```bash
cargo test --test optimize_soundness_tests
```

Test suites are defined in TOML files within the [tests/soundness_cases](tests/soundness_cases) directory.

Every case is run as multiple tests, one for each optimization that can be individually enabled (currently none), one
for each optimization that can be individually disabled (`no-canonicalize`, `no-adce`, `no-sccp`, `no-prune` and
`no-tree-shake`), and one (`all`) with the default optimization configuration. The test names take the form
`optimize_soundness::<suite>::<case>::<variant>`. A suite which targets a single optimization can list the `variants`
worth running at the top of its file (e.g. `variants = ["no-canonicalize", "all"]`), and a case can list its own to
override the suite's.

A case may check at most 65,536 combinations of param and input values, which the suite's `_lint_cases` test enforces.
A case over the limit should be restructured to check fewer, or be given a dedicated sampling of its values: `"edges"`
samples the values of an input's type most likely to expose a bug, and `"sparse-edges"` a smaller set of them for cases
which combine two wide inputs. A param has no type to take a width from, so it samples with an explicit width in bits
instead: `{ edges = 8 }`.

Since the evaluator does not support function calls or stores, the function under test must be side-effect free: it
reads its inputs from globals and returns a value. A `main` is still required in the source as the analyzer requires
an entrypoint, but it is never executed.

A global is never a literal, so an optimization that only matches a literal operand cannot be swept through an input.
For those, a case can declare `params`: each `$NAME$` placeholder in the source is replaced with the value and the
program is compiled and checked once per combination of parameter values.

Matching results prove nothing if the optimization under test never fired. A case can list optimizations under
`expect_rewrite`: the test for each listed optimization also fails if toggling it alone leaves the optimized MIR
unchanged. For an opt-out such as `no-canonicalize`, that means the pass it disables must change the MIR.

### TOML file format

The name of each case must be unique within the file. Unknown fields are rejected.

```toml
# variants (optional) the variants to run for every case in the file which does not list its own. Each is either
# the name of an optimization feature or "all". Must not be empty. defaults to every variant
variants = ["no-canonicalize", "all"]

[[case]]
# name (required) of the test case
name = "case_name"
# function (optional) the function to evaluate. Defaults to "f". It must take no parameters
function = "f"
# preview_features (optional) additional preview features required for the source to compile (i.e. not
# optimization related). defaults to empty
preview_features = []
# inputs (required) the values to feed into each named global. The function is evaluated once per combination of
# input values. Each entry of a global's list is one of:
# - a single value, e.g. `127`
# - an inclusive range, e.g. `{ min = 0, max = 255 }`
# - "edges", a sample of the values of the global's type most likely to expose a bug
# - "sparse-edges", a smaller such sample for cases which combine two wide inputs
# - `{ edges = <bits> }`, the "edges" of an explicit width no wider than the global's type
# Each value is a raw unsigned bit pattern which must fit in the global's type and is encoded to its width
inputs = { N = [{ min = 0, max = 255 }], M = [0, 255, "edges"] }
# params (optional) the values to substitute, in decimal, for each $NAME$ placeholder in the source. The source is
# compiled once per combination of parameter values. Each entry is a single value, an inclusive range or
# `{ edges = <bits> }`: "edges" and "sparse-edges" are not allowed as a param has no type. Every param must have a
# placeholder in the source. defaults to empty
params = { C = [1, { min = 254, max = 255 }] }
# expect_rewrite (optional) optimization features which must change the optimized MIR when toggled on their own.
# Only checked by the variant of the same name. defaults to empty
expect_rewrite = ["no-canonicalize"]
# variants (optional) the variants to run for this case, overriding the file's. Must not be empty. defaults to the
# file's variants
variants = ["no-canonicalize", "all"]
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program
source = """
global N: u8;
global M: u8;

fn f() -> u8 {
    let n = N;
    let a = n + $C$;
    let b = a + n;
    return b ^ M;
}

fn main() {
    N = 1;
    M = 1;
    f();
}
"""

# More than one case can be defined per test suite file
[[case]]
# ...
```

## Emulator Tests

The emulator tests validate the generated code by executing it. Each case compiles a program all the way to a Game Boy
ROM, runs it in the [SameBoy](https://github.com/LIJI32/SameBoy) emulator via a [headless bridging crate](crates/sameboy)
until the CPU reaches a given address and then asserts the contents of memory.

Every case is also watched by runtime checks while it runs, and fails if one reports a violation:

- **OAM corruption hazard**: an instruction that steps a 16-bit register holding an address in `$FE00-$FEFF` (such as
  `INC rr`, `LD [HL+], A` or a `PUSH` with `SP` there), which corrupts OAM on a DMG if the PPU is scanning it. It is
  reported whatever the PPU is doing and on every model.

```bash
cargo test --test emulator_tests
```

The tests require `rgblink` and `rgbfix` (from [RGBDS](https://rgbds.gbdev.io/)) to be on the `PATH`, and the
SameBoy submodule to be initialized (`git submodule update --init`).

Test suites are defined in TOML files within the [tests/emulator_cases](tests/emulator_cases) directory.

Every case is built with optimization, register allocation and peephole optimization, and is run as multiple tests:
one (`default`) with only the case's own preview features, one for each optimization that can be individually
enabled (currently none) and one for each optimization which changes what instruction selection sees and can be
individually disabled (currently `no-canonicalize`). The test names take the form
`emulator::<suite>::<case>::<variant>`. A suite can list the `variants` worth running at the top of its file (e.g.
`variants = ["no-canonicalize"]`), and a case can list its own to override the suite's, for cases which only build, or
only mean anything, one way.

Because `await_pc` is a plain address, a case will typically end by calling an empty function pinned to a fixed
address using `#[address]` and use that address as the `await_pc`. Likewise, values to be asserted are typically stored
in globals pinned to a fixed address (e.g. in WRAM at `0xC000`). This gives a stable checkpoint which does not depend
on incidental codegen layout.

### TOML file format

The name of each case must be unique within the file, which the suite's `_lint_cases` test enforces. Unknown fields
are rejected.

```toml
# variants (optional) the variants to run for every case in the file which does not list its own. Each is either
# "default" or the name of an optimization feature the cases are run with. Must not be empty. defaults to every
# variant
variants = ["default", "no-canonicalize"]

[[case]]
# name (required) of the test case
name = "case_name"
# preview_features (optional) which preview features to activate for this test. defaults to empty
preview_features = []
# variants (optional) the variants to run for this case, overriding the file's. Must not be empty. defaults to the
# file's variants
variants = ["default"]
# model (optional) the Game Boy model to emulate: "dmg", "mgb" or "cgb". Defaults to "dmg"
model = "dmg"
# max_instructions (optional) the maximum number of instructions to execute while waiting to reach await_pc.
# The test fails if it is not reached within this limit, and prints the last instructions it executed.
# Defaults to 1000
max_instructions = 1000
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program
source = """
#[address = 0x0150]
fn checkpoint() {}

#[address = 0xC000]
global OUT: u8;

fn main() {
    OUT = 3;
    checkpoint();
}
"""
# await_pc (required) the address the CPU's PC register must reach. Memory is asserted once it is reached
await_pc = 0x0150

# assertion (optional) zero or more assertions on memory checked once await_pc is reached
[[case.assertion]]
# addr (required) the address of the first byte to check
addr = 0xC000
# expected (required) either a single byte or an array of consecutive bytes starting at addr
expected = 3
# mask (optional) the bits to compare. Either a single value applied to every byte or an array of one value per byte
# of expected.
# Defaults to 0xff (all bits compared). Useful when only some bits are defined, e.g. a `u1` global only defines bit 0
# and the remaining bits of its byte are left as whatever the emulator randomized WRAM to
mask = 0b0000_0001

# More than one assertion can be defined per case
[[case.assertion]]
# ...

# More than one case can be defined per test suite file
[[case]]
# ...
```

## Documentation Example Tests

Every ` ```quokka ` code block in the [mdBook sources](docs/src) is compiled to ensure the examples in the
documentation stay in sync with the compiler. Each example goes through the same pipeline as `quokka build`, up to
(but not including) linking. Examples which don't define an entrypoint have an empty `fn main() {}` appended so that
fragments demonstrating items can be compiled on their own.

```bash
cargo test --test doc_tests
```

Each test is named after the markdown file and line of the opening code fence, e.g. `reference/items/structs.md:15`.
Code blocks are configured through comma-separated attributes in the fence's info string:

- ` ```quokka ` the example must compile
- ` ```quokka,preview=narrow-int ` the example must compile with the given preview feature enabled. Repeat the
  attribute to enable multiple preview features
- ` ```quokka,ignore ` the example is illustrative only and is not compiled
