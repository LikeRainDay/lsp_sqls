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

    fn initialize(&mut self) -> Value {
        self.initialize_with_capabilities(json!({}))
    }

    fn initialize_with_capabilities(&mut self, capabilities: Value) -> Value {
        let result = self.request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": null,
                "capabilities": capabilities
            }),
        );
        self.notify("initialized", json!({}));
        result
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
fn sql_references_include_open_documents_in_the_same_schema() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let first_uri = "file:///workspace/first.postgres.sql";
    let second_uri = "file:///workspace/second.postgres.sql";
    let schema_id = "33333333-3333-4333-8333-333333333333";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "public",
                    "tables": [{
                        "name": "orders",
                        "columns": [],
                        "indexes": [],
                        "constraints": [],
                        "comment": null,
                        "source_location": null
                    }],
                    "functions": [],
                    "source_uri": null
                }],
                "fileSchemas": {
                    (first_uri): schema_id,
                    (second_uri): schema_id
                },
                "fileDialects": {
                    (first_uri): "postgres",
                    (second_uri): "postgres"
                }
            }
        }),
    );
    let first_sql = "SELECT * FROM orders";
    let second_sql = "DELETE FROM orders WHERE id = 1";
    lsp.open(first_uri, "postgres", first_sql);
    lsp.open(second_uri, "postgres", second_sql);

    let offset = first_sql.find("orders").unwrap() + 2;
    let result = lsp.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": first_uri },
            "position": {
                "line": 0,
                "character": utf16_column(first_sql, offset),
            },
            "context": { "includeDeclaration": true },
        }),
    );
    let uris = result
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|location| location.get("uri").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(uris.contains(&first_uri), "{result}");
    assert!(uris.contains(&second_uri), "{result}");
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
fn postgres_definition_preserves_database_object_uri() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///workspace/query.postgres.sql";
    let schema_id = "44444444-4444-4444-8444-444444444444";
    let object_uri = "oxide://database-object/object?connection=main&schema=public&object=users";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "public",
                    "tables": [{
                        "name": "users",
                        "columns": [],
                        "indexes": [],
                        "constraints": [],
                        "comment": null,
                        "source_location": [object_uri, 1]
                    }],
                    "functions": [],
                    "source_uri": null
                }],
                "fileSchemas": { (uri): schema_id },
                "fileDialects": { (uri): "postgres" }
            }
        }),
    );
    let sql = "SELECT * FROM users";
    lsp.open(uri, "postgres", sql);
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

    assert_eq!(result.get("uri").and_then(Value::as_str), Some(object_uri));
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

#[test]
fn completion_documentation_resolves_lazily_for_capable_clients() {
    let mut lsp = LspProcess::spawn();
    let initialized = lsp.initialize_with_capabilities(json!({
        "textDocument": {
            "completion": {
                "completionItem": {
                    "resolveSupport": {
                        "properties": ["documentation", "detail", "additionalTextEdits"]
                    }
                }
            }
        }
    }));
    assert_eq!(
        initialized
            .pointer("/capabilities/completionProvider/resolveProvider")
            .and_then(Value::as_bool),
        Some(true),
        "{initialized}"
    );

    let uri = "file:///workspace/resolve.postgres.sql";
    let schema_id = "45454545-4545-4545-8545-454545454545";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "app",
                    "tables": [{
                        "name": "orders",
                        "columns": [{
                            "name": "id",
                            "data_type": "bigint",
                            "nullable": false,
                            "primary_key": true,
                            "unique": true,
                            "indexed": true,
                            "comment": "Order identifier",
                            "source_location": null
                        }],
                        "indexes": [],
                        "constraints": [],
                        "comment": "Customer orders",
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
    lsp.open(uri, "postgres", "SELECT * FROM ");

    let completion = lsp.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 14 }
        }),
    );
    let items = completion
        .as_array()
        .or_else(|| completion.get("items").and_then(Value::as_array))
        .expect("completion items");
    let item = items
        .iter()
        .find(|item| item.get("label").and_then(Value::as_str) == Some("app.orders"))
        .cloned()
        .unwrap_or_else(|| panic!("orders completion missing: {completion}"));
    assert!(item.get("documentation").is_none(), "{item}");
    assert!(
        item.pointer("/data/oxideSqlLspCompletionResolveId")
            .and_then(Value::as_u64)
            .is_some(),
        "{item}"
    );

    let resolved = lsp.request("completionItem/resolve", item.clone());
    assert!(
        resolved
            .get("documentation")
            .and_then(Value::as_str)
            .is_some_and(|documentation| documentation.contains("Customer orders")),
        "{resolved}"
    );
    for field in ["label", "sortText", "insertText", "textEdit"] {
        assert_eq!(resolved.get(field), item.get(field), "changed {field}");
    }
    assert!(resolved.get("data").is_none(), "{resolved}");
}

