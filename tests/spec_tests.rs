use libtest_mimic::{Arguments, Trial};
use miette::LabeledSpan;
use quokka_lang::multi::{MultiError, MultiResult};
use std::collections::HashMap;
use std::fmt;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Deserialized representation of a test suite TOML file
#[derive(serde::Deserialize)]
struct TestSuite {
    #[serde(skip)]
    category: String,
    #[serde(skip)]
    name: String,
    #[serde(rename = "case")]
    cases: Vec<TestCase>,
}

impl TestSuite {
    fn resolve_sources(&mut self, dir: &Path) {
        for case in self.cases.iter_mut() {
            case.resolve_source(dir);
        }
    }
}

/// Deserialized representation of a test case within test suite
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TestCase {
    #[serde(skip)]
    category: String,
    #[serde(skip)]
    suite: String,
    name: String,
    kind: TestCaseKind,
    #[serde(default)]
    entrypoint: ParseEntrypointKind,
    #[serde(default)]
    optimize: bool,
    #[serde(default)]
    allocate_registers: bool,
    #[serde(default)]
    preview_features: Vec<quokka_lang::feature::PreviewFeature>,
    #[serde(default)]
    ignored: Option<String>,
    source: Source,
    #[serde(default)]
    expected_tokens: String,
    #[serde(default)]
    expected_ast: String,
    #[serde(default)]
    expected_mir: String,
    #[serde(default)]
    expected_lir: String,
    #[serde(default)]
    expected_errors: Vec<String>,
    #[serde(skip)]
    updates_tx: Option<mpsc::Sender<PendingUpdate>>,
    #[serde(skip)]
    force_regenerate_expectations: bool,
}

impl TestCase {
    fn source(&self) -> &str {
        match &self.source {
            Source::Content(content) => content.as_str(),
            Source::File { .. } => panic!("resolve_source has not been called"),
        }
    }

    fn resolve_source(&mut self, dir: &Path) {
        if let Source::File { file } = &self.source {
            self.source = Source::Content(std::fs::read_to_string(dir.join(file)).unwrap());
        }
    }
}

/// Deserialized representation of the kind of case
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TestCaseKind {
    Lexer,
    Parser,
    Analyzer,
    Codegen,
}

#[derive(Clone, serde::Deserialize)]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
enum Source {
    Content(String),
    File { file: String },
}

/// Deserialized representation of the kind entrypoint for a parser test
#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ParseEntrypointKind {
    #[default]
    Root,
    Item,
    Statement,
    Expr,
}

/// Pending update command when regenerating fixtures
struct PendingUpdate {
    pub category: String,
    pub suite: String,
    pub case: String,
    pub data: PendingUpdateData,
}

enum PendingUpdateData {
    UpdateErrors(Vec<String>),
    UpdateTokens(String),
    UpdateAst(String),
    UpdateMir(String),
    UpdateLir(String),
}

