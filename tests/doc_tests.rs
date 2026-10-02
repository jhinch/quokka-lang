//! Checks that every ```` ```quokka ```` code block in the mdBook sources under `docs/src`
//! compiles, so documentation examples cannot silently drift from the compiler. Each example is
//! taken through the same pipeline as `quokka build`, up to (but not including) linking.
//!
//! Code blocks are configured through comma-separated attributes in the fence's info string,
//! in the same style as rustdoc:
//!
//! - ```` ```quokka ```` the example must compile
//! - ```` ```quokka,preview=narrow-int ```` the example must compile with the given preview
//!   feature enabled. May be repeated to enable multiple preview features
//! - ```` ```quokka,ignore ```` the example is illustrative only and is not compiled
//!
//! Examples which don't define an entrypoint (a `main` function or an `#[entrypoint]` function)
//! have an empty `fn main() {}` appended so that fragments demonstrating items can be compiled.
use libtest_mimic::{Arguments, Failed, Trial};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use quokka_lang::ast;
use quokka_lang::feature::{PreviewFeature, PreviewFeatureSet};
use quokka_lang::multi::MultiError;
use std::path::{Path, PathBuf};

const LANGUAGE: &str = "quokka";
const ENTRYPOINT_STUB: &str = "\nfn main() {}\n";

fn main() {
    let args = Arguments::from_args();
    let docs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/src");
    let mut tests = Vec::new();
    for path in markdown_files(&docs_dir) {
        let contents = std::fs::read_to_string(&path).unwrap();
        let relative_path = path.strip_prefix(&docs_dir).unwrap().display().to_string();
        for example in extract_examples(&contents) {
            let name = format!("{relative_path}:{}", example.line);
            let ignored = example.ignored;
            tests.push(Trial::test(name, move || example.check()).with_ignored_flag(ignored));
        }
    }
    libtest_mimic::run(&args, tests).exit()
}

/// recursively find all markdown files in a directory, sorted for a stable test order
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(markdown_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "md") {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// A quokka code block extracted from a markdown file
struct Example {
    /// 1-based line number of the opening code fence
    line: usize,
    source: String,
    ignored: bool,
    /// the preview features to enable, or an error if the info string is invalid
    preview_features: Result<Vec<PreviewFeature>, String>,
}

fn extract_examples(markdown: &str) -> Vec<Example> {
    let mut examples = Vec::new();
    let mut current: Option<Example> = None;
    for (event, range) in Parser::new(markdown).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let mut attributes = info.split(',').map(str::trim);
                if attributes.next() != Some(LANGUAGE) {
                    continue;
                }
                let mut ignored = false;
                let mut preview_features = Ok(Vec::new());
                for attribute in attributes {
                    match attribute.split_once('=') {
                        None if attribute == "ignore" => ignored = true,
                        Some(("preview", feature)) => {
                            match clap::ValueEnum::from_str(feature, false) {
                                Ok(feature) => {
                                    if let Ok(features) = &mut preview_features {
                                        features.push(feature);
                                    }
                                }
                                Err(_) => {
                                    preview_features =
                                        Err(format!("unknown preview feature `{feature}`"));
                                }
                            }
                        }
                        _ => {
                            preview_features = Err(format!(
                                "unknown code block attribute `{attribute}`. \
                                 Expected `ignore` or `preview=<feature>`"
                            ));
                        }
                    }
                }
                current = Some(Example {
                    line: markdown[..range.start].lines().count() + 1,
                    source: String::new(),
                    ignored,
                    preview_features,
                });
            }
            Event::Text(text) => {
                if let Some(example) = &mut current {
                    example.source.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => examples.extend(current.take()),
            _ => {}
        }
    }
    examples
}

impl Example {
    fn check(self) -> Result<(), Failed> {
        let features: PreviewFeatureSet = self.preview_features?.into_iter().collect();
        let mut source = self.source;
        let has_entrypoint = parse(&features, &source)
            .map_err(|errors| render(errors, &source))?
            .items
            .iter()
            .any(is_entrypoint);
        if !has_entrypoint {
            source.push_str(ENTRYPOINT_STUB);
        }
        quokka_lang::pipeline::PipelineConfig::default()
            .with_features(features)
            .start()
            .parse(&source)
            .and_then(|output| output.analyze())
            .and_then(|output| output.optimize())
            .and_then(|output| output.codegen())
            .map(|output| output.allocate().peephole().encode())
            .map(|_encoded| ())
            .map_err(|errors| render(errors, &source))
    }
}

fn parse(features: &PreviewFeatureSet, source: &str) -> Result<ast::Root, MultiError> {
    let output = quokka_lang::pipeline::PipelineConfig::default()
        .with_features(features.clone())
        .start()
        .parse(source)?;
    Ok(output.ast().clone())
}

fn is_entrypoint(item: &ast::Item) -> bool {
    let ast::ItemData::FnDecl(decl) = &item.data else {
        return false;
    };
    decl.signature.name.data.as_str() == "main"
        || item
            .attributes
            .iter()
            .any(|attr| attr.name.data.as_str() == "entrypoint")
}

/// render the errors against the example source in the same style as the CLI
fn render(errors: MultiError, source: &str) -> Failed {
    let report = miette::Report::new(errors).with_source_code(source.to_string());
    let mut rendered = String::new();
    miette::GraphicalReportHandler::new_themed(miette::GraphicalTheme::unicode_nocolor())
        .render_report(&mut rendered, report.as_ref())
        .unwrap();
    Failed::from(rendered)
}
