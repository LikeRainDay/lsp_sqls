use crate::dialect::Dialect;
use crate::dialects::DialectRegistry;
use crate::parser::SqlParser;
use crate::placeholder::SqlPlaceholderDialect;
use crate::position::lsp_position_at_end;
use crate::schema::{Schema, SchemaId, SchemaManager};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

const CURRENT_SQL_DOCUMENT_URI: &str = "file:///current.sql";
const CURRENT_JSON_DOCUMENT_URI: &str = "file:///current.json";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineCompletionCandidate {
    label: String,
    insert_text: Option<String>,
    kind: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineCompletionDiagnostic {
    severity: Option<u32>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineCompletionContextResponse {
    dialect: String,
    clause: String,
    statement_range: Option<Range>,
    ctes: Vec<String>,
    referenced_objects: Vec<String>,
    aliases: HashMap<String, String>,
    expected_kinds: Vec<String>,
    candidates: Vec<InlineCompletionCandidate>,
    diagnostics: Vec<InlineCompletionDiagnostic>,
    error_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineCompletionValidationParams {
    text_document: TextDocumentIdentifier,
    text: String,
}

fn rewrite_current_document_location_uri(location: &mut Location, document_uri: &Url) {
    if matches!(
        location.uri.as_str(),
        CURRENT_SQL_DOCUMENT_URI | CURRENT_JSON_DOCUMENT_URI
    ) {
        location.uri = document_uri.clone();
    }
}

fn rewrite_current_document_location_uris(locations: &mut [Location], document_uri: &Url) {
    for location in locations {
        rewrite_current_document_location_uri(location, document_uri);
    }
}

/// 文档管理器，用于存储和管理打开的文档内容
#[derive(Clone)]
struct DocumentManager {
    documents: Arc<DashMap<String, String>>,
}

#[derive(Clone)]
struct CachedSqlAnalysis {
    source: String,
    tree: tree_sitter::Tree,
}

impl DocumentManager {
    fn new() -> Self {
        Self {
            documents: Arc::new(DashMap::new()),
        }
    }

    fn update(&self, uri: String, text: String) {
        self.documents.insert(uri, text);
    }

    fn get(&self, uri: &str) -> Option<String> {
        self.documents.get(uri).map(|v| v.clone())
    }

    fn entries(&self) -> Vec<(String, String)> {
        self.documents
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    fn remove(&self, uri: &str) {
        self.documents.remove(uri);
    }
}

/// SQL LSP 服务器
pub struct SqlLspServer {
    client: Client,
    /// 方言注册表
    dialect_registry: Arc<DialectRegistry>,
    /// Schema 管理器
    schema_manager: Arc<SchemaManager>,
    /// 客户端显式配置的文件到方言映射
    configured_file_dialects: Arc<DashMap<String, String>>,
    /// 根据 URI/languageId/默认方言推断出的文件到方言映射
    inferred_file_dialects: Arc<DashMap<String, String>>,
    /// 客户端显式配置的文件到 Schema ID 映射
    configured_file_schemas: Arc<DashMap<String, SchemaId>>,
    /// 根据文档内容自动推断出的文件到 Schema ID 映射
    inferred_file_schemas: Arc<DashMap<String, SchemaId>>,
    /// 无法从 URI/languageId 推断时使用的默认方言
    default_dialect: Arc<RwLock<String>>,
    /// 打开文档的 languageId，用于配置刷新后恢复推断方言
    document_languages: Arc<DashMap<String, String>>,
    /// 文档管理器
    document_manager: DocumentManager,
    analysis_cache: Arc<DashMap<String, CachedSqlAnalysis>>,
}

impl SqlLspServer {
    pub fn new(client: Client) -> Self {
        tracing::info!("Creating new SQL LSP server instance");
        Self {
            client,
            dialect_registry: Arc::new(DialectRegistry::new()),
            schema_manager: Arc::new(SchemaManager::new()),
            configured_file_dialects: Arc::new(DashMap::new()),
            inferred_file_dialects: Arc::new(DashMap::new()),
            configured_file_schemas: Arc::new(DashMap::new()),
            inferred_file_schemas: Arc::new(DashMap::new()),
            default_dialect: Arc::new(RwLock::new("postgres".to_string())),
            document_languages: Arc::new(DashMap::new()),
            document_manager: DocumentManager::new(),
            analysis_cache: Arc::new(DashMap::new()),
        }
    }

    fn default_dialect_name(&self) -> String {
        self.default_dialect
            .read()
            .map(|dialect| dialect.clone())
            .unwrap_or_else(|_| "postgres".to_string())
    }

    fn inferred_dialect_for_document(&self, uri: &str, language_id: &str) -> String {
        infer_dialect_from_uri_and_language(uri, language_id, &self.default_dialect_name())
    }

    fn ensure_dialect_for_document(&self, uri: &str, language_id: &str) {
        if self.configured_file_dialects.contains_key(uri) {
            return;
        }

        self.inferred_file_dialects
            .entry(uri.to_string())
            .or_insert_with(|| self.inferred_dialect_for_document(uri, language_id));
    }

    fn ensure_dialects_for_open_documents(&self) {
        for entry in self.document_languages.iter() {
            self.ensure_dialect_for_document(entry.key(), entry.value());
        }
    }

    /// 获取文件的方言
    fn get_dialect_for_file(&self, uri: &str) -> Option<Arc<dyn Dialect>> {
        if let Some(dialect_name) = self.configured_file_dialects.get(uri) {
            return self.dialect_registry.get_by_name(dialect_name.value());
        }

        self.inferred_file_dialects
            .get(uri)
            .and_then(|dialect_name| self.dialect_registry.get_by_name(dialect_name.value()))
    }

    /// 获取文件的 Schema
    /// 如果文件没有显式关联的 Schema，则根据 SQL 内容自动推断最佳匹配的 Schema
    fn get_schema_for_file(&self, uri: &str) -> Option<Schema> {
        let text = self.document_manager.get(uri);
        schema_id_for_file(
            uri,
            text.as_deref(),
            &self.configured_file_schemas,
            &self.inferred_file_schemas,
            &self.schema_manager,
        )
        .and_then(|schema_id| self.schema_manager.get(schema_id))
    }

    fn get_schema_for_position(&self, uri: &str, text: &str, position: Position) -> Option<Schema> {
        schema_for_table_column_at_position(&self.schema_manager, text, position)
            .or_else(|| {
                schema_qualifier_at_position(text, position).and_then(|qualifier| {
                    find_schema_by_qualifier(&self.schema_manager, &qualifier)
                })
            })
            .or_else(|| self.get_schema_for_file(uri))
    }

    /// 将 LSP Position 转换为字符串字节偏移
    fn position_to_offset(&self, text: &str, position: tower_lsp::lsp_types::Position) -> usize {
        position_to_byte_offset(text, position)
    }

    async fn publish_diagnostics_for_open_documents(&self) {
        for (uri, text) in self.document_manager.entries() {
            let Ok(parsed_uri) = Url::parse(&uri) else {
                tracing::warn!("Skipping diagnostics for invalid document URI: {}", uri);
                continue;
            };

            if let Some(dialect) = self.get_dialect_for_file(&uri) {
                let schema = self.get_schema_for_file(&uri);
                let diagnostics = dialect.parse(&text, schema.as_ref()).await;
                self.client
                    .publish_diagnostics(parsed_uri, diagnostics, None)
                    .await;
            }
        }
    }

    fn analysis_tree_for_document(
        &self,
        uri: &str,
        text: &str,
        dialect: &str,
    ) -> Option<tree_sitter::Tree> {
        if !matches!(dialect, "postgres" | "mysql" | "clickhouse" | "hive") {
            self.analysis_cache.remove(uri);
            return None;
        }
        if let Some(cached) = self.analysis_cache.get(uri) {
            if cached.source == text {
                return Some(cached.tree.clone());
            }
        }
        let placeholder_dialect = match dialect {
            "postgres" => SqlPlaceholderDialect::Postgres,
            "mysql" => SqlPlaceholderDialect::Mysql,
            _ => SqlPlaceholderDialect::Generic,
        };
        let mut parser = SqlParser::new_with_placeholder_dialect(placeholder_dialect);
        let result = if let Some(cached) = self.analysis_cache.get(uri) {
            parser.parse_incremental(text, &cached.source, &cached.tree)
        } else {
            parser.parse(text)
        };
        let tree = result.tree?;
        self.analysis_cache.insert(
            uri.to_string(),
            CachedSqlAnalysis {
                source: text.to_string(),
                tree: tree.clone(),
            },
        );
        Some(tree)
    }

    pub async fn inline_completion_context(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<InlineCompletionContextResponse> {
        let uri = params.text_document.uri.to_string();
        let position = params.position;
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let Some(dialect) = self.get_dialect_for_file(&uri) else {
            return Ok(empty_inline_completion_context());
        };
        let schema = self.get_schema_for_position(&uri, &text, position);
        let mut raw_candidates = dialect.completion(&text, position, schema.as_ref()).await;
        raw_candidates.sort_by(|left, right| {
            left.sort_text
                .as_deref()
                .unwrap_or(&left.label)
                .cmp(right.sort_text.as_deref().unwrap_or(&right.label))
                .then_with(|| left.label.cmp(&right.label))
        });
        let candidates = raw_candidates
            .into_iter()
            .take(32)
            .map(|item| InlineCompletionCandidate {
                label: item.label.clone(),
                insert_text: item.insert_text.or(Some(item.label)),
                kind: item.kind.and_then(serialized_lsp_number),
            })
            .collect();
        let raw_diagnostics = dialect.parse(&text, schema.as_ref()).await;
        let error_count = raw_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
            .count();
        let diagnostics = raw_diagnostics
            .into_iter()
            .take(16)
            .map(|diagnostic| InlineCompletionDiagnostic {
                severity: diagnostic.severity.and_then(serialized_lsp_number),
                message: diagnostic.message,
            })
            .collect();

        let parser = SqlParser::new();
        let byte_position = SqlParser::lsp_position_to_byte_position(&text, position);
        let (clause, statement_range, referenced_objects, aliases, ctes) =
            if let Some(tree) = self.analysis_tree_for_document(&uri, &text, dialect.name()) {
                let node = parser.get_node_at_position(&tree, byte_position);
                let context =
                    node.map(|node| parser.analyze_completion_context(node, &text, byte_position));
                (
                    context
                        .as_ref()
                        .map(|context| format!("{context:?}"))
                        .unwrap_or_else(|| "Default".to_string()),
                    node.and_then(|node| statement_range_for_node(&text, node)),
                    parser.extract_referenced_tables_at_position(&tree, &text, byte_position),
                    parser.extract_aliases_at_position(&tree, &text, byte_position),
                    parser.extract_common_table_expressions(&text),
                )
            } else {
                (
                    "Default".to_string(),
                    None,
                    Vec::new(),
                    HashMap::new(),
                    Vec::new(),
                )
            };
        let expected_kinds = expected_kinds_for_clause(&clause);
        let statement_range = statement_range.or_else(|| {
            (!text.is_empty()).then_some(Range {
                start: Position::new(0, 0),
                end: lsp_position_at_end(&text),
            })
        });

        Ok(InlineCompletionContextResponse {
            dialect: dialect.name().to_string(),
            clause,
            statement_range,
            ctes,
            referenced_objects,
            aliases,
            expected_kinds,
            candidates,
            diagnostics,
            error_count,
        })
    }

    pub async fn validate_inline_completion(
        &self,
        params: InlineCompletionValidationParams,
    ) -> Result<Vec<Diagnostic>> {
        let uri = params.text_document.uri.to_string();
        let Some(dialect) = self.get_dialect_for_file(&uri) else {
            return Ok(Vec::new());
        };
        let schema = self.get_schema_for_file(&uri);
        Ok(dialect.parse(&params.text, schema.as_ref()).await)
    }
}

fn empty_inline_completion_context() -> InlineCompletionContextResponse {
    InlineCompletionContextResponse {
        dialect: "unknown".to_string(),
        clause: "Default".to_string(),
        statement_range: None,
        ctes: Vec::new(),
        referenced_objects: Vec::new(),
        aliases: HashMap::new(),
        expected_kinds: Vec::new(),
        candidates: Vec::new(),
        diagnostics: Vec::new(),
        error_count: 0,
    }
}

fn serialized_lsp_number(value: impl Serialize) -> Option<u32> {
    serde_json::to_value(value).ok()?.as_u64()?.try_into().ok()
}

fn statement_range_for_node(source: &str, mut node: tree_sitter::Node<'_>) -> Option<Range> {
    loop {
        let kind = node.kind();
        if kind == "statement" || kind.ends_with("_statement") {
            let start = node.start_position();
            let end = node.end_position();
            return Some(Range {
                start: crate::position::byte_position_to_lsp_position(
                    source,
                    Position::new(start.row as u32, start.column as u32),
                ),
                end: crate::position::byte_position_to_lsp_position(
                    source,
                    Position::new(end.row as u32, end.column as u32),
                ),
            });
        }
        node = node.parent()?;
    }
}

fn expected_kinds_for_clause(clause: &str) -> Vec<String> {
    let values: &[&str] = match clause {
        "FromClause" | "JoinClause" => &["relation", "schema"],
        "TableColumn" | "SelectClause" | "WhereClause" | "HavingClause" | "OrderByClause"
        | "GroupByClause" | "UsingClause" => &["column", "function", "keyword"],
        "DataTypeClause" => &["data-type"],
        "ExpressionValueClause" | "InsertValueClause" | "CaseResultClause" => {
            &["value", "function", "placeholder"]
        }
        _ => &["keyword", "relation", "column"],
    };
    values.iter().map(|value| (*value).to_string()).collect()
}

fn position_to_byte_offset(text: &str, position: Position) -> usize {
    let (line_start, line_end) = line_bounds_for_position(text, position.line);
    line_start + utf16_position_to_line_byte_offset(&text[line_start..line_end], position.character)
}

fn line_bounds_for_position(text: &str, target_line: u32) -> (usize, usize) {
    let bytes = text.as_bytes();
    let mut current_line = 0u32;
    let mut line_start = 0usize;
    let mut index = 0usize;

    while index < bytes.len() && current_line < target_line {
        match bytes[index] {
            b'\n' => {
                current_line += 1;
                index += 1;
                line_start = index;
            }
            b'\r' => {
                current_line += 1;
                index += 1;
                if index < bytes.len() && bytes[index] == b'\n' {
                    index += 1;
                }
                line_start = index;
            }
            _ => index += 1,
        }
    }

    if current_line < target_line {
        return (text.len(), text.len());
    }

    let mut line_end = line_start;
    while line_end < bytes.len() && !matches!(bytes[line_end], b'\n' | b'\r') {
        line_end += 1;
    }

    (line_start, line_end)
}

fn utf16_position_to_line_byte_offset(line: &str, character: u32) -> usize {
    let target_units = character as usize;
    let mut current_units = 0usize;

    for (byte_index, ch) in line.char_indices() {
        if current_units >= target_units {
            return byte_index;
        }

        let next_units = current_units + ch.len_utf16();
        if next_units > target_units {
            return byte_index;
        }

        current_units = next_units;
    }

    line.len()
}

const COMPLETED_SQL_CONTEXT_KEYWORDS: &[&str] = &[
    "select", "from", "join", "where", "on", "by", "having", "limit", "offset", "values", "set",
    "into",
];

fn completed_sql_context_keyword_at_position(text: &str, position: Position) -> Option<String> {
    let offset = position_to_byte_offset(text, position);
    let text_before = text.get(..offset)?;

    if text_before
        .chars()
        .last()
        .is_none_or(|ch| ch.is_whitespace())
    {
        return None;
    }

    let token_start = text_before
        .char_indices()
        .rev()
        .find_map(|(index, ch)| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                None
            } else {
                Some(index + ch.len_utf8())
            }
        })
        .unwrap_or(0);
    let token = text_before[token_start..].to_ascii_lowercase();

    COMPLETED_SQL_CONTEXT_KEYWORDS
        .contains(&token.as_str())
        .then_some(token)
}

fn apply_completed_sql_context_completion_edits(
    text: &str,
    position: Position,
    items: &mut Vec<CompletionItem>,
) {
    let Some(keyword) = completed_sql_context_keyword_at_position(text, position) else {
        return;
    };
    let byte_position = SqlParser::lsp_position_to_byte_position(text, position);
    let ddl_on_relation_target =
        keyword == "on" && SqlParser::ddl_on_relation_target_at_position(text, byte_position);
    items.retain(|item| {
        completion_item_allowed_after_completed_keyword(&keyword, item, ddl_on_relation_target)
    });

    let range = Range {
        start: position,
        end: position,
    };

    for item in items {
        let insert_text = item
            .insert_text
            .clone()
            .unwrap_or_else(|| item.label.clone());
        item.filter_text = Some(keyword.clone());
        item.text_edit = Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: format!(" {insert_text}"),
        }));
    }
}

