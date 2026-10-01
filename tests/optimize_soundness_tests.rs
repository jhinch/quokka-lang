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
//! evaluation) across a range of concrete input values, fed in through the mutable globals the
//! target function loads. A case names its input globals under `inputs`, each with values listed
//! individually or as inclusive `{ min, max }` ranges (e.g.
//! `inputs = { N = [{ min = 0, max = 255 }, 1000] }`), treated as raw unsigned bit patterns and
//! encoded little-endian to the width of the global's type. The function is evaluated for every
//! combination of input values. If a pass changes observable behavior, the two runs diverge and
//! the test fails - this catches semantic (not just structural) bugs that reading the optimizer's
//! source code would miss.
//!
//! Sweeping every value is only affordable for a single axis of up to 8 bits. For wider globals,
//! or for the second axis of a case which sweeps two, an input lists `"edges"` instead of a range
//! (e.g. `inputs = { N = ["edges"] }`), which expands to the values of the global's type most
//! likely to expose a bug (see [`edge_values`]) rather than all of them.
//!
//! Globals are never literals, so a rewrite that only matches a literal operand cannot be swept
//! through `inputs`. For those a case lists `params`: each `$NAME$` placeholder in the source is
//! replaced with the value in decimal and the program is compiled and checked once per
//! combination of parameter values. A param has no type to take a width from, so it samples with
//! an explicit width instead (e.g. `params = { C = [{ edges = 8 }] }`).
//!
//! Agreeing results say nothing if the optimization under test never fired. A case can list
//! features under `expect_rewrite`: the trial for each listed feature additionally fails if
//! toggling that feature alone leaves the optimized MIR unchanged.
//!
//! For every case, one trial is generated per individual opt-in optimization feature (see
//! [`PreviewFeature::optimization_features`]), one per individual opt-out
//! feature (see [`PreviewFeature::optimization_opt_out_features`]), and one with every opt-in
//! optimization feature enabled together (i.e. the default optimizer configuration).
//!
//! Limitations: `consteval::evaluate` does not support function calls or `Store`, so every target
//! function is a single, side-effect-free function that only reads globals and returns a value
//! (never a `main` that calls it - `main` is included in each source only because the analyzer
//! requires an entrypoint).

use libtest_mimic::{Arguments, Trial};
use quokka_lang::feature::PreviewFeature;
use quokka_lang::mir::{Function, GlobalId, Mir};
use quokka_lang::pipeline::{AnalyzerOutput, PipelineConfig};
use quokka_lang::ty::{Ty, TyRegistry};
use slotmap::SparseSecondaryMap;
use std::collections::{BTreeMap, HashMap};

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
    /// additional preview features required for `source` to compile, unrelated to optimization
    /// (e.g. `narrow-int`).
    #[serde(default)]
    preview_features: Vec<PreviewFeature>,
    /// values to feed into each named global, as individual values and/or inclusive ranges. Each
    /// value is treated as a raw unsigned bit pattern and encoded little-endian to the width of
    /// the global's type. The function is evaluated for every combination of input values. An
    /// input may also list `"edges"` to sample the global's type (see [`edge_values`]).
    inputs: BTreeMap<String, Vec<InputValue>>,
    /// values to substitute for each `$NAME$` placeholder in `source`, as individual values
    /// and/or inclusive ranges. The source is compiled once per combination of parameter values.
    /// A param may also list `{ edges = <bits> }` to sample values of that width.
    #[serde(default)]
    params: BTreeMap<String, Vec<InputValue>>,
    /// optimization features which must change the optimized MIR when toggled on their own.
    #[serde(default)]
    expect_rewrite: Vec<PreviewFeature>,
    #[serde(default)]
    ignored: Option<String>,
    source: String,
}

/// A single input value, an inclusive range of input values, or a sample of them.
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(untagged)]
enum InputValue {
    Single(u64),
    Range {
        min: u64,
        max: u64,
    },
    /// the [`edge_values`] of an explicit width.
    Edges {
        edges: u32,
    },
    Sample(Sample),
}

/// A set of input values derived from the width of the input's type.
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Sample {
    /// see [`edge_values`].
    Edges,
}