fn main() {
    let args = Arguments::from_args();
    let suites = load_suites().unwrap();
    let regenerate_expectations = env_as_bool("REGENERATE_EXPECTATIONS");
    let force_regenerate_expectations = env_as_bool("FORCE_REGENERATE_EXPECTATIONS");
    let (tx, rx) = if regenerate_expectations {
        let (tx, rx) = mpsc::channel::<PendingUpdate>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    let mut tests = Vec::new();
    for suite in suites {
        {
            let cases = suite.cases.clone();
            tests.push(Trial::test(format!("{}::{}::_lint_cases", suite.category, suite.name), move || {
                let counts = cases.iter().map(|case| &case.name).fold(HashMap::<&str, u32>::new(), |mut counts, name| {
                    *counts.entry(name).or_default() += 1;
                    counts
                });
                let duplicate_cases = counts.into_iter().filter_map(|(name, count)| if count > 1 { Some(name) } else { None })
                    .collect::<Vec<_>>();
                assert!(duplicate_cases.is_empty(), "duplicate test cases: {duplicate_cases:?}. Ensure that all names are unique within a TOML file");
                Ok(())
            }));
        }
        for mut case in suite.cases {
            case.updates_tx = tx.clone();
            case.force_regenerate_expectations = force_regenerate_expectations;
            let ignored_flag = case
                .ignored
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            tests.push(
                Trial::test(
                    format!("{}::{}::{}", case.category, case.suite, case.name),
                    move || {
                        match case.kind {
                            TestCaseKind::Lexer => test_lexer(case),
                            TestCaseKind::Parser => test_parser(case),
                            TestCaseKind::Analyzer => test_analyzer(case),
                            TestCaseKind::Codegen => test_codegen(case),
                        };
                        Ok(())
                    },
                )
                .with_ignored_flag(ignored_flag),
            );
        }
    }
    // main thread sender must be dropped explicitly otherwise the receiver will hang indefinitely
    drop(tx);
    let conclusion = libtest_mimic::run(&args, tests);
    if let Some(rx) = rx {
        let updates = rx.into_iter().fold(
            HashMap::<(String, String), HashMap<String, Vec<PendingUpdateData>>>::new(),
            |mut map, u| {
                map.entry((u.category, u.suite))
                    .or_default()
                    .entry(u.case)
                    .or_default()
                    .push(u.data);
                map
            },
        );
        save_suites(updates).unwrap();
    }
    conclusion.exit()
}

/// get an environment variable as a boolean
fn env_as_bool(name: &'static str) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .is_some_and(|val| val != 0)
}

/// Load all suites from the tests/cases directory
fn load_suites() -> Result<Vec<TestSuite>, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let mut suites = Vec::new();
    for entry in std::fs::read_dir("tests/cases")? {
        let entry = entry?;
        if !entry.metadata()?.is_dir() {
            continue;
        }
        let category_path = entry.path();
        let category = category_path
            .file_name()
            .ok_or("expected filename")?
            .to_string_lossy();
        for entry in std::fs::read_dir(&category_path)? {
            let entry = entry?;
            let suite_path = entry.path();
            if entry.metadata()?.is_file()
                && suite_path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    == "toml"
            {
                let name = suite_path
                    .file_stem()
                    .ok_or("expected filename")?
                    .to_string_lossy();
                let mut suite: TestSuite =
                    toml_edit::de::from_str(&std::fs::read_to_string(&suite_path)?)?;
                suite.resolve_sources(suite_path.parent().unwrap());
                suite.category = category.to_string();
                suite.name = name.to_string();
                for case in suite.cases.iter_mut() {
                    case.category = category.to_string();
                    case.suite = name.to_string();
                }
                suites.push(suite);
            }
        }
    }
    Ok(suites)
}

