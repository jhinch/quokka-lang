use crate::cli::Execute;
use lsp_server::{
    Connection, ErrorCode, Message, Notification, Request, RequestId, Response, ResponseError,
};
use lsp_types::{
    Full, LspNotificationMethod, LspRequestMethod, SemanticToken, SemanticTokenTypes,
    SemanticTokens, SemanticTokensLegend, SemanticTokensOptions, ServerCapabilities,
    TextDocumentContentChangeEvent, TextDocumentSyncKind, Uri,
};
use quokka_lang::lex;
use std::collections::HashMap;

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
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection
                    .handle_shutdown(&req)
                    .map_err(miette::Report::from_err)?
                {
                    break;
                }
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
        _connection: &Connection,
        params: Self::Params,
    ) -> miette::Result<()> {
        let doc = params.text_document;
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
            state
                .documents
                .insert(params.text_document.text_document_identifier.uri, text);
        }
        Ok(())
    }
}

impl NotificationHandler for lsp_types::DidCloseTextDocumentNotification {
    fn handle(
        state: &mut State,
        _connection: &Connection,
        params: Self::Params,
    ) -> miette::Result<()> {
        state.documents.remove(&params.text_document.uri);
        Ok(())
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
fn position(text: &str, line_starts: &[usize], offset: usize) -> (u32, u32) {
    let line = line_starts.partition_point(|&start| start <= offset) - 1;
    (line as u32, utf16_len(&text[line_starts[line]..offset]))
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
    fn lexer_error_keeps_earlier_tokens() {
        assert_eq!(
            semantic_tokens("let $"),
            vec![tok(0, 0, 3, TokenType::Keyword)]
        );
    }
}
