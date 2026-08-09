use crate::dialect::Dialect;
use crate::dialects::DialectRegistry;
use crate::parser::SqlParser;
use crate::placeholder::SqlPlaceholderDialect;
use crate::position::lsp_position_at_end;
use crate::schema::{Column, Schema, SchemaId, SchemaManager, Table};
use crate::token::Keywords;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

const CURRENT_SQL_DOCUMENT_URI: &str = "file:///current.sql";
const CURRENT_JSON_DOCUMENT_URI: &str = "file:///current.json";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletionPreferences {
    #[serde(default = "default_keyword_case")]
    keyword_case: KeywordCase,
    #[serde(default)]
    table_alias: TableAliasStyle,
}

impl Default for CompletionPreferences {
    fn default() -> Self {
        Self {
            keyword_case: KeywordCase::Upper,
            table_alias: TableAliasStyle::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KeywordCase {
    Upper,
    Lower,
    Preserve,
}

fn default_keyword_case() -> KeywordCase {
    KeywordCase::Upper
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum TableAliasStyle {
    #[default]
    None,
    Initials,
}

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
    completion_preferences: Arc<RwLock<CompletionPreferences>>,
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
            completion_preferences: Arc::new(RwLock::new(CompletionPreferences::default())),
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
        let schema = schema_for_table_column_at_position(&self.schema_manager, text, position)
            .or_else(|| {
                schema_qualifier_at_position(text, position).and_then(|qualifier| {
                    find_schema_by_qualifier(&self.schema_manager, &qualifier)
                })
            })
            .or_else(|| self.get_schema_for_file(uri));
        schema.map(|schema| augment_schema_with_local_relations(schema, text, position, uri))
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
                let diagnostics = document_diagnostics(&*dialect, &text, schema.as_ref()).await;
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
        let raw_diagnostics = document_diagnostics(&*dialect, &text, schema.as_ref()).await;
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
        Ok(document_diagnostics(&*dialect, &params.text, schema.as_ref()).await)
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

/// Recover a high-confidence active statement for completion when users keep
/// multiple query blocks in one console but omit a semicolon between them.
///
/// This is intentionally narrower than the execution splitter. A blank line,
/// a top-level statement keyword, and balanced parentheses are required. CTE,
/// INSERT SELECT, EXPLAIN, CREATE body, ClickHouse mutation, and set-operation
/// continuations remain a single scope.
fn completion_statement_prefix(text: &str, position: Position) -> Option<(&str, Position)> {
    let cursor = position_to_byte_offset(text, position);
    let prefix = text.get(..cursor)?;
    let masked = SqlParser::mask_sql_noise(prefix);
    let semicolon_start = masked.rfind(';').map(|offset| offset + 1).unwrap_or(0);
    let start = SqlParser::active_statement_start(prefix);
    if start == semicolon_start {
        return None;
    }
    let scoped = text.get(start..cursor)?;
    Some((scoped, lsp_position_at_end(scoped)))
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

fn apply_completion_preferences(
    text: &str,
    position: Position,
    items: &mut [CompletionItem],
    preferences: &CompletionPreferences,
) {
    let add_table_alias = preferences.table_alias == TableAliasStyle::Initials
        && relation_alias_context_at_position(text, position);

    for item in items {
        if item.kind == Some(CompletionItemKind::KEYWORD) {
            let transform = |value: &str| match preferences.keyword_case {
                KeywordCase::Upper => value.to_ascii_uppercase(),
                KeywordCase::Lower => value.to_ascii_lowercase(),
                KeywordCase::Preserve => value.to_string(),
            };
            item.label = transform(&item.label);
            if let Some(insert_text) = item.insert_text.as_mut() {
                *insert_text = transform(insert_text);
            }
            update_completion_text_edit(item, &transform);
        } else if add_table_alias && is_relation_completion_kind(item.kind) {
            let Some(alias) = table_alias_initials(&item.label) else {
                continue;
            };
            let append_alias = |value: &str| format!("{value} ${{1:{alias}}}");
            if let Some(insert_text) = item.insert_text.as_mut() {
                *insert_text = append_alias(insert_text);
            } else {
                item.insert_text = Some(append_alias(&item.label));
            }
            item.insert_text_format = Some(InsertTextFormat::SNIPPET);
            update_completion_text_edit(item, &append_alias);
        }
    }
}

fn add_referenced_alias_completions(
    text: &str,
    position: Position,
    items: &mut Vec<CompletionItem>,
) {
    if relation_alias_context_at_position(text, position) {
        return;
    }
    let offset = position_to_byte_offset(text, position);
    let Some(text_before) = text.get(..offset) else {
        return;
    };
    let Some(prefix_start) = identifier_path_start_before_cursor(text_before) else {
        return;
    };
    let prefix_sql = text_before[prefix_start..].trim();
    if prefix_sql.is_empty() || prefix_sql.contains('.') {
        return;
    }
    let masked = SqlParser::mask_sql_noise(text_before);
    if masked
        .get(prefix_start..)
        .is_none_or(|prefix| prefix.trim().is_empty())
    {
        return;
    }
    let prefix = prefix_sql
        .trim_start_matches(['"', '`', '['])
        .trim_end_matches(['"', '`', ']'])
        .to_ascii_lowercase();
    if prefix.is_empty() {
        return;
    }

    // Alias discovery is a bounded, text-only scope scan.  It deliberately
    // avoids constructing a second tree-sitter parse after the dialect has
    // already produced its semantic completion candidates.
    let aliases = SqlParser::relation_aliases_at_position(text, position);
    for alias in aliases {
        let normalized = alias.name.to_ascii_lowercase();
        let initials = table_alias_initials(&alias.name).unwrap_or_default();
        if !normalized.starts_with(&prefix) && !initials.starts_with(&prefix) {
            continue;
        }
        if items.iter().any(|item| {
            item.kind == Some(CompletionItemKind::VARIABLE)
                && item.label.eq_ignore_ascii_case(&alias.name)
        }) {
            continue;
        }
        items.push(CompletionItem {
            label: alias.name.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some(format!("Table alias · {}", alias.relation)),
            filter_text: Some(alias.name.clone()),
            insert_text: Some(alias.sql),
            sort_text: Some(format!("-2:alias:{}", normalized)),
            ..Default::default()
        });
    }
}

fn keyword_with_completion_case(keyword: &str, keyword_case: KeywordCase) -> String {
    match keyword_case {
        KeywordCase::Upper | KeywordCase::Preserve => keyword.to_ascii_uppercase(),
        KeywordCase::Lower => keyword.to_ascii_lowercase(),
    }
}

const SQL_COMPLETION_IDENTIFIER_KEYWORDS: &[&str] = &[
    "all",
    "alter",
    "and",
    "as",
    "by",
    "case",
    "check",
    "constraint",
    "create",
    "default",
    "delete",
    "distinct",
    "drop",
    "else",
    "end",
    "except",
    "false",
    "foreign",
    "from",
    "group",
    "having",
    "index",
    "insert",
    "intersect",
    "into",
    "join",
    "key",
    "limit",
    "not",
    "null",
    "offset",
    "on",
    "or",
    "order",
    "primary",
    "recursive",
    "references",
    "returning",
    "select",
    "set",
    "table",
    "then",
    "true",
    "union",
    "unique",
    "update",
    "using",
    "values",
    "view",
    "when",
    "where",
    "with",
];

fn quote_completion_identifier(identifier: &str, dialect_name: &str) -> String {
    if ((identifier.starts_with('"') && identifier.ends_with('"'))
        || (identifier.starts_with('`') && identifier.ends_with('`'))
        || (identifier.starts_with('[') && identifier.ends_with(']')))
        && identifier.len() >= 2
    {
        return identifier.to_string();
    }
    let simple_lower = identifier
        .chars()
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_lowercase())
        && identifier.chars().skip(1).all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '$')
        });
    if simple_lower
        && !SQL_COMPLETION_IDENTIFIER_KEYWORDS
            .iter()
            .any(|keyword| identifier.eq_ignore_ascii_case(keyword))
    {
        return identifier.to_string();
    }

    if matches!(dialect_name, "mysql" | "hive" | "clickhouse") {
        format!("`{}`", identifier.replace('`', "``"))
    } else {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
}

fn add_insert_all_columns_completion(
    text: &str,
    position: Position,
    schema: Option<&Schema>,
    items: &mut Vec<CompletionItem>,
    preferences: &CompletionPreferences,
    dialect_name: &str,
) {
    let Some(schema) = schema else {
        return;
    };
    let cursor = position_to_byte_offset(text, position);
    let Some(text_before_cursor) = text.get(..cursor) else {
        return;
    };
    let statement_start = SqlParser::active_statement_start(text_before_cursor);
    let statement = &text_before_cursor[statement_start..];
    let masked = SqlParser::mask_sql_noise(statement);
    let trimmed = masked.trim_end();
    if !trimmed.ends_with('(') {
        return;
    }
    let upper = trimmed.to_ascii_uppercase();
    let Some(insert_offset) = upper.rfind("INSERT INTO") else {
        return;
    };
    let open_offset = trimmed.len().saturating_sub(1);
    let insert_tail = &upper[insert_offset + "INSERT INTO".len()..open_offset];
    if ["VALUES", "VALUE", "SELECT", "SET"]
        .iter()
        .any(|keyword| contains_sql_keyword(insert_tail, keyword))
        || insert_tail
            .chars()
            .any(|character| matches!(character, '(' | ')' | ','))
    {
        return;
    }

    let mut parser = SqlParser::new();
    let parse_result = parser.parse(statement);
    let Some(tree) = parse_result.tree.as_ref() else {
        return;
    };
    let references = parser.extract_referenced_tables_at_position(
        tree,
        statement,
        SqlParser::lsp_position_to_byte_position(statement, lsp_position_at_end(statement)),
    );
    let Some(reference) = references.last() else {
        return;
    };
    let Some(table) = schema
        .tables
        .iter()
        .find(|table| schema_table_matches(schema, reference, table))
    else {
        return;
    };
    if table.columns.is_empty() {
        return;
    }

    let label = format!("{}.*", table.name);
    if items.iter().any(|item| item.label == label) {
        return;
    }
    let columns = table
        .columns
        .iter()
        .map(|column| quote_completion_identifier(&column.name, dialect_name))
        .collect::<Vec<_>>();
    let values = columns
        .iter()
        .enumerate()
        .map(|(index, _)| format!("${{{}:value}}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let values_keyword = keyword_with_completion_case("VALUES", preferences.keyword_case);
    let insert_text = format!("{}) {values_keyword} ({values})", columns.join(", "));
    items.push(CompletionItem {
        label,
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(format!(
            "{} columns: {}) {} (value, ...)",
            columns.len(),
            columns.join(", "),
            values_keyword
        )),
        sort_text: Some(format!("0:0000:{}", table.name.to_ascii_lowercase())),
        filter_text: Some(table.name.clone()),
        insert_text: Some(insert_text),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    });
}

fn update_completion_text_edit(item: &mut CompletionItem, transform: &impl Fn(&str) -> String) {
    match item.text_edit.as_mut() {
        Some(CompletionTextEdit::Edit(edit)) => edit.new_text = transform(&edit.new_text),
        Some(CompletionTextEdit::InsertAndReplace(edit)) => {
            edit.new_text = transform(&edit.new_text)
        }
        None => {}
    }
}

fn relation_alias_context_at_position(text: &str, position: Position) -> bool {
    let offset = position_to_byte_offset(text, position);
    let Some(text_before) = text.get(..offset) else {
        return false;
    };
    let mut last_clause = None;
    for token in text_before
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .filter(|token| !token.is_empty())
    {
        let token = token.to_ascii_uppercase();
        if matches!(
            token.as_str(),
            "SELECT"
                | "FROM"
                | "JOIN"
                | "WHERE"
                | "ON"
                | "GROUP"
                | "HAVING"
                | "ORDER"
                | "LIMIT"
                | "UNION"
                | "VALUES"
                | "SET"
                | "INTO"
        ) {
            last_clause = Some(token);
        }
    }
    matches!(last_clause.as_deref(), Some("FROM" | "JOIN"))
}

fn table_alias_initials(label: &str) -> Option<String> {
    let relation = label.rsplit('.').next()?.trim_matches(['`', '"', '[', ']']);
    let mut alias = String::new();
    let mut take_next = true;
    let mut previous_lowercase = false;
    for character in relation.chars() {
        if !character.is_ascii_alphanumeric() {
            take_next = true;
            previous_lowercase = false;
            continue;
        }
        if take_next || (previous_lowercase && character.is_ascii_uppercase()) {
            alias.push(character.to_ascii_lowercase());
            if alias.len() == 4 {
                break;
            }
        }
        take_next = false;
        previous_lowercase = character.is_ascii_lowercase();
    }
    (!alias.is_empty()).then_some(alias)
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
    let mut score = 0;

    for table_name in tables {
        if schema
            .tables
            .iter()
            .any(|table| schema_table_matches(schema, table_name, table))
        {
            score += 10;
        }
    }

    let matched_count = tables
        .iter()
        .filter(|table_name| {
            schema
                .tables
                .iter()
                .any(|table| schema_table_matches(schema, table_name, table))
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
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: Some(vec![",".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR_REWRITE,
                            CodeActionKind::SOURCE,
                        ]),
                        resolve_provider: Some(false),
                        ..Default::default()
                    },
                )),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_range_formatting_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: semantic_tokens_legend(),
                            range: Some(false),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            ..Default::default()
                        },
                    ),
                ),
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        resolve_provider: Some(false),
                        ..Default::default()
                    },
                ))),
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

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let diagnostics = if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_file(&uri);
            document_diagnostics(&*dialect, &text, schema.as_ref()).await
        } else {
            Vec::new()
        };
        Ok(DocumentDiagnosticReportResult::Report(
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diagnostics,
                },
            }),
        ))
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

            if let Some(preferences_value) = settings.get("completionPreferences") {
                match serde_json::from_value::<CompletionPreferences>(preferences_value.clone()) {
                    Ok(preferences) => {
                        if let Ok(mut current_preferences) = self.completion_preferences.write() {
                            *current_preferences = preferences;
                        }
                    }
                    Err(error) => {
                        self.client
                            .log_message(
                                MessageType::WARNING,
                                format!("Ignoring invalid completion preferences: {error}"),
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
            let diagnostics = document_diagnostics(&*dialect, &text, schema.as_ref()).await;
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
                        let diagnostics =
                            document_diagnostics(&*dialect, &current_text, schema.as_ref()).await;
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
                    let diagnostics = document_diagnostics(&*dialect, &text, schema.as_ref()).await;
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
            let (completion_text, completion_position) =
                completion_statement_prefix(&text, position).unwrap_or((&text, position));
            let mut items = dialect
                .completion_with_context(
                    completion_text,
                    completion_position,
                    schema.as_ref(),
                    params.context.as_ref(),
                )
                .await;
            let preferences = self
                .completion_preferences
                .read()
                .map(|preferences| preferences.clone())
                .unwrap_or_default();
            if matches!(
                dialect.name(),
                "postgres" | "mysql" | "sqlite" | "hive" | "clickhouse"
            ) {
                add_insert_all_columns_completion(
                    &text,
                    position,
                    schema.as_ref(),
                    &mut items,
                    &preferences,
                    dialect.name(),
                );
            }
            add_referenced_alias_completions(&text, position, &mut items);
            apply_completed_sql_context_completion_edits(&text, position, &mut items);
            apply_qualified_identifier_completion_edits(&text, position, &mut items);
            if matches!(
                dialect.name(),
                "postgres" | "mysql" | "sqlite" | "hive" | "clickhouse"
            ) {
                apply_completion_preferences(&text, position, &mut items, &preferences);
            }
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
            if locations.is_empty() {
                return Ok(Some(locations));
            }

            if let Some(identifier) = identifier_at_position(&text, position, dialect.name()) {
                let schema_id = schema.as_ref().map(|schema| schema.id);
                for (candidate_uri, candidate_text) in self.document_manager.entries() {
                    if candidate_uri == uri {
                        continue;
                    }
                    let Some(candidate_dialect) = self.get_dialect_for_file(&candidate_uri) else {
                        continue;
                    };
                    if candidate_dialect.name() != dialect.name() {
                        continue;
                    }
                    if let Some(expected_schema_id) = schema_id {
                        if self
                            .get_schema_for_file(&candidate_uri)
                            .is_none_or(|candidate_schema| {
                                candidate_schema.id != expected_schema_id
                            })
                        {
                            continue;
                        }
                    }
                    let Ok(candidate_url) = Url::parse(&candidate_uri) else {
                        continue;
                    };
                    locations.extend(identifier_references(
                        &candidate_text,
                        &identifier,
                        dialect.name(),
                        &candidate_url,
                    ));
                }
                deduplicate_locations(&mut locations);
            }
            return Ok(Some(locations));
        }

        Ok(None)
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let Some((routine_name, active_parameter)) = routine_call_at_position(&text, position)
        else {
            return Ok(None);
        };
        let Some(schema) = self.get_schema_for_position(&uri, &text, position) else {
            return Ok(None);
        };
        let signatures = schema
            .functions
            .iter()
            .filter(|function| function.name.eq_ignore_ascii_case(&routine_name))
            .map(|function| SignatureInformation {
                label: function.signature(),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: function.markdown_documentation(),
                })),
                parameters: Some(
                    function
                        .parameters
                        .iter()
                        .map(|parameter| ParameterInformation {
                            label: ParameterLabel::Simple(if parameter.name.trim().is_empty() {
                                parameter.data_type.clone()
                            } else {
                                format!("{} {}", parameter.name, parameter.data_type)
                            }),
                            documentation: None,
                        })
                        .collect(),
                ),
                active_parameter: Some(active_parameter),
            })
            .collect::<Vec<_>>();
        if signatures.is_empty() {
            return Ok(None);
        }
        Ok(Some(SignatureHelp {
            signatures,
            active_signature: Some(0),
            active_parameter: Some(active_parameter),
        }))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let Some(dialect) = self.get_dialect_for_file(&uri) else {
            return Ok(None);
        };
        if !supports_semantic_rename(dialect.name()) {
            return Ok(None);
        }
        let schema = self.get_schema_for_position(&uri, &text, params.position);
        if dialect
            .references(&text, params.position, schema.as_ref())
            .await
            .is_empty()
        {
            return Ok(None);
        }
        Ok(
            identifier_range_at_position(&text, params.position, dialect.name())
                .map(PrepareRenameResponse::Range),
        )
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let Some(dialect) = self.get_dialect_for_file(&uri) else {
            return Ok(None);
        };
        if !supports_semantic_rename(dialect.name()) || !valid_renamed_identifier(&params.new_name)
        {
            return Ok(None);
        }
        let schema = self.get_schema_for_position(&uri, &text, position);
        if dialect
            .references(&text, position, schema.as_ref())
            .await
            .is_empty()
        {
            return Ok(None);
        }
        let Some(identifier) = identifier_at_position(&text, position, dialect.name()) else {
            return Ok(None);
        };
        let schema_id = schema.as_ref().map(|schema| schema.id);
        let mut changes = HashMap::new();
        for (candidate_uri, candidate_text) in self.document_manager.entries() {
            let Some(candidate_dialect) = self.get_dialect_for_file(&candidate_uri) else {
                continue;
            };
            if candidate_dialect.name() != dialect.name() {
                continue;
            }
            if let Some(expected_schema_id) = schema_id {
                if self
                    .get_schema_for_file(&candidate_uri)
                    .is_none_or(|candidate_schema| candidate_schema.id != expected_schema_id)
                {
                    continue;
                }
            }
            let Ok(candidate_url) = Url::parse(&candidate_uri) else {
                continue;
            };
            let edits =
                identifier_references(&candidate_text, &identifier, dialect.name(), &candidate_url)
                    .into_iter()
                    .map(|location| TextEdit {
                        range: location.range,
                        new_text: params.new_name.clone(),
                    })
                    .collect::<Vec<_>>();
            if !edits.is_empty() {
                changes.insert(candidate_url, edits);
            }
        }
        if changes.is_empty() {
            return Ok(None);
        }
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let uri_string = uri.to_string();
        let text = self.document_manager.get(&uri_string).unwrap_or_default();
        let Some(dialect) = self.get_dialect_for_file(&uri_string) else {
            return Ok(None);
        };
        let formatted = dialect.format(&text).await;
        let mut actions = Vec::new();
        if formatted != text {
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Format document with database dialect".to_string(),
                kind: Some(CodeActionKind::SOURCE),
                edit: Some(single_document_edit(
                    uri.clone(),
                    Range::new(Position::new(0, 0), lsp_position_at_end(&text)),
                    formatted,
                )),
                ..Default::default()
            }));
        }
        if let Some(action) = expand_select_star_action(
            &text,
            &uri,
            params.range,
            self.get_schema_for_file(&uri_string),
            dialect.name(),
        ) {
            actions.push(CodeActionOrCommand::CodeAction(action));
        }
        for diagnostic in &params.context.diagnostics {
            if matches!(
                diagnostic.code.as_ref(),
                Some(NumberOrString::String(code)) if code == "OXIDE001"
            ) {
                if let Some(action) = add_mutation_safety_guard_action(&text, &uri, diagnostic) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
            }
        }
        Ok(Some(actions))
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

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let Some(dialect) = self.get_dialect_for_file(&uri) else {
            return Ok(None);
        };
        let start = position_to_byte_offset(&text, params.range.start);
        let end = position_to_byte_offset(&text, params.range.end);
        let Some(selected) = text.get(start.min(end)..end.max(start)) else {
            return Ok(None);
        };
        let formatted = dialect.format(selected).await;
        Ok(Some(vec![TextEdit {
            range: params.range,
            new_text: formatted,
        }]))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let symbols = document_symbols(&text, self.get_schema_for_file(&uri).as_ref());
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let text = self
            .document_manager
            .get(params.text_document.uri.as_str())
            .unwrap_or_default();
        Ok(Some(folding_ranges(&text)))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let dialect_name = self
            .get_dialect_for_file(&uri)
            .map(|dialect| dialect.name().to_string())
            .unwrap_or_else(|| "postgres".to_string());
        Ok(Some(
            params
                .positions
                .into_iter()
                .map(|position| selection_range_for_position(&text, position, &dialect_name))
                .collect(),
        ))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let schema = self.get_schema_for_file(&uri);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: schema_semantic_tokens(&text, schema.as_ref()),
        })))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();
        let Some(schema) = self.get_schema_for_file(&uri) else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(routine_parameter_hints(&text, params.range, &schema)))
    }

    #[allow(deprecated)]
    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.trim().to_ascii_lowercase();
        let mut symbols = Vec::new();
        for schema_id in self.schema_manager.list_ids() {
            let Some(schema) = self.schema_manager.get(schema_id) else {
                continue;
            };
            for table in &schema.tables {
                if workspace_symbol_matches(&query, &table.name, &schema.database) {
                    if let Some(location) = crate::dialects::common::metadata_location(
                        table.source_location.as_ref(),
                        schema.source_uri.as_ref(),
                        "file:///schema.sql",
                    ) {
                        symbols.push(SymbolInformation {
                            name: table.name.clone(),
                            kind: SymbolKind::CLASS,
                            tags: None,
                            deprecated: None,
                            location,
                            container_name: Some(schema.database.clone()),
                        });
                    }
                }
                for column in &table.columns {
                    if !workspace_symbol_matches(&query, &column.name, &table.name) {
                        continue;
                    }
                    if let Some(location) = crate::dialects::common::metadata_location(
                        column
                            .source_location
                            .as_ref()
                            .or(table.source_location.as_ref()),
                        schema.source_uri.as_ref(),
                        "file:///schema.sql",
                    ) {
                        symbols.push(SymbolInformation {
                            name: column.name.clone(),
                            kind: SymbolKind::FIELD,
                            tags: None,
                            deprecated: None,
                            location,
                            container_name: Some(format!("{}.{}", schema.database, table.name)),
                        });
                    }
                }
            }
            for function in &schema.functions {
                if !workspace_symbol_matches(&query, &function.name, &schema.database) {
                    continue;
                }
                if let Some(location) = crate::dialects::common::metadata_location(
                    None,
                    schema.source_uri.as_ref(),
                    "file:///schema.sql",
                ) {
                    symbols.push(SymbolInformation {
                        name: function.name.clone(),
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        deprecated: None,
                        location,
                        container_name: Some(schema.database.clone()),
                    });
                }
            }
        }
        symbols.sort_by(|left, right| {
            workspace_symbol_rank(&query, &left.name)
                .cmp(&workspace_symbol_rank(&query, &right.name))
                .then_with(|| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                })
                .then_with(|| left.container_name.cmp(&right.container_name))
        });
        symbols.truncate(1_000);
        Ok(Some(symbols))
    }
}

