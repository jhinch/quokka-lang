use crate::cli::Execute;
use lsp_server::{
    Connection, ErrorCode, Message, Notification, Request, RequestId, Response, ResponseError,
};
use lsp_types::{
    Code, Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Full, Location,
    LspNotificationMethod, LspRequestMethod, Position, PublishDiagnosticsParams, Range,
    SemanticToken, SemanticTokenModifiers, SemanticTokenTypes, SemanticTokens,
    SemanticTokensLegend, SemanticTokensOptions, ServerCapabilities,
    TextDocumentContentChangeEvent, TextDocumentSyncKind, Uri,
};
use quokka_lang::lex;
use quokka_lang::pipeline::PipelineConfig;
use std::collections::{HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

/// How long the server waits without hearing from the client before analyzing documents that
/// changed, so a burst of keystrokes results in a single analysis.
const DIAGNOSTICS_DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(clap::Args)]
pub struct LspCommand;

/// The semantic token types this server emits. The discriminant is the index into the legend
/// advertised to the client, so the order here is the legend order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenType {
    Operator,
    Number,
    Keyword,
    Comment,
    Variable,
    Function,
    Type,
    Decorator,
}

impl TokenType {
    const ALL: [TokenType; 8] = [
        TokenType::Operator,
        TokenType::Number,
        TokenType::Keyword,
        TokenType::Comment,
        TokenType::Variable,
        TokenType::Function,
        TokenType::Type,
        TokenType::Decorator,
    ];

    fn lsp_type(self) -> SemanticTokenTypes {
        match self {
            TokenType::Operator => SemanticTokenTypes::Operator,
            TokenType::Number => SemanticTokenTypes::Number,
            TokenType::Keyword => SemanticTokenTypes::Keyword,
            TokenType::Comment => SemanticTokenTypes::Comment,
            TokenType::Variable => SemanticTokenTypes::Variable,
            TokenType::Function => SemanticTokenTypes::Function,
            TokenType::Type => SemanticTokenTypes::Type,
            TokenType::Decorator => SemanticTokenTypes::Decorator,
        }
    }
}

/// The semantic token modifiers this server emits. The discriminant is the bit position in a
/// token's modifier bitset, which is also the index into the legend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenModifier {
    Declaration,
    Readonly,
    Static,
}

impl TokenModifier {
    const ALL: [TokenModifier; 3] = [
        TokenModifier::Declaration,
        TokenModifier::Readonly,
        TokenModifier::Static,
    ];

    fn lsp_modifier(self) -> SemanticTokenModifiers {
        match self {
            TokenModifier::Declaration => SemanticTokenModifiers::Declaration,
            TokenModifier::Readonly => SemanticTokenModifiers::Readonly,
            TokenModifier::Static => SemanticTokenModifiers::Static,
        }
    }

    fn bit(self) -> u32 {
        1 << self as u32
    }
}

fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TokenType::ALL
            .iter()
            .map(|ty| ty.lsp_type().as_str().to_owned())
            .collect(),
        token_modifiers: TokenModifier::ALL
            .iter()
            .map(|modifier| String::from(modifier.lsp_modifier()))
            .collect(),
    }
}

/// In-memory contents of the documents the client has open, keyed by URI.
#[derive(Default)]
struct State {
    documents: HashMap<Uri, String>,
    /// Documents changed since their diagnostics were last published.
    dirty: HashSet<Uri>,
}

impl Execute for LspCommand {
    fn execute(self) -> miette::Result<()> {
        let (connection, io_thread) = Connection::stdio();

        let caps = ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncKind::Full.into()),
            semantic_tokens_provider: Some(
                SemanticTokensOptions {
                    legend: legend(),
                    full: Some(Full::Bool(true)),
                    ..Default::default()
                }
                .into(),
            ),
            ..Default::default()
        };

        connection
            .initialize(serde_json::to_value(caps).map_err(miette::Report::from_err)?)
            .map_err(miette::Report::from_err)?;

        process_connection(connection)?;

        io_thread.join().map_err(miette::Report::from_err)?;

        Ok(())
    }
}

