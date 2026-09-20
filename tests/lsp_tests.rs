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
        json!({ "tokenTypes": ["operator", "number", "keyword", "comment"], "tokenModifiers": [] })
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

const BAD_SOURCE: &str = "fn main() {\n  let x = y;\n}\n";
const GOOD_SOURCE: &str = "fn main() {}\n";

fn diagnostics_for<'a>(messages: &'a [Value], uri: &str) -> Vec<&'a Value> {
    messages
        .iter()
        .filter(|m| m["method"] == "textDocument/publishDiagnostics")
        .filter(|m| m["params"]["uri"] == uri)
        .map(|m| &m["params"])
        .collect()
}

fn did_change(uri: &str, version: u32, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [{ "text": text }],
        },
    })
}

fn did_close(uri: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": uri } },
    })
}

#[test]
fn opening_a_document_publishes_diagnostics_with_spans() {
    let uri = "file:///diag/open.q";
    let session = [
        initialize().to_vec(),
        vec![did_open(uri, BAD_SOURCE)],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    let published = diagnostics_for(&messages, uri);
    assert_eq!(published.len(), 1);
    let diagnostics = published[0]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["message"], "name `y` not defined in scope");
    assert_eq!(diagnostics[0]["severity"], 1);
    assert_eq!(diagnostics[0]["source"], "quokka");
    assert_eq!(
        diagnostics[0]["range"],
        json!({
            "start": { "line": 1, "character": 10 },
            "end": { "line": 1, "character": 11 },
        })
    );
}

#[test]
fn opening_a_valid_document_publishes_empty_diagnostics() {
    let uri = "file:///diag/valid.q";
    let session = [
        initialize().to_vec(),
        vec![did_open(uri, GOOD_SOURCE)],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    let published = diagnostics_for(&messages, uri);
    assert_eq!(published.len(), 1);
    assert_eq!(published[0]["diagnostics"], json!([]));
}

#[test]
fn fixing_a_document_clears_stale_diagnostics() {
    let uri = "file:///diag/fix.q";
    let session = [
        initialize().to_vec(),
        vec![
            did_open(uri, BAD_SOURCE),
            did_change(uri, 2, GOOD_SOURCE),
            // Requests settle pending diagnostics before they are answered.
            semantic_tokens_request(2, uri),
        ],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    let published = diagnostics_for(&messages, uri);
    assert_eq!(published.len(), 2);
    assert_eq!(published[0]["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(published[1]["diagnostics"], json!([]));
}

#[test]
fn a_burst_of_changes_is_analyzed_once() {
    let uri = "file:///diag/burst.q";
    let session = [
        initialize().to_vec(),
        vec![
            did_open(uri, GOOD_SOURCE),
            did_change(uri, 2, BAD_SOURCE),
            did_change(uri, 3, GOOD_SOURCE),
            did_change(uri, 4, BAD_SOURCE),
            semantic_tokens_request(2, uri),
        ],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    // One for open, one for the whole burst, reflecting the final text.
    let published = diagnostics_for(&messages, uri);
    assert_eq!(published.len(), 2);
    assert_eq!(published[1]["diagnostics"].as_array().unwrap().len(), 1);
}

#[test]
fn closing_a_document_clears_its_diagnostics() {
    let uri = "file:///diag/close.q";
    let session = [
        initialize().to_vec(),
        vec![did_open(uri, BAD_SOURCE), did_close(uri)],
        shutdown().to_vec(),
    ]
    .concat();
    let messages = run_session(&session);

    let published = diagnostics_for(&messages, uri);
    assert_eq!(published.len(), 2);
    assert_eq!(published[1]["diagnostics"], json!([]));
}

#[test]
fn changes_are_published_after_the_client_goes_quiet() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    let uri = "file:///diag/debounce.q";
    let mut child = Command::new(env!("CARGO_BIN_EXE_quokka"))
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // Reads frames on a thread so a missing message fails the test instead of hanging it.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            let mut header = String::new();
            let mut len = None;
            loop {
                header.clear();
                if stdout.read_line(&mut header).unwrap() == 0 {
                    return;
                }
                match header.trim().strip_prefix("Content-Length: ") {
                    Some(n) => len = Some(n.parse::<usize>().unwrap()),
                    None if header.trim().is_empty() => break,
                    None => {}
                }
            }
            let mut body = vec![0; len.unwrap()];
            stdout.read_exact(&mut body).unwrap();
            if tx
                .send(serde_json::from_slice::<Value>(&body).unwrap())
                .is_err()
            {
                return;
            }
        }
    });

    let mut send = |message: Value| {
        stdin.write_all(frame(&message).as_bytes()).unwrap();
        stdin.flush().unwrap();
    };
    let wait_for_diagnostics = || {
        loop {
            let message = rx
                .recv_timeout(Duration::from_secs(10))
                .expect("timed out waiting for diagnostics");
            if message["method"] == "textDocument/publishDiagnostics" {
                return message["params"].clone();
            }
        }
    };

    let [init, initialized] = initialize();
    send(init);
    send(initialized);
    send(did_open(uri, GOOD_SOURCE));
    assert_eq!(wait_for_diagnostics()["diagnostics"], json!([]));

    // No further requests: the only thing that can trigger this is the debounce timer.
    send(did_change(uri, 2, BAD_SOURCE));
    assert_eq!(
        wait_for_diagnostics()["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let [shut, exit] = shutdown();
    send(shut);
    send(exit);
    assert!(child.wait().unwrap().success());
}
