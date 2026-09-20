use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};

/// Frames a JSON-RPC message with the LSP `Content-Length` header.
fn frame(message: &Value) -> String {
    let body = message.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

/// Splits the server's stdout back into its JSON-RPC messages.
fn parse_frames(mut output: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    while let Some((header, rest)) = output.split_once("\r\n\r\n") {
        let len: usize = header
            .strip_prefix("Content-Length: ")
            .expect("frame should start with a Content-Length header")
            .parse()
            .expect("Content-Length should be a number");
        let (body, rest) = rest.split_at(len);
        messages.push(serde_json::from_str(body).expect("frame body should be JSON"));
        output = rest;
    }
    assert!(output.is_empty(), "trailing non-frame output: {output:?}");
    messages
}

/// Runs a whole client session against `quokka lsp` and returns everything the server wrote.
///
/// The session is written up front and stdin is then closed; the server processes messages in
/// order, so this behaves like a client that never waits on a response.
fn run_session(messages: &[Value]) -> Vec<Value> {
    let stdin = messages.iter().map(frame).collect::<String>();
    let output = cargo_bin_cmd!("quokka")
        .arg("lsp")
        .write_stdin(stdin)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    parse_frames(std::str::from_utf8(&output).expect("server output should be UTF-8"))
}

fn initialize() -> [Value; 2] {
    [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "processId": null, "rootUri": null, "capabilities": {} },
        }),
        json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    ]
}

fn shutdown() -> [Value; 2] {
    [
        json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }),
        json!({ "jsonrpc": "2.0", "method": "exit" }),
    ]
}

fn did_open(uri: &str, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": { "uri": uri, "languageId": "quokka", "version": 1, "text": text },
        },
    })
}

fn semantic_tokens_request(id: u32, uri: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": uri } },
    })
}

fn response(messages: &[Value], id: u32) -> &Value {
    messages
        .iter()
        .find(|m| m["id"] == json!(id))
        .unwrap_or_else(|| panic!("no response with id {id} in {messages:#?}"))
}

#[test]
fn initialize_advertises_semantic_tokens_legend() {
    let session = [initialize().to_vec(), shutdown().to_vec()].concat();
    let messages = run_session(&session);

    let capabilities = &response(&messages, 1)["result"]["capabilities"];
    assert_eq!(
        capabilities["semanticTokensProvider"]["legend"],
        json!({ "tokenTypes": ["operator", "number", "keyword"], "tokenModifiers": [] })
    );
    assert_eq!(capabilities["semanticTokensProvider"]["full"], json!(true));
    assert_eq!(capabilities["textDocumentSync"], json!(1));
}

#[test]
fn semantic_tokens_are_delta_encoded_from_open_buffer() {
    let uri = "file:///does/not/exist.qk";
    let session = [
        initialize().to_vec(),
        vec![
            did_open(uri, "let x = 12;\n  return 3;"),
            semantic_tokens_request(2, uri),
        ],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    assert_eq!(
        response(&messages, 2)["result"]["data"],
        json!([
            // [delta_line, delta_start, length, type, modifiers]
            0, 0, 3, 2, 0, // `let`
            0, 6, 1, 0, 0, // `=`
            0, 2, 2, 1, 0, // `12`
            1, 2, 6, 2, 0, // `return`, absolute column on a new line
            0, 7, 1, 1, 0, // `3`
        ])
    );
}

#[test]
fn semantic_tokens_reflect_full_sync_changes() {
    let uri = "file:///does/not/exist.qk";
    let session = [
        initialize().to_vec(),
        vec![
            did_open(uri, "let"),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": uri, "version": 2 },
                    "contentChanges": [{ "text": "\n\n7" }],
                },
            }),
            semantic_tokens_request(2, uri),
        ],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    assert_eq!(
        response(&messages, 2)["result"]["data"],
        json!([2, 0, 1, 1, 0])
    );
}

#[test]
fn semantic_tokens_for_unopened_document_is_null() {
    let session = [
        initialize().to_vec(),
        vec![semantic_tokens_request(2, "file:///never/opened.qk")],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    assert_eq!(response(&messages, 2)["result"], Value::Null);
}

#[test]
fn malformed_request_params_do_not_kill_the_server() {
    let session = [
        initialize().to_vec(),
        vec![json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/semanticTokens/full",
            "params": {},
        })],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    // InvalidParams
    assert_eq!(response(&messages, 2)["error"]["code"], json!(-32602));
    assert!(response(&messages, 99).get("result").is_some());
}