fn completion_item_allowed_after_completed_keyword(
    keyword: &str,
    item: &CompletionItem,
    ddl_on_relation_target: bool,
) -> bool {
    match keyword {
        "from" | "join" | "into" => is_relation_completion_kind(item.kind),
        "select" => !is_relation_completion_kind(item.kind),
        "on" if ddl_on_relation_target => is_relation_completion_kind(item.kind),
        "where" | "on" | "by" | "having" | "values" | "set" => {
            !is_relation_completion_kind(item.kind)
                && item.kind != Some(CompletionItemKind::OPERATOR)
        }
        "limit" | "offset" => true,
        _ => true,
    }
}

fn is_relation_completion_kind(kind: Option<CompletionItemKind>) -> bool {
    matches!(
        kind,
        Some(
            CompletionItemKind::CLASS
                | CompletionItemKind::STRUCT
                | CompletionItemKind::MODULE
                | CompletionItemKind::FILE
                | CompletionItemKind::FOLDER
        )
    )
}

fn qualified_identifier_range_at_position(text: &str, position: Position) -> Option<Range> {
    let offset = position_to_byte_offset(text, position);
    let text_before = text.get(..offset)?;
    let token_start = identifier_path_start_before_cursor(text_before)?;
    let token = text_before[token_start..].trim();

    token.contains('.').then_some(Range {
        start: byte_offset_to_lsp_position(text, token_start),
        end: position,
    })
}