impl InputValue {
    /// The inclusive bounds of an explicitly listed value or range, or `None` for a sample.
    fn bounds(self) -> Option<(u64, u64)> {
        match self {
            InputValue::Single(value) => Some((value, value)),
            InputValue::Range { min, max } => Some((min, max)),
            InputValue::Edges { .. } | InputValue::Sample(_) => None,
        }
    }
}

/// The values of a `bits` wide type most likely to expose an unsound rewrite, sorted and without
/// duplicates:
///
/// - a window of values either side of every power of two, which covers zero, the maximum, the
///   signed minimum and maximum, and every carry and borrow between bytes;
/// - every combination of interesting bit patterns across the bytes of the value, so each byte
///   sees its own extremes regardless of what its neighbors hold;
/// - a fixed pseudo-random sample, for anything the structured values miss.
///
/// The window and the random sample are kept small for types of up to 8 bits, which a case can
/// afford to sweep with a range instead when it needs every value.
fn edge_values(bits: u32) -> Vec<u64> {
    let type_max = u64::MAX >> (u64::BITS - bits);
    let (window, random_values) = if bits <= 8 { (1, 8) } else { (16, 256) };
    let mut values = Vec::new();

    for shift in 0..=bits {
        let boundary = if shift == bits { 0 } else { 1u64 << shift };
        for offset in 0..=window {
            values.push(boundary.wrapping_add(offset) & type_max);
            values.push(boundary.wrapping_sub(offset) & type_max);
        }
    }

    const BYTE_PATTERNS: [u64; 11] = [
        0x00, 0x01, 0x0F, 0x55, 0x7F, 0x80, 0x81, 0xAA, 0xF0, 0xFE, 0xFF,
    ];
    let mut patterns = vec![0u64];
    for byte in 0..bits.div_ceil(8) {
        patterns = patterns
            .iter()
            .flat_map(|&value| {
                BYTE_PATTERNS
                    .iter()
                    .map(move |&pattern| value | (pattern << (byte * 8)))
            })
            .collect();
    }
    values.extend(patterns.into_iter().map(|value| value & type_max));

    // xorshift64 from a fixed seed, so every run tests the same values.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for _ in 0..random_values {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        values.push(state & type_max);
    }

    values.sort_unstable();
    values.dedup();
    values
}

/// Every value listed by `values`, in order, for an input or param which is `bits` wide (if it
/// has a type at all - params are substituted into the source as text).
fn expand(values: &[InputValue], bits: Option<u32>) -> Vec<u64> {
    let mut expanded = Vec::new();
    for &value in values {
        match value {
            InputValue::Single(value) => expanded.push(value),
            InputValue::Range { min, max } => expanded.extend(min..=max),
            InputValue::Edges { edges } => expanded.extend(edge_values(edges)),
            InputValue::Sample(Sample::Edges) => {
                expanded.extend(edge_values(bits.expect("params have no type to sample")))
            }
        }
    }
    expanded
}

