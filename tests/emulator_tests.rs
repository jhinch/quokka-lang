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
//!
//! By default a case generates a single trial, built with the case's own configuration (the
//! `default` variant). A suite, or an individual case within it, can list `variants` to generate
//! a trial for each one named instead, for cases which are about an optimization and so need the
//! generated code checked both with and without it (e.g.
//! `variants = ["default", "no-canonicalize"]`). A variant other than `default` is the name of an
//! optimization feature (see [`PreviewFeature::optimization_features`] and
//! [`PreviewFeature::optimization_opt_out_features`]), which is toggled on top of the case's own
//! preview features.

use libtest_mimic::{Arguments, Trial};
use quokka_lang::feature::PreviewFeature;
use quokka_lang::pipeline::PipelineConfig;
use quokka_lang_sameboy::{HaltHazard, Model, OamCorruptionHazard, RunOutcome, Sameboy};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Deserialized representation of a test suite TOML file.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TestSuite {
    /// the variants to generate trials for in every case which does not list its own. Only
    /// [`DEFAULT_VARIANT`] is generated if omitted.
    #[serde(default)]
    variants: Option<Vec<String>>,
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
    /// the variants to generate trials for, each either `default` or the name of an
    /// optimization feature. Falls back to the suite's `variants` if omitted.
    #[serde(default)]
    variants: Option<Vec<String>>,
    #[serde(default)]
    ignored: Option<String>,
    source: String,
    /// address the CPU's `PC` register must reach for the case to pass
    await_pc: u16,
    #[serde(default, rename = "assertion")]
    assertions: Vec<Assertion>,
}

fn default_max_instructions() -> u32 {
    1_000
}

/// The name of the variant which builds a case with only its own preview features, and the only
/// one generated for a case which lists no `variants`.
const DEFAULT_VARIANT: &str = "default";

/// Every variant a case can list, in order: its name, and the optimization feature it toggles on
/// top of the case's own preview features (or `None` for [`DEFAULT_VARIANT`]).
fn all_variants() -> Vec<(String, Option<PreviewFeature>)> {
    std::iter::once((DEFAULT_VARIANT.to_string(), None))
        .chain(
            PreviewFeature::optimization_features()
                .iter()
                .chain(PreviewFeature::optimization_opt_out_features())
                .map(|&feature| (feature.to_string(), Some(feature))),
        )
        .collect()
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
    /// Bits to compare, per byte starting at `addr`. Defaults to `0xff` (every bit compared).
    /// A single value broadcasts across every byte of `expected`; e.g. a `global: u1` only
    /// defines bit 0 of its byte, with the rest left as whatever WRAM happened to contain at
    /// boot (SameBoy deliberately randomizes uninitialized memory, matching real hardware), so
    /// such an assertion should set `mask = 0b0000_0001` to ignore those undefined bits.
    #[serde(default)]
    mask: Option<ExpectedValue>,
}

impl Assertion {
    /// The mask bytes to compare, one per byte of `expected`.
    fn mask_bytes(&self) -> Vec<u8> {
        let len = self.expected.as_bytes().len();
        match &self.mask {
            None => vec![0xff; len],
            Some(mask) => {
                let mask = mask.as_bytes();
                match mask.len() {
                    1 => vec![mask[0]; len],
                    mask_len if mask_len == len => mask,
                    mask_len => panic!(
                        "mask at ${:04x} has {mask_len} byte(s), expected 1 (broadcast) or {len} \
                         to match `expected`",
                        self.addr
                    ),
                }
            }
        }
    }
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
            let variants = case.variants.as_ref().or(suite.variants.as_ref());
            for (variant, toggled) in all_variants() {
                let listed = match variants {
                    Some(variants) => variants.contains(&variant),
                    None => variant == DEFAULT_VARIANT,
                };
                if !listed {
                    continue;
                }
                let name = format!("emulator::{suite_name}::{}::{variant}", case.name);
                let case = case.clone();
                tests.push(
                    Trial::test(name, move || {
                        run_case(&case, toggled);
                        Ok(())
                    })
                    .with_ignored_flag(ignored_flag),
                );
            }
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
        let known_variants: Vec<String> = all_variants()
            .into_iter()
            .map(|(variant, _)| variant)
            .collect();
        let listed_variants = suite
            .variants
            .iter()
            .map(|variants| (&name, variants))
            .chain(
                suite
                    .cases
                    .iter()
                    .filter_map(|case| Some((&case.name, case.variants.as_ref()?))),
            );
        for (owner, variants) in listed_variants {
            if variants.is_empty() {
                return Err(format!("{name}: {owner} lists no variants to run").into());
            }
            for variant in variants {
                if !known_variants.contains(variant) {
                    return Err(format!(
                        "{name}: {owner} lists unknown variant {variant}, expected one of {known_variants:?}"
                    )
                    .into());
                }
            }
        }
        suites.push((name, suite));
    }
    suites.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(suites)
}

