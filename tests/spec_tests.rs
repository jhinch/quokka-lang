use libtest_mimic::{Arguments, Trial};
use miette::LabeledSpan;
use quokka_lang::analyze::Analyzer;
use quokka_lang::multi::{MultiError, MultiResult};
use std::collections::HashMap;
use std::fmt;
use std::fmt::Write;
use std::path::PathBuf;
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
    preview_features: Vec<quokka_lang::feature::PreviewFeature>,
    #[serde(default)]
    ignored: Option<bool>,
    source: String,
    #[serde(default)]
    expected_tokens: String,
    #[serde(default)]
    expected_ast: String,
    #[serde(default)]
    expected_errors: Vec<String>,
    #[serde(skip)]
    updates_tx: Option<mpsc::Sender<PendingUpdate>>,
}

/// Deserialized representation of the kind of case
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TestCaseKind {
    Lexer,
    Parser,
    Analyzer,
}

/// Deserialized representation of the kind entrypoint for a parser test
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ParseEntrypointKind {
    Root,
    Item,
    Statement,
    Expr,
}

impl Default for ParseEntrypointKind {
    fn default() -> Self {
        ParseEntrypointKind::Root
    }
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
}

fn main() {
    let args = Arguments::from_args();
    let suites = load_suites().unwrap();
    let regenerate_expectations = std::env::var("REGENERATE_EXPECTATIONS")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .is_some_and(|val| val != 0);
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
            let ignored_flag = case
                .ignored
                .clone()
                .unwrap_or(!case.preview_features.is_empty());
            tests.push(
                Trial::test(
                    format!("{}::{}::{}", case.category, case.suite, case.name),
                    move || {
                        match case.kind {
                            TestCaseKind::Lexer => test_lexer(case),
                            TestCaseKind::Parser => test_parser(case),
                            TestCaseKind::Analyzer => test_analyzer(case),
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
                }
            }
            case.fmt();
        }
        std::fs::write(suite_path, suite.to_string())?;
    }
    Ok(())
}

fn test_lexer(case: TestCase) {
    let lexer = quokka_lang::lex::Lexer::from(&*case.source);
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
    if let Some(tx) = case.updates_tx {
        if actual_errors != case.expected_errors {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateErrors(actual_errors),
            })
            .unwrap();
        }
        if actual_tokens.trim() != case.expected_tokens.trim() {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateTokens(actual_tokens),
            })
            .unwrap();
        }
        if !case.expected_ast.is_empty() {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateAst(String::new()),
            })
            .unwrap();
        }
        return;
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

fn test_parser(case: TestCase) {
    let mut parser = quokka_lang::parse::Parser::from(&*case.source);
    for feature in case.preview_features.iter().copied() {
        parser.enable_feature(feature);
    }
    let (actual_ast, actual_errors) = match case.entrypoint {
        ParseEntrypointKind::Root => prepare_parse_results(parser.parse()),
        ParseEntrypointKind::Item => prepare_parse_results(parser.parse_item_as_root()),
        ParseEntrypointKind::Statement => prepare_parse_results(parser.parse_statement_as_root()),
        ParseEntrypointKind::Expr => prepare_parse_results(parser.parse_expr_as_root()),
    };
    if let Some(tx) = case.updates_tx {
        if actual_errors != case.expected_errors {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateErrors(actual_errors),
            })
            .unwrap();
        }
        if actual_ast.trim() != case.expected_ast.trim() {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateAst(actual_ast),
            })
            .unwrap();
        }
        if !case.expected_tokens.is_empty() {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateTokens(String::new()),
            })
            .unwrap();
        }
        return;
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

fn test_analyzer(case: TestCase) {
    let mut parser = quokka_lang::parse::Parser::from(&*case.source);
    for feature in case.preview_features.iter().copied() {
        parser.enable_feature(feature);
    }
    let root = parser
        .parse()
        .into_result()
        .unwrap_or_else(|e| panic!("Failed to parse. {e}"));
    let mut analyzer = Analyzer::default();
    let actual_errors = match analyzer.analyze(&root) {
        Ok(()) => Vec::new(),
        Err(errors) => prepare_multi_error(errors),
    };
    if let Some(tx) = case.updates_tx {
        if actual_errors != case.expected_errors {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateErrors(actual_errors),
            })
            .unwrap();
        }
        if !case.expected_tokens.is_empty() {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateTokens(String::new()),
            })
            .unwrap();
        }
        if !case.expected_ast.is_empty() {
            tx.send(PendingUpdate {
                category: case.category.clone(),
                suite: case.suite.clone(),
                case: case.name.clone(),
                data: PendingUpdateData::UpdateAst(String::new()),
            })
            .unwrap();
        }
        return;
    }
    assert_eq!(
        actual_errors, case.expected_errors,
        "actual_errors == expected_errors"
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
