use crate::dialect::Dialect;
use crate::parser::dsl::is_trailing_incomplete_json;
use crate::schema::{Column, Schema, Table};
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, Hover, Location,
    MarkedString, NumberOrString, Position, Range,
};

pub struct MongoDbDialect;

impl Default for MongoDbDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl MongoDbDialect {
    pub fn new() -> Self {
        Self
    }

    fn completion_item(
        label: &str,
        kind: CompletionItemKind,
        detail: &str,
        sort_prefix: &str,
    ) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            kind: Some(kind),
            detail: Some(detail.to_string()),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("{}{}", sort_prefix, label)),
            filter_text: None,
            insert_text: Some(format!("\"{}\"", label)),
            insert_text_format: None,
            text_edit: None,
            additional_text_edits: None,
            commit_characters: None,
            command: None,
            data: None,
            tags: None,
            insert_text_mode: None,
            label_details: None,
        }
    }

    fn collection_item(table: &Table) -> CompletionItem {
        CompletionItem {
            label: table.name.clone(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some(format!("MongoDB collection: {}", table.name)),
            documentation: table
                .comment
                .clone()
                .map(tower_lsp::lsp_types::Documentation::String),
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("2{}", table.name)),
            filter_text: None,
            insert_text: Some(format!("\"{}\"", table.name)),
            insert_text_format: None,
            text_edit: None,
            additional_text_edits: None,
            commit_characters: None,
            command: None,
            data: None,
            tags: None,
            insert_text_mode: None,
            label_details: None,
        }
    }

    fn field_item(collection: &Table, column: &Column) -> CompletionItem {
        CompletionItem {
            label: column.name.clone(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(format!(
                "MongoDB field: {}.{} ({})",
                collection.name, column.name, column.data_type
            )),
            documentation: column
                .comment
                .clone()
                .map(tower_lsp::lsp_types::Documentation::String),
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("3{}.{}", collection.name, column.name)),
            filter_text: None,
            insert_text: Some(format!("\"{}\"", column.name)),
            insert_text_format: None,
            text_edit: None,
            additional_text_edits: None,
            commit_characters: None,
            command: None,
            data: None,
            tags: None,
            insert_text_mode: None,
            label_details: None,
        }
    }
}

#[async_trait]
impl Dialect for MongoDbDialect {
    fn name(&self) -> &str {
        "mongodb"
    }