fn identifier_at_position(text: &str, position: Position, dialect: &str) -> Option<String> {
    let offset = position_to_byte_offset(text, position).min(text.len());
    let mut start = offset;
    while start > 0 {
        let ch = text[..start].chars().next_back()?;
        if !is_identifier_character(ch, dialect) {
            break;
        }
        start -= ch.len_utf8();
    }
    let mut end = offset;
    while end < text.len() {
        let ch = text[end..].chars().next()?;
        if !is_identifier_character(ch, dialect) {
            break;
        }
        end += ch.len_utf8();
    }
    let identifier = text[start..end]
        .trim_matches(|character| matches!(character, '"' | '\'' | '`'))
        .to_string();
    (!identifier.is_empty()).then_some(identifier)
}

fn identifier_references(text: &str, identifier: &str, dialect: &str, uri: &Url) -> Vec<Location> {
    let needle = identifier.to_ascii_lowercase();
    text.lines()
        .enumerate()
        .flat_map(|(line_index, line)| {
            let searchable = line.to_ascii_lowercase();
            searchable
                .match_indices(&needle)
                .filter_map(move |(start, matched)| {
                    let end = start + matched.len();
                    let before = line[..start].chars().next_back();
                    let after = line[end..].chars().next();
                    if before.is_some_and(|ch| is_identifier_character(ch, dialect))
                        || after.is_some_and(|ch| is_identifier_character(ch, dialect))
                    {
                        return None;
                    }
                    Some(Location {
                        uri: uri.clone(),
                        range: Range {
                            start: Position {
                                line: line_index as u32,
                                character: line[..start].encode_utf16().count() as u32,
                            },
                            end: Position {
                                line: line_index as u32,
                                character: line[..end].encode_utf16().count() as u32,
                            },
                        },
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn is_identifier_character(character: char, dialect: &str) -> bool {
    character.is_alphanumeric()
        || character == '_'
        || match dialect {
            "redis" => matches!(character, ':' | '-' | '.' | '@'),
            "mongodb" | "elasticsearch-dsl" | "elasticsearch-eql" => {
                matches!(character, '.' | '-' | '@')
            }
            _ => character == '$',
        }
}

fn deduplicate_locations(locations: &mut Vec<Location>) {
    let mut seen = HashSet::new();
    locations.retain(|location| {
        seen.insert((
            location.uri.to_string(),
            location.range.start.line,
            location.range.start.character,
            location.range.end.line,
            location.range.end.character,
        ))
    });
    locations.sort_by(|left, right| {
        left.uri
            .as_str()
            .cmp(right.uri.as_str())
            .then_with(|| left.range.start.line.cmp(&right.range.start.line))
            .then_with(|| left.range.start.character.cmp(&right.range.start.character))
    });
}

fn supports_semantic_rename(dialect: &str) -> bool {
    matches!(dialect, "postgres" | "mysql" | "clickhouse" | "hive")
}

fn workspace_symbol_matches(query: &str, name: &str, container: &str) -> bool {
    query.is_empty()
        || name.to_ascii_lowercase().contains(query)
        || container.to_ascii_lowercase().contains(query)
}

fn workspace_symbol_rank(query: &str, name: &str) -> u8 {
    let name = name.to_ascii_lowercase();
    if query.is_empty() || name == query {
        0
    } else if name.starts_with(query) {
        1
    } else {
        2
    }
}

async fn document_diagnostics(
    dialect: &dyn Dialect,
    text: &str,
    schema: Option<&Schema>,
) -> Vec<Diagnostic> {
    let mut diagnostics = dialect.parse(text, schema).await;
    if matches!(dialect.name(), "postgres" | "mysql" | "clickhouse" | "hive") {
        diagnostics.extend(sql_inspection_diagnostics(text, schema, dialect.name()));
    }
    diagnostics
}

fn sql_inspection_diagnostics(
    text: &str,
    schema: Option<&Schema>,
    dialect_name: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut offset = 0usize;
    for segment in text.split_inclusive(';') {
        let trimmed_start = segment.len() - segment.trim_start().len();
        let trimmed_end = segment.trim_end().len();
        if trimmed_start >= trimmed_end {
            offset += segment.len();
            continue;
        }
        let statement_start = offset + trimmed_start;
        let statement_end = offset + trimmed_end;
        let statement = &text[statement_start..statement_end];
        let normalized = SqlParser::mask_sql_noise(statement).to_ascii_uppercase();
        let statement_range = range_for_offsets(text, statement_start, statement_end);

        let mutating_without_where = (normalized.trim_start().starts_with("UPDATE ")
            || normalized.trim_start().starts_with("DELETE "))
            && !contains_sql_keyword(&normalized, "WHERE");
        if mutating_without_where && !inspection_suppressed(statement, "OXIDE001") {
            diagnostics.push(Diagnostic {
                range: statement_range,
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String("OXIDE001".to_string())),
                code_description: None,
                source: Some("oxide-inspections".to_string()),
                message: "Mutation has no WHERE clause and may affect every matching row."
                    .to_string(),
                related_information: None,
                tags: None,
                data: None,
            });
        }

        if let Some(join_relative) = join_without_condition_offset(&normalized) {
            if !inspection_suppressed(statement, "OXIDE002") {
                diagnostics.push(Diagnostic {
                    range: range_for_offsets(
                        text,
                        statement_start + join_relative,
                        statement_start + join_relative + "JOIN".len(),
                    ),
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("OXIDE002".to_string())),
                    code_description: None,
                    source: Some("oxide-inspections".to_string()),
                    message:
                        "JOIN has no ON or USING condition and may create a Cartesian product."
                            .to_string(),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }

        if let Some(star_relative) = select_star_offset(&normalized) {
            if !inspection_suppressed(statement, "OXIDE003") {
                diagnostics.push(Diagnostic {
                    range: range_for_offsets(
                        text,
                        statement_start + star_relative,
                        statement_start + star_relative + 1,
                    ),
                    severity: Some(DiagnosticSeverity::HINT),
                    code: Some(NumberOrString::String("OXIDE003".to_string())),
                    code_description: None,
                    source: Some("oxide-inspections".to_string()),
                    message: "Explicit columns are safer when a schema changes.".to_string(),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }

        if !inspection_suppressed(statement, "OXIDE004") {
            diagnostics.extend(ambiguous_column_diagnostics(
                text,
                statement,
                statement_start,
                schema,
            ));
        }

        if !inspection_suppressed(statement, "OXIDE005") {
            diagnostics.extend(dialect_risk_diagnostics(
                text,
                statement,
                statement_start,
                dialect_name,
            ));
        }

        offset += segment.len();
    }
    diagnostics
}

fn inspection_suppressed(statement: &str, code: &str) -> bool {
    statement
        .to_ascii_uppercase()
        .contains(&format!("NOINSPECTION {}", code.to_ascii_uppercase()))
}

fn join_without_condition_offset(normalized: &str) -> Option<usize> {
    for (offset, _) in normalized.match_indices("JOIN") {
        let before = normalized[..offset].trim_end();
        let keyword_before = before.split_whitespace().next_back().unwrap_or_default();
        if matches!(keyword_before, "CROSS" | "NATURAL") {
            continue;
        }
        let tail = &normalized[offset + "JOIN".len()..];
        let boundary = [
            " JOIN ", " WHERE ", " GROUP ", " HAVING ", " ORDER ", " LIMIT ", " UNION ",
        ]
        .iter()
        .filter_map(|keyword| tail.find(keyword))
        .min()
        .unwrap_or(tail.len());
        let join_clause = &tail[..boundary];
        if !contains_sql_keyword(join_clause, "ON") && !contains_sql_keyword(join_clause, "USING") {
            return Some(offset);
        }
    }
    None
}

fn ambiguous_column_diagnostics(
    full_text: &str,
    statement: &str,
    statement_start: usize,
    schema: Option<&Schema>,
) -> Vec<Diagnostic> {
    let Some(schema) = schema else {
        return Vec::new();
    };
    let mut parser = SqlParser::new();
    let Some(tree) = parser.parse(statement).tree else {
        return Vec::new();
    };
    let referenced = parser.extract_tables(&tree, statement);
    if referenced.len() < 2 {
        return Vec::new();
    }
    let referenced_tables = schema
        .tables
        .iter()
        .filter(|table| {
            referenced
                .iter()
                .any(|reference| schema_table_matches(schema, reference, table))
        })
        .collect::<Vec<_>>();
    if referenced_tables.len() < 2 {
        return Vec::new();
    }
    let mut counts = HashMap::<String, usize>::new();
    for table in referenced_tables {
        for column in &table.columns {
            *counts.entry(column.name.to_ascii_lowercase()).or_default() += 1;
        }
    }
    let ambiguous = counts
        .into_iter()
        .filter_map(|(name, count)| (count > 1).then_some(name))
        .collect::<HashSet<_>>();
    if ambiguous.is_empty() {
        return Vec::new();
    }

    sql_identifier_tokens(statement)
        .into_iter()
        .filter_map(|(identifier, start, end)| {
            if !ambiguous.contains(&identifier.to_ascii_lowercase()) {
                return None;
            }
            let qualified = statement[..start]
                .chars()
                .next_back()
                .is_some_and(|character| character == '.');
            if qualified {
                return None;
            }
            Some(Diagnostic {
                range: range_for_offsets(
                    full_text,
                    statement_start + start,
                    statement_start + end,
                ),
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String("OXIDE004".to_string())),
                code_description: None,
                source: Some("oxide-inspections".to_string()),
                message: format!(
                    "Column `{identifier}` exists in multiple referenced relations; qualify it with an alias."
                ),
                related_information: None,
                tags: None,
                data: None,
            })
        })
        .collect()
}

fn dialect_risk_diagnostics(
    full_text: &str,
    statement: &str,
    statement_start: usize,
    dialect_name: &str,
) -> Vec<Diagnostic> {
    if dialect_name == "postgres" {
        return Vec::new();
    }
    let normalized = SqlParser::mask_sql_noise(statement).to_ascii_uppercase();
    let mut risks = Vec::new();
    for (needle, message) in [
        (
            "ILIKE",
            "ILIKE is PostgreSQL-specific and is not portable to this dialect.",
        ),
        (
            "::",
            "The :: cast syntax is PostgreSQL-specific; use CAST(value AS type).",
        ),
    ] {
        for (start, matched) in normalized.match_indices(needle) {
            risks.push(Diagnostic {
                range: range_for_offsets(
                    full_text,
                    statement_start + start,
                    statement_start + start + matched.len(),
                ),
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String("OXIDE005".to_string())),
                code_description: None,
                source: Some("oxide-inspections".to_string()),
                message: message.to_string(),
                related_information: None,
                tags: None,
                data: None,
            });
        }
    }
    risks
}

fn contains_sql_keyword(normalized: &str, keyword: &str) -> bool {
    normalized.match_indices(keyword).any(|(start, matched)| {
        let end = start + matched.len();
        let before = normalized[..start].chars().next_back();
        let after = normalized[end..].chars().next();
        !before.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
            && !after.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn select_star_offset(normalized: &str) -> Option<usize> {
    let select = normalized.find("SELECT")?;
    let mut offset = select + "SELECT".len();
    while normalized[offset..]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        offset += normalized[offset..].chars().next()?.len_utf8();
    }
    (normalized[offset..].starts_with('*')).then_some(offset)
}

fn valid_renamed_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

fn identifier_range_at_position(text: &str, position: Position, dialect: &str) -> Option<Range> {
    let identifier = identifier_at_position(text, position, dialect)?;
    let offset = position_to_byte_offset(text, position).min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line_end = text[offset..]
        .find('\n')
        .map_or(text.len(), |index| offset + index);
    let line = &text[line_start..line_end];
    let cursor = offset.saturating_sub(line_start).min(line.len());
    let searchable = line.to_ascii_lowercase();
    let needle = identifier.to_ascii_lowercase();
    let (start, matched) = searchable
        .match_indices(&needle)
        .find(|(start, matched)| *start <= cursor && cursor <= *start + matched.len())?;
    let line_number = text[..line_start].matches('\n').count() as u32;
    Some(Range {
        start: Position {
            line: line_number,
            character: line[..start].encode_utf16().count() as u32,
        },
        end: Position {
            line: line_number,
            character: line[..start + matched.len()].encode_utf16().count() as u32,
        },
    })
}

fn routine_call_at_position(text: &str, position: Position) -> Option<(String, u32)> {
    let offset = position_to_byte_offset(text, position).min(text.len());
    let before = &text[..offset];
    let mut nesting = 0u32;
    let mut opening = None;
    for (index, character) in before.char_indices().rev() {
        match character {
            ')' => nesting += 1,
            '(' if nesting > 0 => nesting -= 1,
            '(' => {
                opening = Some(index);
                break;
            }
            _ => {}
        }
    }
    let opening = opening?;
    let mut name_end = opening;
    while name_end > 0
        && text[..name_end]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        name_end -= text[..name_end].chars().next_back()?.len_utf8();
    }
    let mut name_start = name_end;
    while name_start > 0 {
        let character = text[..name_start].chars().next_back()?;
        if !(character.is_alphanumeric() || matches!(character, '_' | '.' | '"' | '`')) {
            break;
        }
        name_start -= character.len_utf8();
    }
    let routine_name = text[name_start..name_end]
        .trim_matches(|character| matches!(character, '"' | '`'))
        .rsplit('.')
        .next()?
        .to_string();
    if routine_name.is_empty() {
        return None;
    }

    let mut active_parameter = 0u32;
    let mut nested = 0u32;
    let mut quote = None;
    for character in text[opening + 1..offset].chars() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
            continue;
        }
        match character {
            '(' | '[' | '{' => nested += 1,
            ')' | ']' | '}' if nested > 0 => nested -= 1,
            ',' if nested == 0 => active_parameter += 1,
            _ => {}
        }
    }
    Some((routine_name, active_parameter))
}

fn single_document_edit(uri: Url, range: Range, new_text: String) -> WorkspaceEdit {
    WorkspaceEdit {
        changes: Some(HashMap::from([(uri, vec![TextEdit { range, new_text }])])),
        document_changes: None,
        change_annotations: None,
    }
}

fn add_mutation_safety_guard_action(
    text: &str,
    uri: &Url,
    diagnostic: &Diagnostic,
) -> Option<CodeAction> {
    let start = position_to_byte_offset(text, diagnostic.range.start);
    let end = position_to_byte_offset(text, diagnostic.range.end).min(text.len());
    let statement = text.get(start..end)?;
    let trailing = statement.trim_end();
    let insert_offset = if trailing.ends_with(';') {
        start + trailing.len() - 1
    } else {
        start + trailing.len()
    };
    let insert_range = range_for_offsets(text, insert_offset, insert_offset);
    Some(CodeAction {
        title: "Add non-matching WHERE safety guard".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(single_document_edit(
            uri.clone(),
            insert_range,
            " WHERE 1 = 0 /* replace safety guard before executing */".to_string(),
        )),
        is_preferred: Some(true),
        ..Default::default()
    })
}

fn expand_select_star_action(
    text: &str,
    uri: &Url,
    request_range: Range,
    schema: Option<Schema>,
    dialect_name: &str,
) -> Option<CodeAction> {
    let schema = schema?;
    let selection_start = position_to_byte_offset(text, request_range.start);
    let selection_end = position_to_byte_offset(text, request_range.end);
    let star = text.match_indices('*').find_map(|(offset, _)| {
        let selected = selection_start <= offset && offset <= selection_end.max(selection_start);
        let nearby = offset.abs_diff(selection_start) <= 2;
        (selected || nearby).then_some(offset)
    })?;
    let prefix = text[..star].to_ascii_uppercase();
    let select_start = prefix.rfind("SELECT")?;
    let mut projection_prefix = text[select_start + "SELECT".len()..star].trim();
    if projection_prefix
        .get(.."DISTINCT".len())
        .is_some_and(|value| value.eq_ignore_ascii_case("DISTINCT"))
    {
        projection_prefix = projection_prefix["DISTINCT".len()..].trim();
    }
    let qualifier_sql = if projection_prefix.is_empty() {
        None
    } else if projection_prefix.ends_with('.') {
        let qualifier = projection_prefix.trim_end_matches('.').trim();
        (!qualifier.is_empty()).then_some(qualifier.to_string())
    } else {
        return None;
    };
    let qualifier = qualifier_sql
        .as_deref()
        .map(SqlParser::normalize_identifier);
    let statement_start = text[..star].rfind(';').map_or(0, |offset| offset + 1);
    let statement_end = text[star..]
        .find(';')
        .map_or(text.len(), |offset| star + offset + 1);
    let statement = &text[statement_start..statement_end];
    let mut parser = SqlParser::new();
    let parse_result = parser.parse(statement);
    let tree = parse_result.tree.as_ref()?;
    let references = parser.extract_referenced_tables(tree, statement);
    let aliases = parser.extract_aliases(tree, statement);
    let mut sources = Vec::<(&Table, String)>::new();

    if let Some(qualifier) = qualifier.as_deref() {
        let target = aliases
            .get(qualifier)
            .map(String::as_str)
            .unwrap_or(qualifier);
        let table = schema
            .tables
            .iter()
            .find(|table| schema_table_matches(&schema, target, table))?;
        sources.push((table, qualifier_sql.clone()?));
    } else {
        if references.is_empty() {
            return None;
        }
        let mut used_aliases = HashSet::new();
        for reference in references {
            let table = schema
                .tables
                .iter()
                .find(|table| schema_table_matches(&schema, &reference, table))?;
            let mut matching_aliases = aliases
                .iter()
                .filter(|(alias, target)| {
                    !used_aliases.contains(*alias) && schema_table_matches(&schema, target, table)
                })
                .map(|(alias, _)| alias.clone())
                .collect::<Vec<_>>();
            matching_aliases.sort();
            let source_qualifier = matching_aliases
                .into_iter()
                .next()
                .unwrap_or_else(|| SqlParser::identifier_last_part(&reference));
            used_aliases.insert(source_qualifier.clone());
            sources.push((table, source_qualifier));
        }
    }
    if sources.is_empty() || sources.iter().any(|(table, _)| table.columns.is_empty()) {
        return None;
    }

    let qualified_expansion = qualifier.is_none() && sources.len() > 1;
    let mut replacement_columns = Vec::new();
    for (table, source_qualifier) in &sources {
        for (index, column) in table.columns.iter().enumerate() {
            let column_name = if qualifier.is_some() {
                // The original `alias.` remains before the replacement range.
                if index == 0 {
                    quote_completion_identifier(&column.name, dialect_name)
                } else {
                    format!(
                        "{}.{}",
                        quote_completion_identifier(source_qualifier, dialect_name),
                        quote_completion_identifier(&column.name, dialect_name)
                    )
                }
            } else if qualified_expansion {
                format!(
                    "{}.{}",
                    quote_completion_identifier(source_qualifier, dialect_name),
                    quote_completion_identifier(&column.name, dialect_name)
                )
            } else {
                quote_completion_identifier(&column.name, dialect_name)
            };
            replacement_columns.push(column_name);
        }
    }
    let replacement = replacement_columns.join(", ");
    let range = range_for_offsets(text, star, star + 1);
    Some(CodeAction {
        title: format!("Expand * to {} columns", replacement_columns.len()),
        kind: Some(CodeActionKind::REFACTOR_REWRITE),
        edit: Some(single_document_edit(uri.clone(), range, replacement)),
        is_preferred: Some(true),
        ..Default::default()
    })
}

#[allow(deprecated)]
fn document_symbols(text: &str, schema: Option<&Schema>) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    let mut seen = HashSet::new();
    if let Some(schema) = schema {
        for table in &schema.tables {
            if let Some(range) = range_for_identifier_occurrence(text, &table.name) {
                if seen.insert(("table", table.name.to_ascii_lowercase())) {
                    symbols.push(DocumentSymbol {
                        name: table.name.clone(),
                        detail: Some(table.object_kind().to_string()),
                        kind: SymbolKind::CLASS,
                        tags: None,
                        deprecated: None,
                        range,
                        selection_range: range,
                        children: None,
                    });
                }
            }
        }
        for function in &schema.functions {
            if let Some(range) = range_for_identifier_occurrence(text, &function.name) {
                if seen.insert(("function", function.name.to_ascii_lowercase())) {
                    symbols.push(DocumentSymbol {
                        name: function.name.clone(),
                        detail: Some(function.signature()),
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        deprecated: None,
                        range,
                        selection_range: range,
                        children: None,
                    });
                }
            }
        }
    }
    let parser = SqlParser::new();
    for cte in parser.extract_common_table_expressions(text) {
        if let Some(range) = range_for_identifier_occurrence(text, &cte) {
            if seen.insert(("cte", cte.to_ascii_lowercase())) {
                symbols.push(DocumentSymbol {
                    name: cte,
                    detail: Some("Common table expression".to_string()),
                    kind: SymbolKind::VARIABLE,
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range: range,
                    children: None,
                });
            }
        }
    }
    symbols.sort_by_key(|symbol| {
        (
            symbol.range.start.line,
            symbol.range.start.character,
            symbol.name.clone(),
        )
    });
    symbols
}

fn range_for_identifier_occurrence(text: &str, identifier: &str) -> Option<Range> {
    let searchable = text.to_ascii_lowercase();
    let needle = identifier.to_ascii_lowercase();
    searchable
        .match_indices(&needle)
        .find_map(|(start, matched)| {
            let end = start + matched.len();
            let before = text[..start].chars().next_back();
            let after = text[end..].chars().next();
            if before.is_some_and(|character| character.is_alphanumeric() || character == '_')
                || after.is_some_and(|character| character.is_alphanumeric() || character == '_')
            {
                return None;
            }
            Some(range_for_offsets(text, start, end))
        })
}

fn range_for_offsets(text: &str, start: usize, end: usize) -> Range {
    Range {
        start: position_for_offset(text, start),
        end: position_for_offset(text, end),
    }
}

fn position_for_offset(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let prefix = &text[..offset];
    let line = prefix.matches('\n').count() as u32;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    Position {
        line,
        character: text[line_start..offset].encode_utf16().count() as u32,
    }
}

fn folding_ranges(text: &str) -> Vec<FoldingRange> {
    let mut stack = Vec::new();
    let mut ranges = Vec::new();
    let mut line = 0u32;
    let mut character = 0u32;
    let mut quote = None;
    for current in text.chars() {
        if current == '\n' {
            line += 1;
            character = 0;
            continue;
        }
        if let Some(active_quote) = quote {
            if current == active_quote {
                quote = None;
            }
            character += current.len_utf16() as u32;
            continue;
        }
        if matches!(current, '\'' | '"' | '`') {
            quote = Some(current);
            character += 1;
            continue;
        }
        match current {
            '(' | '[' | '{' => stack.push((current, line, character)),
            ')' | ']' | '}' => {
                let expected = match current {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                if let Some(index) = stack.iter().rposition(|(open, _, _)| *open == expected) {
                    let (_, start_line, start_character) = stack.remove(index);
                    if start_line < line {
                        ranges.push(FoldingRange {
                            start_line,
                            start_character: Some(start_character),
                            end_line: line,
                            end_character: Some(character),
                            kind: None,
                            collapsed_text: None,
                        });
                    }
                }
            }
            _ => {}
        }
        character += current.len_utf16() as u32;
    }
    ranges.sort_by_key(|range| (range.start_line, range.start_character));
    ranges.truncate(500);
    ranges
}

fn selection_range_for_position(text: &str, position: Position, dialect: &str) -> SelectionRange {
    let document_range = Range::new(Position::new(0, 0), lsp_position_at_end(text));
    let (line_start, line_end) = line_bounds_for_position(text, position.line);
    let line_range = range_for_offsets(text, line_start, line_end);
    let parent = SelectionRange {
        range: line_range,
        parent: Some(Box::new(SelectionRange {
            range: document_range,
            parent: None,
        })),
    };
    SelectionRange {
        range: identifier_range_at_position(text, position, dialect)
            .unwrap_or(Range::new(position, position)),
        parent: Some(Box::new(parent)),
    }
}

const LOCAL_RELATION_SCAN_MAX_BYTES: usize = 512 * 1024;

fn augment_schema_with_local_relations(
    mut schema: Schema,
    text: &str,
    position: Position,
    uri: &str,
) -> Schema {
    let cursor = position_to_byte_offset(text, position).min(text.len());
    let visible = &text[..cursor];
    // Completion runs on every keystroke. Keep local-relation discovery
    // bounded rather than repeatedly scanning an arbitrarily large console.
    if visible.len() > LOCAL_RELATION_SCAN_MAX_BYTES {
        return schema;
    }
    // Reuse one length-preserving searchable copy. Besides avoiding three
    // uppercase allocations, masking prevents WITH/FROM/JOIN/CREATE tokens in
    // comments, literals and quoted identifiers from becoming fake relations.
    let searchable_upper = SqlParser::mask_sql_noise(visible).to_ascii_uppercase();
    let mut local_relations =
        temporary_tables_before_cursor(visible, &searchable_upper, &schema, uri);
    local_relations.extend(cte_tables_before_cursor(
        visible,
        &searchable_upper,
        &schema,
        uri,
    ));
    local_relations.extend(derived_tables_before_cursor(
        visible,
        &searchable_upper,
        &schema,
        uri,
    ));
    let mut relation_schema = schema.clone();
    for table in &local_relations {
        relation_schema
            .tables
            .retain(|candidate| !candidate.name.eq_ignore_ascii_case(&table.name));
        relation_schema.tables.push(table.clone());
    }
    local_relations.extend(correlated_tables_before_cursor(
        visible,
        &searchable_upper,
        &relation_schema,
        uri,
    ));

    // Apply in discovery order so the most local shape wins. A correlation
    // column list shadows remote/CTE names positionally, and a CTE shadows a
    // temporary relation with the same name.
    for table in local_relations {
        schema
            .tables
            .retain(|candidate| !candidate.name.eq_ignore_ascii_case(&table.name));
        schema.tables.insert(0, table);
    }
    schema
}

fn temporary_tables_before_cursor(
    source: &str,
    upper: &str,
    schema: &Schema,
    uri: &str,
) -> Vec<Table> {
    let mut tables = HashMap::<String, (usize, Table)>::new();
    for keyword in ["CREATE TEMP TABLE", "CREATE TEMPORARY TABLE"] {
        let mut search = 0;
        while let Some(relative) = upper[search..].find(keyword) {
            let start = search + relative;
            let after_keyword = start + keyword.len();
            let name_position =
                skip_optional_keyword_sequence(source, after_keyword, &["IF", "NOT", "EXISTS"]);
            let Some((name, name_start, name_end)) =
                read_local_identifier_after(source, name_position)
            else {
                search = after_keyword;
                continue;
            };
            let definition_start = skip_local_whitespace(source, name_end);
            let source_line = source[..name_start].matches('\n').count() as u32 + 1;
            let (columns, statement_end) = if source[definition_start..].starts_with('(') {
                let Some(closing) = matching_parenthesis_in_searchable(upper, definition_start)
                else {
                    search = definition_start + 1;
                    continue;
                };
                (
                    split_top_level_ranges_in_searchable(upper, definition_start + 1, closing)
                        .into_iter()
                        .filter_map(|(column_start, column_end)| {
                            parse_temporary_column(
                                &source[column_start..column_end],
                                uri,
                                source[..column_start].matches('\n').count() as u32 + 1,
                            )
                        })
                        .collect(),
                    closing + 1,
                )
            } else if upper[definition_start..].starts_with("AS")
                && local_keyword_boundary(&upper, definition_start, "AS")
            {
                let query_start = skip_local_whitespace(source, definition_start + "AS".len());
                let statement_end = source[query_start..]
                    .find(';')
                    .map_or(source.len(), |relative| query_start + relative);
                (
                    infer_query_output_columns(
                        &source[query_start..statement_end],
                        schema,
                        uri,
                        source_line,
                    ),
                    statement_end,
                )
            } else {
                search = name_end;
                continue;
            };
            let table =
                local_relation_table(name.clone(), "TEMPORARY TABLE", columns, uri, source_line);
            tables.insert(name.to_ascii_lowercase(), (start, table));
            search = statement_end;
        }
    }

    for keyword in ["DROP TABLE", "DROP TEMP TABLE", "DROP TEMPORARY TABLE"] {
        let mut search = 0;
        while let Some(relative) = upper[search..].find(keyword) {
            let start = search + relative;
            let after_keyword = start + keyword.len();
            let name_position =
                skip_optional_keyword_sequence(source, after_keyword, &["IF", "EXISTS"]);
            if let Some((name, _, name_end)) = read_local_identifier_after(source, name_position) {
                let normalized = name.to_ascii_lowercase();
                if tables
                    .get(&normalized)
                    .is_some_and(|(created_at, _)| *created_at < start)
                {
                    tables.remove(&normalized);
                }
                search = name_end;
            } else {
                search = after_keyword;
            }
        }
    }

    let mut tables = tables.into_values().collect::<Vec<_>>();
    tables.sort_by_key(|(offset, _)| *offset);
    tables.into_iter().map(|(_, table)| table).collect()
}

fn parse_temporary_column(definition: &str, uri: &str, line: u32) -> Option<Column> {
    let (name, _, name_end) = read_local_identifier_after(definition, 0)?;
    if matches!(
        name.to_ascii_uppercase().as_str(),
        "PRIMARY" | "UNIQUE" | "CONSTRAINT" | "CHECK" | "FOREIGN" | "EXCLUDE"
    ) {
        return None;
    }
    let (data_type, _, _) = read_local_identifier_after(definition, name_end)
        .unwrap_or_else(|| ("unknown".to_string(), name_end, name_end));
    Some(Column {
        name,
        data_type,
        nullable: !definition.to_ascii_uppercase().contains("NOT NULL"),
        primary_key: definition.to_ascii_uppercase().contains("PRIMARY KEY"),
        unique: definition.to_ascii_uppercase().contains("UNIQUE"),
        indexed: false,
        comment: Some("Column from a temporary table in the current console".to_string()),
        source_location: Some((uri.to_string(), line)),
    })
}

fn cte_tables_before_cursor(source: &str, upper: &str, schema: &Schema, uri: &str) -> Vec<Table> {
    let mut tables = Vec::new();
    let mut search = 0;
    while let Some(relative) = upper[search..].find("WITH") {
        let with_start = search + relative;
        if !local_keyword_boundary(&upper, with_start, "WITH") {
            search = with_start + "WITH".len();
            continue;
        }
        let mut cursor = skip_local_whitespace(source, with_start + "WITH".len());
        if upper[cursor..].starts_with("RECURSIVE")
            && local_keyword_boundary(&upper, cursor, "RECURSIVE")
        {
            cursor = skip_local_whitespace(source, cursor + "RECURSIVE".len());
        }
        loop {
            let Some((name, name_start, name_end)) = read_local_identifier_after(source, cursor)
            else {
                break;
            };
            cursor = skip_local_whitespace(source, name_end);
            let mut explicit_columns = Vec::new();
            if source[cursor..].starts_with('(') {
                let Some(closing) = matching_parenthesis_in_searchable(upper, cursor) else {
                    break;
                };
                explicit_columns = split_top_level_ranges_in_searchable(upper, cursor + 1, closing)
                    .into_iter()
                    .filter_map(|(start, _)| {
                        read_local_identifier_after(source, start).map(|(name, _, _)| name)
                    })
                    .collect();
                cursor = skip_local_whitespace(source, closing + 1);
            }
            if !upper[cursor..].starts_with("AS") || !local_keyword_boundary(&upper, cursor, "AS") {
                break;
            }
            cursor = skip_local_whitespace(source, cursor + "AS".len());
            if !source[cursor..].starts_with('(') {
                break;
            }
            let Some(closing) = matching_parenthesis_in_searchable(upper, cursor) else {
                break;
            };
            let body = &source[cursor + 1..closing];
            let source_line = source[..name_start].matches('\n').count() as u32 + 1;
            let columns = if explicit_columns.is_empty() {
                infer_query_output_columns(body, schema, uri, source_line)
            } else {
                explicit_columns
                    .into_iter()
                    .map(|column_name| {
                        local_relation_column(column_name, "unknown", uri, source_line)
                    })
                    .collect()
            };
            tables.push(local_relation_table(
                name,
                "COMMON TABLE EXPRESSION",
                columns,
                uri,
                source_line,
            ));
            cursor = skip_local_whitespace(source, closing + 1);
            if !source[cursor..].starts_with(',') {
                break;
            }
            cursor = skip_local_whitespace(source, cursor + 1);
        }
        search = cursor.max(with_start + "WITH".len());
    }
    tables
}

fn derived_tables_before_cursor(
    source: &str,
    upper: &str,
    schema: &Schema,
    uri: &str,
) -> Vec<Table> {
    let mut tables = Vec::new();
    for source_start in local_relation_source_starts(upper) {
        let mut opening = skip_local_whitespace(source, source_start);
        let after_lateral = skip_optional_keyword_sequence(source, opening, &["LATERAL"]);
        if after_lateral != opening {
            opening = after_lateral;
        }
        if !source[opening..].starts_with('(') {
            continue;
        }
        let Some(closing) = matching_parenthesis_in_searchable(upper, opening) else {
            continue;
        };
        let mut alias_start = skip_local_whitespace(source, closing + 1);
        if upper[alias_start..].starts_with("AS")
            && local_keyword_boundary(upper, alias_start, "AS")
        {
            alias_start = skip_local_whitespace(source, alias_start + "AS".len());
        }
        let Some((alias, alias_offset, alias_end)) =
            read_local_identifier_after(source, alias_start)
        else {
            continue;
        };
        if Keywords::is_keyword(&alias) {
            continue;
        }
        let body = &source[opening + 1..closing];
        if !upper[opening + 1..closing].contains("SELECT") {
            continue;
        }
        let source_line = source[..alias_offset].matches('\n').count() as u32 + 1;
        let columns = merge_local_column_aliases(
            infer_query_output_columns(body, schema, uri, source_line),
            correlation_column_names_after(source, upper, alias_end).unwrap_or_default(),
            uri,
            source_line,
        );
        tables.push(local_relation_table(
            alias,
            "DERIVED TABLE",
            columns,
            uri,
            source_line,
        ));
    }
    tables
}

fn correlated_tables_before_cursor(
    source: &str,
    upper: &str,
    schema: &Schema,
    uri: &str,
) -> Vec<Table> {
    let mut tables = Vec::new();
    for source_start in local_relation_source_starts(upper) {
        let mut cursor = skip_local_whitespace(source, source_start);
        for modifier in ["LATERAL", "ONLY"] {
            let advanced = skip_optional_keyword_sequence(source, cursor, &[modifier]);
            if advanced != cursor {
                cursor = advanced;
            }
        }
        let Some((reference, reference_start, reference_end)) =
            read_local_identifier_path_after(source, cursor)
        else {
            continue;
        };
        cursor = skip_local_whitespace(source, reference_end);
        let is_table_function = source[cursor..].starts_with('(');
        if is_table_function {
            let Some(closing) = matching_parenthesis_in_searchable(upper, cursor) else {
                continue;
            };
            cursor = skip_local_whitespace(source, closing + 1);
            let after_ordinality =
                skip_optional_keyword_sequence(source, cursor, &["WITH", "ORDINALITY"]);
            if after_ordinality != cursor {
                cursor = after_ordinality;
            }
        }
        let after_as = skip_optional_keyword_sequence(source, cursor, &["AS"]);
        if after_as != cursor {
            cursor = after_as;
        }
        let Some((alias, alias_start, alias_end)) = read_local_identifier_after(source, cursor)
        else {
            continue;
        };
        if Keywords::is_keyword(&alias) {
            continue;
        }
        cursor = skip_local_whitespace(source, alias_end);
        // SQL Server table hints are not PostgreSQL correlation columns. WITH
        // makes the distinction unambiguous; the legacy `(NOLOCK)` form is
        // filtered by the known hint vocabulary below.
        if upper[cursor..].starts_with("WITH") && local_keyword_boundary(upper, cursor, "WITH") {
            continue;
        }
        let Some(column_aliases) = correlation_column_names_after(source, upper, cursor) else {
            continue;
        };
        if column_aliases.is_empty() || is_sqlserver_table_hint_list(&column_aliases) {
            continue;
        }

        let table_name = SqlParser::identifier_last_part(&reference);
        let base_columns = schema
            .tables
            .iter()
            .find(|table| schema_table_matches(schema, &reference, table))
            .map(|table| table.columns.clone())
            .unwrap_or_default();
        let source_line = source[..alias_start.min(reference_start)]
            .matches('\n')
            .count() as u32
            + 1;
        let columns = merge_local_column_aliases(base_columns, column_aliases, uri, source_line);
        tables.push(local_relation_table(
            table_name,
            if is_table_function {
                "TABLE FUNCTION"
            } else {
                "CORRELATED TABLE"
            },
            columns,
            uri,
            source_line,
        ));
    }
    tables
}

fn merge_local_column_aliases(
    mut columns: Vec<Column>,
    aliases: Vec<String>,
    uri: &str,
    line: u32,
) -> Vec<Column> {
    if columns.is_empty() {
        return aliases
            .into_iter()
            .map(|name| local_relation_column(name, "unknown", uri, line))
            .collect();
    }
    for (column, alias) in columns.iter_mut().zip(aliases) {
        column.name = alias;
        column.comment = Some("Column renamed by a correlation list".to_string());
        column.source_location = Some((uri.to_string(), line));
    }
    columns
}

fn correlation_column_names_after(source: &str, upper: &str, start: usize) -> Option<Vec<String>> {
    let opening = skip_local_whitespace(source, start);
    if !source[opening..].starts_with('(') {
        return None;
    }
    let closing = matching_parenthesis_in_searchable(upper, opening)?;
    let mut names = Vec::new();
    for (item_start, item_end) in split_top_level_ranges_in_searchable(upper, opening + 1, closing)
    {
        let (name, _, name_end) = read_local_identifier_after(source, item_start)?;
        if !upper[name_end..item_end].trim().is_empty() {
            return None;
        }
        names.push(name);
    }
    Some(names)
}

fn is_sqlserver_table_hint_list(names: &[String]) -> bool {
    const TABLE_HINTS: &[&str] = &[
        "FORCESEEK",
        "FORCESCAN",
        "HOLDLOCK",
        "INDEX",
        "NOEXPAND",
        "NOLOCK",
        "NOWAIT",
        "PAGLOCK",
        "READCOMMITTED",
        "READCOMMITTEDLOCK",
        "READPAST",
        "READUNCOMMITTED",
        "REPEATABLEREAD",
        "ROWLOCK",
        "SERIALIZABLE",
        "SNAPSHOT",
        "SPATIAL_WINDOW_MAX_CELLS",
        "TABLOCK",
        "TABLOCKX",
        "UPDLOCK",
        "XLOCK",
    ];
    names.iter().any(|name| {
        TABLE_HINTS
            .iter()
            .any(|hint| name.eq_ignore_ascii_case(hint))
    })
}

fn infer_query_output_columns(
    query: &str,
    schema: &Schema,
    uri: &str,
    source_line: u32,
) -> Vec<Column> {
    let upper = query.to_ascii_uppercase();
    let Some(select) = upper.find("SELECT") else {
        return Vec::new();
    };
    let projection_start = select + "SELECT".len();
    let projection_end =
        find_top_level_keyword(query, projection_start, "FROM").unwrap_or(query.len());
    let mut columns = Vec::new();
    for (start, end) in split_top_level_ranges(query, projection_start, projection_end) {
        let expression = query[start..end].trim();
        if expression.is_empty() {
            continue;
        }
        if expression == "*" || expression.ends_with(".*") {
            let mut parser = SqlParser::new();
            if let Some(tree) = parser.parse(query).tree {
                let references = parser.extract_referenced_tables(&tree, query);
                for table in &schema.tables {
                    if references
                        .iter()
                        .any(|reference| schema_table_matches(schema, reference, table))
                    {
                        columns.extend(table.columns.clone());
                    }
                }
            }
            continue;
        }
        let name = select_expression_output_name(expression);
        let Some(name) = name else {
            continue;
        };
        if columns
            .iter()
            .any(|column: &Column| column.name.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        let data_type = schema
            .tables
            .iter()
            .flat_map(|table| table.columns.iter())
            .find(|column| column.name.eq_ignore_ascii_case(&name))
            .map(|column| column.data_type.clone())
            .unwrap_or_else(|| "unknown".to_string());
        columns.push(local_relation_column(name, &data_type, uri, source_line));
    }
    columns
}

fn select_expression_output_name(expression: &str) -> Option<String> {
    let upper = SqlParser::mask_sql_noise(expression).to_ascii_uppercase();
    if let Some(as_offset) = upper.rfind(" AS ") {
        return read_local_identifier_after(expression, as_offset + " AS ".len())
            .map(|(name, _, _)| name);
    }
    let trimmed = expression.trim();
    if trimmed
        .chars()
        .all(|character| character.is_alphanumeric() || matches!(character, '_' | '$' | '.'))
    {
        return trimmed.rsplit('.').next().map(str::to_string);
    }
    let trailing = trimmed
        .split_whitespace()
        .next_back()
        .filter(|value| !Keywords::is_keyword(value))?;
    trailing
        .chars()
        .all(|character| character.is_alphanumeric() || matches!(character, '_' | '$'))
        .then(|| trailing.to_string())
}

fn local_relation_table(
    name: String,
    object_type: &str,
    columns: Vec<Column>,
    uri: &str,
    line: u32,
) -> Table {
    Table {
        name,
        object_type: Some(object_type.to_string()),
        columns,
        indexes: Vec::new(),
        constraints: Vec::new(),
        comment: Some(format!("{object_type} visible in the current console")),
        source_location: Some((uri.to_string(), line)),
    }
}

fn local_relation_column(name: String, data_type: &str, uri: &str, line: u32) -> Column {
    Column {
        name,
        data_type: data_type.to_string(),
        nullable: true,
        primary_key: false,
        unique: false,
        indexed: false,
        comment: Some("Output column inferred from the current console".to_string()),
        source_location: Some((uri.to_string(), line)),
    }
}

fn read_local_identifier_after(source: &str, start: usize) -> Option<(String, usize, usize)> {
    let start = skip_local_whitespace(source, start);
    let first = source[start..].chars().next()?;
    if matches!(first, '"' | '`' | '[') {
        let closing = if first == '[' { ']' } else { first };
        let content_start = start + first.len_utf8();
        let relative_end = source[content_start..].find(closing)?;
        let content_end = content_start + relative_end;
        return Some((
            source[content_start..content_end].to_string(),
            start,
            content_end + closing.len_utf8(),
        ));
    }
    if !(first == '_' || first.is_alphabetic()) {
        return None;
    }
    let mut end = start + first.len_utf8();
    for character in source[end..].chars() {
        if !(character == '_' || character == '$' || character.is_alphanumeric()) {
            break;
        }
        end += character.len_utf8();
    }
    Some((source[start..end].to_string(), start, end))
}

fn read_local_identifier_path_after(source: &str, start: usize) -> Option<(String, usize, usize)> {
    let (first, path_start, mut path_end) = read_local_identifier_after(source, start)?;
    let mut parts = vec![first];
    loop {
        let dot = skip_local_whitespace(source, path_end);
        if !source[dot..].starts_with('.') {
            break;
        }
        let Some((part, _, part_end)) = read_local_identifier_after(source, dot + 1) else {
            break;
        };
        parts.push(part);
        path_end = part_end;
    }
    Some((parts.join("."), path_start, path_end))
}

fn local_relation_source_starts(source_upper: &str) -> Vec<usize> {
    const FROM_BOUNDARIES: &[&str] = &[
        "WHERE",
        "GROUP",
        "HAVING",
        "ORDER",
        "LIMIT",
        "OFFSET",
        "FETCH",
        "WINDOW",
        "QUALIFY",
        "RETURNING",
        "UNION",
        "EXCEPT",
        "INTERSECT",
    ];
    let mut starts = Vec::new();
    for keyword in ["FROM", "JOIN", "APPLY"] {
        let mut search = 0;
        while let Some(relative) = source_upper[search..].find(keyword) {
            let position = search + relative;
            if local_keyword_boundary(source_upper, position, keyword) {
                starts.push(position + keyword.len());
            }
            search = position + keyword.len();
        }
    }
    let mut from_search = 0;
    while let Some(relative) = source_upper[from_search..].find("FROM") {
        let from_position = from_search + relative;
        if !local_keyword_boundary(source_upper, from_position, "FROM") {
            from_search = from_position + "FROM".len();
            continue;
        }
        let mut cursor = from_position + "FROM".len();
        let mut depth = 0u32;
        while cursor < source_upper.len() {
            let Some(character) = source_upper[cursor..].chars().next() else {
                break;
            };
            if depth == 0 {
                if matches!(character, ';' | ')')
                    || FROM_BOUNDARIES.iter().any(|keyword| {
                        source_upper[cursor..].starts_with(keyword)
                            && local_keyword_boundary(source_upper, cursor, keyword)
                    })
                {
                    break;
                }
                if character == ',' {
                    starts.push(cursor + character.len_utf8());
                }
            }
            match character {
                '(' => depth += 1,
                ')' if depth > 0 => depth -= 1,
                _ => {}
            }
            cursor += character.len_utf8();
        }
        from_search = (from_position + "FROM".len()).max(cursor);
    }
    starts.sort_unstable();
    starts.dedup();
    starts
}

fn skip_local_whitespace(source: &str, mut offset: usize) -> usize {
    offset = offset.min(source.len());
    while offset < source.len() {
        let Some(character) = source[offset..].chars().next() else {
            break;
        };
        if !character.is_whitespace() {
            break;
        }
        offset += character.len_utf8();
    }
    offset
}

fn skip_optional_keyword_sequence(source: &str, start: usize, keywords: &[&str]) -> usize {
    let upper = source.to_ascii_uppercase();
    let original = skip_local_whitespace(source, start);
    let mut cursor = original;
    for keyword in keywords {
        if !upper[cursor..].starts_with(keyword) || !local_keyword_boundary(&upper, cursor, keyword)
        {
            return original;
        }
        cursor = skip_local_whitespace(source, cursor + keyword.len());
    }
    cursor
}

fn local_keyword_boundary(source_upper: &str, start: usize, keyword: &str) -> bool {
    let end = start + keyword.len();
    let before = source_upper[..start].chars().next_back();
    let after = source_upper[end.min(source_upper.len())..].chars().next();
    !before.is_some_and(|character| character.is_alphanumeric() || character == '_')
        && !after.is_some_and(|character| character.is_alphanumeric() || character == '_')
}

fn matching_parenthesis_in_searchable(searchable: &str, opening: usize) -> Option<usize> {
    if !searchable[opening..].starts_with('(') {
        return None;
    }
    let mut nesting = 0u32;
    for (relative, character) in searchable[opening..].char_indices() {
        match character {
            '(' => nesting += 1,
            ')' => {
                nesting = nesting.saturating_sub(1);
                if nesting == 0 {
                    return Some(opening + relative);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_ranges(source: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let searchable = SqlParser::mask_sql_noise(source);
    split_top_level_ranges_in_searchable(&searchable, start, end)
}

fn split_top_level_ranges_in_searchable(
    searchable: &str,
    start: usize,
    end: usize,
) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut segment_start = start;
    let mut nesting = 0u32;
    for (relative, character) in searchable[start..end].char_indices() {
        let offset = start + relative;
        match character {
            '(' | '[' | '{' => nesting += 1,
            ')' | ']' | '}' if nesting > 0 => nesting -= 1,
            ',' if nesting == 0 => {
                ranges.push((segment_start, offset));
                segment_start = offset + character.len_utf8();
            }
            _ => {}
        }
    }
    ranges.push((segment_start, end));
    ranges
}

fn find_top_level_keyword(source: &str, start: usize, keyword: &str) -> Option<usize> {
    let upper = SqlParser::mask_sql_noise(source).to_ascii_uppercase();
    let mut nesting = 0u32;
    for (relative, character) in upper[start..].char_indices() {
        let offset = start + relative;
        match character {
            '(' | '[' | '{' => nesting += 1,
            ')' | ']' | '}' if nesting > 0 => nesting -= 1,
            _ if nesting == 0
                && upper[offset..].starts_with(keyword)
                && local_keyword_boundary(&upper, offset, keyword) =>
            {
                return Some(offset);
            }
            _ => {}
        }
    }
    None
}

fn semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::NAMESPACE,
            SemanticTokenType::CLASS,
            SemanticTokenType::PROPERTY,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::VARIABLE,
        ],
        token_modifiers: Vec::new(),
    }
}

fn schema_semantic_tokens(text: &str, schema: Option<&Schema>) -> Vec<SemanticToken> {
    let Some(schema) = schema else {
        return Vec::new();
    };
    let tables = schema
        .tables
        .iter()
        .map(|table| table.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let columns = schema
        .tables
        .iter()
        .flat_map(|table| table.columns.iter())
        .map(|column| column.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let functions = schema
        .functions
        .iter()
        .map(|function| function.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let ctes = SqlParser::new()
        .extract_common_table_expressions(text)
        .into_iter()
        .map(|cte| cte.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let database = schema.database.to_ascii_lowercase();

    let mut absolute = Vec::new();
    for (identifier, start, end) in sql_identifier_tokens(text) {
        let normalized = identifier.to_ascii_lowercase();
        let following_open_parenthesis = text[end..]
            .chars()
            .find(|character| !character.is_whitespace())
            == Some('(');
        let token_type = if functions.contains(&normalized) && following_open_parenthesis {
            3
        } else if tables.contains(&normalized) {
            1
        } else if columns.contains(&normalized) {
            2
        } else if ctes.contains(&normalized) {
            4
        } else if normalized == database {
            0
        } else {
            continue;
        };
        let position = position_for_offset(text, start);
        let length = text[start..end].encode_utf16().count() as u32;
        if length > 0 {
            absolute.push((position.line, position.character, length, token_type));
        }
    }
    absolute.sort_unstable();
    absolute.dedup();

    let mut previous_line = 0;
    let mut previous_character = 0;
    absolute
        .into_iter()
        .map(|(line, character, length, token_type)| {
            let delta_line = line - previous_line;
            let delta_start = if delta_line == 0 {
                character - previous_character
            } else {
                character
            };
            previous_line = line;
            previous_character = character;
            SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type,
                token_modifiers_bitset: 0,
            }
        })
        .collect()
}

fn sql_identifier_tokens(text: &str) -> Vec<(String, usize, usize)> {
    let mut tokens = Vec::new();
    let mut characters = text.char_indices().peekable();
    let mut quote = None;
    let mut line_comment = false;
    let mut block_comment = false;

    while let Some((index, character)) = characters.next() {
        if line_comment {
            if character == '\n' {
                line_comment = false;
            }
            continue;
        }
        if block_comment {
            if character == '*' && characters.peek().is_some_and(|(_, next)| *next == '/') {
                characters.next();
                block_comment = false;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                if characters
                    .peek()
                    .is_some_and(|(_, next)| *next == active_quote)
                {
                    characters.next();
                } else {
                    quote = None;
                }
            }
            continue;
        }
        if character == '-' && characters.peek().is_some_and(|(_, next)| *next == '-') {
            characters.next();
            line_comment = true;
            continue;
        }
        if character == '/' && characters.peek().is_some_and(|(_, next)| *next == '*') {
            characters.next();
            block_comment = true;
            continue;
        }
        if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
            continue;
        }
        if !(character == '_' || character.is_alphabetic()) {
            continue;
        }

        let start = index;
        let mut end = index + character.len_utf8();
        while let Some((next_index, next)) = characters.peek().copied() {
            if !(next == '_' || next == '$' || next.is_alphanumeric()) {
                break;
            }
            characters.next();
            end = next_index + next.len_utf8();
        }
        tokens.push((text[start..end].to_string(), start, end));
    }
    tokens
}

fn routine_parameter_hints(text: &str, range: Range, schema: &Schema) -> Vec<InlayHint> {
    let range_start = position_to_byte_offset(text, range.start);
    let range_end = position_to_byte_offset(text, range.end);
    let mut hints = Vec::new();

    for (identifier, _, name_end) in sql_identifier_tokens(text) {
        let mut opening = name_end;
        while text[opening..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            let Some(character) = text[opening..].chars().next() else {
                break;
            };
            opening += character.len_utf8();
        }
        if !text[opening..].starts_with('(') {
            continue;
        }
        let overloads = schema
            .functions
            .iter()
            .filter(|function| function.name.eq_ignore_ascii_case(&identifier))
            .collect::<Vec<_>>();
        if overloads.is_empty() {
            continue;
        }
        let Some(arguments) = routine_argument_offsets(text, opening) else {
            continue;
        };
        let matching_overloads = overloads
            .into_iter()
            .filter(|function| {
                let required = function
                    .parameters
                    .iter()
                    .filter(|parameter| !parameter.optional)
                    .count();
                required <= arguments.len() && arguments.len() <= function.parameters.len()
            })
            .collect::<Vec<_>>();
        if matching_overloads.is_empty() {
            continue;
        }

        for (parameter_index, argument_start) in arguments.into_iter().enumerate() {
            if argument_start < range_start || argument_start > range_end {
                continue;
            }
            let names = matching_overloads
                .iter()
                .filter_map(|function| function.parameters.get(parameter_index))
                .map(|parameter| parameter.name.trim())
                .filter(|name| !name.is_empty())
                .collect::<HashSet<_>>();
            if names.len() != 1 {
                continue;
            }
            let name = *names.iter().next().expect("one parameter name");
            let argument_tail = &text[argument_start..];
            if argument_tail.strip_prefix(name).is_some_and(|tail| {
                tail.trim_start().starts_with("=>") || tail.trim_start().starts_with(":=")
            }) {
                continue;
            }
            let data_types = matching_overloads
                .iter()
                .filter_map(|function| function.parameters.get(parameter_index))
                .map(|parameter| parameter.data_type.trim())
                .filter(|data_type| !data_type.is_empty())
                .collect::<HashSet<_>>();
            let tooltip = (data_types.len() == 1).then(|| {
                InlayHintTooltip::String(format!(
                    "Parameter type: {}",
                    data_types.iter().next().expect("one data type")
                ))
            });
            hints.push(InlayHint {
                position: position_for_offset(text, argument_start),
                label: InlayHintLabel::String(format!("{name}:")),
                kind: Some(InlayHintKind::PARAMETER),
                text_edits: None,
                tooltip,
                padding_left: None,
                padding_right: Some(true),
                data: None,
            });
        }
    }
    hints.sort_by_key(|hint| (hint.position.line, hint.position.character));
    hints.truncate(500);
    hints
}

fn routine_argument_offsets(text: &str, opening: usize) -> Option<Vec<usize>> {
    let mut arguments = Vec::new();
    let mut nesting = 0u32;
    let mut quote = None;
    let mut argument_start = opening + 1;
    let mut saw_content = false;

    for (relative, character) in text[opening + 1..].char_indices() {
        let offset = opening + 1 + relative;
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            }
            saw_content = true;
            continue;
        }
        if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
            saw_content = true;
            continue;
        }
        match character {
            '(' | '[' | '{' => {
                nesting += 1;
                saw_content = true;
            }
            ')' if nesting == 0 => {
                if saw_content {
                    arguments.push(skip_sql_whitespace(text, argument_start, offset));
                }
                return Some(arguments);
            }
            ')' | ']' | '}' if nesting > 0 => {
                nesting -= 1;
                saw_content = true;
            }
            ',' if nesting == 0 => {
                arguments.push(skip_sql_whitespace(text, argument_start, offset));
                argument_start = offset + character.len_utf8();
                saw_content = false;
            }
            _ if !character.is_whitespace() => saw_content = true,
            _ => {}
        }
    }
    None
}

fn skip_sql_whitespace(text: &str, start: usize, end: usize) -> usize {
    let mut offset = start.min(end);
    while offset < end {
        let Some(character) = text[offset..end].chars().next() else {
            break;
        };
        if !character.is_whitespace() {
            break;
        }
        offset += character.len_utf8();
    }
    offset
}

fn schema_qualifier_at_position(text: &str, position: Position) -> Option<String> {
    let byte_position = SqlParser::lsp_position_to_byte_position(text, position);
    SqlParser::column_qualifier_before_position(text, byte_position)
}

fn schema_table_matches(schema: &Schema, reference: &str, table: &Table) -> bool {
    SqlParser::table_name_matches_with_catalog(
        reference,
        schema.catalog.as_deref(),
        &schema.database,
        &table.name,
    )
}

fn find_schema_by_qualifier(schema_manager: &SchemaManager, qualifier: &str) -> Option<Schema> {
    let normalized_qualifier = SqlParser::normalize_identifier(qualifier);
    if normalized_qualifier.is_empty() {
        return None;
    }

    let qualifier_has_catalog = normalized_qualifier.contains('.');
    let mut matched_schema = None;
    for schema_id in schema_manager.list_ids() {
        let Some(schema) = schema_manager.get(schema_id) else {
            continue;
        };
        let normalized_database = SqlParser::normalize_identifier(&schema.database);
        let normalized_namespace = schema
            .catalog
            .as_deref()
            .map(SqlParser::normalize_identifier)
            .filter(|catalog| !catalog.is_empty())
            .map(|catalog| format!("{catalog}.{normalized_database}"))
            .unwrap_or_else(|| normalized_database.clone());
        let is_match = normalized_namespace.eq_ignore_ascii_case(&normalized_qualifier)
            || (!qualifier_has_catalog
                && normalized_database.eq_ignore_ascii_case(&normalized_qualifier));
        if !is_match {
            continue;
        }
        if matched_schema.is_some() {
            return None;
        }
        matched_schema = Some(schema);
    }

    matched_schema
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

        let has_table = schema
            .tables
            .iter()
            .any(|table| schema_table_matches(&schema, table_reference, table));
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
    } else if uri_lower.ends_with(".sqlite.sql")
        || uri_lower.ends_with(".sqlite")
        || uri_lower.ends_with(".sqlite3")
        // DuckDB currently opts into the shipped SQLite-compatible completion
        // profile. Recognize its URI extension too, so a document never falls
        // back to the previously selected connection while config sync runs.
        || uri_lower.ends_with(".duckdb.sql")
        || uri_lower.ends_with(".duckdb")
    {
        return "sqlite".to_string();
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
        "sqlite" | "sqlite3" | "duckdb" => "sqlite".to_string(),
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
        add_insert_all_columns_completion, add_referenced_alias_completions,
        apply_completed_sql_context_completion_edits, apply_completion_preferences,
        apply_qualified_identifier_completion_edits, augment_schema_with_local_relations,
        calculate_schema_match_score, completed_sql_context_keyword_at_position,
        completion_statement_prefix, expand_select_star_action, find_schema_by_qualifier,
        find_schema_by_table_reference, infer_dialect_from_uri_and_language,
        infer_schema_id_from_tables, position_to_byte_offset, range_for_offsets,
        rewrite_current_document_location_uri, rewrite_current_document_location_uris,
        schema_for_table_column_at_position, schema_id_for_file, schema_qualifier_at_position,
        sql_inspection_diagnostics, table_alias_initials, CompletionPreferences, KeywordCase,
        TableAliasStyle, LOCAL_RELATION_SCAN_MAX_BYTES,
    };
    use crate::dialects::DialectRegistry;
    use crate::position::lsp_position_at_end;
    use crate::schema::{Column, Schema, SchemaId, SchemaManager, Table};
    use dashmap::DashMap;
    use tower_lsp::lsp_types::{
        CompletionItem, CompletionItemKind, CompletionTextEdit, InsertTextFormat, Location,
        Position, Range, Url,
    };

    fn test_schema(database: &str, tables: &[&str]) -> Schema {
        Schema {
            id: SchemaId::new(),
            catalog: None,
            database: database.to_string(),
            server_version: None,
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

    fn diagnostic_codes(diagnostics: &[tower_lsp::lsp_types::Diagnostic]) -> Vec<String> {
        diagnostics
            .iter()
            .filter_map(|diagnostic| match diagnostic.code.as_ref() {
                Some(tower_lsp::lsp_types::NumberOrString::String(code)) => Some(code.clone()),
                _ => None,
            })
            .collect()
    }

    fn semantic_test_schema() -> Schema {
        Schema {
            id: SchemaId::new(),
            catalog: None,
            database: "app".to_string(),
            server_version: None,
            tables: vec![Table {
                name: "users".to_string(),
                columns: vec![
                    Column {
                        name: "id".to_string(),
                        data_type: "bigint".to_string(),
                        ..Default::default()
                    },
                    Column {
                        name: "name".to_string(),
                        data_type: "text".to_string(),
                        ..Default::default()
                    },
                    Column {
                        name: "email".to_string(),
                        data_type: "text".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            functions: Vec::new(),
            source_uri: None,
        }
    }

    #[test]
    fn local_relation_augmentation_extracts_dbx_cte_and_derived_projection_shapes() {
        let sql = "WITH recent_users(id, display_name) AS (\
            SELECT id, concat(name, ',FROM') AS display_name FROM users\
        ) SELECT * FROM recent_users ru JOIN (\
            SELECT id, name AS user_name FROM users\
        ) sq ON sq.id = ru.id WHERE sq.";
        let augmented = augment_schema_with_local_relations(
            semantic_test_schema(),
            sql,
            lsp_position_at_end(sql),
            "oxide://query/semantic.postgres.sql",
        );

        let recent = augmented
            .tables
            .iter()
            .find(|table| table.name == "recent_users")
            .expect("CTE relation");
        assert_eq!(
            recent
                .columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["id", "display_name"]
        );
        assert_eq!(
            recent.object_type.as_deref(),
            Some("COMMON TABLE EXPRESSION")
        );

        let derived = augmented
            .tables
            .iter()
            .find(|table| table.name == "sq")
            .expect("derived relation");
        assert_eq!(
            derived
                .columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["id", "user_name"]
        );
        assert_eq!(derived.object_type.as_deref(), Some("DERIVED TABLE"));
    }

    #[test]
    fn local_relation_scan_ignores_keywords_inside_comments_and_literals() {
        let sql = "-- WITH ghost(id) AS (SELECT id FROM users)\n\
            SELECT 'FROM (SELECT id FROM users) fake' AS note FROM users";
        let augmented = augment_schema_with_local_relations(
            semantic_test_schema(),
            sql,
            lsp_position_at_end(sql),
            "oxide://query/noise.postgres.sql",
        );

        assert!(!augmented.tables.iter().any(|table| table.name == "ghost"));
        assert!(!augmented.tables.iter().any(|table| table.name == "fake"));
    }

    #[test]
    fn local_relation_scan_is_bounded_for_large_console_documents() {
        let mut sql = " ".repeat(LOCAL_RELATION_SCAN_MAX_BYTES + 1);
        sql.push_str("WITH recent(id) AS (SELECT id FROM users) SELECT * FROM recent");
        let augmented = augment_schema_with_local_relations(
            semantic_test_schema(),
            &sql,
            lsp_position_at_end(&sql),
            "oxide://query/large.postgres.sql",
        );

        assert!(!augmented.tables.iter().any(|table| table.name == "recent"));
    }

    #[test]
    fn local_relation_augmentation_models_dbx_correlation_column_shapes() {
        let cases = [
            (
                "SELECT * FROM generate_series(1, 3) g(value) WHERE g.",
                "generate_series",
                vec!["value"],
                "TABLE FUNCTION",
            ),
            (
                "SELECT * FROM generate_series(1, 3) WITH ORDINALITY AS g(value, ord), users u WHERE g.",
                "generate_series",
                vec!["value", "ord"],
                "TABLE FUNCTION",
            ),
            (
                "SELECT * FROM users u(user_id) WHERE u.",
                "users",
                vec!["user_id", "name", "email"],
                "CORRELATED TABLE",
            ),
            (
                "SELECT * FROM (SELECT id, name FROM users) u(user_id, display_name) WHERE u.",
                "u",
                vec!["user_id", "display_name"],
                "DERIVED TABLE",
            ),
            (
                "SELECT * FROM users u, LATERAL (SELECT u.id AS user_id) s WHERE s.",
                "s",
                vec!["user_id"],
                "DERIVED TABLE",
            ),
            (
                "SELECT * FROM users u CROSS APPLY (SELECT u.id AS user_id) s WHERE s.",
                "s",
                vec!["user_id"],
                "DERIVED TABLE",
            ),
            (
                "SELECT * FROM users u, LATERAL generate_series(1, 3) g(value), users o WHERE g.",
                "generate_series",
                vec!["value"],
                "TABLE FUNCTION",
            ),
        ];

        for (sql, table_name, expected_columns, object_type) in cases {
            let augmented = augment_schema_with_local_relations(
                semantic_test_schema(),
                sql,
                lsp_position_at_end(sql),
                "oxide://query/correlation.postgres.sql",
            );
            let table = augmented
                .tables
                .iter()
                .find(|table| table.name == table_name)
                .unwrap_or_else(|| panic!("missing {table_name}: {:?}", augmented.tables));
            assert_eq!(
                table
                    .columns
                    .iter()
                    .map(|column| column.name.as_str())
                    .collect::<Vec<_>>(),
                expected_columns
            );
            assert_eq!(table.object_type.as_deref(), Some(object_type));
        }
    }

    #[test]
    fn sqlserver_table_hints_preserve_remote_column_shapes() {
        for sql in [
            "SELECT * FROM users u (NOLOCK) WHERE u.",
            "SELECT * FROM users u WITH (UPDLOCK, ROWLOCK) WHERE u.",
        ] {
            let augmented = augment_schema_with_local_relations(
                semantic_test_schema(),
                sql,
                lsp_position_at_end(sql),
                "oxide://query/hints.sqlserver.sql",
            );
            let users = augmented
                .tables
                .iter()
                .find(|table| table.name == "users")
                .expect("remote users table");
            assert_eq!(
                users
                    .columns
                    .iter()
                    .map(|column| column.name.as_str())
                    .collect::<Vec<_>>(),
                ["id", "name", "email"]
            );
            assert_ne!(users.object_type.as_deref(), Some("CORRELATED TABLE"));
        }
    }

    #[tokio::test]
    async fn dbx_cte_column_scope_is_shared_by_native_and_compatibility_dialects() {
        let sql = "WITH recent_users(id, display_name) AS (SELECT id, name FROM users) \
                   SELECT * FROM recent_users ru WHERE ru.";
        let position = lsp_position_at_end(sql);
        let augmented = augment_schema_with_local_relations(
            semantic_test_schema(),
            sql,
            position,
            "oxide://query/cte.postgres.sql",
        );
        let registry = DialectRegistry::new();

        for dialect_name in [
            "postgres",
            "mysql",
            "sqlite",
            "hive",
            "clickhouse",
            "oracle",
            "sqlserver",
            "duckdb",
        ] {
            let dialect = registry
                .get_by_name(dialect_name)
                .unwrap_or_else(|| panic!("registered dialect {dialect_name}"));
            let items = dialect.completion(sql, position, Some(&augmented)).await;
            let labels = items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>();
            assert!(
                labels.contains(&"id") && labels.contains(&"display_name"),
                "{dialect_name} should expose CTE projection columns: {labels:?}"
            );
            assert!(
                !labels.contains(&"name"),
                "{dialect_name} should not leak base-table columns into alias scope: {labels:?}"
            );
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
        assert_eq!(
            infer_dialect_from_uri_and_language(
                "oxide://query/tab.duckdb.sqlite",
                "sql",
                "postgres"
            ),
            "sqlite"
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
        assert_eq!(
            infer_dialect_from_uri_and_language("untitled://1", "sqlite", "mysql"),
            "sqlite"
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
    fn semantic_inspections_cover_join_ambiguity_dialect_risk_and_suppression() {
        let shared_id = Column {
            name: "id".to_string(),
            data_type: "bigint".to_string(),
            ..Default::default()
        };
        let schema = Schema {
            id: SchemaId::new(),
            catalog: None,
            database: "app".to_string(),
            server_version: None,
            tables: vec![
                Table {
                    name: "orders".to_string(),
                    columns: vec![shared_id.clone()],
                    ..Default::default()
                },
                Table {
                    name: "customers".to_string(),
                    columns: vec![shared_id],
                    ..Default::default()
                },
            ],
            functions: Vec::new(),
            source_uri: None,
        };
        let diagnostics = sql_inspection_diagnostics(
            "SELECT id FROM orders JOIN customers WHERE name ILIKE 'a%';",
            Some(&schema),
            "mysql",
        );
        let codes = diagnostic_codes(&diagnostics);
        assert!(codes.contains(&"OXIDE002".to_string()), "{diagnostics:?}");
        assert!(codes.contains(&"OXIDE004".to_string()), "{diagnostics:?}");
        assert!(codes.contains(&"OXIDE005".to_string()), "{diagnostics:?}");

        let suppressed = sql_inspection_diagnostics(
            "-- noinspection OXIDE001\nUPDATE orders SET id = 1;",
            Some(&schema),
            "postgres",
        );
        assert!(
            !diagnostic_codes(&suppressed).contains(&"OXIDE001".to_string()),
            "{suppressed:?}"
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
    fn completion_preferences_rewrite_keyword_case_and_text_edit() {
        let mut items = vec![CompletionItem {
            label: "SELECT".to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            insert_text: Some("SELECT".to_string()),
            ..CompletionItem::default()
        }];
        apply_completed_sql_context_completion_edits("WHERE", Position::new(0, 5), &mut items);
        apply_completion_preferences(
            "WHERE",
            Position::new(0, 5),
            &mut items,
            &CompletionPreferences {
                keyword_case: KeywordCase::Lower,
                table_alias: TableAliasStyle::None,
            },
        );

        assert_eq!(items[0].label, "select");
        assert_eq!(items[0].insert_text.as_deref(), Some("select"));
        let Some(CompletionTextEdit::Edit(edit)) = items[0].text_edit.as_ref() else {
            panic!("keyword completion should keep its text edit");
        };
        assert_eq!(edit.new_text, " select");
    }

    #[test]
    fn completion_includes_visible_relation_alias_as_a_first_class_item() {
        let sql = "SELECT us FROM app.users AS us WHERE us.id > 0";
        let position = Position::new(0, "SELECT us".len() as u32);
        let mut items = Vec::new();

        add_referenced_alias_completions(sql, position, &mut items);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "us");
        assert_eq!(items[0].kind, Some(CompletionItemKind::VARIABLE));
        assert_eq!(items[0].insert_text.as_deref(), Some("us"));
        assert_eq!(items[0].detail.as_deref(), Some("Table alias · app.users"));
    }

    #[test]
    fn relation_alias_completion_preserves_quoted_alias_sql() {
        let sql = "SELECT ua FROM app.users AS \"User Alias\"";
        let position = Position::new(0, "SELECT ua".len() as u32);
        let mut items = Vec::new();

        add_referenced_alias_completions(sql, position, &mut items);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "User Alias");
        assert_eq!(items[0].insert_text.as_deref(), Some("\"User Alias\""));
    }

    #[test]
    fn relation_alias_completion_stays_out_of_relation_targets_and_sql_noise() {
        for (sql, position) in [
            ("SELECT * FROM us", Position::new(0, 16)),
            ("SELECT '-- us' FROM app.users us", Position::new(0, 12)),
        ] {
            let mut items = Vec::new();
            add_referenced_alias_completions(sql, position, &mut items);
            assert!(
                items.is_empty(),
                "unexpected alias completion for {sql}: {items:?}"
            );
        }
    }

    #[test]
    fn completion_preferences_add_initial_alias_only_for_relation_context() {
        let preferences = CompletionPreferences {
            keyword_case: KeywordCase::Upper,
            table_alias: TableAliasStyle::Initials,
        };
        let mut from_items = vec![CompletionItem {
            label: "app.user_accounts".to_string(),
            kind: Some(CompletionItemKind::CLASS),
            insert_text: Some("app.user_accounts".to_string()),
            ..CompletionItem::default()
        }];
        apply_completion_preferences(
            "SELECT * FROM user",
            Position::new(0, 18),
            &mut from_items,
            &preferences,
        );
        assert_eq!(
            from_items[0].insert_text.as_deref(),
            Some("app.user_accounts ${1:ua}")
        );

        let mut select_items = from_items.clone();
        select_items[0].insert_text = Some("app.user_accounts".to_string());
        apply_completion_preferences(
            "SELECT user",
            Position::new(0, 11),
            &mut select_items,
            &preferences,
        );
        assert_eq!(
            select_items[0].insert_text.as_deref(),
            Some("app.user_accounts")
        );
        assert_eq!(
            table_alias_initials("audit.userAccounts"),
            Some("ua".to_string())
        );
    }

    fn completion_schema_with_columns() -> Schema {
        Schema {
            id: SchemaId::new(),
            catalog: None,
            database: "app".to_string(),
            server_version: None,
            tables: vec![
                Table {
                    name: "orders".to_string(),
                    columns: ["id", "customer_id", "total"]
                        .into_iter()
                        .map(|name| Column {
                            name: name.to_string(),
                            data_type: "bigint".to_string(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
                Table {
                    name: "customers".to_string(),
                    columns: ["id", "name"]
                        .into_iter()
                        .map(|name| Column {
                            name: name.to_string(),
                            data_type: "text".to_string(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
            ],
            functions: Vec::new(),
            source_uri: None,
        }
    }

    #[test]
    fn insert_all_columns_completion_adds_values_tab_stops() {
        let sql = "INSERT INTO app.orders (";
        let mut items = Vec::new();
        add_insert_all_columns_completion(
            sql,
            lsp_position_at_end(sql),
            Some(&completion_schema_with_columns()),
            &mut items,
            &CompletionPreferences {
                keyword_case: KeywordCase::Lower,
                table_alias: TableAliasStyle::None,
            },
            "postgres",
        );

        let item = items
            .iter()
            .find(|item| item.label == "orders.*")
            .expect("all-column completion");
        assert_eq!(
            item.insert_text.as_deref(),
            Some("id, customer_id, total) values (${1:value}, ${2:value}, ${3:value})")
        );
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));

        let nested_expression = "INSERT INTO app.orders (COALESCE(";
        let mut nested_items = Vec::new();
        add_insert_all_columns_completion(
            nested_expression,
            lsp_position_at_end(nested_expression),
            Some(&completion_schema_with_columns()),
            &mut nested_items,
            &CompletionPreferences::default(),
            "postgres",
        );
        assert!(nested_items.is_empty());
    }

    #[test]
    fn generated_column_lists_quote_reserved_and_unicode_identifiers() {
        let mut schema = completion_schema_with_columns();
        schema.tables.push(Table {
            name: "事件".to_string(),
            columns: ["id", "order", "显示名称"]
                .into_iter()
                .map(|name| Column {
                    name: name.to_string(),
                    data_type: "text".to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        let insert_sql = "INSERT INTO app.事件 (";
        let mut items = Vec::new();
        add_insert_all_columns_completion(
            insert_sql,
            lsp_position_at_end(insert_sql),
            Some(&schema),
            &mut items,
            &CompletionPreferences::default(),
            "mysql",
        );
        assert_eq!(
            items[0].insert_text.as_deref(),
            Some("id, `order`, `显示名称`) VALUES (${1:value}, ${2:value}, ${3:value})")
        );

        let select_sql = "SELECT e.* FROM app.事件 e";
        let uri = Url::parse("file:///query.sql").unwrap();
        let star = select_sql.find('*').unwrap();
        let action = expand_select_star_action(
            select_sql,
            &uri,
            range_for_offsets(select_sql, star, star + 1),
            Some(schema),
            "mysql",
        )
        .expect("quoted star expansion");
        let edit = &action.edit.unwrap().changes.unwrap()[&uri][0];
        assert_eq!(edit.new_text, "id, e.`order`, e.`显示名称`");
    }

    #[test]
    fn select_star_expansion_qualifies_columns_from_multiple_sources() {
        let sql = "SELECT * FROM app.orders o JOIN app.customers c ON c.id = o.customer_id";
        let uri = Url::parse("file:///query.sql").unwrap();
        let star = sql.find('*').unwrap();
        let action = expand_select_star_action(
            sql,
            &uri,
            range_for_offsets(sql, star, star + 1),
            Some(completion_schema_with_columns()),
            "postgres",
        )
        .expect("star expansion");
        let edit = &action.edit.unwrap().changes.unwrap()[&uri][0];
        assert_eq!(edit.new_text, "o.id, o.customer_id, o.total, c.id, c.name");
    }

    #[test]
    fn qualified_star_expansion_preserves_the_typed_alias() {
        let sql = "SELECT o.* FROM app.orders AS o";
        let uri = Url::parse("file:///query.sql").unwrap();
        let star = sql.find('*').unwrap();
        let action = expand_select_star_action(
            sql,
            &uri,
            range_for_offsets(sql, star, star + 1),
            Some(completion_schema_with_columns()),
            "postgres",
        )
        .expect("qualified star expansion");
        let edit = &action.edit.unwrap().changes.unwrap()[&uri][0];
        assert_eq!(edit.new_text, "id, o.customer_id, o.total");
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
    fn finds_catalog_qualified_schema_without_cross_catalog_leakage() {
        let manager = SchemaManager::new();
        let mut app = test_schema("dbo", &["users"]);
        app.catalog = Some("AppDb".to_string());
        let app_id = manager.register(app);
        let mut audit = test_schema("dbo", &["users"]);
        audit.catalog = Some("AuditDb".to_string());
        let audit_id = manager.register(audit);

        assert_eq!(
            find_schema_by_qualifier(&manager, "AppDb.dbo").map(|schema| schema.id),
            Some(app_id)
        );
        assert_eq!(
            find_schema_by_table_reference(&manager, "[ServerOne].[AuditDb].[dbo].[users]")
                .map(|schema| schema.id),
            Some(audit_id)
        );
        assert_eq!(
            find_schema_by_table_reference(&manager, "AppDb..users").map(|schema| schema.id),
            Some(app_id)
        );
        assert!(find_schema_by_qualifier(&manager, "dbo").is_none());
        assert!(find_schema_by_table_reference(&manager, "dbo.users").is_none());
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

    #[test]
    fn completion_recovers_the_second_top_level_query_without_a_semicolon() {
        let sql = "SELECT *\nFROM first_table\n\nSELECT ";
        let position = crate::position::lsp_position_at_end(sql);
        let (scope, local_position) =
            completion_statement_prefix(sql, position).expect("second query should be isolated");

        assert_eq!(scope, "SELECT ");
        assert_eq!(local_position, crate::position::lsp_position_at_end(scope));
    }

    #[test]
    fn completion_keeps_wrapped_and_nested_queries_in_one_scope() {
        for sql in [
            "WITH recent AS (\n  SELECT * FROM events\n)\n\nSELECT ",
            "INSERT INTO archive (id)\n\nSELECT ",
            "EXPLAIN\n\nSELECT ",
            "SELECT * FROM (\n\n  SELECT ",
            "SELECT 1\nUNION ALL\n\nSELECT ",
        ] {
            assert!(
                completion_statement_prefix(sql, crate::position::lsp_position_at_end(sql))
                    .is_none(),
                "{sql:?} must remain one completion scope"
            );
        }
    }

    #[test]
    fn completion_ignores_statement_keywords_inside_sql_noise() {
        let sql =
            "SELECT '-- not a boundary'\n\n-- SELECT ignored\n/* SELECT ignored too */\nFROM logs";

        assert!(
            completion_statement_prefix(sql, crate::position::lsp_position_at_end(sql)).is_none()
        );
    }
}