/// Save all pending updates to the suite TOML files
fn save_suites(
    updates_by_suite: HashMap<(String, String), HashMap<String, Vec<PendingUpdateData>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    for ((category_name, suite_name), updates_by_case) in updates_by_suite {
        let suite_path = PathBuf::from(format!("tests/cases/{category_name}/{suite_name}.toml"));
        let mut suite = std::fs::read_to_string(&suite_path)?.parse::<toml_edit::DocumentMut>()?;
        let cases = suite
            .entry("case")
            .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
            .as_array_of_tables_mut()
            .expect("Expected 'case' to be an array of tables");
        for (case_name, updates) in updates_by_case {
            let case = cases
                .iter_mut()
                .find(|table| {
                    table
                        .get("name")
                        .and_then(toml_edit::Item::as_str)
                        .unwrap_or_default()
                        == case_name
                })
                .ok_or_else(|| {
                    format!("Failed to find case {case_name} in {category_name}::{suite_name}")
                })?;
            for update in updates {
                match update {
                    PendingUpdateData::UpdateErrors(errors) => {
                        if errors.is_empty() {
                            case.remove("expected_errors");
                        } else {
                            case.insert("expected_errors", {
                                let mut array = toml_edit::Array::from_iter(errors);
                                for value in array.iter_mut() {
                                    value.decor_mut().set_prefix("\n    ");
                                }
                                array.set_trailing_comma(true);
                                array.set_trailing("\n");
                                toml_edit::value(array)
                            });
                        }
                    }
                    PendingUpdateData::UpdateTokens(tokens) => {
                        if tokens.is_empty() {
                            case.remove("expected_tokens");
                        } else {
                            case.insert(
                                "expected_tokens",
                                toml_edit::value(format!("{}\n", tokens.trim())),
                            );
                        }
                    }
                    PendingUpdateData::UpdateAst(ast) => {
                        if ast.is_empty() {
                            case.remove("expected_ast");
                        } else {
                            case.insert(
                                "expected_ast",
                                toml_edit::value(format!("{}\n", ast.trim())),
                            );
                        }
                    }
                    PendingUpdateData::UpdateMir(mir) => {
                        if mir.is_empty() {
                            case.remove("expected_mir");
                        } else {
                            case.insert(
                                "expected_mir",
                                toml_edit::value(format!("{}\n", mir.trim())),
                            );
                        }
                    }
                    PendingUpdateData::UpdateLir(lir) => {
                        if lir.is_empty() {
                            case.remove("expected_lir");
                        } else {
                            case.insert(
                                "expected_lir",
                                toml_edit::value(format!("{}\n", lir.trim())),
                            );
                        }
                    }
                }
            }
            case.fmt();
        }
        std::fs::write(suite_path, suite.to_string())?;
    }
    Ok(())
}

fn update_expected_errors(
    tx: &mpsc::Sender<PendingUpdate>,
    case: &mut TestCase,
    actual_errors: &[String],
) {
    if actual_errors == case.expected_errors.as_slice() {
        return;
    }
    let is_removal = actual_errors.is_empty() && !case.expected_errors.is_empty();
    let is_addition = !actual_errors.is_empty()
        && case.expected_errors.is_empty()
        && !(case.expected_tokens.trim().is_empty()
            && case.expected_ast.trim().is_empty()
            && case.expected_mir.trim().is_empty()
            && case.expected_lir.trim().is_empty());
    if (is_removal || is_addition) && !case.force_regenerate_expectations {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateErrors(actual_errors.to_vec()),
    })
    .unwrap();
    case.expected_errors = actual_errors.to_vec();
}

fn update_expected_tokens(tx: &mpsc::Sender<PendingUpdate>, case: &mut TestCase, actual: &str) {
    let is_removal = {
        let expected = &case.expected_tokens;
        if actual.trim() == expected.trim() {
            return;
        }
        !expected.is_empty() && actual.trim().is_empty()
    };
    if is_removal && !case.force_regenerate_expectations {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateTokens(actual.to_string()),
    })
    .unwrap();
    case.expected_tokens = actual.to_string();
}

fn delete_expected_tokens(tx: &mpsc::Sender<PendingUpdate>, case: &mut TestCase) {
    if case.expected_tokens.is_empty() {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateTokens(String::new()),
    })
    .unwrap();
    case.expected_tokens = String::new();
}

fn update_expected_ast(tx: &mpsc::Sender<PendingUpdate>, case: &mut TestCase, actual: &str) {
    let is_removal = {
        let expected = &case.expected_ast;
        if actual.trim() == expected.trim() {
            return;
        }
        !expected.is_empty() && actual.trim().is_empty()
    };
    if is_removal && !case.force_regenerate_expectations {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateAst(actual.to_string()),
    })
    .unwrap();
    case.expected_ast = actual.to_string();
}

fn delete_expected_ast(tx: &mpsc::Sender<PendingUpdate>, case: &mut TestCase) {
    if case.expected_ast.is_empty() {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateAst(String::new()),
    })
    .unwrap();
    case.expected_ast = String::new();
}

fn update_expected_mir(tx: &mpsc::Sender<PendingUpdate>, case: &mut TestCase, actual: &str) {
    let is_removal = {
        let expected = &case.expected_mir;
        if actual.trim() == expected.trim() {
            return;
        }
        !expected.is_empty() && actual.trim().is_empty()
    };
    if is_removal && !case.force_regenerate_expectations {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateMir(actual.to_string()),
    })
    .unwrap();
    case.expected_mir = actual.to_string();
}

