# Quokka Lang

A high level language designed for developing Game Boy games.

WARNING: This is in the early stages of development and large portions
of the compiler are not written yet

# Internals

## Compiler structure

The compilation goes through following phases:

- Lexer
- Parsing
- Type checking / Semantic analysis
- Lowering to Quokka IR
- Conversion to static single assignment form
- Optimisation (dead code elimination, inlining, etc.)
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

### Regenerate expectations

It is common that the expectations in test suites no longer match the compiler's internal and that the expectations
need to be updated rather than it being a bug in the compiler. This can be achieved by running the tests using
the `REGENERATE_EXPECTATIONS` environment variable.

```bash
REGENERATE_EXPECTATIONS=1 cargo test --test spec_tests
```