fn identifier_path_start_before_cursor(text_before: &str) -> Option<usize> {
    let mut token_start = 0;
    let mut quote_end: Option<char> = None;
    let mut chars = text_before.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if let Some(end_quote) = quote_end {
            if ch == end_quote {
                if chars.peek().is_some_and(|(_, next)| *next == end_quote) {
                    chars.next();
                    continue;
                }
                quote_end = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => quote_end = Some(ch),
            '[' => quote_end = Some(']'),
            _ if is_identifier_path_boundary(ch) => token_start = index + ch.len_utf8(),
            _ => {}
        }
    }

    (token_start < text_before.len()).then_some(token_start)
}

fn is_identifier_path_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ',' | '(' | ')' | ';' | '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '%'
        )
}

fn byte_offset_to_lsp_position(text: &str, target_offset: usize) -> Position {
    let target_offset = target_offset.min(text.len());
    let mut line = 0u32;
    let mut character = 0u32;

    for (byte_index, ch) in text.char_indices() {
        if byte_index >= target_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    Position { line, character }
}

fn apply_qualified_identifier_completion_edits(
    text: &str,
    position: Position,
    items: &mut [CompletionItem],
) {
    let Some(range) = qualified_identifier_range_at_position(text, position) else {
        return;
    };

    for item in items {
        if item.text_edit.is_some() {
            continue;
        }

        let insert_text = item
            .insert_text
            .clone()
            .unwrap_or_else(|| item.label.clone());
        if !insert_text.contains('.') {
            continue;
        }

        item.text_edit = Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: insert_text,
        }));
    }
}

fn calculate_schema_match_score(tables: &[String], schema: &Schema) -> i32 {
    use crate::parser::SqlParser;

    let mut score = 0;

    for table_name in tables {
        if schema
            .tables
            .iter()
            .any(|table| SqlParser::table_name_matches(table_name, &schema.database, &table.name))
        {
            score += 10;
        }
    }

    let matched_count = tables
        .iter()
        .filter(|table_name| {
            schema.tables.iter().any(|table| {
                SqlParser::table_name_matches(table_name, &schema.database, &table.name)
            })
        })
        .count();

    if matched_count > 1 {
        score += matched_count as i32 * 2;
    }

    score
}

