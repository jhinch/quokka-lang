use crate::analyze::Analyzer;
use crate::ast;
use crate::codegen::CodeGen;
use crate::codegen::encode::ObjFileEncoder;
use crate::codegen::regalloc::RegAllocator;
use crate::feature::{PreviewFeature, PreviewFeatureSet};
use crate::lir::Lir;
use crate::mir::Mir;
use crate::multi::MultiError;
use crate::optimize::Optimizer;
use crate::parse::Parser;
use crate::ty::TyRegistry;
use quokka_lang_obj::ObjFile;
use std::fs::File;
use std::io::{self, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

#[derive(Default)]
pub struct PipelineConfig {
    features: PreviewFeatureSet,
}

impl PipelineConfig {
    pub fn enable_feature(&mut self, feature: PreviewFeature) {
        self.features.enable(feature);
    }

    pub fn enable_features(&mut self, features: PreviewFeatureSet) {
        self.features.extend(features);
    }

    #[must_use]
    pub fn with_features(mut self, features: PreviewFeatureSet) -> Self {
        self.enable_features(features);
        self
    }
}

impl PipelineConfig {
    #[must_use]
    pub fn start(self) -> Pipeline {
        Pipeline { config: self }
    }
}

pub struct Pipeline {
    config: PipelineConfig,
}

impl Pipeline {
    pub fn parse(self, contents: &str) -> Result<ParserOutput, MultiError> {
        let config = self.config;
        let mut parser = Parser::from(contents);
        parser.enable_features(config.features.clone());
        let ast = parser.parse().into_result()?;
        Ok(ParserOutput { config, ast })
    }

    pub fn parse_item(self, contents: &str) -> Result<ParserOutput<ast::Expr>, MultiError> {
        let config = self.config;
        let mut parser = Parser::from(contents);
        parser.enable_features(config.features.clone());
        let ast = parser.parse_expr_as_root().into_result()?;
        Ok(ParserOutput { config, ast })
    }

    pub fn parse_statement(
        self,
        contents: &str,
    ) -> Result<ParserOutput<ast::Statement>, MultiError> {
        let config = self.config;
        let mut parser = Parser::from(contents);
        parser.enable_features(config.features.clone());
        let ast = parser.parse_statement_as_root().into_result()?;
        Ok(ParserOutput { config, ast })
    }

    pub fn parse_expr(self, contents: &str) -> Result<ParserOutput<ast::Item>, MultiError> {
        let config = self.config;
        let mut parser = Parser::from(contents);
        parser.enable_features(config.features.clone());
        let ast = parser.parse_item_as_root().into_result()?;
        Ok(ParserOutput { config, ast })
    }
}

pub struct ParserOutput<T = ast::Root> {
    config: PipelineConfig,
    ast: T,
}

impl<T> ParserOutput<T> {
    pub fn ast(&self) -> &T {
        &self.ast
    }
}

impl ParserOutput {
    pub fn analyze(self) -> Result<AnalyzerOutput, MultiError> {
        let config = self.config;
        let mut analyzer = Analyzer::default();
        analyzer.enable_features(config.features.clone());
        let (mir, ty_registry) = analyzer.analyze(&self.ast)?;
        Ok(AnalyzerOutput {
            config,
            ty_registry,
            mir,
        })
    }
}

pub struct AnalyzerOutput {
    config: PipelineConfig,
    ty_registry: TyRegistry,
    mir: Mir,
}

impl AnalyzerOutput {
    #[must_use]
    pub fn mir(&self) -> &Mir {
        &self.mir
    }

    #[must_use]
    pub fn ty_registry(&self) -> &TyRegistry {
        &self.ty_registry
    }

    pub fn optimize(mut self) -> Result<AnalyzerOutput, MultiError> {
        let mut optimizer = Optimizer::default();
        optimizer.enable_features(self.config.features.clone());
        optimizer.optimize(&self.ty_registry, &mut self.mir)?;
        Ok(self)
    }

    pub fn codegen(self) -> Result<CodegenOutput, MultiError> {
        let config = self.config;
        let codegen = CodeGen::from(self.ty_registry);
        let lir = codegen.generate(&self.mir)?;
        Ok(CodegenOutput { config, lir })
    }
}

pub struct CodegenOutput {
    config: PipelineConfig,
    lir: Lir,
}

impl CodegenOutput {
    #[must_use]
    pub fn lir(&self) -> &Lir {
        &self.lir
    }

    #[must_use]
    pub fn into_lir(self) -> Lir {
        self.lir
    }

    #[must_use]
    pub fn allocate(mut self) -> AllocateOutput {
        let mut allocator = RegAllocator::default();
        allocator.enable_features(self.config.features.clone());
        allocator.allocate(&mut self.lir);
        AllocateOutput {
            config: self.config,
            lir: self.lir,
        }
    }
}

pub struct AllocateOutput {
    #[allow(dead_code)]
    config: PipelineConfig,
    lir: Lir,
}

impl AllocateOutput {
    #[must_use]
    pub fn lir(&self) -> &Lir {
        &self.lir
    }

    #[must_use]
    pub fn into_lir(self) -> Lir {
        self.lir
    }

    #[must_use]
    pub fn encode(self) -> EncodedOutput {
        let mut encoder = ObjFileEncoder::default();
        encoder.enable_features(self.config.features.clone());
        let obj = encoder.encode(&self.lir);
        EncodedOutput { obj }
    }
}

pub struct EncodedOutput {
    obj: ObjFile,
}

impl EncodedOutput {
    #[must_use]
    pub fn obj(&self) -> &ObjFile {
        &self.obj
    }

    #[must_use]
    pub fn into_obj(self) -> ObjFile {
        self.obj
    }

    /// Write the encoded RGBDS object file to `path`.
    pub fn write_obj(self, path: impl AsRef<Path>) -> Result<ObjOutput, BuildError> {
        let path = path.as_ref();
        let mut file = BufWriter::new(File::create(path)?);
        quokka_lang_obj::write(&mut file, &self.obj)?;
        file.flush()?;
        Ok(ObjOutput {
            obj_path: path.to_path_buf(),
        })
    }
}

pub struct ObjOutput {
    obj_path: PathBuf,
}

impl ObjOutput {
    #[must_use]
    pub fn obj_path(&self) -> &Path {
        &self.obj_path
    }

    /// Link the object file into a Game Boy ROM using `rgblink`, writing the ROM to `gb_path`
    /// and its symbol table to `sym_path`.
    pub fn link(
        self,
        gb_path: impl AsRef<Path>,
        sym_path: impl AsRef<Path>,
    ) -> Result<LinkedOutput, BuildError> {
        let gb_path = gb_path.as_ref().to_path_buf();
        let sym_path = sym_path.as_ref().to_path_buf();
        run_tool(
            "rgblink",
            Command::new("rgblink")
                .arg("--output")
                .arg(&gb_path)
                .arg("--sym")
                .arg(&sym_path)
                .arg(&self.obj_path),
        )?;
        Ok(LinkedOutput { gb_path, sym_path })
    }
}

pub struct LinkedOutput {
    gb_path: PathBuf,
    sym_path: PathBuf,
}

impl LinkedOutput {
    #[must_use]
    pub fn gb_path(&self) -> &Path {
        &self.gb_path
    }

    #[must_use]
    pub fn sym_path(&self) -> &Path {
        &self.sym_path
    }

    /// Fix up the ROM's header (logo, checksums) in place using `rgbfix`, producing a ROM ready
    /// to load into an emulator or flash cart.
    pub fn fix(self) -> Result<RomOutput, BuildError> {
        run_tool(
            "rgbfix",
            Command::new("rgbfix")
                .arg("-v")
                .arg("-p")
                .arg("0xFF")
                .arg(&self.gb_path),
        )?;
        Ok(RomOutput {
            gb_path: self.gb_path,
            sym_path: self.sym_path,
        })
    }
}

pub struct RomOutput {
    gb_path: PathBuf,
    sym_path: PathBuf,
}

impl RomOutput {
    #[must_use]
    pub fn gb_path(&self) -> &Path {
        &self.gb_path
    }

    #[must_use]
    pub fn sym_path(&self) -> &Path {
        &self.sym_path
    }

    /// Read the built ROM's bytes back off disk.
    pub fn read_rom(&self) -> io::Result<Vec<u8>> {
        std::fs::read(&self.gb_path)
    }
}

fn run_tool(tool: &'static str, command: &mut Command) -> Result<(), BuildError> {
    let status = command.stdin(Stdio::null()).status()?;
    if !status.success() {
        return Err(BuildError::ToolFailed { tool, status });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum BuildError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("`{tool}` exited with {status}")]
    ToolFailed {
        tool: &'static str,
        status: ExitStatus,
    },
}