fn process_connection(connection: Connection) -> miette::Result<()> {
    let mut state = State::default();
    loop {
        // While documents are waiting on diagnostics, only wait `DIAGNOSTICS_DEBOUNCE` for the
        // next message; a quiet client means it is time to analyze.
        let msg = if state.dirty.is_empty() {
            match connection.receiver.recv() {
                Ok(msg) => msg,
                Err(_) => break,
            }
        } else {
            match connection.receiver.recv_timeout(DIAGNOSTICS_DEBOUNCE) {
                Ok(msg) => msg,
                Err(e) if e.is_timeout() => {
                    publish_dirty_diagnostics(&connection, &mut state)?;
                    continue;
                }
                Err(_) => break,
            }
        };
        match msg {
            Message::Request(req) => {
                if connection
                    .handle_shutdown(&req)
                    .map_err(miette::Report::from_err)?
                {
                    break;
                }
                // Responses should never be based on state newer than the diagnostics the client
                // has seen, so settle any pending analysis first.
                publish_dirty_diagnostics(&connection, &mut state)?;
                process_request(&connection, &mut state, &req)?;
            }
            Message::Notification(note) => {
                process_notification(&connection, &mut state, &note)?;
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn process_request(
    connection: &Connection,
    state: &mut State,
    req: &Request,
) -> miette::Result<()> {
    macro_rules! delegate_to_handlers {
        ($($paths: path)*) => {{
            let result = match LspRequestMethod::from(req.method.as_str()) {
                $(<$paths as lsp_types::Request>::METHOD => {
                    let params = match serde_json::from_value::<<$paths as lsp_types::Request>::Params>(req.params.clone()) {
                        Ok(params) => params,
                        Err(e) => return respond_error(
                            connection,
                            req.id.clone(),
                            ErrorCode::InvalidParams,
                            e.to_string(),
                        ),
                    };
                    let result = <$paths as RequestHandler>::handle(state, connection, params)?;
                    serde_json::to_value(result).map_err(miette::Report::from_err)?
                })*
                _ => return respond_error(
                    connection,
                    req.id.clone(),
                    ErrorCode::MethodNotFound,
                    "unhandled method",
                ),
            };
            respond_ok(connection, req.id.clone(), result)
        }};
    }
    delegate_to_handlers!(lsp_types::SemanticTokensRequest)
}

fn process_notification(
    connection: &Connection,
    state: &mut State,
    notification: &Notification,
) -> miette::Result<()> {
    macro_rules! delegate_to_handlers {
        ($($paths: path)*) => {
            match LspNotificationMethod::from(notification.method.as_str()) {
                $(<$paths as lsp_types::Notification>::METHOD => {
                    // Notifications have no response channel, so malformed ones are dropped.
                    if let Ok(params) = serde_json::from_value::<<$paths as lsp_types::Notification>::Params>(notification.params.clone()) {
                        <$paths as NotificationHandler>::handle(state, connection, params)?;
                    }
                })*
                _ => {}
            }
        };
    }
    delegate_to_handlers!(
        lsp_types::DidOpenTextDocumentNotification
        lsp_types::DidChangeTextDocumentNotification
        lsp_types::DidCloseTextDocumentNotification
    );
    Ok(())
}

trait RequestHandler: lsp_types::Request {
    fn handle(
        state: &mut State,
        connection: &Connection,
        params: Self::Params,
    ) -> miette::Result<Self::Result>;
}

trait NotificationHandler: lsp_types::Notification {
    fn handle(
        state: &mut State,
        connection: &Connection,
        params: Self::Params,
    ) -> miette::Result<()>;
}

fn respond_ok(conn: &Connection, id: RequestId, result: serde_json::Value) -> miette::Result<()> {
    let resp = Response {
        id,
        response_result: Ok(result),
    };
    conn.sender
        .send(Message::Response(resp))
        .map_err(miette::Report::from_err)?;
    Ok(())
}

fn respond_error(
    conn: &Connection,
    id: RequestId,
    code: ErrorCode,
    message: impl Into<String>,
) -> miette::Result<()> {
    let resp = Response {
        id,
        response_result: Err(ResponseError {
            code: code as i32,
            message: message.into(),
            data: None,
        }),
    };
    conn.sender
        .send(Message::Response(resp))
        .map_err(miette::Report::from_err)?;
    Ok(())
}

impl NotificationHandler for lsp_types::DidOpenTextDocumentNotification {
    fn handle(
        state: &mut State,
        connection: &Connection,
        params: Self::Params,
    ) -> miette::Result<()> {
        let doc = params.text_document;
        // Opening is not part of a typing burst, so there is nothing to wait for.
        publish_diagnostics(connection, &doc.uri, &doc.text)?;
        state.dirty.remove(&doc.uri);
        state.documents.insert(doc.uri, doc.text);
        Ok(())
    }
}

impl NotificationHandler for lsp_types::DidChangeTextDocumentNotification {
    fn handle(
        state: &mut State,
        _connection: &Connection,
        params: Self::Params,
    ) -> miette::Result<()> {
        // Full-text sync: each change event carries the entire new document, so the last wins.
        let new_text = params
            .content_changes
            .into_iter()
            .rev()
            .find_map(|change| match change {
                TextDocumentContentChangeEvent::TextDocumentContentChangeWholeDocument(whole) => {
                    Some(whole.text)
                }
                TextDocumentContentChangeEvent::TextDocumentContentChangePartial(_) => None,
            });
        if let Some(text) = new_text {
            let uri = params.text_document.text_document_identifier.uri;
            state.dirty.insert(uri.clone());
            state.documents.insert(uri, text);
        }
        Ok(())
    }
}

impl NotificationHandler for lsp_types::DidCloseTextDocumentNotification {
    fn handle(
        state: &mut State,
        connection: &Connection,
        params: Self::Params,
    ) -> miette::Result<()> {
        let uri = params.text_document.uri;
        state.documents.remove(&uri);
        state.dirty.remove(&uri);
        // Clear whatever the client is still displaying for the closed document.
        send_diagnostics(connection, uri, Vec::new())
    }
}

impl RequestHandler for lsp_types::SemanticTokensRequest {
    fn handle(
        state: &mut State,
        _connection: &Connection,
        params: Self::Params,
    ) -> miette::Result<Self::Result> {
        let Some(text) = state.documents.get(&params.text_document.uri) else {
            return Ok(None);
        };
        Ok(Some(SemanticTokens {
            data: semantic_tokens(text),
            ..Default::default()
        }))
    }
}

/// Analyzes every document changed since its diagnostics were last published.
fn publish_dirty_diagnostics(connection: &Connection, state: &mut State) -> miette::Result<()> {
    for uri in std::mem::take(&mut state.dirty) {
        if let Some(text) = state.documents.get(&uri) {
            publish_diagnostics(connection, &uri, text)?;
        }
    }
    Ok(())
}

fn publish_diagnostics(connection: &Connection, uri: &Uri, text: &str) -> miette::Result<()> {
    match diagnostics(uri, text) {
        Some(diagnostics) => send_diagnostics(connection, uri.clone(), diagnostics),
        // The compiler crashed. Leave the previous diagnostics up rather than claim the document
        // is now clean.
        None => Ok(()),
    }
}

fn send_diagnostics(
    connection: &Connection,
    uri: Uri,
    diagnostics: Vec<Diagnostic>,
) -> miette::Result<()> {
    let params = PublishDiagnosticsParams {
        uri,
        version: None,
        diagnostics,
    };
    connection
        .sender
        .send(Message::Notification(Notification::new(
            <lsp_types::PublishDiagnosticsNotification as lsp_types::Notification>::METHOD
                .to_string(),
            params,
        )))
        .map_err(miette::Report::from_err)
}

/// Runs the compiler front end (parse and analyze, with no preview features) over `text` and
/// converts everything it reports into LSP diagnostics.
///
/// Returns `None` if the compiler panicked.
fn diagnostics(uri: &Uri, text: &str) -> Option<Vec<Diagnostic>> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        PipelineConfig::default()
            .start()
            .parse(text)
            .and_then(|parsed| parsed.analyze())
            .err()
    }));
    let errors = match result {
        Ok(errors) => errors,
        Err(_) => {
            tracing::error!("compiler panicked while analyzing {}", uri.as_str());
            return None;
        }
    };

    let line_starts = line_starts(text);
    let mut diagnostics = Vec::new();
    for error in errors.iter().flat_map(|multi| &multi.errors) {
        convert_diagnostic(uri, text, &line_starts, error.as_ref(), &mut diagnostics);
    }
    Some(diagnostics)
}