fn infer_schema_id_from_tables(
    tables: &[String],
    schema_manager: &SchemaManager,
) -> Option<SchemaId> {
    let mut best_match: Option<(SchemaId, i32)> = None;
    let mut has_tie = false;

    for schema_id in schema_manager.list_ids() {
        let Some(schema) = schema_manager.get(schema_id) else {
            continue;
        };
        let score = calculate_schema_match_score(tables, &schema);
        if score <= 0 {
            continue;
        }

        match best_match {
            None => {
                best_match = Some((schema_id, score));
                has_tie = false;
            }
            Some((_, best_score)) if score > best_score => {
                best_match = Some((schema_id, score));
                has_tie = false;
            }
            Some((_, best_score)) if score == best_score => {
                has_tie = true;
            }
            _ => {}
        }
    }

    if has_tie {
        None
    } else {
        best_match.map(|(schema_id, _)| schema_id)
    }
}

fn infer_schema_id_from_text(text: &str, schema_manager: &SchemaManager) -> Option<SchemaId> {
    let mut parser = SqlParser::new();
    let parse_result = parser.parse(text);
    let tree = parse_result.tree?;
    let tables = parser.extract_referenced_tables(&tree, text);
    if tables.is_empty() {
        return None;
    }

    // 只有唯一最高分才推断，避免多个 schema 拥有同名表时随机绑定错误上下文。
    infer_schema_id_from_tables(&tables, schema_manager)
}

fn schema_id_for_file(
    uri: &str,
    text: Option<&str>,
    configured_file_schemas: &DashMap<String, SchemaId>,
    inferred_file_schemas: &DashMap<String, SchemaId>,
    schema_manager: &SchemaManager,
) -> Option<SchemaId> {
    if let Some(schema_id) = configured_file_schemas
        .get(uri)
        .map(|schema_id| *schema_id.value())
    {
        if schema_manager.get(schema_id).is_some() {
            return Some(schema_id);
        }
    }

    if let Some(schema_id) = inferred_file_schemas
        .get(uri)
        .map(|schema_id| *schema_id.value())
    {
        if schema_manager.get(schema_id).is_some() {
            return Some(schema_id);
        }
        inferred_file_schemas.remove(uri);
    }

    let schema_id = infer_schema_id_from_text(text?, schema_manager)?;
    inferred_file_schemas.insert(uri.to_string(), schema_id);
    Some(schema_id)
}

#[tower_lsp::async_trait]
impl LanguageServer for SqlLspServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "sql-lsp".to_string(),
                version: Some("0.1.0".to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        " ".to_string(),
                        "(".to_string(),
                    ]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: Some("sql-lsp".to_string()),
                        inter_file_dependencies: true,
                        workspace_diagnostics: false,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        tracing::info!("SQL LSP server initialized and ready");
        self.client
            .log_message(MessageType::INFO, "SQL LSP server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        tracing::debug!("Received configuration change");
        // 解析配置 JSON
        if let Some(settings) = params.settings.as_object() {
            // 处理 schemas 配置
            if let Some(default_dialect_value) = settings.get("defaultDialect") {
                if let Some(default_dialect) = default_dialect_value.as_str() {
                    if self.dialect_registry.get_by_name(default_dialect).is_some() {
                        if let Ok(mut current_default) = self.default_dialect.write() {
                            *current_default = default_dialect.to_string();
                        }
                        self.client
                            .log_message(
                                MessageType::INFO,
                                format!("Updated default dialect to {}", default_dialect),
                            )
                            .await;
                    } else {
                        self.client
                            .log_message(
                                MessageType::WARNING,
                                format!(
                                    "Ignoring unsupported default dialect: {}",
                                    default_dialect
                                ),
                            )
                            .await;
                    }
                }
            }

            // 处理 schemas 配置
            if let Some(schemas_value) = settings.get("schemas") {
                if let Ok(schemas) =
                    serde_json::from_value::<Vec<crate::schema::Schema>>(schemas_value.clone())
                {
                    // 清空旧的 schema 并注册新的
                    self.schema_manager.clear();
                    let count = schemas.len();
                    for schema in schemas {
                        self.schema_manager.register(schema);
                    }
                    self.inferred_file_schemas.clear();
                    self.client
                        .log_message(MessageType::INFO, format!("Updated {} schemas", count))
                        .await;
                } else {
                    self.client
                        .log_message(
                            MessageType::WARNING,
                            "Failed to parse schemas configuration",
                        )
                        .await;
                }
            }

            // 处理文件到 schema 的映射配置
            if let Some(file_schemas_value) = settings.get("fileSchemas") {
                self.configured_file_schemas.clear();
                self.inferred_file_schemas.clear();
                if let Some(file_schemas_obj) = file_schemas_value.as_object() {
                    for (uri, schema_id_str) in file_schemas_obj {
                        if let Some(id_str) = schema_id_str.as_str() {
                            if let Ok(schema_id) = id_str.parse::<crate::schema::SchemaId>() {
                                self.configured_file_schemas.insert(uri.clone(), schema_id);
                            }
                        }
                    }
                    self.client
                        .log_message(MessageType::INFO, "Updated file-schema mappings")
                        .await;
                }
            }

            // 处理文件到 dialect 的显式映射配置
            if let Some(file_dialects_value) = settings.get("fileDialects") {
                self.configured_file_dialects.clear();
                if let Some(file_dialects_obj) = file_dialects_value.as_object() {
                    for (uri, dialect_name) in file_dialects_obj {
                        if let Some(name) = dialect_name.as_str() {
                            if self.dialect_registry.get_by_name(name).is_some() {
                                self.configured_file_dialects
                                    .insert(uri.clone(), name.to_string());
                            }
                        }
                    }
                    self.client
                        .log_message(MessageType::INFO, "Updated file-dialect mappings")
                        .await;
                }
            }

            self.inferred_file_dialects.clear();
            self.ensure_dialects_for_open_documents();
            self.publish_diagnostics_for_open_documents().await;
        }
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        let text = params.text_document.text.clone();
        let language_id = params.text_document.language_id.clone();

        self.inferred_file_schemas.remove(&uri);

        // 存储文档内容
        self.document_manager.update(uri.clone(), text.clone());
        self.document_languages
            .insert(uri.clone(), language_id.clone());

        // 尝试从 URI 和 languageId 推断方言。客户端显式配置的 fileDialects
        // 优先级更高，因此只在当前文件没有映射时写入推断结果。
        self.ensure_dialect_for_document(&uri, &language_id);

        // 发布诊断
        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_file(&uri);
            let diagnostics = dialect.parse(&text, schema.as_ref()).await;
            self.client
                .publish_diagnostics(params.text_document.uri, diagnostics, None)
                .await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        self.inferred_file_schemas.remove(&uri);

        // 处理增量同步
        for change in params.content_changes {
            if let Some(range) = change.range {
                // 增量更新：应用部分文本变更
                if let Some(mut current_text) = self.document_manager.get(&uri) {
                    // 将 LSP Range 转换为字节偏移
                    let start_offset = self.position_to_offset(&current_text, range.start);
                    let end_offset = self.position_to_offset(&current_text, range.end);

                    // 应用变更
                    current_text.replace_range(start_offset..end_offset, &change.text);
                    self.document_manager
                        .update(uri.clone(), current_text.clone());

                    // 重新解析并发布诊断
                    if let Some(dialect) = self.get_dialect_for_file(&uri) {
                        let schema = self.get_schema_for_file(&uri);
                        let diagnostics = dialect.parse(&current_text, schema.as_ref()).await;
                        self.client
                            .publish_diagnostics(
                                params.text_document.uri.clone(),
                                diagnostics,
                                None,
                            )
                            .await;
                    }
                }
            } else {
                // 完整文档更新
                let text = change.text.clone();
                self.document_manager.update(uri.clone(), text.clone());

                if let Some(dialect) = self.get_dialect_for_file(&uri) {
                    let schema = self.get_schema_for_file(&uri);
                    let diagnostics = dialect.parse(&text, schema.as_ref()).await;
                    self.client
                        .publish_diagnostics(params.text_document.uri.clone(), diagnostics, None)
                        .await;
                }
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        // 清理文档
        self.document_manager.remove(&uri);
        self.document_languages.remove(&uri);
        self.inferred_file_dialects.remove(&uri);
        self.inferred_file_schemas.remove(&uri);
        self.analysis_cache.remove(&uri);
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_position(&uri, &text, position);
            let mut items = dialect
                .completion_with_context(&text, position, schema.as_ref(), params.context.as_ref())
                .await;
            apply_completed_sql_context_completion_edits(&text, position, &mut items);
            apply_qualified_identifier_completion_edits(&text, position, &mut items);
            return Ok(Some(CompletionResponse::Array(items)));
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_position(&uri, &text, position);
            return Ok(dialect.hover(&text, position, schema.as_ref()).await);
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let document_uri = params.text_document_position_params.text_document.uri;
        let uri = document_uri.to_string();
        let position = params.text_document_position_params.position;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_position(&uri, &text, position);
            if let Some(mut location) = dialect
                .goto_definition(&text, position, schema.as_ref())
                .await
            {
                rewrite_current_document_location_uri(&mut location, &document_uri);
                return Ok(Some(GotoDefinitionResponse::Scalar(location)));
            }
        }

        Ok(None)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let document_uri = params.text_document_position.text_document.uri;
        let uri = document_uri.to_string();
        let position = params.text_document_position.position;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_position(&uri, &text, position);
            let mut locations = dialect.references(&text, position, schema.as_ref()).await;
            rewrite_current_document_location_uris(&mut locations, &document_uri);
            return Ok(Some(locations));
        }

        Ok(None)
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let formatted = dialect.format(&text).await;
            let range = Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: lsp_position_at_end(&text),
            };
            return Ok(Some(vec![TextEdit {
                range,
                new_text: formatted,
            }]));
        }

        Ok(None)
    }
}