    async fn parse(&self, json: &str, _schema: Option<&Schema>) -> Vec<Diagnostic> {
        let trimmed = json.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(value) => mongodb_command_hints(&value, json),
            Err(error) if error.is_eof() && is_trailing_incomplete_json(trimmed) => Vec::new(),
            Err(error) => vec![json_error_diagnostic(error)],
        }
    }

    async fn completion(
        &self,
        _json: &str,
        _position: Position,
        schema: Option<&Schema>,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        for field in MONGODB_TOP_LEVEL_FIELDS {
            items.push(Self::completion_item(
                field,
                CompletionItemKind::FIELD,
                "MongoDB command field",
                "0",
            ));
        }

        for command in MONGODB_COMMANDS {
            items.push(Self::completion_item(
                command,
                CompletionItemKind::FUNCTION,
                "MongoDB command",
                "1",
            ));
        }

        if let Some(schema) = schema {
            for table in &schema.tables {
                items.push(Self::collection_item(table));
                for column in &table.columns {
                    items.push(Self::field_item(table, column));
                }
            }
        }

        items
    }

    async fn hover(
        &self,
        json: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Option<Hover> {
        let token = token_at_position(json, position);
        let schema = schema?;

        for table in &schema.tables {
            if table.name == token {
                return Some(Hover {
                    contents: tower_lsp::lsp_types::HoverContents::Scalar(MarkedString::String(
                        format!(
                            "**MongoDB collection**: `{}`\n\n{}",
                            table.name,
                            table.comment.as_deref().unwrap_or("No description")
                        ),
                    )),
                    range: None,
                });
            }

            if let Some(column) = table.columns.iter().find(|column| column.name == token) {
                return Some(Hover {
                    contents: tower_lsp::lsp_types::HoverContents::Scalar(MarkedString::String(
                        format!(
                            "**MongoDB field**: `{}.{}`\n\nType: `{}`",
                            table.name, column.name, column.data_type
                        ),
                    )),
                    range: None,
                });
            }
        }

        None
    }

    async fn goto_definition(
        &self,
        json: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Option<Location> {
        let token = token_at_position(json, position);
        let schema = schema?;

        for table in &schema.tables {
            if table.name == token {
                return schema_location(table.source_location.as_ref(), "file:///schema.json");
            }

            if let Some(column) = table.columns.iter().find(|column| column.name == token) {
                return schema_location(column.source_location.as_ref(), "file:///schema.json");
            }
        }

        None
    }

    async fn references(
        &self,
        json: &str,
        position: Position,
        _schema: Option<&Schema>,
    ) -> Vec<Location> {
        let token = token_at_position(json, position);
        if token.is_empty() {
            return Vec::new();
        }

        let uri = tower_lsp::lsp_types::Url::parse("file:///current.json").unwrap();
        find_token_references(json, &token, &uri)
    }

    async fn format(&self, json: &str) -> String {
        serde_json::from_str::<serde_json::Value>(json)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .unwrap_or_else(|_| json.to_string())
    }

    async fn validate(&self, json: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(json, schema).await
    }
}

const MONGODB_TOP_LEVEL_FIELDS: &[&str] = &[
    "database",
    "collection",
    "filter",
    "find",
    "aggregate",
    "pipeline",
    "sort",
    "limit",
    "skip",
    "projection",
    "documents",
    "document",
    "update",
    "many",
    "upsert",
    "command",
    "create",
    "drop",
    "dropDatabase",
    "dropIndexes",
    "renameCollection",
    "collMod",
];

const MONGODB_COMMANDS: &[&str] = &[
    "find",
    "aggregate",
    "countDocuments",
    "distinct",
    "insertOne",
    "insertMany",
    "updateOne",
    "updateMany",
    "deleteOne",
    "deleteMany",
    "create",
    "drop",
    "dropDatabase",
    "dropIndexes",
    "renameCollection",
    "collMod",
    "listCollections",
    "listDatabases",
    "listIndexes",
    "collStats",
    "dbStats",
];

fn mongodb_command_hints(value: &serde_json::Value, source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let values = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(std::slice::from_ref(value));

    for value in values {
        let Some(object) = value.as_object() else {
            diagnostics.push(hint_diagnostic(
                source,
                "MongoDB request should be a JSON object or an array of command objects",
            ));
            continue;
        };

        let has_collection = object
            .get("collection")
            .and_then(|value| value.as_str())
            .is_some();
        let has_command = MONGODB_COMMANDS
            .iter()
            .any(|command| object.contains_key(*command))
            || object.get("command").is_some();

        if !has_collection && !has_command {
            diagnostics.push(hint_diagnostic(
                source,
                "MongoDB command usually includes a collection or command field",
            ));
        }
    }

    diagnostics
}

fn json_error_diagnostic(error: serde_json::Error) -> Diagnostic {
    let line = error.line().saturating_sub(1) as u32;
    let column = error.column().saturating_sub(1) as u32;
    Diagnostic {
        range: Range {
            start: Position {
                line,
                character: column,
            },
            end: Position {
                line,
                character: column.saturating_add(1),
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String("MONGODB_JSON_ERROR".to_string())),
        code_description: None,
        source: Some("mongodb".to_string()),
        message: format!("JSON syntax error: {}", error),
        related_information: None,
        tags: None,
        data: None,
    }
}

fn hint_diagnostic(source: &str, message: &str) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: source.lines().next().unwrap_or("").len() as u32,
            },
        },
        severity: Some(DiagnosticSeverity::HINT),
        code: Some(NumberOrString::String("MONGODB_HINT".to_string())),
        code_description: None,
        source: Some("mongodb".to_string()),
        message: message.to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}

fn token_at_position(text: &str, position: Position) -> String {
    let position = crate::position::lsp_position_to_byte_position(text, position);
    let line = text.split('\n').nth(position.line as usize).unwrap_or("");
    let byte_index = position.character.min(line.len() as u32) as usize;
    let bytes = line.as_bytes();
    let mut start = byte_index.min(bytes.len());
    while start > 0 && is_token_char(bytes[start - 1] as char) {
        start -= 1;
    }
    let mut end = byte_index.min(bytes.len());
    while end < bytes.len() && is_token_char(bytes[end] as char) {
        end += 1;
    }
    line[start..end].trim_matches('"').to_string()
}

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

fn schema_location(
    source_location: Option<&(String, u32)>,
    fallback_uri: &str,
) -> Option<Location> {
    let (uri, line) = source_location
        .cloned()
        .unwrap_or_else(|| (fallback_uri.to_string(), 0));
    Some(Location {
        uri: tower_lsp::lsp_types::Url::parse(&uri).ok()?,
        range: Range {
            start: Position { line, character: 0 },
            end: Position { line, character: 0 },
        },
    })
}

fn find_token_references(
    text: &str,
    token: &str,
    uri: &tower_lsp::lsp_types::Url,
) -> Vec<Location> {
    text.lines()
        .enumerate()
        .flat_map(|(line_index, line)| {
            let mut matches = Vec::new();
            let mut search_from = 0usize;
            while let Some(offset) = line[search_from..].find(token) {
                let start = search_from + offset;
                let end = start + token.len();
                matches.push(Location {
                    uri: uri.clone(),
                    range: Range {
                        start: Position {
                            line: line_index as u32,
                            character: start as u32,
                        },
                        end: Position {
                            line: line_index as u32,
                            character: end as u32,
                        },
                    },
                });
                search_from = end;
            }
            matches
        })
        .collect()
}
