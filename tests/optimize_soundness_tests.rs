//! Differential soundness testing for the MIR optimizer.
//!
//! Test cases live as TOML files under `tests/soundness_cases/`, in the same spirit as
//! `tests/spec_tests.rs`, but this is a distinct kind of test (differential execution rather than
//! golden-output comparison) so it gets its own directory and its own dynamically generated
//! `libtest_mimic` trials rather than being folded into the spec test suite.
//!
//! Each case compiles a small Quokka program twice under an identical [`PipelineConfig`]: once
//! without running `Optimizer::optimize` and once with it. Both MIRs are then interpreted with
//! [`quokka_lang::consteval::evaluate`] (the same interpreter used for compile-time constant
//! evaluation) across a range of concrete input values, fed in through a mutable global the
//! target function loads. Input values are listed individually or as inclusive `{ min, max }`
//! ranges (e.g. `values = [{ min = 0, max = 255 }, 1000]`), treated as raw unsigned bit patterns
//! and encoded little-endian to the width of the global's type. If a pass changes observable
//! behavior, the two runs diverge and the test fails - this catches semantic (not just
//! structural) bugs that reading the optimizer's source code would miss.
//!
//! For every case, one trial is generated per individual opt-in optimization feature (see
//! [`PreviewFeature::optimization_features`], currently empty), one per individual opt-out
//! feature (see [`PreviewFeature::optimization_opt_out_features`]), and one with every opt-in
//! optimization feature enabled together (i.e. the default optimizer configuration).
//!
//! Limitations: `consteval::evaluate` does not support function calls or `Store`, so every target
//! function is a single, side-effect-free function that only reads globals and returns a value
//! (never a `main` that calls it - `main` is included in each source only because the analyzer
//! requires an entrypoint).

use libtest_mimic::{Arguments, Trial};
use quokka_lang::feature::PreviewFeature;
use quokka_lang::mir::{GlobalId, Mir};
use quokka_lang::pipeline::PipelineConfig;
use quokka_lang::ty::{Ty, TyRegistry};
use slotmap::SparseSecondaryMap;
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
    #[serde(default = "default_function")]
    function: String,
    #[serde(default = "default_global")]
    global: String,
    /// additional preview features required for `source` to compile, unrelated to optimization
    /// (e.g. `narrow-int`).
    #[serde(default)]
    preview_features: Vec<PreviewFeature>,
    /// values to feed into `global`, as individual values and/or inclusive ranges. Each value is
    /// treated as a raw unsigned bit pattern and encoded little-endian to the width of `global`'s
    /// type.
    values: Vec<InputValue>,
    #[serde(default)]
    ignored: Option<String>,
    source: String,
}

/// A single input value or an inclusive range of input values.
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(untagged)]
enum InputValue {
    Single(u64),
    Range { min: u64, max: u64 },
}

impl InputValue {
    fn bounds(self) -> (u64, u64) {
        match self {
            InputValue::Single(value) => (value, value),
            InputValue::Range { min, max } => (min, max),
        }
    }
}

fn default_function() -> String {
    "f".to_string()
}

fn default_global() -> String {
    "N".to_string()
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
            format!("optimize_soundness::{suite_name}::_lint_cases"),
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
            for &feature in PreviewFeature::optimization_features() {
                tests.push(make_trial(
                    &suite_name,
                    &case,
                    &format!("{feature}"),
                    std::iter::once(feature).chain(case.preview_features.iter().copied()),
                    ignored_flag,
                ));
            }
            for &feature in PreviewFeature::optimization_opt_out_features() {
                tests.push(make_trial(
                    &suite_name,
                    &case,
                    &format!("{feature}"),
                    std::iter::once(feature).chain(case.preview_features.iter().copied()),
                    ignored_flag,
                ));
            }
            tests.push(make_trial(
                &suite_name,
                &case,
                "all",
                PreviewFeature::optimization_features()
                    .iter()
                    .copied()
                    .chain(case.preview_features.iter().copied()),
                ignored_flag,
            ));
        }
    }
    libtest_mimic::run(&args, tests).exit()
}

fn make_trial(
    suite_name: &str,
    case: &TestCase,
    variant: &str,
    features: impl IntoIterator<Item = PreviewFeature>,
    ignored_flag: bool,
) -> Trial {
    let name = format!("optimize_soundness::{suite_name}::{}::{variant}", case.name);
    let features: Vec<PreviewFeature> = features.into_iter().collect();
    let case = case.clone();
    Trial::test(name, move || {
        assert_optimization_preserves_semantics(&case, &features);
        Ok(())
    })
    .with_ignored_flag(ignored_flag)
}

