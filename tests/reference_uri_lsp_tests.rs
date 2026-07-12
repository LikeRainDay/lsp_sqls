use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct LspProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl LspProcess {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sql-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sql-lsp test binary");
        let stdin = child.stdin.take().expect("sql-lsp stdin");
        let stdout = BufReader::new(child.stdout.take().expect("sql-lsp stdout"));
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn initialize(&mut self) {
        self.request(
            "initialize",
            json!({ "processId": null, "rootUri": null, "capabilities": {} }),
        );
        self.notify("initialized", json!({}));
    }

    fn open(&mut self, uri: &str, language_id: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        );

        loop {
            let message = self.read_message();
            if message.get("method").and_then(Value::as_str)
                == Some("textDocument/publishDiagnostics")
                && message.pointer("/params/uri").and_then(Value::as_str) == Some(uri)
            {
                break;
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));

        loop {
            let message = self.read_message();
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                assert!(
                    message.get("error").is_none(),
                    "LSP request {method} failed: {message}"
                );
                return message.get("result").cloned().unwrap_or(Value::Null);
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }));
    }

    fn write_message(&mut self, message: &Value) {
        let body = serde_json::to_vec(message).expect("serialize LSP message");
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len()).expect("write LSP header");
        self.stdin.write_all(&body).expect("write LSP body");
        self.stdin.flush().expect("flush LSP body");
    }

    fn read_message(&mut self) -> Value {
        let mut content_length = None;
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).expect("read LSP header") > 0,
                "sql-lsp stdout closed"
            );
            let header = line.trim_end_matches(['\r', '\n']);
            if header.is_empty() {
                break;
            }
            if let Some(value) = header.strip_prefix("Content-Length:") {
                content_length = Some(value.trim().parse::<usize>().expect("content length"));
            }
        }

        let mut body = vec![0; content_length.expect("Content-Length header")];
        self.stdout.read_exact(&mut body).expect("read LSP body");
        serde_json::from_slice(&body).expect("parse LSP body")
    }
}

impl Drop for LspProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn utf16_column(text: &str, byte_offset: usize) -> u64 {
    text[..byte_offset].encode_utf16().count() as u64
}

fn reference_starts(result: &Value, expected_uri: &str) -> Vec<u64> {
    let locations = result.as_array().expect("reference location array");
    assert!(!locations.is_empty(), "expected at least one reference");
    assert!(locations
        .iter()
        .all(|location| { location.get("uri").and_then(Value::as_str) == Some(expected_uri) }));

    let mut starts = locations
        .iter()
        .map(|location| {
            location
                .pointer("/range/start/character")
                .and_then(Value::as_u64)
                .expect("reference start character")
        })
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    starts
}

#[test]
fn sql_references_use_the_requested_uri_and_utf16_ranges() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///workspace/%E6%9F%A5%E8%AF%A2.postgres.sql";
    let sql = "SELECT customer_id, '😀中文' FROM orders WHERE customer_id = 1";
    lsp.open(uri, "postgres", sql);

    let first_identifier = sql.find("customer_id").unwrap();
    let result = lsp.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": 0,
                "character": utf16_column(sql, first_identifier + 2),
            },
            "context": { "includeDeclaration": true },
        }),
    );

    let expected = sql
        .match_indices("customer_id")
        .map(|(offset, _)| utf16_column(sql, offset))
        .collect::<Vec<_>>();
    assert_eq!(reference_starts(&result, uri), expected);
}

#[test]
fn json_references_use_the_requested_uri_and_utf16_ranges() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///workspace/%E6%9F%A5%E8%AF%A2.mongo.json";
    let json = r#"{"customer_id": 1, "标签": "😀", "nested": {"customer_id": 2}}"#;
    lsp.open(uri, "mongodb", json);

    let first_identifier = json.find("customer_id").unwrap();
    let result = lsp.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": 0,
                "character": utf16_column(json, first_identifier + 2),
            },
            "context": { "includeDeclaration": true },
        }),
    );

    let expected = json
        .match_indices("customer_id")
        .map(|(offset, _)| utf16_column(json, offset))
        .collect::<Vec<_>>();
    assert_eq!(reference_starts(&result, uri), expected);
}

#[test]
fn goto_definition_preserves_a_schema_source_uri() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///workspace/query.mysql.sql";
    let schema_id = "11111111-1111-4111-8111-111111111111";
    let source_uri = "file:///schemas/%E5%AE%A2%E6%88%B7.mysql.sql";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "app",
                    "tables": [{
                        "name": "users",
                        "columns": [],
                        "indexes": [],
                        "constraints": [],
                        "comment": null,
                        "source_location": [source_uri, 7]
                    }],
                    "functions": [],
                    "source_uri": "file:///schemas/fallback.mysql.sql"
                }],
                "fileSchemas": { (uri): schema_id },
                "fileDialects": { (uri): "mysql" }
            }
        }),
    );

    let sql = "SELECT * FROM users";
    lsp.open(uri, "mysql", sql);
    let table_offset = sql.find("users").unwrap();
    let result = lsp.request(
        "textDocument/definition",
        json!({
            "textDocument": { "uri": uri },
            "position": {
                "line": 0,
                "character": utf16_column(sql, table_offset + 2),
            }
        }),
    );

    assert_eq!(result.get("uri").and_then(Value::as_str), Some(source_uri));
    assert_eq!(
        result.pointer("/range/start/line").and_then(Value::as_u64),
        Some(6)
    );
}

#[test]
fn inline_completion_context_combines_ast_schema_and_validation() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();
    let uri = "file:///workspace/query.postgres.sql";
    let schema_id = "22222222-2222-4222-8222-222222222222";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "app",
                    "tables": [{
                        "name": "users",
                        "columns": [{
                            "name": "email",
                            "data_type": "text",
                            "nullable": false,
                            "primary_key": false,
                            "unique": false,
                            "indexed": false,
                            "comment": null,
                            "source_location": null
                        }],
                        "indexes": [],
                        "constraints": [],
                        "comment": null,
                        "source_location": null
                    }],
                    "functions": [],
                    "source_uri": null
                }],
                "fileSchemas": { (uri): schema_id },
                "fileDialects": { (uri): "postgres" }
            }
        }),
    );
    let sql = "WITH active AS (SELECT * FROM users) SELECT * FROM users u WHERE u.";
    lsp.open(uri, "postgres", sql);

    let context = lsp.request(
        "oxide/inlineCompletionContext",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": utf16_column(sql, sql.len()) }
        }),
    );
    assert_eq!(
        context.get("dialect").and_then(Value::as_str),
        Some("postgres")
    );
    assert_eq!(
        context.get("clause").and_then(Value::as_str),
        Some("TableColumn")
    );
    assert!(context.get("statementRange").is_some_and(Value::is_object));
    assert!(context
        .get("ctes")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("active"))));
    assert!(
        context
            .get("candidates")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| { item.get("label").and_then(Value::as_str) == Some("email") })),
        "{context}"
    );

    let diagnostics = lsp.request(
        "oxide/validateInlineCompletion",
        json!({
            "textDocument": { "uri": uri },
            "text": "SELECT * FROM users WHERE id = ) AND active = true"
        }),
    );
    assert!(diagnostics.as_array().is_some_and(|items| items
        .iter()
        .any(|item| { item.get("severity").and_then(Value::as_u64) == Some(1) })));
}
