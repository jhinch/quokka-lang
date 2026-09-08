//! Differential soundness testing for the MIR optimizer.
//!
//! Each test compiles a small Quokka program twice under an identical [`PipelineConfig`]: once
//! without running `Optimizer::optimize` and once with it. Both MIRs are then interpreted with
//! [`quokka_lang::consteval::evaluate`] (the same interpreter used for compile-time constant
//! evaluation) across a range of concrete input values, fed in through mutable globals the
//! target function loads. If a pass changes observable behavior, the two runs diverge and the
//! test fails - this catches semantic (not just structural) bugs that reading the optimizer's
//! source code would miss.
//!
//! Limitations: `consteval::evaluate` does not support function calls or `Store`, so every
//! target function here is a single, side-effect-free function that only reads globals and
//! returns a value (never a `main` that calls it - `main` is included in each source only
//! because the analyzer requires an entrypoint).

use quokka_lang::feature::PreviewFeature;
use quokka_lang::mir::{GlobalId, Mir};
use quokka_lang::pipeline::PipelineConfig;
use quokka_lang::ty::{Ty, TyRegistry};
use slotmap::SparseSecondaryMap;

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

/// Compiles `source` once with `features` disabled and once with them enabled, then asserts
/// that `fn_name` produces byte-identical results before and after optimization for every value
/// in `sample_inputs`, fed in through the global named `global_name`.
fn assert_optimization_preserves_semantics(
    source: &str,
    features: &[PreviewFeature],
    fn_name: &str,
    global_name: &str,
    sample_inputs: &[Vec<u8>],
) {
    let mut config = PipelineConfig::default();
    for &feature in features {
        config.enable_feature(feature);
    }
    let analyzed = config
        .start()
        .parse(source)
        .unwrap_or_else(|e| panic!("parse failed: {e:?}"))
        .analyze()
        .unwrap_or_else(|e| panic!("analyze failed: {e:?}"));
    let unoptimized_mir = analyzed.mir().clone();

    let optimized = analyzed
        .optimize()
        .unwrap_or_else(|e| panic!("optimize failed: {e:?}"));
    let ty_registry = optimized.ty_registry();
    let optimized_mir = optimized.mir();

    for input in sample_inputs {
        let inputs = [(global_name, input.clone())];
        let before = eval_function(&unoptimized_mir, ty_registry, fn_name, &inputs);
        let after = eval_function(optimized_mir, ty_registry, fn_name, &inputs);
        assert_eq!(
            before, after,
            "optimizing with {features:?} changed the result of `{fn_name}` for {global_name} = {input:?}"
        );
    }
}

fn u8_values() -> Vec<Vec<u8>> {
    [0u8, 1, 2, 3, 5, 10, 127, 128, 200, 255]
        .into_iter()
        .map(|v| vec![v])
        .collect()
}

const ALL_PASSES: &[PreviewFeature] = &[
    PreviewFeature::Adce,
    PreviewFeature::Sccp,
    PreviewFeature::Prune,
    PreviewFeature::TreeShake,
];

#[test]
fn straight_line_dead_code_is_eliminated_soundly() {
    let source = r#"
global N: u8;

fn f() -> u8 {
    let n = N;
    let dead = n + n;
    let x = n - 1;
    return x;
}

fn main() {
    N = 1;
    f();
}
"#;
    for &pass in ALL_PASSES {
        assert_optimization_preserves_semantics(source, &[pass], "f", "N", &u8_values());
    }
    assert_optimization_preserves_semantics(source, ALL_PASSES, "f", "N", &u8_values());
}

#[test]
fn branch_whose_result_is_discarded_is_eliminated_soundly() {
    let source = r#"
global N: u8;

fn f() -> u8 {
    let n = N;
    let x = n;
    if n == 0 then {
        x = 1;
    } else {
        x = 2;
    }
    return n;
}

fn main() {
    N = 1;
    f();
}
"#;
    for &pass in ALL_PASSES {
        assert_optimization_preserves_semantics(source, &[pass], "f", "N", &u8_values());
    }
    assert_optimization_preserves_semantics(source, ALL_PASSES, "f", "N", &u8_values());
}

