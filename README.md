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
- Optimization (dead code elimination, inlining, etc. Currently, not implemented)
- Code gen (lowering MIR to LIR)
- Phi elimination (Happens concurrently with code gen)
- Register allocation (Currently, not implemented)
- Object file creation (Currently, not implemented)
- Linking (Currently, not implemented)

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