/// Converts `diagnostic`, and any diagnostics nested under it, appending them to `out`.
///
/// The primary label (or the first label when none is marked primary) provides the range; any
/// other labels become related information.
fn convert_diagnostic(
    uri: &Uri,
    text: &str,
    line_starts: &[usize],
    diagnostic: &dyn miette::Diagnostic,
    out: &mut Vec<Diagnostic>,
) {
    let range_of = |label: &miette::LabeledSpan| Range {
        start: lsp_position(text, line_starts, label.offset()),
        end: lsp_position(text, line_starts, label.offset() + label.len()),
    };
    let labels = diagnostic
        .labels()
        .map(|labels| labels.collect::<Vec<_>>())
        .unwrap_or_default();
    let primary = labels
        .iter()
        .position(|label| label.primary())
        .unwrap_or_default();
    let range = labels.get(primary).map(range_of).unwrap_or_default();
    let related_information = labels
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != primary)
        .map(|(_, label)| DiagnosticRelatedInformation {
            location: Location {
                uri: uri.clone(),
                range: range_of(label),
            },
            message: label.label().unwrap_or_default().to_owned(),
        })
        .collect::<Vec<_>>();

    let mut message = diagnostic.to_string();
    if let Some(help) = diagnostic.help() {
        message.push_str(&format!("\nhelp: {help}"));
    }

    out.push(Diagnostic {
        range,
        severity: Some(match diagnostic.severity() {
            None | Some(miette::Severity::Error) => DiagnosticSeverity::Error,
            Some(miette::Severity::Warning) => DiagnosticSeverity::Warning,
            Some(miette::Severity::Advice) => DiagnosticSeverity::Hint,
        }),
        code: diagnostic.code().map(|code| Code::String(code.to_string())),
        code_description: None,
        source: Some("quokka".to_owned()),
        message: message.into(),
        tags: None,
        related_information: (!related_information.is_empty()).then_some(related_information),
        data: None,
    });

    for related in diagnostic.related().into_iter().flatten() {
        convert_diagnostic(uri, text, line_starts, related, out);
    }
}