#[test]
fn advanced_editor_capabilities_are_advertised_and_operational() {
    let mut lsp = LspProcess::spawn();
    let initialized = lsp.initialize();
    for capability in [
        "signatureHelpProvider",
        "documentSymbolProvider",
        "workspaceSymbolProvider",
        "codeActionProvider",
        "documentRangeFormattingProvider",
        "renameProvider",
        "foldingRangeProvider",
        "selectionRangeProvider",
        "semanticTokensProvider",
        "inlayHintProvider",
        "diagnosticProvider",
    ] {
        assert!(
            initialized
                .pointer(&format!("/capabilities/{capability}"))
                .is_some(),
            "missing {capability}: {initialized}"
        );
    }

    let uri = "file:///workspace/advanced.postgres.sql";
    let schema_id = "55555555-5555-4555-8555-555555555555";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "app",
                    "tables": [{
                        "name": "orders",
                        "columns": [{
                            "name": "amount",
                            "data_type": "numeric",
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
                    "functions": [
                        {
                            "name": "calculate",
                            "parameters": [{
                                "name": "value",
                                "data_type": "numeric",
                                "optional": false
                            }],
                            "return_type": "numeric",
                            "description": "Calculate a numeric result"
                        },
                        {
                            "name": "calculate",
                            "parameters": [
                                {
                                    "name": "value",
                                    "data_type": "integer",
                                    "optional": false
                                },
                                {
                                    "name": "precision",
                                    "data_type": "integer",
                                    "optional": true
                                }
                            ],
                            "return_type": "integer",
                            "description": "Calculate an integer result"
                        }
                    ],
                    "source_uri": null
                }],
                "fileSchemas": { (uri): schema_id },
                "fileDialects": { (uri): "postgres" }
            }
        }),
    );
    let sql = concat!(
        "SELECT * FROM orders;\n",
        "SELECT calculate(\n  amount,\n  2\n) FROM orders;\n",
        "UPDATE orders SET amount = 1;\n",
        "INSERT INTO orders VALUES (3);"
    );
    lsp.open(uri, "postgres", sql);

    let call_line = "  amount,";
    let signature = lsp.request(
        "textDocument/signatureHelp",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": call_line.len() }
        }),
    );
    assert_eq!(
        signature
            .get("signatures")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "{signature}"
    );
    assert_eq!(
        signature.get("activeSignature").and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        signature.get("activeParameter").and_then(Value::as_u64),
        Some(1)
    );
    assert!(
        signature["signatures"][0]["label"]
            .as_str()
            .is_some_and(|label| label.contains("precision")),
        "the overload accepting the active parameter should be ranked first: {signature}"
    );

    let semantic_tokens = lsp.request(
        "textDocument/semanticTokens/full",
        json!({ "textDocument": { "uri": uri } }),
    );
    let semantic_data = semantic_tokens
        .get("data")
        .and_then(Value::as_array)
        .expect("semantic token data");
    assert!(!semantic_data.is_empty(), "{semantic_tokens}");
    assert_eq!(semantic_data.len() % 5, 0);
    assert!(semantic_data
        .chunks(5)
        .any(|token| token.get(3).and_then(Value::as_u64) == Some(3)));

    let hints = lsp.request(
        "textDocument/inlayHint",
        json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 6, "character": 30 }
            }
        }),
    );
    assert!(
        hints.as_array().is_some_and(|items| items
            .iter()
            .any(|hint| { hint.get("label").and_then(Value::as_str) == Some("value:") })),
        "{hints}"
    );
    assert!(
        hints.as_array().is_some_and(|items| items
            .iter()
            .any(|hint| { hint.get("label").and_then(Value::as_str) == Some("amount:") })),
        "implicit INSERT columns should be resolved through synchronized metadata: {hints}"
    );

    let symbols = lsp.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert!(symbols.as_array().is_some_and(|items| !items.is_empty()));

    let workspace_symbols = lsp.request("workspace/symbol", json!({ "query": "amount" }));
    assert!(workspace_symbols.as_array().is_some_and(|items| items
        .iter()
        .any(|symbol| symbol.get("name").and_then(Value::as_str) == Some("amount"))));

    let folds = lsp.request(
        "textDocument/foldingRange",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert!(folds.as_array().is_some_and(|items| !items.is_empty()));

    let selections = lsp.request(
        "textDocument/selectionRange",
        json!({
            "textDocument": { "uri": uri },
            "positions": [{ "line": 2, "character": 3 }]
        }),
    );
    assert!(selections
        .pointer("/0/parent/parent/range")
        .is_some_and(Value::is_object));

    let formatted = lsp.request(
        "textDocument/rangeFormatting",
        json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 20 }
            },
            "options": { "tabSize": 2, "insertSpaces": true }
        }),
    );
    assert!(formatted
        .pointer("/0/newText")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("SELECT")));

    let diagnostic_report = lsp.request(
        "textDocument/diagnostic",
        json!({
            "textDocument": { "uri": uri },
            "identifier": "sql-lsp"
        }),
    );
    let diagnostics = diagnostic_report
        .get("items")
        .and_then(Value::as_array)
        .expect("pull diagnostics");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.get("code").and_then(Value::as_str) == Some("OXIDE001")
        }),
        "{diagnostic_report}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.get("code").and_then(Value::as_str) == Some("OXIDE003")
        }),
        "{diagnostic_report}"
    );

    let actions = lsp.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 7 },
                "end": { "line": 0, "character": 8 }
            },
            "context": { "diagnostics": diagnostics }
        }),
    );
    let titles = actions
        .as_array()
        .expect("code actions")
        .iter()
        .filter_map(|action| action.get("title").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(
        titles.iter().any(|title| title.starts_with("Expand *")),
        "{actions}"
    );
    assert!(
        titles.contains(&"Add non-matching WHERE safety guard"),
        "{actions}"
    );

    let qualify_actions = lsp.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 2, "character": 8 },
                "end": { "line": 2, "character": 8 }
            },
            "context": {
                "diagnostics": [],
                "only": ["refactor.rewrite"]
            }
        }),
    );
    assert!(
        qualify_actions
            .as_array()
            .is_some_and(|items| items.iter().any(|action| {
                action
                    .get("title")
                    .and_then(Value::as_str)
                    .is_some_and(|title| title.starts_with("Qualify column as orders.amount"))
            })),
        "{qualify_actions}"
    );
}

