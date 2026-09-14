//! Emulator-driven execution tests.
//!
//! Test cases live as TOML files under `tests/emulator_cases/`, in the same spirit as
//! `tests/spec_tests.rs` and `tests/optimize_soundness_tests.rs`, but this is a distinct kind of
//! test again: each case compiles a Quokka program all the way down to a Game Boy ROM (using the
//! same fluent [`quokka_lang::pipeline`] API the `build` CLI command uses), loads it into a
//! [`Sameboy`] instance, runs it until the CPU's `PC` register reaches `await_pc`, and then
//! asserts the contents of memory at a set of addresses.
//!
//! `await_pc` is a plain address rather than a symbolic breakpoint, so a case's interesting code
//! typically ends with a call to an empty function pinned to a fixed address via `#[address]`
//! (see `docs/src/reference/items/functions.md`), and uses that fixed address as `await_pc` -
//! this gives a stable, explicit checkpoint that doesn't depend on incidental codegen layout.

use libtest_mimic::{Arguments, Trial};
use quokka_lang::feature::PreviewFeature;
use quokka_lang::pipeline::PipelineConfig;
use quokka_lang_sameboy::{Model, Sameboy};
use std::collections::HashMap;

/// Deserialized representation of a test suite TOML file.
#[derive(serde::Deserialize)]
struct TestSuite {
    #[serde(rename = "case")]
    cases: Vec<TestCase>,
}

/// Deserialized representation of a test case within a test suite.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TestCase {
    name: String,
    #[serde(default)]
    preview_features: Vec<PreviewFeature>,
    #[serde(default)]
    model: ModelKind,
    #[serde(default = "default_max_instructions")]
    max_instructions: u32,
    #[serde(default)]
    ignored: Option<String>,
    source: String,
    /// address the CPU's `PC` register must reach for the case to pass
    await_pc: u16,
    #[serde(default, rename = "assertion")]
    assertions: Vec<Assertion>,
}

fn default_max_instructions() -> u32 {
    1_000_000
}

/// Deserialized representation of the Game Boy model to emulate.
#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ModelKind {
    #[default]
    Dmg,
    Mgb,
    Cgb,
}

impl From<ModelKind> for Model {
    fn from(kind: ModelKind) -> Self {
        match kind {
            ModelKind::Dmg => Model::Dmg,
            ModelKind::Mgb => Model::Mgb,
            ModelKind::Cgb => Model::Cgb,
        }
    }
}

/// A single memory assertion checked once `await_pc` is reached.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Assertion {
    addr: u16,
    expected: ExpectedValue,
}

/// The expected value of an [`Assertion`]: either a single byte, or an array of consecutive
/// bytes starting at `addr` (e.g. for asserting on a struct or other multi-byte value).
#[derive(Clone, serde::Deserialize)]
#[serde(untagged)]
enum ExpectedValue {
    Byte(u8),
    Bytes(Vec<u8>),
}

impl ExpectedValue {
    fn as_bytes(&self) -> Vec<u8> {
        match self {
            ExpectedValue::Byte(byte) => vec![*byte],
            ExpectedValue::Bytes(bytes) => bytes.clone(),
        }
    }
}

fn main() {
    let args = Arguments::from_args();
    let suites = load_suites().unwrap();
    let mut tests = Vec::new();
    for (suite_name, suite) in &suites {
        let counts = suite.cases.iter().map(|case| case.name.clone()).fold(
            HashMap::<String, u32>::new(),
            |mut counts, name| {
                *counts.entry(name).or_default() += 1;
                counts
            },
        );
        let duplicate_cases = counts
            .into_iter()
            .filter_map(|(name, count)| if count > 1 { Some(name) } else { None })
            .collect::<Vec<_>>();
        let suite_name = suite_name.clone();
        tests.push(Trial::test(
            format!("emulator::{suite_name}::_lint_cases"),
            move || {
                assert!(
                    duplicate_cases.is_empty(),
                    "duplicate test cases: {duplicate_cases:?}. Ensure that all names are unique within a TOML file"
                );
                Ok(())
            },
        ));
    }
    for (suite_name, suite) in suites {
        for case in suite.cases {
            let ignored_flag = case
                .ignored
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            let name = format!("emulator::{suite_name}::{}", case.name);
            tests.push(
                Trial::test(name, move || {
                    run_case(&case);
                    Ok(())
                })
                .with_ignored_flag(ignored_flag),
            );
        }
    }
    libtest_mimic::run(&args, tests).exit()
}

/// Load all suites from the `tests/emulator_cases` directory, keyed by suite name (the file
/// stem).
fn load_suites()
-> Result<Vec<(String, TestSuite)>, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let mut suites = Vec::new();
    for entry in std::fs::read_dir("tests/emulator_cases")? {
        let entry = entry?;
        let suite_path = entry.path();
        if !entry.metadata()?.is_file()
            || suite_path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                != "toml"
        {
            continue;
        }
        let name = suite_path
            .file_stem()
            .ok_or("expected filename")?
            .to_string_lossy()
            .to_string();
        let suite: TestSuite = toml_edit::de::from_str(&std::fs::read_to_string(&suite_path)?)?;
        suites.push((name, suite));
    }
    suites.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(suites)
}

/// Compiles `case.source` to a ROM, runs it in a fresh emulator until `case.await_pc` is
/// reached, and checks every `case.assertions` entry against emulator memory.
fn run_case(case: &TestCase) {
    let rom = build_rom(case);

    let mut emulator = Sameboy::new(case.model.into());
    emulator.load_rom(&rom);
    let reached = emulator.run_until_pc(case.await_pc, case.max_instructions);
    assert!(
        reached,
        "did not reach PC ${:04x} within {} instructions",
        case.await_pc, case.max_instructions
    );

    for assertion in &case.assertions {
        let expected = assertion.expected.as_bytes();
        let actual: Vec<u8> = (0..expected.len() as u16)
            .map(|offset| emulator.read_memory(assertion.addr.wrapping_add(offset)))
            .collect();
        assert_eq!(
            actual, expected,
            "memory at ${:04x} did not match expected value",
            assertion.addr
        );
    }
}

/// Builds `case.source` all the way down to ROM bytes, using the same fluent pipeline API the
/// `build` CLI command uses. Intermediate `.obj`/`.gb`/`.sym` files are written to a scratch
/// temporary directory that is cleaned up once this function returns.
fn build_rom(case: &TestCase) -> Vec<u8> {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let obj_path = dir.path().join("case.obj");
    let gb_path = dir.path().join("case.gb");
    let sym_path = dir.path().join("case.sym");

    PipelineConfig::default()
        .with_features(case.preview_features.iter().copied().collect())
        .start()
        .parse(&case.source)
        .unwrap_or_else(|e| panic!("parse failed: {e:?}"))
        .analyze()
        .unwrap_or_else(|e| panic!("analyze failed: {e:?}"))
        .optimize()
        .unwrap_or_else(|e| panic!("optimize failed: {e:?}"))
        .codegen()
        .unwrap_or_else(|e| panic!("codegen failed: {e:?}"))
        .allocate()
        .encode()
        .write_obj(&obj_path)
        .unwrap_or_else(|e| panic!("failed to write obj file: {e}"))
        .link(&gb_path, &sym_path)
        .unwrap_or_else(|e| panic!("failed to link rom: {e}"))
        .fix()
        .unwrap_or_else(|e| panic!("failed to fix rom: {e}"))
        .read_rom()
        .unwrap_or_else(|e| panic!("failed to read rom: {e}"))
}