fn schema_qualifier_at_position(text: &str, position: Position) -> Option<String> {
    let byte_position = SqlParser::lsp_position_to_byte_position(text, position);
    SqlParser::column_qualifier_before_position(text, byte_position)
}

fn find_schema_by_qualifier(schema_manager: &SchemaManager, qualifier: &str) -> Option<Schema> {
    let normalized_qualifier = SqlParser::normalize_identifier(qualifier);
    if normalized_qualifier.is_empty() {
        return None;
    }

    for schema_id in schema_manager.list_ids() {
        let Some(schema) = schema_manager.get(schema_id) else {
            continue;
        };
        if SqlParser::normalize_identifier(&schema.database)
            .eq_ignore_ascii_case(&normalized_qualifier)
        {
            return Some(schema);
        }
    }

    None
}

fn find_schema_by_table_reference(
    schema_manager: &SchemaManager,
    table_reference: &str,
) -> Option<Schema> {
    let mut matched_schema: Option<Schema> = None;

    for schema_id in schema_manager.list_ids() {
        let Some(schema) = schema_manager.get(schema_id) else {
            continue;
        };

        let has_table = schema.tables.iter().any(|table| {
            SqlParser::table_name_matches(table_reference, &schema.database, &table.name)
        });
        if !has_table {
            continue;
        }

        if matched_schema.is_some() {
            return None;
        }
        matched_schema = Some(schema);
    }

    matched_schema
}

fn schema_for_table_column_at_position(
    schema_manager: &SchemaManager,
    text: &str,
    position: Position,
) -> Option<Schema> {
    let mut parser = SqlParser::new();
    let byte_position = SqlParser::lsp_position_to_byte_position(text, position);
    let parse_result = parser.parse(text);
    let tree = parse_result.tree.as_ref()?;
    let table_name =
        SqlParser::column_qualifier_before_position(text, byte_position).or_else(|| {
            let node = parser.get_node_at_position(tree, byte_position)?;
            parser.get_table_name_for_column(node, text)
        })?;
    let aliases = parser.extract_aliases_at_position(tree, text, byte_position);
    let table_reference = aliases
        .get(&table_name)
        .map(String::as_str)
        .unwrap_or(table_name.as_str());

    find_schema_by_table_reference(schema_manager, table_reference)
}

