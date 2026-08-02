use crate::dialect::Dialect;
use crate::dialects::common;
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
        quoted_insert: bool,
    ) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            kind: Some(kind),
            detail: Some(detail.to_string()),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("{}{}", sort_prefix, label)),
            filter_text: Some(label.to_string()),
            insert_text: Some(if quoted_insert {
                format!("\"{}\"", label)
            } else {
                label.to_string()
            }),
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

    fn collection_item(table: &Table, quoted_insert: bool) -> CompletionItem {
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
            filter_text: Some(table.name.clone()),
            insert_text: Some(if quoted_insert {
                format!("\"{}\"", table.name)
            } else {
                table.name.clone()
            }),
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

    fn field_item(collection: &Table, column: &Column, quoted_insert: bool) -> CompletionItem {
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
            filter_text: Some(column.name.clone()),
            insert_text: Some(if quoted_insert {
                format!("\"{}\"", column.name)
            } else {
                column.name.clone()
            }),
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

    fn add_top_level_items(items: &mut Vec<CompletionItem>, prefix: &str, quoted_insert: bool) {
        for field in MONGODB_TOP_LEVEL_FIELDS {
            if !prefix.is_empty() && !field.to_ascii_lowercase().starts_with(prefix) {
                continue;
            }

            items.push(Self::completion_item(
                field,
                CompletionItemKind::FIELD,
                "MongoDB command field",
                "0",
                quoted_insert,
            ));
        }

        for command in MONGODB_COMMANDS {
            if !prefix.is_empty() && !command.to_ascii_lowercase().starts_with(prefix) {
                continue;
            }

            items.push(Self::completion_item(
                command,
                CompletionItemKind::FUNCTION,
                "MongoDB command",
                "1",
                quoted_insert,
            ));
        }
    }

    fn add_collection_items(
        items: &mut Vec<CompletionItem>,
        schema: Option<&Schema>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        let Some(schema) = schema else {
            return;
        };

        for table in &schema.tables {
            if prefix.is_empty() || table.name.to_ascii_lowercase().starts_with(prefix) {
                items.push(Self::collection_item(table, quoted_insert));
            }
        }
    }

    fn add_field_items(
        items: &mut Vec<CompletionItem>,
        schema: Option<&Schema>,
        collection_name: Option<&str>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        let Some(schema) = schema else {
            return;
        };

        let tables = schema.tables.iter().filter(|table| {
            collection_name
                .map(|name| table.name.eq_ignore_ascii_case(name))
                .unwrap_or(true)
        });
        for table in tables {
            for column in &table.columns {
                if prefix.is_empty() || column.name.to_ascii_lowercase().starts_with(prefix) {
                    items.push(Self::field_item(table, column, quoted_insert));
                }
            }
        }
    }

    fn add_named_items(
        items: &mut Vec<CompletionItem>,
        names: &[&str],
        prefix: &str,
        quoted_insert: bool,
        detail: &str,
        kind: CompletionItemKind,
    ) {
        for name in names {
            if prefix.is_empty() || name.to_ascii_lowercase().starts_with(prefix) {
                items.push(Self::completion_item(
                    name,
                    kind,
                    detail,
                    "1",
                    quoted_insert,
                ));
            }
        }
    }

    fn add_pipeline_stage_items(
        items: &mut Vec<CompletionItem>,
        schema: Option<&Schema>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        for stage in MONGODB_PIPELINE_STAGES {
            if !mongodb_pipeline_stage_supported(stage, schema)
                || (!prefix.is_empty() && !stage.to_ascii_lowercase().starts_with(prefix))
            {
                continue;
            }
            items.push(Self::completion_item(
                stage,
                CompletionItemKind::FUNCTION,
                "MongoDB aggregation pipeline stage",
                "1",
                quoted_insert,
            ));
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

        match parse_mongodb_json_stream(json) {
            Ok(mut values) => {
                let value = if values.len() == 1 {
                    values.remove(0)
                } else {
                    serde_json::Value::Array(values)
                };
                mongodb_command_hints(&value, json)
            }
            Err(error) if error.is_eof() && is_trailing_incomplete_json(trimmed) => Vec::new(),
            Err(error) => vec![json_error_diagnostic(error, json)],
        }
    }

    async fn completion(
        &self,
        json: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();
        let prefix = crate::position::cursor_token_prefix(json, position, is_token_char);
        let context = mongodb_completion_context(json, position);
        let quoted_insert = !context.inside_string;

        match context.kind {
            MongoCompletionKind::TopLevel => {
                Self::add_top_level_items(&mut items, &prefix, quoted_insert);
            }
            MongoCompletionKind::CollectionValue => {
                Self::add_collection_items(&mut items, schema, &prefix, quoted_insert);
            }
            MongoCompletionKind::FieldName => {
                Self::add_field_items(
                    &mut items,
                    schema,
                    context.collection.as_deref(),
                    &prefix,
                    quoted_insert,
                );
            }
            MongoCompletionKind::FieldOperator => {
                Self::add_named_items(
                    &mut items,
                    MONGODB_FIELD_OPERATORS,
                    &prefix,
                    quoted_insert,
                    "MongoDB field operator",
                    CompletionItemKind::OPERATOR,
                );
            }
            MongoCompletionKind::PipelineStage => {
                Self::add_pipeline_stage_items(&mut items, schema, &prefix, quoted_insert);
            }
            MongoCompletionKind::Broad => {
                Self::add_top_level_items(&mut items, &prefix, quoted_insert);
                Self::add_collection_items(&mut items, schema, &prefix, quoted_insert);
                Self::add_field_items(
                    &mut items,
                    schema,
                    context.collection.as_deref(),
                    &prefix,
                    quoted_insert,
                );
            }
            MongoCompletionKind::None => {}
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
                return common::metadata_location(
                    table.source_location.as_ref(),
                    schema.source_uri.as_ref(),
                    "file:///schema.json",
                );
            }

            if let Some(column) = table.columns.iter().find(|column| column.name == token) {
                return common::metadata_location(
                    column
                        .source_location
                        .as_ref()
                        .or(table.source_location.as_ref()),
                    schema.source_uri.as_ref(),
                    "file:///schema.json",
                );
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
        parse_mongodb_json_stream(json)
            .and_then(|values| {
                values
                    .iter()
                    .map(serde_json::to_string_pretty)
                    .collect::<Result<Vec<_>, _>>()
            })
            .map(|values| values.join("\n\n"))
            .unwrap_or_else(|_| json.to_string())
    }

    async fn validate(&self, json: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(json, schema).await
    }
}

fn parse_mongodb_json_stream(json: &str) -> Result<Vec<serde_json::Value>, serde_json::Error> {
    serde_json::Deserializer::from_str(json)
        .into_iter::<serde_json::Value>()
        .collect()
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

const MONGODB_FIELD_OPERATORS: &[&str] = &[
    "$eq",
    "$ne",
    "$gt",
    "$gte",
    "$lt",
    "$lte",
    "$in",
    "$nin",
    "$exists",
    "$type",
    "$regex",
    "$size",
    "$all",
    "$elemMatch",
    "$not",
];

const MONGODB_PIPELINE_STAGES: &[&str] = &[
    "$match",
    "$project",
    "$group",
    "$sort",
    "$limit",
    "$skip",
    "$unwind",
    "$lookup",
    "$addFields",
    "$set",
    "$unset",
    "$count",
    "$facet",
    "$replaceRoot",
    "$replaceWith",
    "$sample",
    "$bucket",
    "$bucketAuto",
    "$unionWith",
    "$setWindowFields",
    "$densify",
    "$fill",
    "$documents",
];

fn mongodb_pipeline_stage_supported(stage: &str, schema: Option<&Schema>) -> bool {
    let Some(schema) = schema else {
        return true;
    };
    match stage {
        "$set" | "$unset" | "$replaceWith" => schema.supports_server_version(4, 2),
        "$unionWith" => schema.supports_server_version(4, 4),
        "$setWindowFields" => schema.supports_server_version(5, 0),
        "$densify" | "$documents" => schema.supports_server_version(5, 1),
        "$fill" => schema.supports_server_version(5, 3),
        _ => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MongoCompletionKind {
    None,
    Broad,
    TopLevel,
    CollectionValue,
    FieldName,
    FieldOperator,
    PipelineStage,
}

#[derive(Debug, Clone)]
struct MongoCompletionContext {
    kind: MongoCompletionKind,
    inside_string: bool,
    collection: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct JsonObjectFrame {
    owner_key: Option<String>,
    last_key: Option<String>,
    string_values: Vec<(String, String)>,
    after_colon: bool,
}

#[derive(Debug, Clone, Default)]
struct JsonScanState {
    frames: Vec<JsonObjectFrame>,
    array_owner_keys: Vec<Option<String>>,
    pending_array_object_owner: Option<String>,
    previous_significant: Option<char>,
}

fn mongodb_completion_context(source: &str, position: Position) -> MongoCompletionContext {
    let byte_offset = byte_offset_at_position(source, position);
    let before = &source[..byte_offset.min(source.len())];
    let open_string_start = current_open_string_start(before);
    let context_source = open_string_start
        .map(|start| &before[..start])
        .unwrap_or(before);
    let state = scan_json_context(context_source);
    let inside_string = open_string_start.is_some();
    let previous = state.previous_significant;
    let current_key = state
        .frames
        .last()
        .and_then(|frame| frame.last_key.as_deref());
    let owner_key = state
        .frames
        .last()
        .and_then(|frame| frame.owner_key.as_deref());
    let collection = state.frames.iter().rev().find_map(|frame| {
        frame
            .string_values
            .iter()
            .rev()
            .find_map(|(key, value)| is_mongodb_collection_key(key).then(|| value.clone()))
    });

    let kind = if matches!(previous, Some(':')) {
        match current_key {
            Some(key) if is_mongodb_collection_key(key) => MongoCompletionKind::CollectionValue,
            _ => MongoCompletionKind::None,
        }
    } else if is_json_key_position(previous) {
        if owner_key.is_none() {
            MongoCompletionKind::TopLevel
        } else if owner_key == Some("pipeline") {
            MongoCompletionKind::PipelineStage
        } else if owner_key.map(is_mongodb_field_object_key).unwrap_or(false) {
            MongoCompletionKind::FieldName
        } else if owner_key
            .map(|key| !is_mongodb_structural_object_key(key))
            .unwrap_or(false)
        {
            MongoCompletionKind::FieldOperator
        } else {
            MongoCompletionKind::Broad
        }
    } else {
        MongoCompletionKind::Broad
    };

    MongoCompletionContext {
        kind,
        inside_string,
        collection,
    }
}

fn byte_offset_at_position(source: &str, position: Position) -> usize {
    let position = crate::position::lsp_position_to_byte_position(source, position);
    let mut offset = 0usize;

    for (line_index, line) in source.split('\n').enumerate() {
        if line_index == position.line as usize {
            return offset + (position.character as usize).min(line.len());
        }
        offset += line.len() + 1;
    }

    source.len()
}

fn current_open_string_start(source: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escaping = false;
    let mut start = 0usize;

    for (index, ch) in source.char_indices() {
        if in_string {
            if escaping {
                escaping = false;
            } else if ch == '\\' {
                escaping = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            start = index;
        }
    }

    in_string.then_some(start)
}

fn scan_json_context(source: &str) -> JsonScanState {
    let mut state = JsonScanState::default();
    let mut in_string = false;
    let mut escaping = false;
    let mut string_start = 0usize;
    let mut previous_significant = None;

    for (index, ch) in source.char_indices() {
        if in_string {
            if escaping {
                escaping = false;
            } else if ch == '\\' {
                escaping = true;
            } else if ch == '"' {
                in_string = false;
                let value = source[string_start..index].to_string();
                if let Some(frame) = state.frames.last_mut() {
                    if previous_significant == Some(':') {
                        if let Some(key) = frame.last_key.clone() {
                            frame.string_values.push((key, value));
                        }
                        frame.after_colon = false;
                    } else {
                        frame.last_key = Some(value);
                    }
                }
                previous_significant = Some('"');
            }
            continue;
        }

        if ch.is_whitespace() {
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                escaping = false;
                string_start = index + ch.len_utf8();
            }
            '{' => {
                let owner_key = if previous_significant == Some(':') {
                    state.frames.last().and_then(|frame| frame.last_key.clone())
                } else if previous_significant == Some('[') {
                    state.array_owner_keys.last().cloned().flatten()
                } else if previous_significant == Some(',') {
                    state.pending_array_object_owner.take()
                } else {
                    None
                };
                if let Some(frame) = state.frames.last_mut() {
                    if previous_significant == Some(':') {
                        frame.after_colon = false;
                    }
                }
                state.frames.push(JsonObjectFrame {
                    owner_key,
                    last_key: None,
                    string_values: Vec::new(),
                    after_colon: false,
                });
                previous_significant = Some('{');
            }
            '}' => {
                state.frames.pop();
                previous_significant = Some('}');
            }
            '[' => {
                let owner_key = if previous_significant == Some(':') {
                    state.frames.last().and_then(|frame| frame.last_key.clone())
                } else {
                    None
                };
                if let Some(frame) = state.frames.last_mut() {
                    if previous_significant == Some(':') {
                        frame.after_colon = false;
                    }
                }
                state.array_owner_keys.push(owner_key);
                state.pending_array_object_owner = None;
                previous_significant = Some('[');
            }
            ']' => {
                state.array_owner_keys.pop();
                state.pending_array_object_owner = None;
                previous_significant = Some(']');
            }
            ':' => {
                if let Some(frame) = state.frames.last_mut() {
                    frame.after_colon = true;
                }
                previous_significant = Some(':');
            }
            ',' => {
                let comma_in_array = matches!(previous_significant, Some('}' | ']'))
                    && !state.array_owner_keys.is_empty();
                if comma_in_array {
                    state.pending_array_object_owner =
                        state.array_owner_keys.last().cloned().flatten();
                } else if let Some(frame) = state.frames.last_mut() {
                    frame.last_key = None;
                    frame.after_colon = false;
                    state.pending_array_object_owner = None;
                }
                previous_significant = Some(',');
            }
            _ => {
                if let Some(frame) = state.frames.last_mut() {
                    if frame.after_colon {
                        frame.after_colon = false;
                    }
                }
                previous_significant = Some(ch);
            }
        }
    }

    state.previous_significant = previous_significant;
    state
}

fn is_json_key_position(previous: Option<char>) -> bool {
    matches!(previous, Some('{') | Some(',') | Some('"'))
}

fn is_mongodb_collection_key(key: &str) -> bool {
    matches!(
        key,
        "collection"
            | "find"
            | "aggregate"
            | "count"
            | "from"
            | "to"
            | "renameCollection"
            | "drop"
            | "create"
    )
}

fn is_mongodb_field_object_key(key: &str) -> bool {
    matches!(
        key,
        "find"
            | "filter"
            | "projection"
            | "sort"
            | "update"
            | "document"
            | "documents"
            | "deleteOne"
            | "deleteMany"
            | "updateOne"
            | "updateMany"
            | "insertOne"
            | "insertMany"
            | "$set"
            | "$unset"
            | "$inc"
            | "$mul"
            | "$min"
            | "$max"
            | "$rename"
            | "$setOnInsert"
            | "$push"
            | "$pull"
            | "$addToSet"
            | "$match"
            | "$and"
            | "$or"
            | "$nor"
            | "$elemMatch"
    )
}

fn is_mongodb_structural_object_key(key: &str) -> bool {
    matches!(
        key,
        "pipeline"
            | "$group"
            | "$lookup"
            | "$facet"
            | "$bucket"
            | "$bucketAuto"
            | "cursor"
            | "command"
    )
}

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

fn json_error_diagnostic(error: serde_json::Error, source: &str) -> Diagnostic {
    let line = error.line().saturating_sub(1) as u32;
    let byte_column = error.column().saturating_sub(1);
    let source_line = source.split('\n').nth(line as usize).unwrap_or("");
    let column = crate::position::byte_column_to_utf16_column(source_line, byte_column);
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
                character: source
                    .split('\n')
                    .next()
                    .unwrap_or("")
                    .encode_utf16()
                    .count() as u32,
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
                let start_character = line[..start].encode_utf16().count() as u32;
                let end_character = line[..end].encode_utf16().count() as u32;
                matches.push(Location {
                    uri: uri.clone(),
                    range: Range {
                        start: Position {
                            line: line_index as u32,
                            character: start_character,
                        },
                        end: Position {
                            line: line_index as u32,
                            character: end_character,
                        },
                    },
                });
                search_from = end;
            }
            matches
        })
        .collect()
}
