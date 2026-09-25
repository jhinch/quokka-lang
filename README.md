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

Test suites are defined in TOML files grouped by category within [tests/cases](tests/cases) directory.

### TOML file format

The TOML file specifies one or more test cases. A file can contain cases of different kinds as specified
below. Regardless of the kind of test, the name of the test must be unique within the file.

#### Lexer test

```toml
[[case]]
# name (required) of the test case
name = "case_name"
# kind (required) always "lexer" for lexer test
kind = "lexer"
# preview_features (optional) which preview features to activate for this test. defaults to none
preview_features = []
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program. Alternatively, `source.file = "filename.q"` for an external file.
source = """
fn main() {
}
"""
# either expected_tokens or expected_errors is required
# expected_tokens the list of token kinds and token text in order
expected_tokens = """
...
"""
# expected_errors array or strings for each of the errors expected
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
# name (optional) the parsing entrypoint. Defaults to root. "expr" and "item" can also be used
entrypoint = "root"
# preview_features (optional) which preview features to activate for this test. defaults to empty
preview_features = []
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program. Alternatively, `source.file = "filename.q"` for an external file.
source = """
fn main() {
}
"""
# either expected_ast or expected_errors is required
# expected_ast a string for the RON representation of the AST
expected_ast = """
...
"""
# expected_errors array or strings for each of the errors expected
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
# Individual optimizations can be opted in to (if off by default) or opted out of (if on by default) via preview_features
optimize = true
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program. Alternatively, `source.file = "filename.q"` for an external file.
source = """
fn main() {
}
"""
# either expected_mir or expected_errors is required
# expected_mir a string for the generated MIR
expected_mir = """
...
"""
# expected_errors array or strings for each of the errors expected
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
# kind (required) always "codegen" for analyzer test
kind = "codegen"
# preview_features (optional) which preview features to activate for this test. defaults to empty
preview_features = []
# optimize (optional) enables optimization on MIR. Defaults to false.
# Individual optimizations can be opted in to (if off by default) or opted out of (if on by default) via preview_features
optimize = true
# allocate_registers (optional) enables register allocation on LIR. Defaults to false.
allocate_registers = true
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program. Alternatively, `source.file = "filename.q"` for an external file.
source = """
fn main() {
}
"""
# either expected_lir or expected_errors is required
# expected_lir a string for the generated LIR
expected_lir = """
...
"""
# expected_errors array or strings for each of the errors expected
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

Every case is run as multiple tests, one for each optimization that can be individually enabled, one for each
optimization that can be individually disabled, and one (`all`) with the default optimization configuration. The
test names take the form `optimize_soundness::<suite>::<case>::<variant>`.

Since the evaluator does not support function calls or stores, the function under test must be side-effect free: it
reads the input from a global and returns a value. A `main` is still required in the source as the analyzer requires
an entrypoint, but it is never executed.

### TOML file format

The name of each case must be unique within the file.

```toml
[[case]]
# name (required) of the test case
name = "case_name"
# function (optional) the function to evaluate. Defaults to "f". It must take no parameters
function = "f"
# global (optional) the global that each value is fed into. Defaults to "N"
global = "N"
# preview_features (optional) additional preview features required for the source to compile (i.e. not
# optimization related). defaults to empty
preview_features = []
# values (required) single byte values to feed into the global. The function is evaluated once per value
values = [0, 1, 2, 127, 128, 255]
# ignored (optional) marks the test as ignored for the specified reason. when absent or empty the test is not ignored
ignored = "free form reason"
# source (required) the source code for the program
source = """
global N: u8;

fn f() -> u8 {
    return N + 1;
}

fn main() {
    N = 1;
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

```bash
cargo test --test emulator_tests
```

The tests require `rgblink` and `rgbfix` (from [RGBDS](https://rgbds.gbdev.io/)) to be on the `PATH`, and the
SameBoy submodule to be initialized (`git submodule update --init`).

Test suites are defined in TOML files within the [tests/emulator_cases](tests/emulator_cases) directory.

Because `await_pc` is a plain address, a case will typically end by calling an empty function pinned to a fixed
address using `#[address]` and use that address as the `await_pc`. Likewise, values to be asserted are typically stored
in globals pinned to a fixed address (e.g. in WRAM at `0xC000`). This gives a stable checkpoint which does not depend
on incidental codegen layout.

### TOML file format

The name of each case must be unique within the file.

```toml
[[case]]
# name (required) of the test case
name = "case_name"
# preview_features (optional) which preview features to activate for this test. defaults to empty
preview_features = []
# model (optional) the Game Boy model to emulate: "dmg", "mgb" or "cgb". Defaults to "dmg"
model = "dmg"
# max_instructions (optional) the maximum number of instructions to execute while waiting to reach await_pc.
# The test fails if it is not reached within this limit. Defaults to 1000
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
# mask (optional) the bits to compare. Either a single value applied to every byte or one value per byte of expected.
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