#[test]
fn missing_join_condition_code_action_uses_metadata_and_product_quoting() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///workspace/join-fix.sqlserver.sql";
    let schema_id = "77777777-7777-4777-8777-777777777777";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "app",
                    "tables": [
                        {
                            "name": "orders",
                            "columns": [
                                {
                                    "name": "Tenant ID",
                                    "data_type": "bigint",
                                    "nullable": false,
                                    "primary_key": false,
                                    "unique": false,
                                    "indexed": true,
                                    "comment": null,
                                    "source_location": null
                                },
                                {
                                    "name": "Customer ID",
                                    "data_type": "bigint",
                                    "nullable": false,
                                    "primary_key": false,
                                    "unique": false,
                                    "indexed": true,
                                    "comment": null,
                                    "source_location": null
                                }
                            ],
                            "indexes": [],
                            "constraints": [{
                                "name": "orders_customer_fk",
                                "constraint_type": "FOREIGN KEY",
                                "columns": ["Tenant ID", "Customer ID"],
                                "referenced_schema": "app",
                                "referenced_table": "customers",
                                "referenced_columns": ["Tenant ID", "Customer ID"],
                                "definition": null
                            }],
                            "comment": null,
                            "source_location": null
                        },
                        {
                            "name": "customers",
                            "columns": [
                                {
                                    "name": "Tenant ID",
                                    "data_type": "bigint",
                                    "nullable": false,
                                    "primary_key": true,
                                    "unique": true,
                                    "indexed": true,
                                    "comment": null,
                                    "source_location": null
                                },
                                {
                                    "name": "Customer ID",
                                    "data_type": "bigint",
                                    "nullable": false,
                                    "primary_key": true,
                                    "unique": true,
                                    "indexed": true,
                                    "comment": null,
                                    "source_location": null
                                }
                            ],
                            "indexes": [],
                            "constraints": [],
                            "comment": null,
                            "source_location": null
                        }
                    ],
                    "functions": [],
                    "source_uri": null
                }],
                "fileSchemas": { (uri): schema_id },
                "fileDialects": { (uri): "sqlserver" }
            }
        }),
    );
    let sql = "SELECT o.[Customer ID] FROM app.orders o JOIN app.customers c;";
    lsp.open(uri, "sqlserver", sql);

    let diagnostic_report = lsp.request(
        "textDocument/diagnostic",
        json!({
            "textDocument": { "uri": uri },
            "identifier": "sql-lsp"
        }),
    );
    let diagnostic = diagnostic_report
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|diagnostic| {
                diagnostic.get("code").and_then(Value::as_str) == Some("OXIDE002")
            })
        })
        .cloned()
        .expect("missing JOIN diagnostic");
    let join_start = sql.find("JOIN").unwrap();
    let actions = lsp.request(
        "textDocument/codeAction",
        json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": join_start },
                "end": { "line": 0, "character": join_start + 4 }
            },
            "context": {
                "diagnostics": [diagnostic],
                "only": ["quickfix"]
            }
        }),
    );
    let inferred = actions
        .as_array()
        .and_then(|items| {
            items.iter().find(|action| {
                action.get("title").and_then(Value::as_str)
                    == Some("Add JOIN condition via orders_customer_fk")
            })
        })
        .expect("foreign-key JOIN quick fix");
    assert_eq!(
        inferred.get("isPreferred").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        inferred
            .pointer("/edit/changes/file:~1~1~1workspace~1join-fix.sqlserver.sql/0/newText")
            .and_then(Value::as_str),
        Some(" ON o.[Tenant ID] = c.[Tenant ID] AND o.[Customer ID] = c.[Customer ID]"),
        "{inferred}"
    );
}