#[test]
fn branch_whose_result_is_actually_returned_is_preserved_soundly() {
    let source = r#"
global N: u8;

fn f() -> u8 {
    let n = N;
    let x = n;
    if n == 0 then {
        x = 9;
    } else {
        x = n + 1;
    }
    return x;
}

fn main() {
    N = 1;
    f();
}
"#;
    assert_optimization_preserves_semantics(source, ALL_PASSES, "f", "N", &u8_values());
}

#[test]
fn constant_folded_comparison_prunes_the_correct_branch_soundly() {
    // the condition itself is a compile-time constant once N is folded away isn't possible here
    // (N is a runtime input), but nested constant subexpressions inside the branch bodies are
    // exactly what Sccp is meant to fold; this checks that folding them doesn't change which
    // value ends up observable.
    let source = r#"
global N: u8;

fn f() -> u8 {
    let n = N;
    if n == 0 then {
        return 1 + 1;
    } else {
        return 2 + 2;
    }
}

fn main() {
    N = 1;
    f();
}
"#;
    assert_optimization_preserves_semantics(source, ALL_PASSES, "f", "N", &u8_values());
}

#[test]
fn loop_that_never_iterates_is_optimized_soundly() {
    // `consteval::evaluate` can only run this loop for N=0 - see the doc comment on
    // `loop_with_accumulator_is_optimized_soundly` below for why. This at least confirms the
    // optimizer + harness are fine for the one iteration count that's currently evaluable.
    let source = r#"
global N: u8;

fn f() -> u8 {
    let n = N;
    let i = 0;
    let sum = 0;
    loop {
        if i == n then {
            break;
        }
        sum = sum + i;
        i = i + 1;
    }
    return sum;
}

fn main() {
    N = 1;
    f();
}
"#;
    assert_optimization_preserves_semantics(source, ALL_PASSES, "f", "N", &[vec![0u8]]);
}

#[test]
fn loop_with_accumulator_is_optimized_soundly() {
    let source = r#"
global N: u8;

fn f() -> u8 {
    let n = N;
    let i = 0;
    let sum = 0;
    loop {
        if i == n then {
            break;
        }
        sum = sum + i;
        i = i + 1;
    }
    return sum;
}

fn main() {
    N = 1;
    f();
}
"#;
    // keep inputs small: this loop is O(n) in the interpreter and n=255 would run 255 times,
    // which is unnecessary to prove the point
    let inputs = [0u8, 1, 2, 3, 4, 10, 20]
        .into_iter()
        .map(|v| vec![v])
        .collect::<Vec<_>>();
    for &pass in ALL_PASSES {
        assert_optimization_preserves_semantics(source, &[pass], "f", "N", &inputs);
    }
    assert_optimization_preserves_semantics(source, ALL_PASSES, "f", "N", &inputs);
}

#[test]
fn loop_with_early_return_is_optimized_soundly() {
    let source = r#"
global N: u8;

fn f() -> u8 {
    let n = N;
    let i = 0;
    loop {
        if i == n then {
            return i;
        }
        if i == 5 then {
            return 100;
        }
        i = i + 1;
    }
}

fn main() {
    N = 1;
    f();
}
"#;
    let inputs = [0u8, 1, 3, 5, 6, 9]
        .into_iter()
        .map(|v| vec![v])
        .collect::<Vec<_>>();
    assert_optimization_preserves_semantics(source, ALL_PASSES, "f", "N", &inputs);
}

#[test]
fn narrow_int_wrapping_arithmetic_is_optimized_soundly() {
    let source = r#"
global N: u2;

fn f() -> u2 {
    let n = N;
    let x = n + n;
    let y = x - 1;
    return y;
}

fn main() {}
"#;
    let features = [
        PreviewFeature::NarrowInt,
        PreviewFeature::Adce,
        PreviewFeature::Sccp,
        PreviewFeature::Prune,
    ];
    let inputs = [0u8, 1, 2, 3]
        .into_iter()
        .map(|v| vec![v])
        .collect::<Vec<_>>();
    assert_optimization_preserves_semantics(source, &features, "f", "N", &inputs);
}