/// What the previous significant token says about the identifier that may follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    None,
    /// After a declaring keyword (`fn`, `let`, `const`, `global`, `struct`): the next identifier is the
    /// name being declared.
    Declaring(Declaration),
    /// After the name of a `let`, `const` or `global`: a `:` introduces its type.
    DeclaredName,
    /// After the `:` of a declaration or a `->`: the next identifier is a type.
    Type,
    /// After the name of a `struct`: a `{` opens its field list.
    StructName,
    /// After `#`.
    Hash,
    /// After `#[`: the next identifier is the attribute name.
    AttributeOpen,
    /// After the attribute name: `=` introduces a value.
    AttributeName,
    /// After `#[name =`: the next identifier is an attribute value.
    AttributeValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Declaration {
    Fn,
    Let,
    Const,
    Global,
    Struct,
}

/// Classifies tokens one at a time by remembering just enough of what came before.
///
/// This works from the token stream alone, so unlike a parser-based approach it keeps working on
/// programs that do not parse. The trade-off is that it cannot resolve names: every identifier
/// that is not in a recognized declaration, type or attribute position is a plain variable.
struct Classifier {
    context: Context,
    /// Whether the previous tokens are inside the braces of a `struct` declaration, where every
    /// `:` introduces a field's type.
    in_struct_body: bool,
}