fn delete_expected_mir(tx: &mpsc::Sender<PendingUpdate>, case: &mut TestCase) {
    if case.expected_mir.is_empty() {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateMir(String::new()),
    })
    .unwrap();
    case.expected_mir = String::new();
}

fn update_expected_lir(tx: &mpsc::Sender<PendingUpdate>, case: &mut TestCase, actual: &str) {
    let is_removal = {
        let expected = &case.expected_lir;
        if actual.trim() == expected.trim() {
            return;
        }
        !expected.is_empty() && actual.trim().is_empty()
    };
    if is_removal && !case.force_regenerate_expectations {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateLir(actual.to_string()),
    })
    .unwrap();
    case.expected_lir = actual.to_string();
}

fn delete_expected_lir(tx: &mpsc::Sender<PendingUpdate>, case: &mut TestCase) {
    if case.expected_lir.is_empty() {
        return;
    }
    tx.send(PendingUpdate {
        category: case.category.clone(),
        suite: case.suite.clone(),
        case: case.name.clone(),
        data: PendingUpdateData::UpdateLir(String::new()),
    })
    .unwrap();
    case.expected_lir = String::new();
}

fn test_lexer(mut case: TestCase) {
    let lexer = quokka_lang::lex::Lexer::from(case.source());
    let mut actual_tokens = String::new();
    let mut actual_errors = Vec::new();
    for token in lexer {
        match token {
            Ok(token) => {
                writeln!(
                    actual_tokens,
                    "{:?}:{}",
                    token.kind(),
                    token.text().escape_default()
                )
                .unwrap();
            }
            Err(e) => {
                actual_errors = vec![format!("{}", DiagnosticTreeDisplay(&e))];
                break;
            }
        }
    }
    if let Some(tx) = case.updates_tx.clone() {
        update_expected_errors(&tx, &mut case, &actual_errors);
        update_expected_tokens(&tx, &mut case, &actual_tokens);
        delete_expected_ast(&tx, &mut case);
        delete_expected_mir(&tx, &mut case);
        delete_expected_lir(&tx, &mut case);
    }
    assert_eq!(
        actual_errors, case.expected_errors,
        "actual_errors == expected_errors"
    );
    assert_eq!(
        actual_tokens.trim(),
        case.expected_tokens.trim(),
        "actual_tokens == expected_tokens"
    );
}

fn test_parser(mut case: TestCase) {
    let mut parser = quokka_lang::parse::Parser::from(case.source());
    for feature in case.preview_features.iter().copied() {
        parser.enable_feature(feature);
    }
    let (actual_ast, actual_errors) = match case.entrypoint {
        ParseEntrypointKind::Root => prepare_parse_results(parser.parse()),
        ParseEntrypointKind::Item => prepare_parse_results(parser.parse_item_as_root()),
        ParseEntrypointKind::Statement => prepare_parse_results(parser.parse_statement_as_root()),
        ParseEntrypointKind::Expr => prepare_parse_results(parser.parse_expr_as_root()),
    };
    if let Some(tx) = case.updates_tx.clone() {
        update_expected_errors(&tx, &mut case, &actual_errors);
        update_expected_ast(&tx, &mut case, &actual_ast);
        delete_expected_tokens(&tx, &mut case);
        delete_expected_mir(&tx, &mut case);
        delete_expected_lir(&tx, &mut case);
    }
    assert_eq!(
        actual_errors, case.expected_errors,
        "actual_errors == expected_errors"
    );
    assert_eq!(
        actual_ast.trim(),
        case.expected_ast.trim(),
        "actual_ast == expected_ast"
    );
}