/// Load all suites from the `tests/soundness_cases` directory, keyed by suite name (the file
/// stem).
fn load_suites()
-> Result<Vec<(String, TestSuite)>, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let mut suites = Vec::new();
    for entry in std::fs::read_dir("tests/soundness_cases")? {
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
        for case in &suite.cases {
            for value in &case.values {
                let (min, max) = value.bounds();
                if min > max {
                    return Err(format!(
                        "{name}::{}: empty value range {{ min = {min}, max = {max} }}",
                        case.name
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

fn eval_function(
    mir: &Mir,
    ty_registry: &TyRegistry,
    fn_name: &str,
    inputs: &[(&str, Vec<u8>)],
) -> Vec<u8> {
    let function = mir
        .functions
        .values()
        .find(|f| f.name.to_string() == fn_name)
        .unwrap_or_else(|| panic!("function {fn_name} not found in compiled MIR"));

    // resolve every global's bytes up front (mutable "inputs" override, everything else keeps
    // its compiled-in constant value) so the SparseSecondaryMap below can borrow from something
    // that outlives the `evaluate` call.
    let owned: Vec<(GlobalId, Ty, Vec<u8>)> = mir
        .globals
        .iter()
        .map(|(id, global)| {
            let name = global.name.data.to_string();
            let bytes = inputs
                .iter()
                .find(|(input_name, _)| *input_name == name)
                .map(|(_, bytes)| bytes.clone())
                .unwrap_or_else(|| global.value.clone());
            (id, global.ty, bytes)
        })
        .collect();

    let mut globals_map = SparseSecondaryMap::new();
    for (id, ty, bytes) in &owned {
        globals_map.insert(*id, (*ty, bytes.as_slice()));
    }

    quokka_lang::consteval::evaluate(function, &globals_map, ty_registry)
}

/// Compiles `case.source` once with `features` disabled and once with them enabled, then asserts
/// that `case.function` produces byte-identical results before and after optimization for every
/// value in `case.values`, fed in through the global named `case.global`.
fn assert_optimization_preserves_semantics(case: &TestCase, features: &[PreviewFeature]) {
    let mut config = PipelineConfig::default();
    for &feature in features {
        config.enable_feature(feature);
    }
    let analyzed = config
        .start()
        .parse(&case.source)
        .unwrap_or_else(|e| panic!("parse failed: {e:?}"))
        .analyze()
        .unwrap_or_else(|e| panic!("analyze failed: {e:?}"));
    let unoptimized_mir = analyzed.mir().clone();
    let global_ty = unoptimized_mir
        .globals
        .values()
        .find(|global| global.name.data.to_string() == case.global)
        .unwrap_or_else(|| panic!("global {} not found in compiled MIR", case.global))
        .ty;

    let optimized = analyzed
        .optimize()
        .unwrap_or_else(|e| panic!("optimize failed: {e:?}"));
    let ty_registry = optimized.ty_registry();
    let optimized_mir = optimized.mir();

    let bit_size = global_ty.bit_size(ty_registry);
    let bits = bit_size.into_u32();
    let byte_len = usize::from(bit_size.padded_bytes());
    assert!(
        (1..=u64::BITS).contains(&bits),
        "global {} has unsupported type {} for input values",
        case.global,
        global_ty.display(ty_registry)
    );
    let type_max = u64::MAX >> (u64::BITS - bits);
    for value in &case.values {
        let (min, max) = value.bounds();
        assert!(
            max <= type_max,
            "value range {min}..={max} does not fit in {} (max {type_max})",
            global_ty.display(ty_registry)
        );
    }
    let values = case.values.iter().flat_map(|value| {
        let (min, max) = value.bounds();
        min..=max
    });

    for value in values {
        let bytes = value.to_le_bytes()[..byte_len].to_vec();
        let inputs = [(case.global.as_str(), bytes)];
        let before = eval_function(&unoptimized_mir, ty_registry, &case.function, &inputs);
        let after = eval_function(optimized_mir, ty_registry, &case.function, &inputs);
        assert_eq!(
            before, after,
            "optimizing with {features:?} changed the result of `{}` for {} = {value}",
            case.function, case.global
        );
    }
}
