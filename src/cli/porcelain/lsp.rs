use crate::cli::Execute;
use lsp_server::{
    Connection, ErrorCode, Message, Notification, Request, RequestId, Response, ResponseError,
};
use lsp_types::{
    Code, Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Full, Location,
    LspNotificationMethod, LspRequestMethod, Position, PublishDiagnosticsParams, Range,
    SemanticToken, SemanticTokenTypes, SemanticTokens, SemanticTokensLegend, SemanticTokensOptions,
    ServerCapabilities, TextDocumentContentChangeEvent, TextDocumentSyncKind, Uri,
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
}

impl TokenType {
    const ALL: [TokenType; 3] = [TokenType::Operator, TokenType::Number, TokenType::Keyword];

    fn lsp_type(self) -> SemanticTokenTypes {
        match self {
            TokenType::Operator => SemanticTokenTypes::Operator,
            TokenType::Number => SemanticTokenTypes::Number,
            TokenType::Keyword => SemanticTokenTypes::Keyword,
        }
    }

    fn legend() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: Self::ALL
                .iter()
                .map(|ty| ty.lsp_type().as_str().to_owned())
                .collect(),
            token_modifiers: Vec::new(),
        }
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
                    legend: TokenType::legend(),
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

fn classify(kind: lex::TokenKind) -> Option<TokenType> {
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
        lex::TokenKind::Fn
        | lex::TokenKind::Let
        | lex::TokenKind::Loop
        | lex::TokenKind::Break
        | lex::TokenKind::Yield
        | lex::TokenKind::Return
        | lex::TokenKind::Continue
        | lex::TokenKind::If
        | lex::TokenKind::Then
        | lex::TokenKind::Else
        | lex::TokenKind::Struct
        | lex::TokenKind::Global
        | lex::TokenKind::Const
        | lex::TokenKind::Asm => Some(TokenType::Keyword),
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
    for token in lex::Lexer::from(text) {
        let Ok(token) = token else { break };
        let Some(token_type) = classify(token.kind()) else {
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
            token_modifiers_bitset: 0,
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
    (line as u32, utf16_len(&text[line_starts[line]..offset]))
}

fn lsp_position(text: &str, line_starts: &[usize], offset: usize) -> Position {
    let (line, character) = position(text, line_starts, offset);
    Position { line, character }
}

fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(delta_line: u32, delta_start: u32, length: u32, ty: TokenType) -> SemanticToken {
        SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: ty as u32,
            token_modifiers_bitset: 0,
        }
    }

    #[test]
    fn same_line_tokens_are_relative_to_previous_start() {
        assert_eq!(
            semantic_tokens("let x = 12;"),
            vec![
                tok(0, 0, 3, TokenType::Keyword),
                tok(0, 6, 1, TokenType::Operator),
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
                tok(0, 6, 1, TokenType::Operator),
                tok(0, 2, 1, TokenType::Number),
                tok(2, 2, 6, TokenType::Keyword),
                tok(0, 7, 1, TokenType::Number),
            ]
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