/// Compiles `case.source` to a ROM, with `toggled` enabled on top of the case's own preview
/// features if given, runs it in a fresh emulator until `case.await_pc` is reached, and checks
/// every `case.assertions` entry against emulator memory. The runtime checkers watch the run,
/// and any violation they report fails the case.
fn run_case(case: &TestCase, toggled: Option<PreviewFeature>) {
    let rom = build_rom(case, toggled);

    let mut emulator = Sameboy::new(case.model.into());
    emulator.load_rom(&rom);
    // Restoring a save state captured right after a previous emulator instance finished
    // executing the boot ROM is behaviorally equivalent to (but far faster than) running the
    // boot ROM again here: the boot ROM's CPU/RAM/VRAM end state only depends on the emulated
    // model and the cartridge header, both of which are fixed for every case (the header is
    // always written the same way by `fix()`/`rgbfix`, regardless of source), and never on the
    // ROM's actual code, which the boot ROM doesn't execute.
    if !emulator.load_state(boot_snapshot(case.model)) {
        panic!("failed to restore post-boot-ROM save state");
    }
    emulator.add_checker(Box::new(OamCorruptionHazard::new()));
    emulator.add_checker(Box::new(HaltHazard::new()));
    match emulator.run_until_pc(case.await_pc, case.max_instructions) {
        RunOutcome::Reached => {}
        RunOutcome::InstructionLimit => panic!(
            "did not reach PC ${:04x} within {} instructions\n\n{}",
            case.await_pc,
            case.max_instructions,
            trace_report(&emulator)
        ),
        RunOutcome::Violation => panic!("{}", violation_report(&emulator)),
    }
    if !emulator.finish().is_empty() {
        panic!("{}", violation_report(&emulator));
    }

    for assertion in &case.assertions {
        let expected = assertion.expected.as_bytes();
        let mask = assertion.mask_bytes();
        let actual: Vec<u8> = (0..expected.len() as u16)
            .map(|offset| emulator.read_memory(assertion.addr.wrapping_add(offset)))
            .collect();
        let masked_actual: Vec<u8> = actual.iter().zip(&mask).map(|(byte, m)| byte & m).collect();
        let masked_expected: Vec<u8> = expected
            .iter()
            .zip(&mask)
            .map(|(byte, m)| byte & m)
            .collect();
        assert_eq!(
            masked_actual, masked_expected,
            "memory at ${:04x} did not match expected value (mask {mask:02x?}, raw actual \
             {actual:02x?}, raw expected {expected:02x?})",
            assertion.addr
        );
    }
}

/// Describes every violation the emulator's checkers recorded, followed by the instructions
/// leading up to where it stopped.
fn violation_report(emulator: &Sameboy) -> String {
    let violations: Vec<String> = emulator
        .violations()
        .iter()
        .map(ToString::to_string)
        .collect();
    format!(
        "runtime check failed:\n{}\n\n{}",
        violations.join("\n"),
        trace_report(emulator)
    )
}

/// Lists the last instructions the emulator executed, oldest first.
fn trace_report(emulator: &Sameboy) -> String {
    let trace = emulator.trace();
    format!("last {} instructions executed:\n{trace}", trace.len())
}

/// Returns a save state snapshot, captured right after the emulator finished executing the boot
/// ROM for `model`, that can be restored onto any freshly-loaded ROM to skip re-running the boot
/// ROM. Computed once per model and cached for the remainder of the process.
fn boot_snapshot(model: ModelKind) -> &'static [u8] {
    static DMG: OnceLock<Vec<u8>> = OnceLock::new();
    static MGB: OnceLock<Vec<u8>> = OnceLock::new();
    static CGB: OnceLock<Vec<u8>> = OnceLock::new();
    let cell = match model {
        ModelKind::Dmg => &DMG,
        ModelKind::Mgb => &MGB,
        ModelKind::Cgb => &CGB,
    };
    cell.get_or_init(|| {
        let mut emulator = Sameboy::new(model.into());
        emulator.load_rom(template_rom());
        if emulator.run_until_pc(0x100, 1_000_000) != RunOutcome::Reached {
            panic!("did not execute boot ROM within 1M instructions while building save state");
        }
        emulator.save_state()
    })
}

/// A minimal, otherwise-blank ROM with a valid header (correct Nintendo logo and header
/// checksum, as `rgbfix` would produce for any real case ROM), used only to get an emulator
/// through the boot ROM when building a [`boot_snapshot`]. Its code is never executed: the boot
/// ROM only inspects the header, and the snapshot is captured the instant `PC` reaches `$0100`,
/// before any cartridge code runs.
fn template_rom() -> &'static [u8] {
    static ROM: OnceLock<Vec<u8>> = OnceLock::new();
    ROM.get_or_init(|| {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let rom_path = dir.path().join("template.gb");
        std::fs::write(&rom_path, vec![0u8; 0x8000]).expect("failed to write template rom");
        run_tool(
            "rgbfix",
            std::process::Command::new("rgbfix")
                .arg("-v")
                .arg("-p")
                .arg("0xFF")
                .arg(&rom_path),
        );
        std::fs::read(&rom_path).expect("failed to read template rom")
    })
}

fn run_tool(tool: &str, command: &mut std::process::Command) {
    let status = command
        .stdin(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to run `{tool}`: {e}"));
    assert!(status.success(), "`{tool}` exited with {status}");
}

/// Builds `case.source` all the way down to ROM bytes, with `toggled` enabled on top of the case's
/// own preview features if given, using the same fluent pipeline API the `build` CLI command
/// uses. Intermediate `.obj`/`.gb`/`.sym` files are written to a scratch temporary directory that
/// is cleaned up once this function returns.
fn build_rom(case: &TestCase, toggled: Option<PreviewFeature>) -> Vec<u8> {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let obj_path = dir.path().join("case.obj");
    let gb_path = dir.path().join("case.gb");
    let sym_path = dir.path().join("case.sym");

    PipelineConfig::default()
        .with_features(
            case.preview_features
                .iter()
                .copied()
                .chain(toggled)
                .collect(),
        )
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
        .peephole()
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