impl Classifier {
    fn new() -> Self {
        Self {
            context: Context::None,
            in_struct_body: false,
        }
    }

    /// Classifies the next token, returning its type and modifier bitset if it is highlighted.
    fn classify(&mut self, kind: lex::TokenKind) -> Option<(TokenType, u32)> {
        use lex::TokenKind;

        // Comments can appear anywhere and say nothing about what follows.
        if kind == TokenKind::Comment {
            return Some((TokenType::Comment, 0));
        }

        let previous = std::mem::replace(&mut self.context, Context::None);
        // Items cannot appear inside a struct body, so a declaring keyword means the body was left
        // unclosed (most likely mid-edit) and must not affect the rest of the file.
        if matches!(
            kind,
            TokenKind::Fn
                | TokenKind::Let
                | TokenKind::Const
                | TokenKind::Global
                | TokenKind::Struct
        ) {
            self.in_struct_body = false;
        }
        match kind {
            TokenKind::Fn => self.context = Context::Declaring(Declaration::Fn),
            TokenKind::Let => self.context = Context::Declaring(Declaration::Let),
            TokenKind::Const => self.context = Context::Declaring(Declaration::Const),
            TokenKind::Global => self.context = Context::Declaring(Declaration::Global),
            TokenKind::Struct => self.context = Context::Declaring(Declaration::Struct),
            TokenKind::Colon if previous == Context::DeclaredName || self.in_struct_body => {
                self.context = Context::Type;
            }
            TokenKind::LeftBrace if previous == Context::StructName => self.in_struct_body = true,
            TokenKind::RightBrace => self.in_struct_body = false,
            TokenKind::MinusGreaterThan => self.context = Context::Type,
            TokenKind::Hash => self.context = Context::Hash,
            TokenKind::LeftBracket if previous == Context::Hash => {
                self.context = Context::AttributeOpen;
            }
            TokenKind::Equal if previous == Context::AttributeName => {
                self.context = Context::AttributeValue;
            }
            TokenKind::Ident => return self.classify_ident(previous),
            _ => {}
        }
        classify_static(kind).map(|ty| (ty, 0))
    }

    fn classify_ident(&mut self, previous: Context) -> Option<(TokenType, u32)> {
        let declaration = TokenModifier::Declaration.bit();
        match previous {
            Context::Declaring(declaring) => {
                match declaring {
                    Declaration::Fn => {}
                    Declaration::Struct => self.context = Context::StructName,
                    _ => self.context = Context::DeclaredName,
                }
                Some(match declaring {
                    Declaration::Fn => (TokenType::Function, declaration),
                    Declaration::Struct => (TokenType::Type, declaration),
                    Declaration::Let => (TokenType::Variable, declaration),
                    Declaration::Const => (
                        TokenType::Variable,
                        declaration | TokenModifier::Readonly.bit(),
                    ),
                    Declaration::Global => (
                        TokenType::Variable,
                        declaration | TokenModifier::Static.bit(),
                    ),
                })
            }
            Context::Type => Some((TokenType::Type, 0)),
            Context::AttributeOpen => {
                self.context = Context::AttributeName;
                Some((TokenType::Decorator, 0))
            }
            // Attribute values are not variables, and there is nothing better to call them.
            Context::AttributeValue => None,
            _ => Some((TokenType::Variable, 0)),
        }
    }
}