#[test]
fn signature_help_falls_back_to_dialect_builtins_without_schema_metadata() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///workspace/builtin.sqlserver.sql";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "fileDialects": { (uri): "sqlserver" }
            }
        }),
    );
    let sql = "SELECT JSON_VALUE(payload, ";
    lsp.open(uri, "sql", sql);

    let completion = lsp.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": sql.len() },
            "context": { "triggerKind": 1 }
        }),
    );
    assert!(
        completion
            .as_array()
            .is_some_and(|items| items.iter().any(|item| {
                item.get("label").and_then(Value::as_str) == Some("JSON_VALUE")
                    && item.get("insertText").and_then(Value::as_str)
                        == Some("JSON_VALUE(${1:expression}, ${2:path})")
                    && item.get("insertTextFormat").and_then(Value::as_u64) == Some(2)
            })),
        "{completion}"
    );

    let signature = lsp.request(
        "textDocument/signatureHelp",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": sql.len() }
        }),
    );
    assert_eq!(
        signature["signatures"][0]["label"].as_str(),
        Some("JSON_VALUE(expression, path)")
    );
    assert_eq!(
        signature.get("activeParameter").and_then(Value::as_u64),
        Some(1)
    );
}

#[test]
fn oracle_system_values_complete_without_function_parentheses() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///workspace/system-value.oracle.sql";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "fileDialects": { (uri): "oracle" }
            }
        }),
    );
    let sql = "SELECT sys";
    lsp.open(uri, "sql", sql);

    let completion = lsp.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": sql.len() },
            "context": { "triggerKind": 1 }
        }),
    );
    let sysdate = completion
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("label").and_then(Value::as_str) == Some("SYSDATE"))
        })
        .expect("Oracle SYSDATE completion");
    assert_eq!(sysdate.get("kind").and_then(Value::as_u64), Some(12));
    assert_eq!(
        sysdate.get("insertText").and_then(Value::as_str),
        Some("SYSDATE")
    );
    assert_eq!(
        sysdate.get("insertTextFormat").and_then(Value::as_u64),
        Some(1)
    );
}