/// 从 URI 和 languageId 推断方言类型
///
/// 支持多种 URI scheme：
/// - `file://` - 文件系统文件
/// - `untitled://` - 未保存的文档（VS Code 等编辑器）
/// - 其他自定义 scheme
///
/// 推断优先级：
/// 1. URI 扩展名（如 `.mysql.sql`）
/// 2. languageId（如 `mysql`, `postgresql`, `sql`）
/// 3. 配置的默认方言
fn infer_dialect_from_uri_and_language(
    uri: &str,
    language_id: &str,
    default_dialect: &str,
) -> String {
    // 首先尝试从 URI 扩展名推断
    let uri_lower = uri.to_lowercase();

    if uri_lower.ends_with(".mysql.sql") || uri_lower.ends_with(".mysql") {
        return "mysql".to_string();
    } else if uri_lower.ends_with(".postgres.sql")
        || uri_lower.ends_with(".postgresql.sql")
        || uri_lower.ends_with(".pgsql")
        || uri_lower.ends_with(".psql")
    {
        return "postgres".to_string();
    } else if uri_lower.ends_with(".hive.sql") || uri_lower.ends_with(".hql") {
        return "hive".to_string();
    } else if uri_lower.ends_with(".es.eql") || uri_lower.ends_with(".eql") {
        return "elasticsearch-eql".to_string();
    } else if uri_lower.ends_with(".es.dsl")
        || uri_lower.ends_with(".es.json")
        || uri_lower.ends_with(".elasticsearch")
    {
        return "elasticsearch-dsl".to_string();
    } else if uri_lower.ends_with(".ch.sql") || uri_lower.ends_with(".clickhouse") {
        return "clickhouse".to_string();
    } else if uri_lower.ends_with(".redis.sql") || uri_lower.ends_with(".redis") {
        return "redis".to_string();
    } else if uri_lower.ends_with(".mongo.json")
        || uri_lower.ends_with(".mongodb.json")
        || uri_lower.ends_with(".mongo")
        || uri_lower.ends_with(".mongodb")
    {
        return "mongodb".to_string();
    }

    // 如果 URI 无法推断，尝试从 languageId 推断
    let lang_lower = language_id.to_lowercase();
    match lang_lower.as_str() {
        "mysql" | "mysql-sql" => "mysql".to_string(),
        "postgresql" | "postgres" | "postgres-sql" | "pgsql" | "psql" => "postgres".to_string(),
        "hive" | "hql" => "hive".to_string(),
        "elasticsearch-eql" | "eql" => "elasticsearch-eql".to_string(),
        "elasticsearch-dsl" | "es-dsl" | "json" if uri_lower.contains("elasticsearch") => {
            "elasticsearch-dsl".to_string()
        }
        "clickhouse" | "ch" => "clickhouse".to_string(),
        "redis" => "redis".to_string(),
        "mongodb" | "mongo" | "mongodb-json" | "mongo-json" | "json" => "mongodb".to_string(),
        _ => default_dialect.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_completed_sql_context_completion_edits, apply_qualified_identifier_completion_edits,
        calculate_schema_match_score, completed_sql_context_keyword_at_position,
        find_schema_by_qualifier, find_schema_by_table_reference,
        infer_dialect_from_uri_and_language, infer_schema_id_from_tables, position_to_byte_offset,
        rewrite_current_document_location_uri, rewrite_current_document_location_uris,
        schema_for_table_column_at_position, schema_id_for_file, schema_qualifier_at_position,
    };
    use crate::schema::{Schema, SchemaId, SchemaManager, Table};
    use dashmap::DashMap;
    use tower_lsp::lsp_types::{
        CompletionItem, CompletionItemKind, CompletionTextEdit, Location, Position, Range, Url,
    };

    fn test_schema(database: &str, tables: &[&str]) -> Schema {
        Schema {
            id: SchemaId::new(),
            database: database.to_string(),
            tables: tables
                .iter()
                .map(|name| Table {
                    name: (*name).to_string(),
                    ..Default::default()
                })
                .collect(),
            functions: Vec::new(),
            source_uri: None,
        }
    }

    fn test_location(uri: &str) -> Location {
        Location {
            uri: Url::parse(uri).expect("valid test URI"),
            range: Range::default(),
        }
    }

    #[test]
    fn rewrites_sql_and_json_current_document_sentinels() {
        let document_uri = Url::parse("untitled:OxideStudio/%E6%9F%A5%E8%AF%A2-1").unwrap();
        let mut locations = vec![
            test_location("file:///current.sql"),
            test_location("file:///current.json"),
        ];

        rewrite_current_document_location_uris(&mut locations, &document_uri);

        assert!(locations
            .iter()
            .all(|location| location.uri == document_uri));
    }

    #[test]
    fn preserves_schema_and_other_source_uris() {
        let document_uri = Url::parse("file:///workspace/query.sql").unwrap();
        let source_uri = Url::parse("file:///schemas/app.sql").unwrap();
        let mut schema_fallback = test_location("file:///schema.sql");
        let mut schema_source = Location {
            uri: source_uri.clone(),
            range: Range::default(),
        };

        rewrite_current_document_location_uri(&mut schema_fallback, &document_uri);
        rewrite_current_document_location_uri(&mut schema_source, &document_uri);

        assert_eq!(schema_fallback.uri.as_str(), "file:///schema.sql");
        assert_eq!(schema_source.uri, source_uri);
    }

    #[test]
    fn infers_dialect_from_uri_before_language_id() {
        assert_eq!(
            infer_dialect_from_uri_and_language("file:///query.mysql.sql", "postgres", "postgres"),
            "mysql"
        );
        assert_eq!(
            infer_dialect_from_uri_and_language("file:///query.psql", "mysql", "mysql"),
            "postgres"
        );
        assert_eq!(
            infer_dialect_from_uri_and_language("file:///query.mongo.json", "sql", "postgres"),
            "mongodb"
        );
    }

    #[test]
    fn infers_dialect_from_language_id_before_default() {
        assert_eq!(
            infer_dialect_from_uri_and_language("untitled://1", "pgsql", "mysql"),
            "postgres"
        );
        assert_eq!(
            infer_dialect_from_uri_and_language("untitled://1", "mysql-sql", "postgres"),
            "mysql"
        );
        assert_eq!(
            infer_dialect_from_uri_and_language("untitled://1", "json", "postgres"),
            "mongodb"
        );
    }

    #[test]
    fn falls_back_to_configured_default_dialect() {
        assert_eq!(
            infer_dialect_from_uri_and_language("untitled://1", "sql", "postgres"),
            "postgres"
        );
        assert_eq!(
            infer_dialect_from_uri_and_language("untitled://1", "sql", "clickhouse"),
            "clickhouse"
        );
    }

    #[test]
    fn schema_match_score_requires_exact_table_matches() {
        let schema = test_schema("app", &["users", "orders"]);

        assert_eq!(
            calculate_schema_match_score(&["users".to_string()], &schema),
            10
        );
        assert_eq!(
            calculate_schema_match_score(&["users".to_string(), "orders".to_string()], &schema),
            24
        );
        assert_eq!(
            calculate_schema_match_score(&["app.users".to_string()], &schema),
            10
        );
        assert_eq!(
            calculate_schema_match_score(&["other.users".to_string()], &schema),
            0
        );
        assert_eq!(
            calculate_schema_match_score(&["user".to_string()], &schema),
            0
        );
        assert_eq!(
            calculate_schema_match_score(&["users_backup".to_string()], &schema),
            0
        );
    }

    #[test]
    fn schema_inference_uses_unique_highest_score() {
        let manager = SchemaManager::new();
        let app_id = manager.register(test_schema("app", &["users", "orders"]));
        manager.register(test_schema("audit", &["users"]));

        assert_eq!(
            infer_schema_id_from_tables(&["users".to_string(), "orders".to_string()], &manager),
            Some(app_id)
        );
    }

    #[test]
    fn schema_inference_ignores_cte_references_when_scoring_tables() {
        let manager = SchemaManager::new();
        let app_id = manager.register(test_schema("app", &["users"]));
        manager.register(test_schema("scratch", &["recent_users"]));
        let configured = DashMap::new();
        let inferred = DashMap::new();

        assert_eq!(
            schema_id_for_file(
                "file:///cte.sql",
                Some("WITH recent_users AS (SELECT * FROM app.users) SELECT * FROM recent_users"),
                &configured,
                &inferred,
                &manager,
            ),
            Some(app_id)
        );
    }

    #[test]
    fn schema_inference_rejects_ambiguous_equal_scores() {
        let manager = SchemaManager::new();
        manager.register(test_schema("app", &["users"]));
        manager.register(test_schema("audit", &["users"]));

        assert_eq!(
            infer_schema_id_from_tables(&["users".to_string()], &manager),
            None
        );
    }

    #[test]
    fn schema_id_for_file_prefers_configured_mapping_over_inference() {
        let manager = SchemaManager::new();
        let app_id = manager.register(test_schema("app", &["users"]));
        let audit_id = manager.register(test_schema("audit", &["events"]));
        let configured = DashMap::new();
        let inferred = DashMap::new();
        configured.insert("file:///query.sql".to_string(), app_id);
        inferred.insert("file:///query.sql".to_string(), audit_id);

        assert_eq!(
            schema_id_for_file(
                "file:///query.sql",
                Some("SELECT * FROM audit.events"),
                &configured,
                &inferred,
                &manager,
            ),
            Some(app_id)
        );
    }

    #[test]
    fn inferred_schema_cache_can_be_invalidated_after_text_changes() {
        let manager = SchemaManager::new();
        let app_id = manager.register(test_schema("app", &["users"]));
        let audit_id = manager.register(test_schema("audit", &["events"]));
        let configured = DashMap::new();
        let inferred = DashMap::new();
        let uri = "file:///query.sql";

        assert_eq!(
            schema_id_for_file(
                uri,
                Some("SELECT * FROM app.users"),
                &configured,
                &inferred,
                &manager,
            ),
            Some(app_id)
        );
        assert_eq!(
            schema_id_for_file(
                uri,
                Some("SELECT * FROM audit.events"),
                &configured,
                &inferred,
                &manager,
            ),
            Some(app_id),
            "cached inference should be used until the document invalidates it"
        );

        inferred.remove(uri);

        assert_eq!(
            schema_id_for_file(
                uri,
                Some("SELECT * FROM audit.events"),
                &configured,
                &inferred,
                &manager,
            ),
            Some(audit_id)
        );
    }

    #[test]
    fn schema_id_for_file_ignores_missing_configured_schema_ids() {
        let manager = SchemaManager::new();
        let app_id = manager.register(test_schema("app", &["users"]));
        let configured = DashMap::new();
        let inferred = DashMap::new();
        configured.insert("file:///query.sql".to_string(), SchemaId::new());

        assert_eq!(
            schema_id_for_file(
                "file:///query.sql",
                Some("SELECT * FROM app.users"),
                &configured,
                &inferred,
                &manager,
            ),
            Some(app_id)
        );
    }

    #[test]
    fn schema_id_for_file_clears_missing_inferred_schema_ids() {
        let manager = SchemaManager::new();
        let app_id = manager.register(test_schema("app", &["users"]));
        let configured = DashMap::new();
        let inferred = DashMap::new();
        let uri = "file:///query.sql";
        inferred.insert(uri.to_string(), SchemaId::new());

        assert_eq!(
            schema_id_for_file(
                uri,
                Some("SELECT * FROM app.users"),
                &configured,
                &inferred,
                &manager,
            ),
            Some(app_id)
        );
        assert_eq!(inferred.get(uri).map(|entry| *entry.value()), Some(app_id));
    }

    #[test]
    fn lsp_position_offsets_use_utf16_characters_and_byte_boundaries() {
        let text = "éabc\n😀z\r\ntail";

        assert_eq!(
            position_to_byte_offset(
                text,
                tower_lsp::lsp_types::Position {
                    line: 0,
                    character: 1,
                },
            ),
            "é".len()
        );
        assert_eq!(
            position_to_byte_offset(
                text,
                tower_lsp::lsp_types::Position {
                    line: 1,
                    character: 1,
                },
            ),
            "éabc\n".len(),
            "positions inside a surrogate pair should stay on the emoji boundary"
        );
        assert_eq!(
            position_to_byte_offset(
                text,
                tower_lsp::lsp_types::Position {
                    line: 1,
                    character: 2,
                },
            ),
            "éabc\n😀".len()
        );
        assert_eq!(
            position_to_byte_offset(
                text,
                tower_lsp::lsp_types::Position {
                    line: 2,
                    character: 4,
                },
            ),
            text.len()
        );
        assert_eq!(
            position_to_byte_offset(
                text,
                tower_lsp::lsp_types::Position {
                    line: 99,
                    character: 0,
                },
            ),
            text.len()
        );
    }

    #[test]
    fn detects_completed_sql_context_keyword_without_consuming_trailing_space() {
        let from_sql = "SELECT * from";
        assert_eq!(
            completed_sql_context_keyword_at_position(
                from_sql,
                Position {
                    line: 0,
                    character: from_sql.len() as u32,
                },
            )
            .as_deref(),
            Some("from")
        );

        let where_space_sql = "SELECT * FROM users where ";
        assert_eq!(
            completed_sql_context_keyword_at_position(
                where_space_sql,
                Position {
                    line: 0,
                    character: where_space_sql.len() as u32,
                },
            ),
            None
        );
    }

    #[test]
    fn completion_after_completed_keyword_uses_zero_width_text_edit() {
        let sql = "SELECT * from";
        let position = Position {
            line: 0,
            character: sql.len() as u32,
        };
        let mut items = vec![CompletionItem {
            label: "public.users".to_string(),
            kind: Some(CompletionItemKind::CLASS),
            insert_text: Some("public.users".to_string()),
            ..Default::default()
        }];

        apply_completed_sql_context_completion_edits(sql, position, &mut items);

        assert_eq!(items[0].filter_text.as_deref(), Some("from"));
        let Some(CompletionTextEdit::Edit(text_edit)) = &items[0].text_edit else {
            panic!("completion should use a plain text edit");
        };
        assert_eq!(text_edit.range.start, position);
        assert_eq!(text_edit.range.end, position);
        assert_eq!(text_edit.new_text, " public.users");
    }

    #[test]
    fn completion_after_from_keyword_drops_non_relation_items() {
        let sql = "SELECT * from";
        let position = Position {
            line: 0,
            character: sql.len() as u32,
        };
        let mut items = vec![
            CompletionItem {
                label: "public.webhook".to_string(),
                kind: Some(CompletionItemKind::CLASS),
                insert_text: Some("public.webhook".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "form_background_url".to_string(),
                kind: Some(CompletionItemKind::FIELD),
                insert_text: Some("form_background_url".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "FROM".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                insert_text: Some("FROM".to_string()),
                ..Default::default()
            },
        ];

        apply_completed_sql_context_completion_edits(sql, position, &mut items);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "public.webhook");
        assert_eq!(items[0].filter_text.as_deref(), Some("from"));
    }

    #[test]
    fn completion_after_where_keyword_drops_relation_and_operator_items() {
        let sql = "SELECT * FROM public.webhook where";
        let position = Position {
            line: 0,
            character: sql.len() as u32,
        };
        let mut items = vec![
            CompletionItem {
                label: "owner".to_string(),
                kind: Some(CompletionItemKind::FIELD),
                insert_text: Some("owner".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "public.form".to_string(),
                kind: Some(CompletionItemKind::CLASS),
                insert_text: Some("public.form".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "LIKE".to_string(),
                kind: Some(CompletionItemKind::OPERATOR),
                insert_text: Some("LIKE".to_string()),
                ..Default::default()
            },
        ];

        apply_completed_sql_context_completion_edits(sql, position, &mut items);

        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["owner"]
        );
        assert!(items
            .iter()
            .all(|item| item.filter_text.as_deref() == Some("where")));
    }

    #[test]
    fn completion_after_ddl_on_keyword_keeps_relation_items() {
        let sql = "CREATE INDEX webhook_owner_idx ON";
        let position = Position {
            line: 0,
            character: sql.len() as u32,
        };
        let mut items = vec![
            CompletionItem {
                label: "public.webhook".to_string(),
                kind: Some(CompletionItemKind::CLASS),
                insert_text: Some("public.webhook".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "owner".to_string(),
                kind: Some(CompletionItemKind::FIELD),
                insert_text: Some("owner".to_string()),
                ..Default::default()
            },
        ];

        apply_completed_sql_context_completion_edits(sql, position, &mut items);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "public.webhook");
        assert_eq!(items[0].filter_text.as_deref(), Some("on"));
    }

    #[test]
    fn completion_after_keyword_with_existing_space_keeps_original_insert_text() {
        let sql = "SELECT * FROM users where ";
        let position = Position {
            line: 0,
            character: sql.len() as u32,
        };
        let mut items = vec![CompletionItem {
            label: "id".to_string(),
            filter_text: Some("id".to_string()),
            insert_text: Some("id".to_string()),
            ..Default::default()
        }];

        apply_completed_sql_context_completion_edits(sql, position, &mut items);

        assert_eq!(items[0].filter_text.as_deref(), Some("id"));
        assert!(items[0].text_edit.is_none());
        assert_eq!(items[0].insert_text.as_deref(), Some("id"));
    }

    #[test]
    fn completion_after_qualified_identifier_replaces_full_identifier_path() {
        let sql = "SELECT * FROM public.us";
        let position = Position {
            line: 0,
            character: sql.len() as u32,
        };
        let mut items = vec![CompletionItem {
            label: "public.users".to_string(),
            insert_text: Some("public.users".to_string()),
            ..Default::default()
        }];

        apply_qualified_identifier_completion_edits(sql, position, &mut items);

        let Some(CompletionTextEdit::Edit(text_edit)) = &items[0].text_edit else {
            panic!("qualified completion should use a text edit");
        };
        assert_eq!(
            text_edit.range.start,
            Position {
                line: 0,
                character: "SELECT * FROM ".len() as u32,
            }
        );
        assert_eq!(text_edit.range.end, position);
        assert_eq!(text_edit.new_text, "public.users");
    }

    #[test]
    fn qualified_completion_edit_preserves_function_call_insert_text() {
        let sql = "SELECT test_db.calc";
        let position = Position {
            line: 0,
            character: sql.len() as u32,
        };
        let mut items = vec![CompletionItem {
            label: "test_db.calculate_score".to_string(),
            insert_text: Some("test_db.calculate_score()".to_string()),
            ..Default::default()
        }];

        apply_qualified_identifier_completion_edits(sql, position, &mut items);

        let Some(CompletionTextEdit::Edit(text_edit)) = &items[0].text_edit else {
            panic!("qualified function completion should use a text edit");
        };
        assert_eq!(text_edit.new_text, "test_db.calculate_score()");
    }

    #[test]
    fn qualified_completion_edit_does_not_replace_alias_member_columns() {
        let sql = "SELECT * FROM users u WHERE u.";
        let position = Position {
            line: 0,
            character: sql.len() as u32,
        };
        let mut items = vec![CompletionItem {
            label: "id".to_string(),
            insert_text: Some("id".to_string()),
            ..Default::default()
        }];

        apply_qualified_identifier_completion_edits(sql, position, &mut items);

        assert!(items[0].text_edit.is_none());
    }

    #[test]
    fn extracts_schema_qualifier_at_completion_position() {
        assert_eq!(
            schema_qualifier_at_position(
                "SELECT audit.calculate",
                tower_lsp::lsp_types::Position {
                    line: 0,
                    character: 22,
                },
            )
            .as_deref(),
            Some("audit")
        );
        assert_eq!(
            schema_qualifier_at_position(
                r#"SELECT "App Schema".calculate"#,
                tower_lsp::lsp_types::Position {
                    line: 0,
                    character: 29,
                },
            )
            .as_deref(),
            Some("App Schema")
        );
    }

    #[test]
    fn finds_schema_by_qualified_identifier_prefix() {
        let manager = SchemaManager::new();
        manager.register(test_schema("app", &["users"]));
        let audit_id = manager.register(test_schema("audit", &["events"]));

        assert_eq!(
            find_schema_by_qualifier(&manager, "audit").map(|schema| schema.id),
            Some(audit_id)
        );
        assert!(find_schema_by_qualifier(&manager, "missing").is_none());
    }

    #[test]
    fn finds_schema_by_table_reference_without_random_ambiguous_match() {
        let manager = SchemaManager::new();
        manager.register(test_schema("app", &["users"]));
        let audit_id = manager.register(test_schema("audit", &["users", "events"]));

        assert_eq!(
            find_schema_by_table_reference(&manager, "audit.users").map(|schema| schema.id),
            Some(audit_id)
        );
        assert!(find_schema_by_table_reference(&manager, "users").is_none());
    }

    #[test]
    fn uses_alias_table_reference_to_select_schema_at_column_completion() {
        let manager = SchemaManager::new();
        manager.register(test_schema("app", &["users"]));
        let audit_id = manager.register(test_schema("audit", &["users"]));
        let sql = "SELECT '😀', u. FROM audit.users u";
        let before_cursor = "SELECT '😀', u.";

        assert_eq!(
            schema_for_table_column_at_position(
                &manager,
                sql,
                tower_lsp::lsp_types::Position {
                    line: 0,
                    character: before_cursor.encode_utf16().count() as u32,
                },
            )
            .map(|schema| schema.id),
            Some(audit_id)
        );
    }

    #[test]
    fn uses_trailing_alias_member_to_select_schema_at_column_completion() {
        let manager = SchemaManager::new();
        manager.register(test_schema("app", &["users"]));
        let audit_id = manager.register(test_schema("audit", &["events"]));
        let sql = "SELECT * FROM audit.events e WHERE e.";

        assert_eq!(
            schema_for_table_column_at_position(
                &manager,
                sql,
                tower_lsp::lsp_types::Position {
                    line: 0,
                    character: sql.encode_utf16().count() as u32,
                },
            )
            .map(|schema| schema.id),
            Some(audit_id)
        );
    }
}