fn classify_static(kind: lex::TokenKind) -> Option<TokenType> {
    match kind {
        lex::TokenKind::Equal
        | lex::TokenKind::EqualEqual
        | lex::TokenKind::Ampersand
        | lex::TokenKind::AmpersandAmpersand
        | lex::TokenKind::Pipe
        | lex::TokenKind::PipePipe
        | lex::TokenKind::Bang
        | lex::TokenKind::BangEqual
        | lex::TokenKind::Dot
        | lex::TokenKind::Carrot
        | lex::TokenKind::Plus
        | lex::TokenKind::Minus
        | lex::TokenKind::MinusGreaterThan
        | lex::TokenKind::LessThan
        | lex::TokenKind::LessThanLessThan
        | lex::TokenKind::LessThanEqual
        | lex::TokenKind::LessThanPercent
        | lex::TokenKind::GreaterThan
        | lex::TokenKind::GreaterThanGreaterThan
        | lex::TokenKind::GreaterThanEqual
        | lex::TokenKind::GreaterThanPercent
        | lex::TokenKind::PlusEqual
        | lex::TokenKind::MinusEqual
        | lex::TokenKind::AmpersandEqual
        | lex::TokenKind::PipeEqual
        | lex::TokenKind::CarrotEqual => Some(TokenType::Operator),
        lex::TokenKind::Number => Some(TokenType::Number),
        lex::TokenKind::Comment => Some(TokenType::Comment),
        lex::TokenKind::Fn
        | lex::TokenKind::Let
        | lex::TokenKind::Loop
        | lex::TokenKind::Break
        | lex::TokenKind::Yield
        | lex::TokenKind::Return
        | lex::TokenKind::Continue
        | lex::TokenKind::Breakpoint
        | lex::TokenKind::If
        | lex::TokenKind::Then
        | lex::TokenKind::Else
        | lex::TokenKind::Struct
        | lex::TokenKind::Global
        | lex::TokenKind::Const => Some(TokenType::Keyword),
        _ => None,
    }
}

/// Lexes `text` and returns its semantic tokens in the LSP delta encoding.
///
/// Lexing stops at the first lexical error; tokens before it are still returned so highlighting
/// degrades gracefully while the user is mid-edit.
fn semantic_tokens(text: &str) -> Vec<SemanticToken> {
    let line_starts = line_starts(text);
    let mut data = Vec::new();
    let mut prev_line = 0;
    let mut prev_start = 0;
    let mut classifier = Classifier::new();
    for token in lex::Lexer::from(text) {
        let Ok(token) = token else { break };
        let Some((token_type, modifiers)) = classifier.classify(token.kind()) else {
            continue;
        };
        let (line, start) = position(text, &line_starts, token.span().offset());
        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 {
            start - prev_start
        } else {
            start
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length: utf16_len(token.text()),
            token_type: token_type as u32,
            token_modifiers_bitset: modifiers,
        });
        prev_line = line;
        prev_start = start;
    }
    data
}

/// Byte offsets at which each line of `text` begins.
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// Converts a byte offset into a zero-based `(line, UTF-16 column)` pair.
///
/// Offsets past the end of `text`, or inside a character, are moved back to the nearest valid
/// boundary rather than panicking.
fn position(text: &str, line_starts: &[usize], offset: usize) -> (u32, u32) {
    let offset = text.floor_char_boundary(offset);
    let line = line_starts.partition_point(|&start| start <= offset) - 1;
    (
        u32::try_from(line).expect("line number should fit in a u32"),
        utf16_len(&text[line_starts[line]..offset]),
    )
}

fn lsp_position(text: &str, line_starts: &[usize], offset: usize) -> Position {
    let (line, character) = position(text, line_starts, offset);
    Position { line, character }
}