#[test]
fn semantic_rename_updates_open_documents_in_the_same_schema() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();
    let first_uri = "file:///workspace/rename-one.postgres.sql";
    let second_uri = "file:///workspace/rename-two.postgres.sql";
    let schema_id = "66666666-6666-4666-8666-666666666666";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "app",
                    "tables": [{
                        "name": "orders",
                        "columns": [],
                        "indexes": [],
                        "constraints": [],
                        "comment": null,
                        "source_location": null
                    }],
                    "functions": [],
                    "source_uri": null
                }],
                "fileSchemas": {
                    (first_uri): schema_id,
                    (second_uri): schema_id
                },
                "fileDialects": {
                    (first_uri): "postgres",
                    (second_uri): "postgres"
                }
            }
        }),
    );
    let first_sql = "SELECT * FROM orders";
    let second_sql = "DELETE FROM orders WHERE id = 1";
    lsp.open(first_uri, "postgres", first_sql);
    lsp.open(second_uri, "postgres", second_sql);
    let offset = first_sql.find("orders").unwrap() + 2;

    let prepared = lsp.request(
        "textDocument/prepareRename",
        json!({
            "textDocument": { "uri": first_uri },
            "position": {
                "line": 0,
                "character": utf16_column(first_sql, offset)
            }
        }),
    );
    assert!(prepared.get("start").is_some(), "{prepared}");

    let edit = lsp.request(
        "textDocument/rename",
        json!({
            "textDocument": { "uri": first_uri },
            "position": {
                "line": 0,
                "character": utf16_column(first_sql, offset)
            },
            "newName": "archived_orders"
        }),
    );
    assert!(
        edit.pointer(&format!(
            "/changes/{}/0/newText",
            first_uri.replace('/', "~1")
        ))
        .is_some(),
        "{edit}"
    );
    assert!(
        edit.pointer(&format!(
            "/changes/{}/0/newText",
            second_uri.replace('/', "~1")
        ))
        .is_some(),
        "{edit}"
    );
}

#[test]
fn project_sql_index_drives_definition_symbols_references_and_safe_rename() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();
    let definition_uri = "oxide://project/project-a/migrations/001_views.postgres.sql";
    let query_uri = "oxide://project/project-a/queries/active_orders.postgres.sql";
    let schema_id = "67676767-6767-4767-8767-676767676767";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "app",
                    "tables": [{
                        "name": "orders",
                        "columns": [],
                        "indexes": [],
                        "constraints": [],
                        "comment": null,
                        "source_location": null
                    }],
                    "functions": [],
                    "source_uri": null
                }],
                "fileSchemas": {
                    (definition_uri): schema_id,
                    (query_uri): schema_id
                },
                "fileDialects": {
                    (definition_uri): "postgres",
                    (query_uri): "postgres"
                }
            }
        }),
    );
    let definition_sql = "CREATE VIEW reporting.active_orders AS SELECT * FROM app.orders;";
    let query_sql =
        "SELECT * FROM reporting.active_orders;\nSELECT 'active_orders';\n-- FROM active_orders";
    lsp.open(definition_uri, "postgres", definition_sql);
    lsp.open(query_uri, "postgres", query_sql);
    let target = query_sql.find("active_orders").unwrap() + 2;
    let position = json!({
        "line": 0,
        "character": utf16_column(query_sql, target)
    });

    let definition = lsp.request(
        "textDocument/definition",
        json!({
            "textDocument": { "uri": query_uri },
            "position": position.clone()
        }),
    );
    assert_eq!(
        definition.get("uri").and_then(Value::as_str),
        Some(definition_uri),
        "{definition}"
    );

    let references = lsp.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": query_uri },
            "position": position.clone(),
            "context": { "includeDeclaration": true }
        }),
    );
    let reference_locations = references.as_array().expect("reference locations");
    assert_eq!(reference_locations.len(), 2, "{references}");
    assert!(reference_locations
        .iter()
        .any(|location| { location.get("uri").and_then(Value::as_str) == Some(definition_uri) }));
    assert!(reference_locations
        .iter()
        .any(|location| { location.get("uri").and_then(Value::as_str) == Some(query_uri) }));

    let usages_only = lsp.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": query_uri },
            "position": position.clone(),
            "context": { "includeDeclaration": false }
        }),
    );
    assert_eq!(
        usages_only.as_array().map(Vec::len),
        Some(1),
        "{usages_only}"
    );
    assert_eq!(
        usages_only.pointer("/0/uri").and_then(Value::as_str),
        Some(query_uri),
        "{usages_only}"
    );

    let symbols = lsp.request("workspace/symbol", json!({ "query": "active_orders" }));
    assert!(
        symbols
            .as_array()
            .is_some_and(|items| items.iter().any(|symbol| {
                symbol.get("name").and_then(Value::as_str) == Some("active_orders")
                    && symbol.pointer("/location/uri").and_then(Value::as_str)
                        == Some(definition_uri)
            })),
        "{symbols}"
    );

    let edit = lsp.request(
        "textDocument/rename",
        json!({
            "textDocument": { "uri": query_uri },
            "position": position,
            "newName": "current_orders"
        }),
    );
    for uri in [definition_uri, query_uri] {
        let pointer = format!("/changes/{}", uri.replace('~', "~0").replace('/', "~1"));
        assert_eq!(
            edit.pointer(&pointer)
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1),
            "{edit}"
        );
    }
}