fn test_analyzer(mut case: TestCase) {
    let pipeline = quokka_lang::pipeline::PipelineConfig::default()
        .with_features(case.preview_features.iter().copied().collect())
        .start();
    let root = pipeline
        .parse(case.source())
        .unwrap_or_else(|e| panic!("Failed to parse. {e:?}"));
    let result = root.analyze().and_then(|output| {
        if case.optimize {
            output.optimize()
        } else {
            Ok(output)
        }
    });
    let (actual_mir, actual_errors) = match result {
        Ok(output) => (
            format!("{}", output.mir().display(output.ty_registry())),
            Vec::new(),
        ),
        Err(errors) => (String::new(), prepare_multi_error(errors)),
    };
    if let Some(tx) = case.updates_tx.clone() {
        update_expected_errors(&tx, &mut case, &actual_errors);
        update_expected_mir(&tx, &mut case, &actual_mir);
        delete_expected_tokens(&tx, &mut case);
        delete_expected_ast(&tx, &mut case);
        delete_expected_lir(&tx, &mut case);
    }
    assert_eq!(
        actual_errors, case.expected_errors,
        "actual_errors == expected_errors"
    );
    assert_eq!(
        actual_mir.trim(),
        case.expected_mir.trim(),
        "actual_mir == expected_mir"
    );
}

fn test_codegen(mut case: TestCase) {
    let pipeline = quokka_lang::pipeline::PipelineConfig::default()
        .with_features(case.preview_features.iter().copied().collect())
        .start();
    let root = pipeline
        .parse(case.source())
        .unwrap_or_else(|e| panic!("Failed to parse. {e:?}"))
        .analyze()
        .and_then(|output| {
            if case.optimize {
                output.optimize()
            } else {
                Ok(output)
            }
        })
        .unwrap_or_else(|e| panic!("Failed to type check. {e:?}"));
    let result = root.codegen().map(|output| {
        if case.allocate_registers {
            output.allocate().into_lir()
        } else {
            output.into_lir()
        }
    });
    let (actual_lir, actual_errors) = match result {
        Ok(lir) => (format!("{}", lir), Vec::new()),
        Err(errors) => (String::new(), prepare_multi_error(errors)),
    };
    if let Some(tx) = case.updates_tx.clone() {
        update_expected_errors(&tx, &mut case, &actual_errors);
        update_expected_lir(&tx, &mut case, &actual_lir);
        delete_expected_tokens(&tx, &mut case);
        delete_expected_ast(&tx, &mut case);
        delete_expected_mir(&tx, &mut case);
    }
    assert_eq!(
        actual_errors, case.expected_errors,
        "actual_errors == expected_errors"
    );
    assert_eq!(
        actual_lir.trim(),
        case.expected_lir.trim(),
        "actual_lir == expected_lir"
    );
}

fn prepare_parse_results<T: serde::Serialize>(result: MultiResult<T>) -> (String, Vec<String>) {
    let (actual, actual_errors) = result.into_parts();
    let actual = match actual {
        Some(actual) => ron::ser::to_string_pretty(
            &actual,
            ron::ser::PrettyConfig::new()
                .struct_names(true)
                .indentor("  "),
        )
        .unwrap(),
        None => String::new(),
    };
    let actual_errors = prepare_multi_error(actual_errors);
    (actual, actual_errors)
}

fn prepare_multi_error(actual_errors: MultiError) -> Vec<String> {
    actual_errors
        .errors
        .into_iter()
        .map(|e| format!("{}", DiagnosticTreeDisplay(&*e)))
        .collect()
}

struct DiagnosticTreeDisplay<'a>(&'a dyn miette::Diagnostic);

impl DiagnosticTreeDisplay<'_> {
    fn primary_label(&self) -> Option<LabeledSpan> {
        self.0
            .labels()?
            .reduce(|a, b| if a.primary() && !b.primary() { a } else { b })
    }
}

impl fmt::Display for DiagnosticTreeDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = self.0.code() {
            write!(f, "{code}: ")?;
        }
        write!(f, "{}", self.0)?;
        if let Some(label) = self.primary_label() {
            write!(
                f,
                " @ [{}:{}]",
                label.offset(),
                label.offset() + label.len()
            )?;
        }
        Ok(())
    }
}
