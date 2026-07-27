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

    fn open_and_read_diagnostics(&mut self, uri: &str, text: &str) -> Vec<Value> {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": "postgres",
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
                return message
                    .pointer("/params/diagnostics")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
            }
        }
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

#[test]
fn lsp_accepts_placeholders_and_keeps_static_schema_inference() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///placeholder/query.postgres.sql";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "defaultDialect": "postgres",
                "schemas": [{
                    "id": "11111111-1111-4111-8111-111111111111",
                    "database": "public",
                    "source_uri": null,
                    "tables": [{
                        "name": "users",
                        "object_type": "BASE TABLE",
                        "columns": [{
                            "name": "id",
                            "data_type": "integer",
                            "nullable": false,
                            "primary_key": true,
                            "unique": true,
                            "indexed": true,
                            "comment": null,
                            "source_location": null
                        }],
                        "indexes": [],
                        "constraints": [],
                        "comment": null,
                        "source_location": null
                    }],
                    "functions": []
                }],
                "fileDialects": { (uri): "postgres" }
            }
        }),
    );

    let placeholder_values = ["$1", "?", "?1", "@id", ":id", "{{id}}", "${id}", "%(id)s"];
    for (index, placeholder) in placeholder_values.iter().enumerate() {
        let case_uri = format!("file:///placeholder/value-{index}.postgres.sql");
        let sql = format!("SELECT * FROM users WHERE id = {placeholder};");
        let diagnostics = lsp.open_and_read_diagnostics(&case_uri, &sql);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.get("severity").and_then(Value::as_u64) != Some(1)),
            "{placeholder} produced diagnostics: {diagnostics:?}"
        );
    }

    let sql = "SELECT {{column}} FROM {{schema}}.users u WHERE u.id = :id AND ";
    let diagnostics = lsp.open_and_read_diagnostics(uri, sql);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.get("severity").and_then(Value::as_u64) != Some(1)),
        "template query diagnostics: {diagnostics:?}"
    );

    let result = lsp.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": sql.len() },
            "context": { "triggerKind": 1 }
        }),
    );
    let items = result
        .as_array()
        .or_else(|| result.get("items").and_then(Value::as_array))
        .expect("completion item array");
    assert!(
        items
            .iter()
            .any(|item| item.get("label").and_then(Value::as_str) == Some("id")),
        "static users table should still drive completion: {items:?}"
    );
}

#[test]
fn lsp_preserves_postgres_json_operators_and_question_bindings() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///placeholder/postgres-json-operators.postgres.sql";
    let sql = r#"SELECT payload ? 'key',
       payload ?| ARRAY['a', 'b'],
       payload ?& ARRAY['a', 'b'],
       payload ? ?,
       payload #> '{profile,name}',
       payload #>> '{profile,name}',
       payload #- '{obsolete}'
FROM events
WHERE id = ? AND tenant_id = ?1;"#;
    let diagnostics = lsp.open_and_read_diagnostics(uri, sql);

    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.get("severity").and_then(Value::as_u64) != Some(1)),
        "PostgreSQL JSON operators or question-mark binds produced diagnostics: {diagnostics:?}"
    );
}

#[test]
fn lsp_publishes_multiline_non_ascii_diagnostics_in_utf16_columns() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///diagnostics/non-ascii.postgres.sql";
    let invalid_line = "SELECT '😀中文' AS label FROM users WHERE id = )";
    let sql = format!("SELECT 1;\n{invalid_line}");
    let error_byte_column = invalid_line.rfind("= )").expect("error marker");
    let expected_start = invalid_line[..error_byte_column].encode_utf16().count() as u64;
    let expected_end = invalid_line.encode_utf16().count() as u64;

    let diagnostics = lsp.open_and_read_diagnostics(uri, &sql);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains(')'))
        })
        .unwrap_or_else(|| panic!("expected a syntax diagnostic for ')': {diagnostics:?}"));

    assert_eq!(
        diagnostic
            .pointer("/range/start/line")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        diagnostic
            .pointer("/range/start/character")
            .and_then(Value::as_u64),
        Some(expected_start)
    );
    assert_eq!(
        diagnostic
            .pointer("/range/end/line")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        diagnostic
            .pointer("/range/end/character")
            .and_then(Value::as_u64),
        Some(expected_end)
    );
    assert_ne!(expected_start, error_byte_column as u64);
}
