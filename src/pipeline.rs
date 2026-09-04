use crate::analyze::Analyzer;
use crate::ast;
use crate::codegen::CodeGen;
use crate::feature::{PreviewFeature, PreviewFeatureSet};
use crate::lir::Lir;
use crate::mir::Mir;
use crate::multi::MultiError;
use crate::optimize::Optimizer;
use crate::parse::Parser;
use crate::regalloc::RegAllocator;
use crate::ty::TyRegistry;

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
}
