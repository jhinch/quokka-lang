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
- HIR gen (lowering AST to High IR)
- Conversion to static single assignment form
- Optimization (dead code elimination, inlining, etc.)
- Phi elimination
- Register allocation
- Assembly Code generation
- Linking

## Spec Tests

The conformity to the language specification defined in the [Quokka Lang Reference](docs/src/reference) is validated
via the spec test suites.

```bash
cargo test --test spec_tests
```

Test suites are defined in TOML files grouped by category within [tests/cases](tests/cases) directory.

### TOML file format

#### Lexer test

```toml
[[case]]
# name (required) of the test case
name = "case_name"
# kind (required) always "lexer" for lexer test
kind = "lexer"
# preview_features (optional) which preview features to activate for this test. defaults to none
preview_features = []
# ignored (optional) ignores the test case. Defaults to true when preview_features is not empty, false otherwise
ignored = false
# source (required) the source code for the program
source = """
fn main() {
}
"""
# either expected_tokens or expected_errors is required
# expected_tokens the list of token kinds and token text in order
expected_tokens = """
Fn:fn
Ident:main
LeftParen:(
RightParen:)
LeftBrace:{
RightBrace:}
Eof:
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
# ignored (optional) ignores the test case. Defaults to true when preview_features is not empty, false otherwise
ignored = false
# source (required) the source code for the program
source = """
fn main() {
}
"""
# either expected_ast or expected_errors is required
# expected_ast a string for the RON representation of the AST
expected_ast = """
Root(
  items: [
    Item(
      data: FnDecl(FnDecl(
        signature: FnSig(
          name: ("main", 3, 4),
        ),
        body: Block(
          statements: [],
        ),
      )),
    ),
  ],
)
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
# kind (required) always "analyzer" for parser test
kind = "analyzer"
# preview_features (optional) which preview features to activate for this test. defaults to empty
preview_features = []
# ignored (optional) ignores the test case. Defaults to true when preview_features is not empty, false otherwise
ignored = false
# source (required) the source code for the program
source = """
fn main() {
}
"""
# either expected_ast or expected_errors is required
# expected_ast a string for the RON representation of the AST
expected_ast = """
Root(
  items: [
    Item(
      data: FnDecl(FnDecl(
        signature: FnSig(
          name: ("main", 3, 4),
        ),
        body: Block(
          statements: [],
        ),
      )),
    ),
  ],
)
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