fn default_function() -> String {
    "f".to_string()
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
                    Some(feature),
                    std::iter::once(feature).chain(case.preview_features.iter().copied()),
                    ignored_flag,
                ));
            }
            for &feature in PreviewFeature::optimization_opt_out_features() {
                tests.push(make_trial(
                    &suite_name,
                    &case,
                    &format!("{feature}"),
                    Some(feature),
                    std::iter::once(feature).chain(case.preview_features.iter().copied()),
                    ignored_flag,
                ));
            }
            tests.push(make_trial(
                &suite_name,
                &case,
                "all",
                None,
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
    toggled: Option<PreviewFeature>,
    features: impl IntoIterator<Item = PreviewFeature>,
    ignored_flag: bool,
) -> Trial {
    let name = format!("optimize_soundness::{suite_name}::{}::{variant}", case.name);
    let features: Vec<PreviewFeature> = features.into_iter().collect();
    let case = case.clone();
    Trial::test(name, move || {
        assert_optimization_preserves_semantics(&case, &features, toggled);
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
            for value in case.params.values().flatten() {
                if matches!(value, InputValue::Sample(_)) {
                    return Err(format!(
                        "{name}::{}: params have no type to sample, use {{ edges = <bits> }} instead",
                        case.name
                    )
                    .into());
                }
            }
            for value in case.inputs.values().chain(case.params.values()).flatten() {
                if let InputValue::Edges { edges } = *value
                    && !(1..=u64::BITS).contains(&edges)
                {
                    return Err(format!(
                        "{name}::{}: unsupported width {{ edges = {edges} }}",
                        case.name
                    )
                    .into());
                }
            }
            for value in case.inputs.values().chain(case.params.values()).flatten() {
                let Some((min, max)) = value.bounds() else {
                    continue;
                };
                if min > max {
                    return Err(format!(
                        "{name}::{}: empty value range {{ min = {min}, max = {max} }}",
                        case.name
                    )
                    .into());
                }
            }
            for param in case.params.keys() {
                if !case.source.contains(&placeholder(param)) {
                    return Err(format!(
                        "{name}::{}: param {param} has no {} placeholder in the source",
                        case.name,
                        placeholder(param)
                    )
                    .into());
                }
            }
            for feature in &case.expect_rewrite {
                if !PreviewFeature::optimization_features().contains(feature)
                    && !PreviewFeature::optimization_opt_out_features().contains(feature)
                {
                    return Err(format!(
                        "{name}::{}: expect_rewrite feature {feature} is not an optimization feature",
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

/// The text in a case's source which is replaced by the value of the param named `name`.
fn placeholder(name: &str) -> String {
    format!("${name}$")
}

/// Calls `f` with every combination of one value from each of `axes`, in order.
fn for_each_combination<F: FnMut(&[u64])>(axes: &[Vec<u64>], f: &mut F) {
    fn go<F: FnMut(&[u64])>(axes: &[Vec<u64>], current: &mut Vec<u64>, f: &mut F) {
        let Some((axis, rest)) = axes.split_first() else {
            f(current);
            return;
        };
        for &value in axis {
            current.push(value);
            go(rest, current, f);
            current.pop();
        }
    }
    go(axes, &mut Vec::new(), f);
}

/// Formats named values for an assertion message, e.g. `N = 1, M = 2`.
fn describe<'a>(names: impl IntoIterator<Item = &'a String>, values: &[u64]) -> String {
    names
        .into_iter()
        .zip(values)
        .map(|(name, value)| format!("{name} = {value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Evaluates a function of a compiled MIR repeatedly with different values for its input globals.
struct Evaluator<'a> {
    function: &'a Function,
    ty_registry: &'a TyRegistry,
    /// every global's bytes: the inputs are overwritten before each evaluation, everything else
    /// keeps its compiled-in constant value.
    globals: Vec<(GlobalId, Ty, Vec<u8>)>,
    /// for each input, in order, its index in `globals` (if it survived optimization).
    inputs: Vec<Option<usize>>,
}

impl<'a> Evaluator<'a> {
    fn new<'b>(
        mir: &'a Mir,
        ty_registry: &'a TyRegistry,
        fn_name: &str,
        input_names: impl IntoIterator<Item = &'b String>,
    ) -> Self {
        let function = mir
            .functions
            .values()
            .find(|f| f.name.to_string() == fn_name)
            .unwrap_or_else(|| panic!("function {fn_name} not found in compiled MIR"));
        let names: Vec<String> = mir
            .globals
            .values()
            .map(|global| global.name.data.to_string())
            .collect();
        let globals = mir
            .globals
            .iter()
            .map(|(id, global)| (id, global.ty, global.value.clone()))
            .collect();
        let inputs = input_names
            .into_iter()
            .map(|input| names.iter().position(|name| name == input))
            .collect();
        Evaluator {
            function,
            ty_registry,
            globals,
            inputs,
        }
    }

    fn evaluate(&mut self, values: &[u64]) -> Vec<u8> {
        for (&index, value) in self.inputs.iter().zip(values) {
            if let Some(index) = index {
                let (_, ty, bytes) = &mut self.globals[index];
                let byte_len = usize::from(ty.bit_size(self.ty_registry).padded_bytes());
                bytes.clear();
                bytes.extend_from_slice(&value.to_le_bytes()[..byte_len]);
            }
        }
        let mut globals_map = SparseSecondaryMap::new();
        for (id, ty, bytes) in &self.globals {
            globals_map.insert(*id, (*ty, bytes.as_slice()));
        }
        quokka_lang::consteval::evaluate(self.function, &globals_map, self.ty_registry)
    }
}

fn analyze(source: &str, features: impl IntoIterator<Item = PreviewFeature>) -> AnalyzerOutput {
    let mut config = PipelineConfig::default();
    for feature in features {
        config.enable_feature(feature);
    }
    config
        .start()
        .parse(source)
        .unwrap_or_else(|e| panic!("parse failed: {e:?}"))
        .analyze()
        .unwrap_or_else(|e| panic!("analyze failed: {e:?}"))
}

/// For every combination of `case.params`, compiles `case.source` once without optimizing and
/// once optimized with `features`, then asserts that `case.function` produces byte-identical
/// results before and after optimization for every combination of `case.inputs`.
///
/// `toggled` is the single optimization feature this trial turns on, if any. When the case lists
/// it under `expect_rewrite`, also asserts that the optimized MIR differs from the one produced
/// with only that feature left off.
fn assert_optimization_preserves_semantics(
    case: &TestCase,
    features: &[PreviewFeature],
    toggled: Option<PreviewFeature>,
) {
    let expect_rewrite = toggled.filter(|feature| case.expect_rewrite.contains(feature));
    let param_axes: Vec<Vec<u64>> = case
        .params
        .values()
        .map(|values| expand(values, None))
        .collect();

    for_each_combination(&param_axes, &mut |params| {
        let mut source = case.source.clone();
        for (name, value) in case.params.keys().zip(params) {
            source = source.replace(&placeholder(name), &value.to_string());
        }
        let with_params = if params.is_empty() {
            String::new()
        } else {
            format!(" with {}", describe(case.params.keys(), params))
        };

        let analyzed = analyze(&source, features.iter().copied());
        let unoptimized_mir = analyzed.mir().clone();
        let optimized = analyzed
            .optimize()
            .unwrap_or_else(|e| panic!("optimize failed{with_params}: {e:?}"));
        let ty_registry = optimized.ty_registry();
        let optimized_mir = optimized.mir();

        if let Some(toggled) = expect_rewrite {
            let baseline = analyze(
                &source,
                features
                    .iter()
                    .copied()
                    .filter(|&feature| feature != toggled),
            )
            .optimize()
            .unwrap_or_else(|e| panic!("optimize failed{with_params}: {e:?}"));
            let optimized_mir = optimized_mir.display(ty_registry).to_string();
            assert!(
                baseline.mir().display(baseline.ty_registry()).to_string() != optimized_mir,
                "expected {toggled} to rewrite the MIR{with_params}, but it was left unchanged:\n{optimized_mir}"
            );
        }

        let mut input_axes = Vec::new();
        for (name, values) in &case.inputs {
            let global_ty = unoptimized_mir
                .globals
                .values()
                .find(|global| global.name.data.to_string() == *name)
                .unwrap_or_else(|| panic!("global {name} not found in compiled MIR"))
                .ty;
            let bits = global_ty.bit_size(ty_registry).into_u32();
            assert!(
                (1..=u64::BITS).contains(&bits),
                "global {name} has unsupported type {} for input values",
                global_ty.display(ty_registry)
            );
            let type_max = u64::MAX >> (u64::BITS - bits);
            for (min, max) in values.iter().filter_map(|value| value.bounds()) {
                assert!(
                    max <= type_max,
                    "value range {min}..={max} does not fit in {} (max {type_max})",
                    global_ty.display(ty_registry)
                );
            }
            for value in values {
                if let InputValue::Edges { edges } = *value {
                    assert!(
                        edges <= bits,
                        "{{ edges = {edges} }} does not fit in {}",
                        global_ty.display(ty_registry)
                    );
                }
            }
            input_axes.push(expand(values, Some(bits)));
        }

        let mut before = Evaluator::new(
            &unoptimized_mir,
            ty_registry,
            &case.function,
            case.inputs.keys(),
        );
        let mut after = Evaluator::new(
            optimized_mir,
            ty_registry,
            &case.function,
            case.inputs.keys(),
        );
        for_each_combination(&input_axes, &mut |inputs| {
            assert_eq!(
                before.evaluate(inputs),
                after.evaluate(inputs),
                "optimizing with {features:?} changed the result of `{}` for {}{with_params}",
                case.function,
                describe(case.inputs.keys(), inputs)
            );
        });
    });
}