#[test]
fn cte_rename_remains_document_scoped_and_ignores_sql_noise() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();
    let uri = "file:///workspace/cte-rename.postgres.sql";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "fileDialects": { (uri): "postgres" }
            }
        }),
    );
    let sql = "WITH recent AS (SELECT 1 AS id) SELECT * FROM recent;\nSELECT 'recent'; -- recent";
    lsp.open(uri, "postgres", sql);
    let target = sql.find("FROM recent").unwrap() + "FROM ".len() + 2;
    let position = json!({
        "line": 0,
        "character": utf16_column(sql, target)
    });

    let prepared = lsp.request(
        "textDocument/prepareRename",
        json!({
            "textDocument": { "uri": uri },
            "position": position.clone()
        }),
    );
    assert!(prepared.get("start").is_some(), "{prepared}");

    let edit = lsp.request(
        "textDocument/rename",
        json!({
            "textDocument": { "uri": uri },
            "position": position,
            "newName": "latest"
        }),
    );
    let pointer = format!("/changes/{}", uri.replace('~', "~0").replace('/', "~1"));
    assert_eq!(
        edit.pointer(&pointer)
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "{edit}"
    );
}

#[test]
fn completion_models_cte_derived_and_temporary_relation_columns() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();
    let schema_id = "77777777-7777-4777-8777-777777777777";
    let cte_uri = "file:///workspace/local-cte.postgres.sql";
    let derived_uri = "file:///workspace/local-derived.postgres.sql";
    let temporary_uri = "file:///workspace/local-temp.postgres.sql";
    let temporary_as_uri = "file:///workspace/local-temp-as.postgres.sql";
    let temporary_drop_uri = "file:///workspace/local-temp-drop.postgres.sql";
    let table_function_uri = "file:///workspace/table-function.postgres.sql";
    let ordinality_uri = "file:///workspace/table-function-ordinality.postgres.sql";
    let correlation_uri = "file:///workspace/table-correlation.postgres.sql";
    let comma_source_uri = "file:///workspace/comma-source.postgres.sql";
    let table_hint_uri = "file:///workspace/table-hint.sqlserver.sql";
    lsp.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": {
                "schemas": [{
                    "id": schema_id,
                    "database": "app",
                    "tables": [
                        {
                            "name": "orders",
                            "columns": [{
                                "name": "amount",
                                "data_type": "numeric",
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
                        },
                        {
                            "name": "users",
                            "columns": (["id", "name", "email"].into_iter().map(|name| json!({
                                "name": name,
                                "data_type": "text",
                                "nullable": false,
                                "primary_key": false,
                                "unique": false,
                                "indexed": false,
                                "comment": null,
                                "source_location": null
                            })).collect::<Vec<_>>()),
                            "indexes": [],
                            "constraints": [],
                            "comment": null,
                            "source_location": null
                        },
                        {
                            "name": "audit_log",
                            "columns": (["event_id", "action"].into_iter().map(|name| json!({
                                "name": name,
                                "data_type": "text",
                                "nullable": false,
                                "primary_key": false,
                                "unique": false,
                                "indexed": false,
                                "comment": null,
                                "source_location": null
                            })).collect::<Vec<_>>()),
                            "indexes": [],
                            "constraints": [],
                            "comment": null,
                            "source_location": null
                        }
                    ],
                    "functions": [],
                    "source_uri": null
                }],
                "fileSchemas": {
                    (cte_uri): schema_id,
                    (derived_uri): schema_id,
                    (temporary_uri): schema_id,
                    (temporary_as_uri): schema_id,
                    (temporary_drop_uri): schema_id,
                    (table_function_uri): schema_id,
                    (ordinality_uri): schema_id,
                    (correlation_uri): schema_id,
                    (comma_source_uri): schema_id,
                    (table_hint_uri): schema_id
                },
                "fileDialects": {
                    (cte_uri): "postgres",
                    (derived_uri): "postgres",
                    (temporary_uri): "postgres",
                    (temporary_as_uri): "postgres",
                    (temporary_drop_uri): "postgres",
                    (table_function_uri): "postgres",
                    (ordinality_uri): "postgres",
                    (correlation_uri): "postgres",
                    (comma_source_uri): "postgres",
                    (table_hint_uri): "sqlserver"
                }
            }
        }),
    );

    for (uri, sql, expected) in [
        (
            cte_uri,
            "WITH recent AS (SELECT amount AS total FROM orders) SELECT * FROM recent r WHERE r.",
            vec!["total"],
        ),
        (
            derived_uri,
            "SELECT * FROM (SELECT amount AS total FROM orders) d WHERE d.",
            vec!["total"],
        ),
        (
            temporary_uri,
            "CREATE TEMP TABLE IF NOT EXISTS scratch (temp_id bigint, note text); SELECT * FROM scratch s WHERE s.",
            vec!["temp_id", "note"],
        ),
        (
            temporary_as_uri,
            "CREATE TEMP TABLE scratch AS SELECT amount AS copied_amount FROM orders; SELECT * FROM scratch s WHERE s.",
            vec!["copied_amount"],
        ),
        (
            table_function_uri,
            "SELECT * FROM generate_series(1, 3) g(value) WHERE g.",
            vec!["value"],
        ),
        (
            ordinality_uri,
            "SELECT * FROM generate_series(1, 3) WITH ORDINALITY AS g(value, ord), orders o WHERE g.",
            vec!["value", "ord"],
        ),
        (
            correlation_uri,
            "SELECT * FROM users u(user_id) WHERE u.",
            vec!["user_id", "name", "email"],
        ),
        (
            comma_source_uri,
            "SELECT * FROM users u JOIN orders o ON true, audit_log a WHERE a.",
            vec!["event_id", "action"],
        ),
        (
            table_hint_uri,
            "SELECT * FROM users u (NOLOCK) WHERE u.",
            vec!["id", "name", "email"],
        ),
    ] {
        let language_id = if uri.ends_with(".sqlserver.sql") {
            "sqlserver"
        } else {
            "postgres"
        };
        lsp.open(uri, language_id, sql);
        let result = lsp.request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": 0,
                    "character": utf16_column(sql, sql.len())
                },
                "context": { "triggerKind": 2, "triggerCharacter": "." }
            }),
        );
        let labels = result
            .as_array()
            .expect("completion array")
            .iter()
            .filter_map(|item| item.get("label").and_then(Value::as_str))
            .collect::<Vec<_>>();
        for label in expected {
            assert!(labels.contains(&label), "missing {label}: {result}");
        }
    }

    let dropped_sql = "CREATE TEMP TABLE scratch (temp_id bigint); DROP TABLE IF EXISTS scratch; SELECT * FROM scratch s WHERE s.";
    lsp.open(temporary_drop_uri, "postgres", dropped_sql);
    let dropped_result = lsp.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": temporary_drop_uri },
            "position": {
                "line": 0,
                "character": utf16_column(dropped_sql, dropped_sql.len())
            },
            "context": { "triggerKind": 2, "triggerCharacter": "." }
        }),
    );
    assert!(
        !dropped_result.as_array().is_some_and(|items| items
            .iter()
            .any(|item| item.get("label").and_then(Value::as_str) == Some("temp_id"))),
        "{dropped_result}"
    );
}