fn utf16_len(s: &str) -> u32 {
    u32::try_from(s.encode_utf16().count()).expect("UTF-16 length should fit in a u32")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(delta_line: u32, delta_start: u32, length: u32, ty: TokenType) -> SemanticToken {
        tok_mod(delta_line, delta_start, length, ty, &[])
    }

    fn tok_mod(
        delta_line: u32,
        delta_start: u32,
        length: u32,
        ty: TokenType,
        modifiers: &[TokenModifier],
    ) -> SemanticToken {
        SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: ty as u32,
            token_modifiers_bitset: modifiers.iter().fold(0, |bits, m| bits | m.bit()),
        }
    }

    /// Classifies `text` into `(source text, type, modifiers)` triples for readable assertions.
    fn classified(text: &str) -> Vec<(&str, TokenType, Vec<TokenModifier>)> {
        let mut classifier = Classifier::new();
        lex::Lexer::from(text)
            .map_while(Result::ok)
            .filter_map(|token| {
                let (ty, bits) = classifier.classify(token.kind())?;
                let modifiers = TokenModifier::ALL
                    .into_iter()
                    .filter(|m| bits & m.bit() != 0)
                    .collect();
                let start = token.span().offset();
                Some((&text[start..start + token.text().len()], ty, modifiers))
            })
            .filter(|(_, ty, _)| {
                matches!(
                    ty,
                    TokenType::Variable
                        | TokenType::Function
                        | TokenType::Type
                        | TokenType::Decorator
                )
            })
            .collect()
    }

    #[test]
    fn same_line_tokens_are_relative_to_previous_start() {
        assert_eq!(
            semantic_tokens("let x = 12;"),
            vec![
                tok(0, 0, 3, TokenType::Keyword),
                tok_mod(0, 4, 1, TokenType::Variable, &[TokenModifier::Declaration]),
                tok(0, 2, 1, TokenType::Operator),
                tok(0, 2, 2, TokenType::Number),
            ]
        );
    }

    #[test]
    fn new_line_resets_start_to_absolute_column() {
        assert_eq!(
            semantic_tokens("let x = 1;\n\n  return 2;"),
            vec![
                tok(0, 0, 3, TokenType::Keyword),
                tok_mod(0, 4, 1, TokenType::Variable, &[TokenModifier::Declaration]),
                tok(0, 2, 1, TokenType::Operator),
                tok(0, 2, 1, TokenType::Number),
                tok(2, 2, 6, TokenType::Keyword),
                tok(0, 7, 1, TokenType::Number),
            ]
        );
    }

    #[test]
    fn fn_name_is_a_function_declaration() {
        use TokenModifier::Declaration;
        assert_eq!(
            classified("fn add() -> u8 { }"),
            vec![
                ("add", TokenType::Function, vec![Declaration]),
                ("u8", TokenType::Type, vec![]),
            ]
        );
    }

    #[test]
    fn struct_name_is_a_type_declaration() {
        use TokenModifier::Declaration;
        assert_eq!(
            classified("struct Point { x: u8 }\nlet p: Point;"),
            vec![
                ("Point", TokenType::Type, vec![Declaration]),
                ("x", TokenType::Variable, vec![]),
                ("u8", TokenType::Type, vec![]),
                ("p", TokenType::Variable, vec![Declaration]),
                ("Point", TokenType::Type, vec![]),
            ]
        );
    }

    #[test]
    fn unclosed_struct_body_does_not_leak_into_later_items() {
        use TokenModifier::Declaration;
        assert_eq!(
            classified("struct S { a: u8,\nfn f() { g = S { a: b }; }"),
            vec![
                ("S", TokenType::Type, vec![Declaration]),
                ("a", TokenType::Variable, vec![]),
                ("u8", TokenType::Type, vec![]),
                ("f", TokenType::Function, vec![Declaration]),
                ("g", TokenType::Variable, vec![]),
                ("S", TokenType::Variable, vec![]),
                ("a", TokenType::Variable, vec![]),
                ("b", TokenType::Variable, vec![]),
            ]
        );
    }

    #[test]
    fn let_const_and_global_declare_variables_with_modifiers() {
        use TokenModifier::*;
        assert_eq!(
            classified("global g: u8; const c: u16 = 1; let l: i8 = 2;"),
            vec![
                ("g", TokenType::Variable, vec![Declaration, Static]),
                ("u8", TokenType::Type, vec![]),
                ("c", TokenType::Variable, vec![Declaration, Readonly]),
                ("u16", TokenType::Type, vec![]),
                ("l", TokenType::Variable, vec![Declaration]),
                ("i8", TokenType::Type, vec![]),
            ]
        );
    }

    #[test]
    fn other_identifiers_are_variables() {
        use TokenModifier::Declaration;
        assert_eq!(
            classified("let a = 1; a = a + b;"),
            vec![
                ("a", TokenType::Variable, vec![Declaration]),
                ("a", TokenType::Variable, vec![]),
                ("a", TokenType::Variable, vec![]),
                ("b", TokenType::Variable, vec![]),
            ]
        );
    }

    #[test]
    fn colon_outside_a_declaration_does_not_start_a_type() {
        assert_eq!(
            classified("x = S { f: y };"),
            vec![
                ("x", TokenType::Variable, vec![]),
                ("S", TokenType::Variable, vec![]),
                ("f", TokenType::Variable, vec![]),
                ("y", TokenType::Variable, vec![]),
            ]
        );
    }

    #[test]
    fn attribute_names_are_decorators() {
        use TokenModifier::Declaration;
        assert_eq!(
            classified("#[entrypoint]\n#[address = 0x0150]\n#[inline = always]\nfn main() {}"),
            vec![
                ("entrypoint", TokenType::Decorator, vec![]),
                ("address", TokenType::Decorator, vec![]),
                ("inline", TokenType::Decorator, vec![]),
                ("main", TokenType::Function, vec![Declaration]),
            ]
        );
    }

    #[test]
    fn comments_do_not_disturb_context() {
        use TokenModifier::Declaration;
        assert_eq!(
            classified("fn // note\n main() {}"),
            vec![("main", TokenType::Function, vec![Declaration])]
        );
    }

    #[test]
    fn context_does_not_leak_past_the_declared_name() {
        use TokenModifier::Declaration;
        assert_eq!(
            classified("fn f() { g(); }"),
            vec![
                ("f", TokenType::Function, vec![Declaration]),
                ("g", TokenType::Variable, vec![]),
            ]
        );
    }

    #[test]
    fn incomplete_declarations_still_classify() {
        use TokenModifier::Declaration;
        // Nothing follows the keyword, or a keyword follows it; neither may panic or leak.
        assert_eq!(classified("let"), vec![]);
        assert_eq!(
            classified("let let x"),
            vec![("x", TokenType::Variable, vec![Declaration])]
        );
    }

    #[test]
    fn positions_use_utf16_code_units() {
        let text = "😀x\ny";
        let starts = line_starts(text);
        // The emoji is 4 UTF-8 bytes but 2 UTF-16 units.
        assert_eq!(position(text, &starts, 4), (0, 2));
        assert_eq!(position(text, &starts, 6), (1, 0));
        assert_eq!(utf16_len("é"), 1);
    }

    #[test]
    fn positions_are_clamped_to_valid_offsets() {
        let text = "é\nb";
        let starts = line_starts(text);
        // Inside the two-byte `é` and past the end of the text.
        assert_eq!(position(text, &starts, 1), (0, 0));
        assert_eq!(position(text, &starts, 100), (1, 1));
    }

    fn test_uri() -> Uri {
        "file:///test.q".parse().unwrap()
    }

    #[test]
    fn valid_source_has_no_diagnostics() {
        assert_eq!(diagnostics(&test_uri(), "fn main() {}\n"), Some(vec![]));
    }

    #[test]
    fn lexer_error_is_reported_at_its_span() {
        let diagnostics = diagnostics(&test_uri(), "fn main() {\n  let x = $;\n}\n").unwrap();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::Error));
        assert_eq!(diagnostic.source.as_deref(), Some("quokka"));
        assert_eq!(
            diagnostic.range,
            Range {
                start: Position {
                    line: 1,
                    character: 10
                },
                end: Position {
                    line: 1,
                    character: 11
                },
            }
        );
    }

    #[test]
    fn analyzer_error_is_reported_at_its_span() {
        let diagnostics = diagnostics(&test_uri(), "fn main() {\n  let x = y;\n}\n").unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].range.start,
            Position {
                line: 1,
                character: 10
            }
        );
        assert_eq!(
            diagnostics[0].range.end,
            Position {
                line: 1,
                character: 11
            }
        );
    }

    #[test]
    fn lexer_error_keeps_earlier_tokens() {
        assert_eq!(
            semantic_tokens("let $"),
            vec![tok(0, 0, 3, TokenType::Keyword)]
        );
    }
}
