use crate::builtin_signatures::{
    builtin_function_catalog_is_available_for, builtin_function_is_known_for,
    builtin_signature_completion_catalog_for, builtin_signatures_for,
    builtin_table_signature_completion_catalog_for, builtin_value_catalog_for,
    builtin_window_function_catalog_is_available_for, BuiltinSignature,
};
use crate::dialect::Dialect;
use crate::dialects::common::{
    apply_formatter_function_case, apply_formatter_layout, format_sql_pretty_with_keyword_case,
    identifier_match_rank, LogicalOperatorLayout,
};
use crate::dialects::DialectRegistry;
use crate::parser::SqlParser;
use crate::placeholder::SqlPlaceholderDialect;
use crate::position::lsp_position_at_end;
use crate::schema::{Column, Constraint, Schema, SchemaId, SchemaManager, Table};
use crate::token::Keywords;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

const CURRENT_SQL_DOCUMENT_URI: &str = "file:///current.sql";
const CURRENT_JSON_DOCUMENT_URI: &str = "file:///current.json";
const PROJECT_SQL_INDEX_MAX_BYTES: usize = 512 * 1024;
const PROJECT_SQL_INDEX_MAX_DOCUMENTS: usize = 256;
const PROJECT_SQL_INDEX_MAX_OCCURRENCES: usize = 10_000;
const PROJECT_SQL_INDEX_MAX_RESULTS: usize = 2_000;
const SIGNATURE_HELP_MAX_SCAN_BYTES: usize = 64 * 1024;
const SIGNATURE_HELP_MAX_OVERLOADS: usize = 50;
const BUILTIN_FUNCTION_COMPLETION_MAX_ITEMS: usize = 100;
const COMPLETION_RESOLVE_CACHE_MAX_ENTRIES: usize = 8_192;
const COMPLETION_RESOLVE_CACHE_MAX_BYTES: usize = 4 * 1024 * 1024;
const COMPLETION_RESOLVE_DOCUMENTATION_MAX_BYTES: usize = 256 * 1024;
const INLAY_HINT_STATEMENT_SCAN_BYTES: usize = 32 * 1024;
const INLAY_HINT_REQUEST_MAX_BYTES: usize = 64 * 1024;
const INLAY_HINT_MAX_ITEMS: usize = 500;
const COMPLETION_RESOLVE_ID_FIELD: &str = "oxideSqlLspCompletionResolveId";
const COMPLETION_RESOLVE_ORIGINAL_DATA_FIELD: &str = "originalData";

#[derive(Clone)]
struct CompletionResolveEntry {
    documentation: Documentation,
    bytes: usize,
}

#[derive(Default)]
struct CompletionResolveCache {
    entries: HashMap<u64, CompletionResolveEntry>,
    order: VecDeque<u64>,
    bytes: usize,
}

impl CompletionResolveCache {
    fn insert(&mut self, id: u64, documentation: Documentation) -> bool {
        let Ok(bytes) = serde_json::to_vec(&documentation).map(|encoded| encoded.len()) else {
            return false;
        };
        if bytes > COMPLETION_RESOLVE_DOCUMENTATION_MAX_BYTES
            || bytes > COMPLETION_RESOLVE_CACHE_MAX_BYTES
        {
            return false;
        }

        while self.entries.len() >= COMPLETION_RESOLVE_CACHE_MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > COMPLETION_RESOLVE_CACHE_MAX_BYTES
        {
            let Some(expired_id) = self.order.pop_front() else {
                break;
            };
            if let Some(expired) = self.entries.remove(&expired_id) {
                self.bytes = self.bytes.saturating_sub(expired.bytes);
            }
        }

        self.bytes = self.bytes.saturating_add(bytes);
        self.order.push_back(id);
        self.entries.insert(
            id,
            CompletionResolveEntry {
                documentation,
                bytes,
            },
        );
        true
    }

    fn documentation(&self, id: u64) -> Option<Documentation> {
        self.entries
            .get(&id)
            .map(|entry| entry.documentation.clone())
    }
}

fn completion_resolve_data(id: u64, original_data: Option<Value>) -> Value {
    let mut data = JsonMap::new();
    data.insert(COMPLETION_RESOLVE_ID_FIELD.to_string(), Value::from(id));
    if let Some(original_data) = original_data {
        data.insert(
            COMPLETION_RESOLVE_ORIGINAL_DATA_FIELD.to_string(),
            original_data,
        );
    }
    Value::Object(data)
}

fn completion_resolve_identity(data: Option<&Value>) -> Option<u64> {
    data?.get(COMPLETION_RESOLVE_ID_FIELD)?.as_u64()
}

fn original_completion_data(data: Option<&Value>) -> Option<Value> {
    data?.get(COMPLETION_RESOLVE_ORIGINAL_DATA_FIELD).cloned()
}

fn defer_completion_documentation(
    items: &mut [CompletionItem],
    cache: &Mutex<CompletionResolveCache>,
    next_id: &AtomicU64,
) {
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    for item in items {
        let Some(documentation) = item.documentation.take() else {
            continue;
        };
        let id = next_id.fetch_add(1, Ordering::Relaxed);
        if cache.insert(id, documentation.clone()) {
            item.data = Some(completion_resolve_data(id, item.data.take()));
        } else {
            item.documentation = Some(documentation);
        }
    }
}

fn resolve_completion_documentation(
    mut item: CompletionItem,
    cache: &Mutex<CompletionResolveCache>,
) -> CompletionItem {
    let Some(id) = completion_resolve_identity(item.data.as_ref()) else {
        return item;
    };
    let original_data = original_completion_data(item.data.as_ref());
    if let Ok(cache) = cache.lock() {
        if let Some(documentation) = cache.documentation(id) {
            item.documentation = Some(documentation);
        }
    }
    item.data = original_data;
    item
}

fn client_supports_completion_documentation_resolve(params: &InitializeParams) -> bool {
    params
        .capabilities
        .text_document
        .as_ref()
        .and_then(|capabilities| capabilities.completion.as_ref())
        .and_then(|capabilities| capabilities.completion_item.as_ref())
        .and_then(|capabilities| capabilities.resolve_support.as_ref())
        .is_some_and(|support| {
            support
                .properties
                .iter()
                .any(|property| property == "documentation")
        })
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum ProjectSqlSymbolKind {
    Table,
    View,
    Function,
    Procedure,
}

impl ProjectSqlSymbolKind {
    fn symbol_kind(self) -> SymbolKind {
        match self {
            Self::Table | Self::View => SymbolKind::CLASS,
            Self::Function => SymbolKind::FUNCTION,
            Self::Procedure => SymbolKind::METHOD,
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Table => "Project table",
            Self::View => "Project view",
            Self::Function => "Project function",
            Self::Procedure => "Project procedure",
        }
    }

    fn is_routine(self) -> bool {
        matches!(self, Self::Function | Self::Procedure)
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum ProjectSqlSymbolRole {
    Definition,
    Reference,
}

#[derive(Clone, Debug)]
struct ProjectSqlSymbolOccurrence {
    name: String,
    normalized_name: String,
    kind: ProjectSqlSymbolKind,
    role: ProjectSqlSymbolRole,
    range: Range,
}

#[derive(Clone)]
struct CachedProjectSqlIndex {
    revision: u64,
    occurrences: Vec<ProjectSqlSymbolOccurrence>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletionPreferences {
    #[serde(default = "default_keyword_case")]
    keyword_case: KeywordCase,
    #[serde(default = "default_function_case")]
    function_case: KeywordCase,
    #[serde(default)]
    table_alias: TableAliasStyle,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormattingPreferences {
    #[serde(default = "default_keyword_case")]
    keyword_case: KeywordCase,
    #[serde(default = "default_function_case")]
    function_case: KeywordCase,
    #[serde(default)]
    logical_operator_newline: LogicalOperatorNewline,
    #[serde(default)]
    from_clause_layout: FromClauseLayout,
}

impl Default for FormattingPreferences {
    fn default() -> Self {
        Self {
            keyword_case: KeywordCase::Upper,
            function_case: KeywordCase::Preserve,
            logical_operator_newline: LogicalOperatorNewline::Before,
            from_clause_layout: FromClauseLayout::NewLine,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum LogicalOperatorNewline {
    #[default]
    Before,
    After,
    None,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum FromClauseLayout {
    #[default]
    NewLine,
    SameLine,
}

impl FormattingPreferences {
    fn logical_operator_layout(&self) -> LogicalOperatorLayout {
        match self.logical_operator_newline {
            LogicalOperatorNewline::Before => LogicalOperatorLayout::Before,
            LogicalOperatorNewline::After => LogicalOperatorLayout::After,
            LogicalOperatorNewline::None => LogicalOperatorLayout::SameLine,
        }
    }

    fn keyword_uppercase(&self) -> Option<bool> {
        self.keyword_case.uppercase_option()
    }

    fn function_uppercase(&self) -> Option<bool> {
        self.function_case.uppercase_option()
    }
}

impl Default for CompletionPreferences {
    fn default() -> Self {
        Self {
            keyword_case: KeywordCase::Upper,
            function_case: KeywordCase::Preserve,
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

impl KeywordCase {
    fn uppercase_option(self) -> Option<bool> {
        match self {
            Self::Upper => Some(true),
            Self::Lower => Some(false),
            Self::Preserve => None,
        }
    }
}

fn default_keyword_case() -> KeywordCase {
    KeywordCase::Upper
}

fn default_function_case() -> KeywordCase {
    KeywordCase::Preserve
}

fn dialect_uses_sql_preferences(dialect: &str) -> bool {
    matches!(
        dialect,
        "postgres" | "mysql" | "sqlite" | "hive" | "clickhouse"
    )
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
    revisions: Arc<DashMap<String, u64>>,
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
            revisions: Arc::new(DashMap::new()),
        }
    }

    fn update(&self, uri: String, text: String) {
        self.documents.insert(uri.clone(), text);
        self.revisions
            .entry(uri)
            .and_modify(|revision| *revision = revision.saturating_add(1))
            .or_insert(1);
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

    fn revision(&self, uri: &str) -> Option<u64> {
        self.revisions.get(uri).map(|revision| *revision)
    }

    fn remove(&self, uri: &str) {
        self.documents.remove(uri);
        self.revisions.remove(uri);
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
    formatting_preferences: Arc<RwLock<FormattingPreferences>>,
    /// 打开文档的 languageId，用于配置刷新后恢复推断方言
    document_languages: Arc<DashMap<String, String>>,
    /// 文档管理器
    document_manager: DocumentManager,
    analysis_cache: Arc<DashMap<String, CachedSqlAnalysis>>,
    project_sql_index_cache: Arc<DashMap<String, CachedProjectSqlIndex>>,
    completion_resolve_cache: Mutex<CompletionResolveCache>,
    next_completion_resolve_id: AtomicU64,
    completion_documentation_resolve_supported: AtomicBool,
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
            formatting_preferences: Arc::new(RwLock::new(FormattingPreferences::default())),
            document_languages: Arc::new(DashMap::new()),
            document_manager: DocumentManager::new(),
            analysis_cache: Arc::new(DashMap::new()),
            project_sql_index_cache: Arc::new(DashMap::new()),
            completion_resolve_cache: Mutex::new(CompletionResolveCache::default()),
            next_completion_resolve_id: AtomicU64::new(1),
            completion_documentation_resolve_supported: AtomicBool::new(false),
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

    fn dialect_identity_for_file(&self, uri: &str) -> Option<String> {
        self.configured_file_dialects
            .get(uri)
            .map(|dialect| dialect.value().clone())
            .or_else(|| {
                self.inferred_file_dialects
                    .get(uri)
                    .map(|dialect| dialect.value().clone())
            })
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

    fn project_sql_index_for_document(
        &self,
        uri: &str,
        text: &str,
    ) -> Vec<ProjectSqlSymbolOccurrence> {
        let revision = self.document_manager.revision(uri).unwrap_or_default();
        if let Some(cached) = self.project_sql_index_cache.get(uri) {
            if cached.revision == revision {
                return cached.occurrences.clone();
            }
        }
        let occurrences = project_sql_symbol_occurrences(text);
        if self.document_manager.revision(uri) == Some(revision)
            && self
                .document_manager
                .get(uri)
                .as_deref()
                .is_some_and(|current| current == text)
        {
            self.project_sql_index_cache.insert(
                uri.to_string(),
                CachedProjectSqlIndex {
                    revision,
                    occurrences: occurrences.clone(),
                },
            );
        }
        occurrences
    }

    fn project_sql_documents(&self) -> Vec<(String, String)> {
        let mut documents = self.document_manager.entries();
        documents.sort_by(|left, right| left.0.cmp(&right.0));
        documents.truncate(PROJECT_SQL_INDEX_MAX_DOCUMENTS);
        documents
    }

    fn project_sql_symbol_at_position(
        &self,
        uri: &str,
        text: &str,
        position: Position,
    ) -> Option<ProjectSqlSymbolOccurrence> {
        self.project_sql_index_for_document(uri, text)
            .into_iter()
            .find(|occurrence| range_contains_position(occurrence.range, position))
    }

    fn matching_project_sql_locations(
        &self,
        origin_uri: &str,
        origin_dialect: &str,
        origin_schema_id: Option<SchemaId>,
        target: &ProjectSqlSymbolOccurrence,
        include_declarations: bool,
    ) -> Vec<Location> {
        let mut locations = Vec::new();
        for (candidate_uri, candidate_text) in self.project_sql_documents() {
            let Some(candidate_dialect) = self.get_dialect_for_file(&candidate_uri) else {
                continue;
            };
            if candidate_dialect.name() != origin_dialect
                || !documents_share_project_sql_scope(
                    origin_uri,
                    &candidate_uri,
                    origin_schema_id,
                    self.get_schema_for_file(&candidate_uri)
                        .as_ref()
                        .map(|schema| schema.id),
                )
            {
                continue;
            }
            let Ok(candidate_url) = Url::parse(&candidate_uri) else {
                continue;
            };
            for occurrence in self.project_sql_index_for_document(&candidate_uri, &candidate_text) {
                if (!include_declarations && occurrence.role == ProjectSqlSymbolRole::Definition)
                    || !project_sql_symbols_match(target, &occurrence)
                {
                    continue;
                }
                locations.push(Location {
                    uri: candidate_url.clone(),
                    range: occurrence.range,
                });
                if locations.len() >= PROJECT_SQL_INDEX_MAX_RESULTS {
                    return locations;
                }
            }
        }
        locations
    }

    fn project_sql_definition(
        &self,
        origin_uri: &str,
        origin_dialect: &str,
        origin_schema_id: Option<SchemaId>,
        target: &ProjectSqlSymbolOccurrence,
    ) -> Option<Location> {
        for (candidate_uri, candidate_text) in self.project_sql_documents() {
            let Some(candidate_dialect) = self.get_dialect_for_file(&candidate_uri) else {
                continue;
            };
            if candidate_dialect.name() != origin_dialect
                || !documents_share_project_sql_scope(
                    origin_uri,
                    &candidate_uri,
                    origin_schema_id,
                    self.get_schema_for_file(&candidate_uri)
                        .as_ref()
                        .map(|schema| schema.id),
                )
            {
                continue;
            }
            let Ok(candidate_url) = Url::parse(&candidate_uri) else {
                continue;
            };
            if let Some(definition) = self
                .project_sql_index_for_document(&candidate_uri, &candidate_text)
                .into_iter()
                .find(|occurrence| {
                    occurrence.role == ProjectSqlSymbolRole::Definition
                        && project_sql_symbols_match(target, occurrence)
                })
            {
                return Some(Location {
                    uri: candidate_url,
                    range: definition.range,
                });
            }
        }
        None
    }

    fn project_sql_symbol_is_renameable(
        &self,
        uri: &str,
        dialect: &str,
        schema: Option<&Schema>,
        target: &ProjectSqlSymbolOccurrence,
    ) -> bool {
        if target.kind.is_routine() {
            let metadata_matches = schema.map_or(0, |schema| {
                schema
                    .functions
                    .iter()
                    .filter(|function| function.name.eq_ignore_ascii_case(&target.name))
                    .count()
            });
            let project_definitions = self.project_sql_definition_count(
                uri,
                dialect,
                schema.map(|schema| schema.id),
                target,
            );
            return metadata_matches <= 1
                && project_definitions <= 1
                && (metadata_matches == 1 || project_definitions == 1);
        }
        let metadata_match = schema.is_some_and(|schema| {
            schema.tables.iter().any(|table| {
                SqlParser::table_name_matches_with_catalog(
                    &target.normalized_name,
                    schema.catalog.as_deref(),
                    &schema.database,
                    &table.name,
                )
            })
        });
        metadata_match
            || self
                .project_sql_definition(uri, dialect, schema.map(|schema| schema.id), target)
                .is_some()
    }

    fn project_sql_definition_count(
        &self,
        origin_uri: &str,
        origin_dialect: &str,
        origin_schema_id: Option<SchemaId>,
        target: &ProjectSqlSymbolOccurrence,
    ) -> usize {
        let mut count = 0;
        for (candidate_uri, candidate_text) in self.project_sql_documents() {
            let Some(candidate_dialect) = self.get_dialect_for_file(&candidate_uri) else {
                continue;
            };
            if candidate_dialect.name() != origin_dialect
                || !documents_share_project_sql_scope(
                    origin_uri,
                    &candidate_uri,
                    origin_schema_id,
                    self.get_schema_for_file(&candidate_uri)
                        .as_ref()
                        .map(|schema| schema.id),
                )
            {
                continue;
            }
            count += self
                .project_sql_index_for_document(&candidate_uri, &candidate_text)
                .into_iter()
                .filter(|occurrence| {
                    occurrence.role == ProjectSqlSymbolRole::Definition
                        && project_sql_symbols_match(target, occurrence)
                })
                .take(2 - count)
                .count();
            if count >= 2 {
                return count;
            }
        }
        count
    }

    async fn completion_candidates_at(
        &self,
        uri: &str,
        text: &str,
        position: Position,
        context: Option<&CompletionContext>,
    ) -> Option<Vec<CompletionItem>> {
        let dialect = self.get_dialect_for_file(uri)?;
        let schema = self.get_schema_for_position(uri, text, position);
        let (completion_text, completion_position) =
            completion_statement_prefix(text, position).unwrap_or((text, position));
        let mut items = dialect
            .completion_with_context(
                completion_text,
                completion_position,
                schema.as_ref(),
                context,
            )
            .await;
        let dialect_identity = self
            .dialect_identity_for_file(uri)
            .unwrap_or_else(|| dialect.name().to_string());
        add_builtin_function_completions(
            completion_text,
            completion_position,
            &dialect_identity,
            schema.as_ref(),
            &mut items,
        );
        add_window_function_completions(
            completion_text,
            completion_position,
            &dialect_identity,
            &mut items,
        );
        add_clickhouse_table_function_completions(
            completion_text,
            completion_position,
            &dialect_identity,
            &mut items,
        );
        add_oracle_table_function_completions(
            completion_text,
            completion_position,
            &dialect_identity,
            schema.as_ref(),
            &mut items,
        );
        let preferences = self
            .completion_preferences
            .read()
            .map(|preferences| preferences.clone())
            .unwrap_or_default();
        let relational_completion = dialect_uses_sql_preferences(dialect.name());
        if relational_completion {
            add_insert_statement_snippets(
                text,
                position,
                schema.as_ref(),
                &mut items,
                &preferences,
                dialect.name(),
            );
            add_insert_all_columns_completion(
                text,
                position,
                schema.as_ref(),
                &mut items,
                &preferences,
                dialect.name(),
            );
        }
        add_referenced_alias_completions(text, position, &mut items);
        apply_completed_sql_context_completion_edits(text, position, &mut items);
        apply_qualified_identifier_completion_edits(text, position, &mut items);
        if relational_completion {
            apply_completion_preferences(text, position, &mut items, &preferences);
        }
        deduplicate_simple_completion_items(&mut items);
        apply_high_frequency_keyword_ranking(&mut items);
        Some(items)
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
        let mut raw_candidates = self
            .completion_candidates_at(&uri, &text, position, None)
            .await
            .unwrap_or_default();
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
                insert_text: completion_item_effective_insert_text(&item),
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

fn completion_item_effective_insert_text(item: &CompletionItem) -> Option<String> {
    item.text_edit
        .as_ref()
        .map(|edit| match edit {
            CompletionTextEdit::Edit(edit) => edit.new_text.clone(),
            CompletionTextEdit::InsertAndReplace(edit) => edit.new_text.clone(),
        })
        .or_else(|| item.insert_text.clone())
        .or_else(|| Some(item.label.clone()))
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

// Mirrors DBX's static frequency boost. The alphabetical rank deliberately
// stays behind the semantic group and ahead of the label: Monaco's local
// numeric usage rank can still be inserted between the group and this rank.
const HIGH_FREQUENCY_SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "AND", "OR", "JOIN", "ON", "IN", "AS", "GROUP BY", "ORDER BY",
    "LEFT", "RIGHT", "INNER", "OUTER", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE",
    "NOT", "NULL", "IS", "LIKE", "DISTINCT", "HAVING", "LIMIT", "COUNT", "SUM", "AVG", "MAX",
    "MIN", "CASE", "UNION", "ALL", "ASC", "DESC", "BETWEEN", "EXISTS",
];

fn apply_high_frequency_keyword_ranking(items: &mut [CompletionItem]) {
    for item in items {
        if item.kind != Some(CompletionItemKind::KEYWORD) {
            continue;
        }

        let frequency_rank = if HIGH_FREQUENCY_SQL_KEYWORDS
            .iter()
            .any(|keyword| item.label.eq_ignore_ascii_case(keyword))
        {
            "a"
        } else {
            "b"
        };
        let fallback_label = item.label.to_ascii_lowercase();
        let current = item.sort_text.as_deref().unwrap_or_default();

        if let Some((semantic_group, remainder)) = current.split_once(':') {
            item.sort_text = Some(format!(
                "{semantic_group}:{frequency_rank}:{}",
                remainder.to_ascii_lowercase()
            ));
            continue;
        }

        let bytes = current.as_bytes();
        let mut group_end = usize::from(bytes.first() == Some(&b'-'));
        while bytes
            .get(group_end)
            .is_some_and(|character| character.is_ascii_digit())
        {
            group_end += 1;
        }
        let has_numeric_group = group_end > 0 && !(group_end == 1 && bytes.first() == Some(&b'-'));
        if has_numeric_group {
            let semantic_group = &current[..group_end];
            let remainder = current[group_end..].trim_start_matches(':');
            let normalized_remainder = if remainder.is_empty() {
                fallback_label.clone()
            } else {
                remainder.to_ascii_lowercase()
            };
            item.sort_text = Some(format!(
                "{semantic_group}:{frequency_rank}:{normalized_remainder}"
            ));
        } else {
            item.sort_text = Some(format!("0:{frequency_rank}:{fallback_label}"));
        }
    }
}

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

fn deduplicate_simple_completion_items(items: &mut Vec<CompletionItem>) {
    let mut seen = HashSet::new();
    items.retain(|item| {
        let family = match item.kind {
            Some(CompletionItemKind::KEYWORD) => "keyword",
            Some(CompletionItemKind::OPERATOR) => "operator",
            _ => return true,
        };
        seen.insert(format!("{family}:{}", item.label.to_ascii_lowercase()))
    });
}

fn next_snippet_tabstop_index(value: &str) -> usize {
    let bytes = value.as_bytes();
    let mut largest = 0usize;
    let mut cursor = 0usize;

    while cursor + 1 < bytes.len() {
        if bytes[cursor] != b'$' {
            cursor += 1;
            continue;
        }

        let mut digit_start = cursor + 1;
        let braced = bytes[digit_start] == b'{';
        if braced {
            digit_start += 1;
        }
        let mut digit_end = digit_start;
        while digit_end < bytes.len() && bytes[digit_end].is_ascii_digit() {
            digit_end += 1;
        }
        if digit_end == digit_start
            || (braced
                && !bytes
                    .get(digit_end)
                    .is_some_and(|character| matches!(*character, b':' | b'}' | b'|')))
        {
            cursor += 1;
            continue;
        }
        if let Ok(index) = value[digit_start..digit_end].parse::<usize>() {
            largest = largest.max(index);
        }
        cursor = digit_end;
    }

    largest.saturating_add(1).max(1)
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
            let transform = |value: &str| apply_identifier_case(value, preferences.keyword_case);
            item.label = transform(&item.label);
            if let Some(insert_text) = item.insert_text.as_mut() {
                *insert_text = transform(insert_text);
            }
            update_completion_text_edit(item, &transform);
        } else if completion_item_uses_generated_keyword_case(item) {
            apply_generated_function_case(item, preferences.keyword_case);
        }

        if completion_item_uses_generated_function_case(item) {
            apply_generated_function_case(item, preferences.function_case);
            if item
                .sort_text
                .as_deref()
                .is_some_and(|sort_text| sort_text.starts_with("1:window-function:"))
            {
                apply_generated_template_keyword_case(item, preferences.keyword_case);
            }
        }

        if add_table_alias && is_relation_completion_kind(item.kind) {
            let Some(alias) = table_alias_initials(&item.label) else {
                continue;
            };
            let append_alias = |value: &str| {
                let tabstop = next_snippet_tabstop_index(value);
                format!("{value} ${{{tabstop}:{alias}}}")
            };
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

fn apply_identifier_case(value: &str, requested_case: KeywordCase) -> String {
    match requested_case {
        KeywordCase::Upper => value.to_ascii_uppercase(),
        KeywordCase::Lower => value.to_ascii_lowercase(),
        KeywordCase::Preserve => value.to_string(),
    }
}

fn completion_item_uses_generated_keyword_case(item: &CompletionItem) -> bool {
    let sort_text = item.sort_text.as_deref().unwrap_or_default();
    sort_text.starts_with("1:builtin-value:")
        || (sort_text.starts_with("1:oracle-table-function:")
            && matches!(item.label.to_ascii_uppercase().as_str(), "TABLE" | "THE"))
}

fn completion_item_uses_generated_function_case(item: &CompletionItem) -> bool {
    let sort_text = item.sort_text.as_deref().unwrap_or_default();
    sort_text.starts_with("1:builtin:")
        || sort_text.starts_with("1:window-function:")
        || sort_text.starts_with("1:table-function:")
        || (sort_text.starts_with("1:oracle-table-function:")
            && !matches!(item.label.to_ascii_uppercase().as_str(), "TABLE" | "THE"))
}

fn replace_identifier_prefix(value: &str, original: &str, replacement: &str) -> String {
    let trimmed = value.trim_start_matches(char::is_whitespace);
    let leading_len = value.len().saturating_sub(trimmed.len());
    let Some(prefix) = trimmed.get(..original.len()) else {
        return value.to_string();
    };
    if !prefix.eq_ignore_ascii_case(original) {
        return value.to_string();
    }
    format!(
        "{}{}{}",
        &value[..leading_len],
        replacement,
        &trimmed[original.len()..]
    )
}

fn apply_generated_function_case(item: &mut CompletionItem, function_case: KeywordCase) {
    let original = item.label.clone();
    let replacement = apply_identifier_case(&original, function_case);
    item.label = replacement.clone();
    if item
        .filter_text
        .as_deref()
        .is_some_and(|filter_text| filter_text.eq_ignore_ascii_case(&original))
    {
        item.filter_text = Some(replacement.clone());
    }
    let transform = |value: &str| replace_identifier_prefix(value, &original, &replacement);
    if let Some(insert_text) = item.insert_text.as_mut() {
        *insert_text = transform(insert_text);
    }
    update_completion_text_edit(item, &transform);
}

fn generated_template_keyword_case(value: &str, keyword_case: KeywordCase) -> String {
    if matches!(keyword_case, KeywordCase::Preserve) {
        return value.to_string();
    }
    const KEYWORDS: &[&str] = &["AS", "INTERVAL", "OVER", "PARTITION", "BY", "ORDER"];
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while cursor < characters.len() {
        if !characters[cursor].is_ascii_alphanumeric() && characters[cursor] != '_' {
            output.push(characters[cursor]);
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < characters.len()
            && (characters[cursor].is_ascii_alphanumeric() || characters[cursor] == '_')
        {
            cursor += 1;
        }
        let word = characters[start..cursor].iter().collect::<String>();
        if KEYWORDS
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
        {
            output.push_str(&apply_identifier_case(&word, keyword_case));
        } else {
            output.push_str(&word);
        }
    }
    output
}

fn apply_generated_template_keyword_case(item: &mut CompletionItem, keyword_case: KeywordCase) {
    let transform = |value: &str| generated_template_keyword_case(value, keyword_case);
    if let Some(insert_text) = item.insert_text.as_mut() {
        *insert_text = transform(insert_text);
    }
    update_completion_text_edit(item, &transform);
}

fn builtin_function_context(text: &str, position: Position) -> Option<String> {
    let offset = position_to_byte_offset(text, position).min(text.len());
    let mut scan_start = offset.saturating_sub(SIGNATURE_HELP_MAX_SCAN_BYTES);
    while scan_start < offset && !text.is_char_boundary(scan_start) {
        scan_start += 1;
    }
    let window = text.get(scan_start..offset)?;
    let masked = SqlParser::mask_sql_noise(window);
    if let Some((last_non_whitespace, character)) = window
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_whitespace())
    {
        let masked_character = masked[last_non_whitespace..].chars().next()?;
        if !character.is_whitespace() && masked_character == ' ' {
            return None;
        }
    }

    let context = SqlParser::completion_context_from_text(window, lsp_position_at_end(window));
    if !matches!(
        context,
        crate::parser::CompletionContext::SelectClause
            | crate::parser::CompletionContext::WhereClause
            | crate::parser::CompletionContext::ExpressionValueClause
            | crate::parser::CompletionContext::CaseResultClause
            | crate::parser::CompletionContext::JoinConditionClause
            | crate::parser::CompletionContext::OrderByClause
            | crate::parser::CompletionContext::GroupByClause
            | crate::parser::CompletionContext::HavingClause
            | crate::parser::CompletionContext::InsertValueClause
    ) {
        return None;
    }

    let prefix_start = identifier_path_start_before_cursor(window).unwrap_or(window.len());
    let raw_prefix = window.get(prefix_start..)?.trim();
    if raw_prefix.contains('.')
        || raw_prefix.starts_with(['"', '\'', '`', '['])
        || raw_prefix.ends_with([']', '"', '`'])
    {
        return None;
    }
    Some(SqlParser::identifier_last_part(raw_prefix).to_ascii_lowercase())
}

fn relation_function_context(text: &str, position: Position) -> Option<String> {
    let offset = position_to_byte_offset(text, position).min(text.len());
    let mut scan_start = offset.saturating_sub(SIGNATURE_HELP_MAX_SCAN_BYTES);
    while scan_start < offset && !text.is_char_boundary(scan_start) {
        scan_start += 1;
    }
    let window = text.get(scan_start..offset)?;
    let masked = SqlParser::mask_sql_noise(window);
    if let Some((last_non_whitespace, character)) = window
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_whitespace())
    {
        let masked_character = masked[last_non_whitespace..].chars().next()?;
        if !character.is_whitespace() && masked_character == ' ' {
            return None;
        }
    }

    let context = SqlParser::completion_context_from_text(window, lsp_position_at_end(window));
    if !matches!(
        context,
        crate::parser::CompletionContext::FromClause | crate::parser::CompletionContext::JoinClause
    ) {
        return None;
    }

    let prefix_start = identifier_path_start_before_cursor(window).unwrap_or(window.len());
    let raw_prefix = window.get(prefix_start..)?.trim();
    if raw_prefix.contains('.')
        || raw_prefix.starts_with(['"', '\'', '`', '['])
        || raw_prefix.ends_with([']', '"', '`'])
    {
        return None;
    }
    Some(SqlParser::identifier_last_part(raw_prefix).to_ascii_lowercase())
}

fn snippet_placeholder_label(parameter: &str) -> String {
    parameter
        .trim()
        .trim_start_matches("...")
        .trim_end_matches('?')
        .replace('\\', "\\\\")
        .replace('$', "\\$")
        .replace('}', "\\}")
}

fn builtin_function_snippet(signature: &BuiltinSignature) -> String {
    let mut placeholder = 1usize;
    let groups = signature
        .parameter_groups
        .iter()
        .map(|parameters| {
            let parameters = parameters
                .iter()
                .map(|parameter| {
                    let label = snippet_placeholder_label(parameter);
                    let value = format!("${{{placeholder}:{label}}}");
                    placeholder += 1;
                    value
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("({parameters})")
        })
        .collect::<String>();
    format!("{}{groups}", signature.name)
}

fn completion_item_is_live_routine(item: &CompletionItem) -> bool {
    item.kind == Some(CompletionItemKind::FUNCTION)
        && item.detail.as_deref().is_some_and(|detail| {
            detail.starts_with("Function:") || detail.starts_with("Procedure:")
        })
}

fn add_builtin_function_completions(
    text: &str,
    position: Position,
    dialect: &str,
    schema: Option<&Schema>,
    items: &mut Vec<CompletionItem>,
) {
    let Some(prefix) = builtin_function_context(text, position) else {
        return;
    };
    let server_version = schema.and_then(Schema::server_version_tuple);
    let mut catalog = builtin_signature_completion_catalog_for(
        dialect,
        server_version,
        &prefix,
        BUILTIN_FUNCTION_COMPLETION_MAX_ITEMS,
    );
    let full_values = builtin_value_catalog_for(dialect);
    if catalog.is_empty()
        && full_values.is_empty()
        && !builtin_function_catalog_is_available_for(dialect)
    {
        return;
    }
    let authoritative_value_names = full_values
        .iter()
        .map(|value| value.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut values = full_values
        .into_iter()
        .filter(|value| prefix.is_empty() || value.name.to_ascii_lowercase().starts_with(&prefix))
        .take(BUILTIN_FUNCTION_COMPLETION_MAX_ITEMS.saturating_sub(catalog.len()))
        .collect::<Vec<_>>();

    let live_names = items
        .iter()
        .filter(|item| completion_item_is_live_routine(item))
        .map(|item| SqlParser::identifier_last_part(&item.label).to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let catalog_names = catalog
        .iter()
        .map(|signature| signature.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let value_names = values
        .iter()
        .map(|value| value.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    items.retain(|item| {
        let name = SqlParser::identifier_last_part(&item.label).to_ascii_lowercase();
        if completion_item_is_live_routine(item) {
            return true;
        }
        if item.kind == Some(CompletionItemKind::FUNCTION)
            && !builtin_function_is_known_for(dialect, &name, server_version)
        {
            return false;
        }
        if catalog_names.contains(&name) || value_names.contains(&name) {
            return false;
        }
        !authoritative_value_names.contains(&name)
    });

    catalog.retain(|signature| !live_names.contains(&signature.name.to_ascii_lowercase()));
    values.retain(|value| !live_names.contains(&value.name.to_ascii_lowercase()));
    items.extend(catalog.into_iter().map(|signature| CompletionItem {
        label: signature.name.to_string(),
        kind: Some(CompletionItemKind::FUNCTION),
        detail: Some(format!("Built-in function: {}", signature.label())),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!(
                "Built-in function signature for the active `{dialect}` SQL profile. Live routine metadata takes precedence when available."
            ),
        })),
        sort_text: Some(format!(
            "1:builtin:{}:{}",
            signature.name.to_ascii_lowercase(),
            signature.label().to_ascii_lowercase()
        )),
        filter_text: Some(if signature
            .name
            .to_ascii_lowercase()
            .starts_with(&prefix)
        {
            signature.name.to_string()
        } else {
            format!("{prefix} {}", signature.name)
        }),
        insert_text: Some(builtin_function_snippet(&signature)),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    }));
    items.extend(values.into_iter().map(|value| CompletionItem {
        label: value.name.to_string(),
        kind: Some(CompletionItemKind::VALUE),
        detail: Some(value.category.to_string()),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!(
                "Built-in system value for the active `{dialect}` SQL profile. It is inserted as a bare expression without function-call parentheses."
            ),
        })),
        sort_text: Some(format!("1:builtin-value:{}", value.name.to_ascii_lowercase())),
        filter_text: Some(value.name.to_string()),
        insert_text: Some(value.name.to_string()),
        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
        ..Default::default()
    }));
}

fn add_clickhouse_table_function_completions(
    text: &str,
    position: Position,
    dialect: &str,
    items: &mut Vec<CompletionItem>,
) {
    let Some(prefix) = relation_function_context(text, position) else {
        return;
    };
    let catalog = builtin_table_signature_completion_catalog_for(
        dialect,
        &prefix,
        BUILTIN_FUNCTION_COMPLETION_MAX_ITEMS,
    );
    if catalog.is_empty() {
        return;
    }
    let existing_relations = items
        .iter()
        .filter(|item| is_relation_completion_kind(item.kind))
        .map(|item| item.label.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    items.extend(catalog.into_iter().filter_map(|signature| {
        if existing_relations.contains(&signature.name.to_ascii_lowercase()) {
            return None;
        }
        Some(CompletionItem {
            label: signature.name.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some(format!("ClickHouse table function: {}", signature.label())),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: "Static ClickHouse table-function signature aligned with the DBX catalog."
                    .to_string(),
            })),
            sort_text: Some(format!(
                "1:table-function:{}:{}",
                signature.name.to_ascii_lowercase(),
                signature.label().to_ascii_lowercase()
            )),
            filter_text: Some(signature.name.to_string()),
            insert_text: Some(builtin_function_snippet(&signature)),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        })
    }));
}

fn is_oracle_like_dialect(dialect: &str) -> bool {
    matches!(
        dialect.to_ascii_lowercase().as_str(),
        "oracle" | "oceanbase-oracle"
    )
}

#[derive(Debug, PartialEq, Eq)]
struct OracleTableRoutineContext {
    prefix: String,
    qualifier: Option<String>,
}

fn previous_ascii_word(text: &str) -> (&str, &str) {
    let trimmed = text.trim_end();
    let start = trimmed
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!character.is_ascii_alphanumeric() && character != '_')
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    (&trimmed[..start], &trimmed[start..])
}

fn oracle_table_routine_context(
    text: &str,
    position: Position,
) -> Option<OracleTableRoutineContext> {
    let offset = position_to_byte_offset(text, position).min(text.len());
    let mut scan_start = offset.saturating_sub(SIGNATURE_HELP_MAX_SCAN_BYTES);
    while scan_start < offset && !text.is_char_boundary(scan_start) {
        scan_start += 1;
    }
    let window = text.get(scan_start..offset)?;
    let masked = SqlParser::mask_sql_noise(window);
    let path_start = identifier_path_start_before_cursor(window).unwrap_or(window.len());
    let raw_path = window.get(path_start..)?.trim();
    let masked_path = masked.get(path_start..)?.trim();
    if raw_path.is_empty()
        || masked_path.is_empty()
        || raw_path.starts_with(['\'', '`', '['])
        || raw_path.ends_with([']', '`'])
    {
        return None;
    }

    let before_path = masked.get(..path_start)?.trim_end();
    let before_open = before_path.strip_suffix('(')?.trim_end();
    let (before_table, table_word) = previous_ascii_word(before_open);
    if !table_word.eq_ignore_ascii_case("table") {
        return None;
    }
    let (_, relation_word) = previous_ascii_word(before_table);
    if !matches!(relation_word.to_ascii_lowercase().as_str(), "from" | "join") {
        return None;
    }

    let (qualifier, prefix) = raw_path
        .rsplit_once('.')
        .map(|(qualifier, prefix)| (Some(qualifier.trim().to_string()), prefix))
        .unwrap_or((None, raw_path));
    if qualifier
        .as_ref()
        .is_some_and(|qualifier| qualifier.is_empty())
    {
        return None;
    }
    Some(OracleTableRoutineContext {
        prefix: SqlParser::identifier_last_part(prefix).to_ascii_lowercase(),
        qualifier,
    })
}

fn add_oracle_table_function_completions(
    text: &str,
    position: Position,
    dialect: &str,
    schema: Option<&Schema>,
    items: &mut Vec<CompletionItem>,
) {
    if !is_oracle_like_dialect(dialect) {
        return;
    }

    if let Some(context) = oracle_table_routine_context(text, position) {
        items.clear();
        let Some(schema) = schema else {
            return;
        };
        let existing = items
            .iter()
            .map(|item| item.label.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        items.extend(schema.functions.iter().filter_map(|function| {
            if function
                .routine_type
                .as_deref()
                .is_some_and(|routine_type| routine_type.eq_ignore_ascii_case("procedure"))
            {
                return None;
            }
            let simple_name = SqlParser::identifier_last_part(&function.name);
            if !context.prefix.is_empty()
                && !simple_name
                    .to_ascii_lowercase()
                    .starts_with(&context.prefix)
            {
                return None;
            }
            if existing.contains(&simple_name.to_ascii_lowercase()) {
                return None;
            }
            let apply_name = context
                .qualifier
                .as_ref()
                .map(|qualifier| format!("{qualifier}.{simple_name}"))
                .unwrap_or_else(|| function.name.clone());
            let arguments = function
                .parameters
                .iter()
                .enumerate()
                .map(|(index, parameter)| {
                    let label = if parameter.name.trim().is_empty() {
                        parameter.data_type.as_str()
                    } else {
                        parameter.name.as_str()
                    };
                    format!("${{{}:{}}}", index + 1, snippet_placeholder_label(label))
                })
                .collect::<Vec<_>>()
                .join(", ");
            Some(CompletionItem {
                label: simple_name.to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(format!(
                    "Oracle table function: {} -> {}",
                    function.signature(),
                    function.return_type
                )),
                documentation: Some(Documentation::String(function.documentation())),
                sort_text: Some(format!(
                    "0:oracle-table-routine:{}",
                    simple_name.to_ascii_lowercase()
                )),
                filter_text: Some(simple_name.to_string()),
                insert_text: Some(format!("{apply_name}({arguments})")),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            })
        }));
        return;
    }

    let Some(prefix) = relation_function_context(text, position) else {
        return;
    };
    const ORACLE_TABLE_FUNCTIONS: &[(&str, &[&str], &str)] = &[
        ("TABLE", &["function_call"], "Oracle table function"),
        ("THE", &["subquery"], "Oracle nested-table expression"),
        ("XMLTABLE", &["xpath"], "XML to relational rows"),
        ("JSON_TABLE", &["expr", "path"], "JSON to relational rows"),
    ];
    let existing_relations = items
        .iter()
        .filter(|item| is_relation_completion_kind(item.kind))
        .map(|item| item.label.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    items.extend(
        ORACLE_TABLE_FUNCTIONS
            .iter()
            .filter(|(name, _, _)| {
                prefix.is_empty() || name.to_ascii_lowercase().starts_with(&prefix)
            })
            .filter_map(|(name, parameters, detail)| {
                if existing_relations.contains(&name.to_ascii_lowercase()) {
                    return None;
                }
                let arguments = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| format!("${{{}:{parameter}}}", index + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(CompletionItem {
                    label: (*name).to_string(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some((*detail).to_string()),
                    documentation: Some(Documentation::String(format!(
                        "Oracle-family relation-source block for `{name}`."
                    ))),
                    sort_text: Some(format!(
                        "1:oracle-table-function:{}",
                        name.to_ascii_lowercase()
                    )),
                    filter_text: Some((*name).to_string()),
                    insert_text: Some(format!("{name}({arguments})")),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                })
            }),
    );
}

fn add_window_function_completions(
    text: &str,
    position: Position,
    dialect: &str,
    items: &mut Vec<CompletionItem>,
) {
    if !builtin_window_function_catalog_is_available_for(dialect) {
        return;
    }
    let Some(prefix) = builtin_function_context(text, position) else {
        return;
    };
    const WINDOW_FUNCTIONS: &[(&str, &[&str])] = &[
        ("ROW_NUMBER", &[]),
        ("RANK", &[]),
        ("DENSE_RANK", &[]),
        ("LAG", &["value"]),
        ("LEAD", &["value"]),
        ("FIRST_VALUE", &["value"]),
        ("LAST_VALUE", &["value"]),
        ("NTILE", &["buckets"]),
    ];
    let candidates = WINDOW_FUNCTIONS
        .iter()
        .filter(|(name, _)| prefix.is_empty() || name.to_ascii_lowercase().starts_with(&prefix))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return;
    }
    let candidate_names = candidates
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let live_names = items
        .iter()
        .filter(|item| completion_item_is_live_routine(item))
        .map(|item| SqlParser::identifier_last_part(&item.label).to_ascii_lowercase())
        .collect::<HashSet<_>>();
    items.retain(|item| {
        let name = SqlParser::identifier_last_part(&item.label).to_ascii_lowercase();
        completion_item_is_live_routine(item)
            || !matches!(
                item.kind,
                Some(CompletionItemKind::FUNCTION | CompletionItemKind::KEYWORD)
            )
            || !candidate_names.contains(&name)
    });

    items.extend(candidates.into_iter().filter_map(|(name, parameters)| {
        if live_names.contains(&name.to_ascii_lowercase()) {
            return None;
        }
        let mut placeholder = 1usize;
        let arguments = parameters
            .iter()
            .map(|parameter| {
                let argument = format!("${{{placeholder}:{parameter}}}");
                placeholder += 1;
                argument
            })
            .collect::<Vec<_>>()
            .join(", ");
        let partition = placeholder;
        let order = placeholder + 1;
        Some(CompletionItem {
            label: (*name).to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("Window function with OVER clause".to_string()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!(
                    "Portable window-function block for the active `{dialect}` SQL profile. Edit or remove the partition expression as needed."
                ),
            })),
            sort_text: Some(format!("1:window-function:{}", name.to_ascii_lowercase())),
            filter_text: Some((*name).to_string()),
            insert_text: Some(format!(
                "{name}({arguments}) OVER (PARTITION BY ${{{partition}:column}} ORDER BY ${{{order}:column}})"
            )),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        })
    }));
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
        .to_lowercase();
    if prefix.is_empty() {
        return;
    }

    // Alias discovery is a bounded, text-only scope scan.  It deliberately
    // avoids constructing a second tree-sitter parse after the dialect has
    // already produced its semantic completion candidates.
    let aliases = SqlParser::relation_aliases_at_position(text, position);
    for alias in aliases {
        let Some(match_rank) = identifier_match_rank(&alias.name, &prefix) else {
            continue;
        };
        let normalized = alias.name.to_lowercase();
        if items.iter().any(|item| {
            item.kind == Some(CompletionItemKind::VARIABLE)
                && item.label == alias.name
                && item.insert_text.as_deref() == Some(alias.sql.as_str())
        }) {
            continue;
        }
        items.push(CompletionItem {
            label: alias.name.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some(format!("Table alias · {}", alias.relation)),
            filter_text: Some(alias.name.clone()),
            insert_text: Some(alias.sql),
            sort_text: Some(format!("-2:alias:{match_rank}:{normalized}")),
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
    } else if dialect_name == "sqlserver" {
        format!("[{}]", identifier.replace(']', "]]"))
    } else {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
}

const INSERT_STATEMENT_SNIPPET_MAX_TABLES: usize = 32;
const INSERT_STATEMENT_SNIPPET_MAX_COLUMNS: usize = 64;

fn add_insert_statement_snippets(
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
    let fragment = &text_before_cursor[statement_start..];
    if fragment.chars().last().is_some_and(char::is_whitespace) {
        return;
    }
    let prefix = fragment.trim().to_ascii_lowercase();
    if prefix.len() < 2
        || !prefix
            .chars()
            .all(|character| character.is_ascii_alphabetic())
        || !"insert".starts_with(&prefix)
    {
        return;
    }

    let insert = keyword_with_completion_case("INSERT", preferences.keyword_case);
    let into = keyword_with_completion_case("INTO", preferences.keyword_case);
    let values = keyword_with_completion_case("VALUES", preferences.keyword_case);
    let schema_name = (!schema.database.trim().is_empty())
        .then(|| quote_completion_identifier(schema.database.trim(), dialect_name));
    let mut tables = schema.tables.iter().collect::<Vec<_>>();
    tables.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });

    let mut emitted_tables = 0usize;
    for table in tables {
        if table
            .object_type
            .as_deref()
            .is_some_and(|kind| kind.to_ascii_lowercase().contains("view"))
        {
            continue;
        }
        let writable = table
            .columns
            .iter()
            .filter(|column| !column.auto_increment && !column.generated && !column.hidden)
            .collect::<Vec<_>>();
        if writable.is_empty() || writable.len() > INSERT_STATEMENT_SNIPPET_MAX_COLUMNS {
            continue;
        }
        if emitted_tables >= INSERT_STATEMENT_SNIPPET_MAX_TABLES {
            break;
        }
        emitted_tables += 1;
        let required = writable
            .iter()
            .copied()
            .filter(|column| {
                !column.nullable
                    && column
                        .default_value
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty())
            })
            .collect::<Vec<_>>();
        if !required.is_empty() && required.len() != writable.len() {
            push_insert_statement_snippet(
                items,
                table,
                schema_name.as_deref(),
                &required,
                &insert,
                &into,
                &values,
                dialect_name,
                true,
            );
        }
        push_insert_statement_snippet(
            items,
            table,
            schema_name.as_deref(),
            &writable,
            &insert,
            &into,
            &values,
            dialect_name,
            false,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_insert_statement_snippet(
    items: &mut Vec<CompletionItem>,
    table: &Table,
    schema_name: Option<&str>,
    columns: &[&Column],
    insert: &str,
    into: &str,
    values: &str,
    dialect_name: &str,
    required_only: bool,
) {
    let quoted_table = quote_completion_identifier(&table.name, dialect_name);
    let relation = schema_name
        .map(|schema| format!("{schema}.{quoted_table}"))
        .unwrap_or(quoted_table);
    let quoted_columns = columns
        .iter()
        .map(|column| quote_completion_identifier(&column.name, dialect_name))
        .collect::<Vec<_>>();
    let column_lines = quoted_columns.join(",\n  ");
    let value_lines = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let placeholder = column
                .name
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() || character == '_' {
                        character
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            format!("${{{}:{placeholder}_value}}", index + 1)
        })
        .collect::<Vec<_>>()
        .join(",\n  ");
    let label = if required_only {
        format!("insert {} required", table.name)
    } else {
        format!("insert {}", table.name)
    };
    if items.iter().any(|item| item.label == label) {
        return;
    }
    items.push(CompletionItem {
        label,
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(format!(
            "{} INSERT · {} writable column{}",
            if required_only {
                "Required"
            } else {
                "Complete"
            },
            columns.len(),
            if columns.len() == 1 { "" } else { "s" }
        )),
        filter_text: Some(format!("ins insert {}", table.name)),
        sort_text: Some(format!(
            "0:{}:{}",
            if required_only { 0 } else { 1 },
            table.name.to_ascii_lowercase()
        )),
        insert_text: Some(format!(
            "{insert} {into} {relation} (\n  {column_lines}\n)\n{values} (\n  {value_lines}\n);"
        )),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    });
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
    let writable_columns = table
        .columns
        .iter()
        .filter(|column| !column.auto_increment && !column.generated && !column.hidden)
        .collect::<Vec<_>>();
    if writable_columns.is_empty() {
        return;
    }

    let required_columns = writable_columns
        .iter()
        .copied()
        .filter(|column| {
            !column.nullable
                && column
                    .default_value
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
        })
        .collect::<Vec<_>>();

    if !required_columns.is_empty() && required_columns.len() != writable_columns.len() {
        add_insert_columns_completion_item(
            table,
            &format!("{}.required", table.name),
            "Required writable",
            &required_columns,
            items,
            preferences,
            dialect_name,
            0,
        );
    }
    add_insert_columns_completion_item(
        table,
        &format!("{}.*", table.name),
        "All writable",
        &writable_columns,
        items,
        preferences,
        dialect_name,
        1,
    );
}

#[allow(clippy::too_many_arguments)]
fn add_insert_columns_completion_item(
    table: &Table,
    label: &str,
    detail_prefix: &str,
    source_columns: &[&Column],
    items: &mut Vec<CompletionItem>,
    preferences: &CompletionPreferences,
    dialect_name: &str,
    sort_rank: usize,
) {
    if source_columns.is_empty() || items.iter().any(|item| item.label == label) {
        return;
    }
    let columns = source_columns
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
        label: label.to_string(),
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(format!(
            "{detail_prefix} · {} columns: {}) {} (value, ...)",
            columns.len(),
            columns.join(", "),
            values_keyword
        )),
        sort_text: Some(format!(
            "0:{sort_rank:04}:{}",
            table.name.to_ascii_lowercase()
        )),
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
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        self.completion_documentation_resolve_supported.store(
            client_supports_completion_documentation_resolve(&params),
            Ordering::Relaxed,
        );
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
                    resolve_provider: Some(true),
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

            if let Some(preferences_value) = settings.get("formattingPreferences") {
                match serde_json::from_value::<FormattingPreferences>(preferences_value.clone()) {
                    Ok(preferences) => {
                        if let Ok(mut current_preferences) = self.formatting_preferences.write() {
                            *current_preferences = preferences;
                        }
                    }
                    Err(error) => {
                        self.client
                            .log_message(
                                MessageType::WARNING,
                                format!("Ignoring invalid formatting preferences: {error}"),
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
        self.project_sql_index_cache.remove(&uri);
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        let Some(mut items) = self
            .completion_candidates_at(&uri, &text, position, params.context.as_ref())
            .await
        else {
            return Ok(None);
        };
        if self
            .completion_documentation_resolve_supported
            .load(Ordering::Relaxed)
        {
            defer_completion_documentation(
                &mut items,
                &self.completion_resolve_cache,
                &self.next_completion_resolve_id,
            );
        }
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        Ok(resolve_completion_documentation(
            item,
            &self.completion_resolve_cache,
        ))
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
            if let Some(target) = self.project_sql_symbol_at_position(&uri, &text, position) {
                if let Some(location) = self.project_sql_definition(
                    &uri,
                    dialect.name(),
                    schema.as_ref().map(|schema| schema.id),
                    &target,
                ) {
                    return Ok(Some(GotoDefinitionResponse::Scalar(location)));
                }
            }
        }

        Ok(None)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let document_uri = params.text_document_position.text_document.uri;
        let uri = document_uri.to_string();
        let position = params.text_document_position.position;
        let include_declarations = params.context.include_declaration;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_position(&uri, &text, position);
            let mut locations = dialect.references(&text, position, schema.as_ref()).await;
            rewrite_current_document_location_uris(&mut locations, &document_uri);
            if let Some(target) = self.project_sql_symbol_at_position(&uri, &text, position) {
                locations = self.matching_project_sql_locations(
                    &uri,
                    dialect.name(),
                    schema.as_ref().map(|schema| schema.id),
                    &target,
                    include_declarations,
                );
            }
            deduplicate_locations(&mut locations);
            locations.truncate(PROJECT_SQL_INDEX_MAX_RESULTS);
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
        let Some(call) = routine_call_at_position(&text, position) else {
            return Ok(None);
        };
        let schema = self.get_schema_for_position(&uri, &text, position);
        let mut live_overloads = schema
            .as_ref()
            .map(|schema| {
                schema
                    .functions
                    .iter()
                    .filter(|function| routine_names_match(&function.name, &call.name))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        live_overloads.sort_by_key(|function| !live_overload_accepts_call(function, &call));
        live_overloads.truncate(SIGNATURE_HELP_MAX_OVERLOADS);

        let mut signatures = live_overloads
            .iter()
            .filter(|_| call.active_group == 0)
            .map(|function| live_signature_information(function, &call))
            .collect::<Vec<_>>();
        if signatures.is_empty() {
            let dialect = self
                .dialect_identity_for_file(&uri)
                .or_else(|| {
                    self.get_dialect_for_file(&uri)
                        .map(|dialect| dialect.name().into())
                })
                .unwrap_or_else(|| self.default_dialect_name());
            let mut builtins = builtin_signatures_for(
                &dialect,
                &call.name,
                schema.as_ref().and_then(Schema::server_version_tuple),
            );
            builtins.sort_by_key(|signature| !builtin_overload_accepts_call(signature, &call));
            builtins.truncate(SIGNATURE_HELP_MAX_OVERLOADS);
            signatures = builtins
                .iter()
                .filter_map(|signature| builtin_signature_information(signature, &call, &dialect))
                .collect();
        }
        if signatures.is_empty() {
            return Ok(None);
        }
        let active_parameter = signatures[0]
            .active_parameter
            .unwrap_or(call.active_parameter);
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
        if let Some(target) = self.project_sql_symbol_at_position(&uri, &text, params.position) {
            let schema = self.get_schema_for_position(&uri, &text, params.position);
            return Ok(self
                .project_sql_symbol_is_renameable(&uri, dialect.name(), schema.as_ref(), &target)
                .then_some(PrepareRenameResponse::Range(target.range)));
        }
        let Some(range) = cte_identifier_range_at_position(&text, params.position, dialect.name())
        else {
            return Ok(None);
        };
        let schema = self.get_schema_for_position(&uri, &text, params.position);
        Ok((!dialect
            .references(&text, params.position, schema.as_ref())
            .await
            .is_empty())
        .then_some(PrepareRenameResponse::Range(range)))
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
        let target = self.project_sql_symbol_at_position(&uri, &text, position);
        if target.is_none()
            && cte_identifier_range_at_position(&text, position, dialect.name()).is_some()
        {
            let mut locations = dialect.references(&text, position, schema.as_ref()).await;
            let Ok(document_uri) = Url::parse(&uri) else {
                return Ok(None);
            };
            rewrite_current_document_location_uris(&mut locations, &document_uri);
            deduplicate_locations(&mut locations);
            let edits = locations
                .into_iter()
                .map(|location| TextEdit {
                    range: location.range,
                    new_text: params.new_name.clone(),
                })
                .collect::<Vec<_>>();
            if edits.is_empty() {
                return Ok(None);
            }
            return Ok(Some(WorkspaceEdit {
                changes: Some(HashMap::from([(document_uri, edits)])),
                document_changes: None,
                change_annotations: None,
            }));
        }
        let Some(target) = target else {
            return Ok(None);
        };
        if !self.project_sql_symbol_is_renameable(&uri, dialect.name(), schema.as_ref(), &target) {
            return Ok(None);
        }
        let mut changes = HashMap::new();
        for location in self.matching_project_sql_locations(
            &uri,
            dialect.name(),
            schema.as_ref().map(|schema| schema.id),
            &target,
            true,
        ) {
            changes
                .entry(location.uri)
                .or_insert_with(Vec::new)
                .push(TextEdit {
                    range: location.range,
                    new_text: params.new_name.clone(),
                });
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
        let dialect_identity = self
            .dialect_identity_for_file(&uri_string)
            .unwrap_or_else(|| dialect.name().to_string());
        let schema = self.get_schema_for_position(&uri_string, &text, params.range.start);
        let mut actions = Vec::new();
        // Formatting is already exposed through textDocument/formatting. Do
        // not format the full console for every automatic lightbulb request;
        // build the source action only when the client explicitly asks for it.
        if code_action_kind_explicitly_requested(&params.context, &CodeActionKind::SOURCE) {
            let formatted = dialect.format(&text).await;
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
        }
        if code_action_kind_available(&params.context, &CodeActionKind::REFACTOR_REWRITE) {
            if let Some(action) = expand_select_star_action(
                &text,
                &uri,
                params.range,
                schema.clone(),
                &dialect_identity,
            ) {
                actions.push(CodeActionOrCommand::CodeAction(action));
            }
            actions.extend(
                qualify_identifier_actions(
                    &text,
                    &uri,
                    params.range,
                    schema.as_ref(),
                    &dialect_identity,
                )
                .into_iter()
                .map(CodeActionOrCommand::CodeAction),
            );
        }
        for diagnostic in params
            .context
            .diagnostics
            .iter()
            .filter(|_| code_action_kind_available(&params.context, &CodeActionKind::QUICKFIX))
        {
            if matches!(
                diagnostic.code.as_ref(),
                Some(NumberOrString::String(code)) if code == "OXIDE001"
            ) {
                if let Some(action) = add_mutation_safety_guard_action(&text, &uri, diagnostic) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
            } else if matches!(
                diagnostic.code.as_ref(),
                Some(NumberOrString::String(code)) if code == "OXIDE002"
            ) {
                actions.extend(
                    add_join_condition_actions(
                        &text,
                        &uri,
                        diagnostic,
                        schema.as_ref(),
                        &dialect_identity,
                    )
                    .into_iter()
                    .map(CodeActionOrCommand::CodeAction),
                );
            }
        }
        Ok(Some(actions))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let preferences = self
                .formatting_preferences
                .read()
                .map(|preferences| preferences.clone())
                .unwrap_or_default();
            let formatted = if dialect_uses_sql_preferences(dialect.name()) {
                let formatted =
                    format_sql_pretty_with_keyword_case(&text, preferences.keyword_uppercase());
                let formatted =
                    apply_formatter_function_case(&formatted, preferences.function_uppercase());
                apply_formatter_layout(
                    &formatted,
                    preferences.logical_operator_layout(),
                    matches!(preferences.from_clause_layout, FromClauseLayout::SameLine),
                )
            } else {
                dialect.format(&text).await
            };
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
        let preferences = self
            .formatting_preferences
            .read()
            .map(|preferences| preferences.clone())
            .unwrap_or_default();
        let formatted = if dialect_uses_sql_preferences(dialect.name()) {
            let formatted =
                format_sql_pretty_with_keyword_case(selected, preferences.keyword_uppercase());
            let formatted =
                apply_formatter_function_case(&formatted, preferences.function_uppercase());
            apply_formatter_layout(
                &formatted,
                preferences.logical_operator_layout(),
                matches!(preferences.from_clause_layout, FromClauseLayout::SameLine),
            )
        } else {
            dialect.format(selected).await
        };
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
        let project_symbols = self.project_sql_index_for_document(&uri, &text);
        let symbols = document_symbols(
            &text,
            self.get_schema_for_file(&uri).as_ref(),
            &project_symbols,
        );
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
        let mut hints = insert_value_hints(&text, params.range, &schema);
        hints.extend(routine_parameter_hints(&text, params.range, &schema));
        hints.sort_by_key(|hint| (hint.position.line, hint.position.character));
        hints.truncate(INLAY_HINT_MAX_ITEMS);
        Ok(Some(hints))
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
        for (uri, text) in self.project_sql_documents() {
            let Ok(location_uri) = Url::parse(&uri) else {
                continue;
            };
            let container_name = project_sql_symbol_container(&location_uri);
            for occurrence in self.project_sql_index_for_document(&uri, &text) {
                if occurrence.role != ProjectSqlSymbolRole::Definition
                    || !workspace_symbol_matches(
                        &query,
                        &occurrence.name,
                        &format!("{} {}", occurrence.normalized_name, container_name),
                    )
                {
                    continue;
                }
                symbols.push(SymbolInformation {
                    name: occurrence.name,
                    kind: occurrence.kind.symbol_kind(),
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: location_uri.clone(),
                        range: occurrence.range,
                    },
                    container_name: Some(container_name.clone()),
                });
                if symbols.len() >= PROJECT_SQL_INDEX_MAX_RESULTS {
                    break;
                }
            }
            if symbols.len() >= PROJECT_SQL_INDEX_MAX_RESULTS {
                break;
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
        symbols.dedup_by(|left, right| {
            left.name.eq_ignore_ascii_case(&right.name)
                && left.location == right.location
                && left.kind == right.kind
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

fn cte_identifier_range_at_position(
    text: &str,
    position: Position,
    dialect: &str,
) -> Option<Range> {
    let identifier = identifier_at_position(text, position, dialect)?;
    let parser = SqlParser::new();
    parser
        .extract_common_table_expressions(text)
        .into_iter()
        .any(|cte| SqlParser::identifier_last_part(&cte).eq_ignore_ascii_case(&identifier))
        .then(|| identifier_range_at_position(text, position, dialect))?
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

fn range_contains_position(range: Range, position: Position) -> bool {
    position >= range.start && position <= range.end
}

fn project_uri_scope(uri: &str) -> Option<&str> {
    let prefix = "oxide://project/";
    let rest = uri.strip_prefix(prefix)?;
    let project_end = rest.find('/').unwrap_or(rest.len());
    Some(&rest[..project_end])
}

fn documents_share_project_sql_scope(
    origin_uri: &str,
    candidate_uri: &str,
    origin_schema_id: Option<SchemaId>,
    candidate_schema_id: Option<SchemaId>,
) -> bool {
    match (origin_schema_id, candidate_schema_id) {
        (Some(origin), Some(candidate)) => origin == candidate,
        (Some(_), None) | (None, Some(_)) => false,
        (None, None) => {
            origin_uri == candidate_uri
                || project_uri_scope(origin_uri)
                    .zip(project_uri_scope(candidate_uri))
                    .is_some_and(|(origin, candidate)| origin == candidate)
        }
    }
}

fn project_sql_symbols_match(
    left: &ProjectSqlSymbolOccurrence,
    right: &ProjectSqlSymbolOccurrence,
) -> bool {
    if left.kind.is_routine() != right.kind.is_routine() {
        return false;
    }
    if left.kind.is_routine()
        && left.kind != right.kind
        && left.role == ProjectSqlSymbolRole::Definition
        && right.role == ProjectSqlSymbolRole::Definition
    {
        return false;
    }
    let left_parts = left.normalized_name.split('.').collect::<Vec<_>>();
    let right_parts = right.normalized_name.split('.').collect::<Vec<_>>();
    let shared = left_parts.len().min(right_parts.len());
    shared > 0
        && left_parts[left_parts.len() - shared..] == right_parts[right_parts.len() - shared..]
}

fn project_sql_symbol_container(uri: &Url) -> String {
    if uri.scheme() == "oxide" && uri.host_str() == Some("project") {
        return uri
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|path| !path.is_empty())
            .unwrap_or("Project SQL")
            .to_string();
    }
    uri.path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|path| !path.is_empty())
        .unwrap_or("SQL document")
        .to_string()
}

fn project_sql_symbol_occurrences(source: &str) -> Vec<ProjectSqlSymbolOccurrence> {
    if source.len() > PROJECT_SQL_INDEX_MAX_BYTES {
        return Vec::new();
    }
    let searchable = SqlParser::mask_sql_noise(source);
    let upper = searchable.to_ascii_uppercase();
    let cte_scopes = project_cte_scopes(source, &searchable);
    let mut occurrences = Vec::new();

    let definitions = [
        ("CREATE TABLE", ProjectSqlSymbolKind::Table),
        ("CREATE OR REPLACE TABLE", ProjectSqlSymbolKind::Table),
        ("CREATE TEMP TABLE", ProjectSqlSymbolKind::Table),
        ("CREATE TEMPORARY TABLE", ProjectSqlSymbolKind::Table),
        ("CREATE GLOBAL TEMPORARY TABLE", ProjectSqlSymbolKind::Table),
        ("CREATE FOREIGN TABLE", ProjectSqlSymbolKind::Table),
        ("CREATE EXTERNAL TABLE", ProjectSqlSymbolKind::Table),
        ("CREATE VIEW", ProjectSqlSymbolKind::View),
        ("CREATE OR REPLACE VIEW", ProjectSqlSymbolKind::View),
        ("CREATE MATERIALIZED VIEW", ProjectSqlSymbolKind::View),
        (
            "CREATE OR REPLACE MATERIALIZED VIEW",
            ProjectSqlSymbolKind::View,
        ),
        ("CREATE FUNCTION", ProjectSqlSymbolKind::Function),
        ("CREATE OR REPLACE FUNCTION", ProjectSqlSymbolKind::Function),
        ("CREATE PROCEDURE", ProjectSqlSymbolKind::Procedure),
        (
            "CREATE OR REPLACE PROCEDURE",
            ProjectSqlSymbolKind::Procedure,
        ),
    ];
    for (phrase, kind) in definitions {
        for start in project_keyword_phrase_starts(&upper, phrase) {
            let mut name_start = start + phrase.len();
            name_start =
                skip_project_keyword_sequence(source, &upper, name_start, &["IF", "NOT", "EXISTS"]);
            push_project_sql_occurrence(
                source,
                name_start,
                kind,
                ProjectSqlSymbolRole::Definition,
                &cte_scopes,
                &mut occurrences,
            );
        }
    }

    let references = [
        ("ALTER TABLE", ProjectSqlSymbolKind::Table),
        ("DROP TABLE", ProjectSqlSymbolKind::Table),
        ("TRUNCATE TABLE", ProjectSqlSymbolKind::Table),
        ("ALTER VIEW", ProjectSqlSymbolKind::View),
        ("DROP VIEW", ProjectSqlSymbolKind::View),
        ("REFRESH MATERIALIZED VIEW", ProjectSqlSymbolKind::View),
        ("DROP MATERIALIZED VIEW", ProjectSqlSymbolKind::View),
        ("INSERT INTO", ProjectSqlSymbolKind::Table),
        ("MERGE INTO", ProjectSqlSymbolKind::Table),
        ("DELETE FROM", ProjectSqlSymbolKind::Table),
        ("REFERENCES", ProjectSqlSymbolKind::Table),
        ("CALL", ProjectSqlSymbolKind::Procedure),
    ];
    for (phrase, kind) in references {
        for start in project_keyword_phrase_starts(&upper, phrase) {
            let mut name_start = start + phrase.len();
            name_start =
                skip_project_keyword_sequence(source, &upper, name_start, &["IF", "EXISTS"]);
            name_start = skip_project_relation_modifiers(source, &upper, name_start);
            push_project_sql_occurrence(
                source,
                name_start,
                kind,
                ProjectSqlSymbolRole::Reference,
                &cte_scopes,
                &mut occurrences,
            );
        }
    }

    for start in local_relation_source_starts(&upper) {
        let name_start = skip_project_relation_modifiers(source, &upper, start);
        push_project_sql_occurrence(
            source,
            name_start,
            ProjectSqlSymbolKind::Table,
            ProjectSqlSymbolRole::Reference,
            &cte_scopes,
            &mut occurrences,
        );
    }

    for start in project_keyword_phrase_starts(&upper, "UPDATE") {
        if project_update_is_relation_target(&upper, start) {
            let name_start =
                skip_project_relation_modifiers(source, &upper, start + "UPDATE".len());
            push_project_sql_occurrence(
                source,
                name_start,
                ProjectSqlSymbolKind::Table,
                ProjectSqlSymbolRole::Reference,
                &cte_scopes,
                &mut occurrences,
            );
        }
    }

    append_project_function_call_occurrences(source, &searchable, &mut occurrences);

    occurrences.sort_by_key(|occurrence| {
        (
            occurrence.range.start.line,
            occurrence.range.start.character,
            occurrence.range.end.line,
            occurrence.range.end.character,
            occurrence.role == ProjectSqlSymbolRole::Reference,
        )
    });
    occurrences.dedup_by(|left, right| {
        left.range == right.range && left.role == right.role && left.kind == right.kind
    });
    occurrences.truncate(PROJECT_SQL_INDEX_MAX_OCCURRENCES);
    occurrences
}

fn project_keyword_phrase_starts(source_upper: &str, phrase: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut search = 0;
    while let Some(relative) = source_upper[search..].find(phrase) {
        let start = search + relative;
        if local_keyword_boundary(source_upper, start, phrase) {
            starts.push(start);
        }
        search = start + phrase.len();
    }
    starts
}

fn skip_project_keyword_sequence(
    source: &str,
    source_upper: &str,
    start: usize,
    keywords: &[&str],
) -> usize {
    let original = skip_local_whitespace(source, start);
    let mut cursor = original;
    for keyword in keywords {
        cursor = skip_local_whitespace(source, cursor);
        if !source_upper[cursor..].starts_with(keyword)
            || !local_keyword_boundary(source_upper, cursor, keyword)
        {
            return original;
        }
        cursor += keyword.len();
    }
    skip_local_whitespace(source, cursor)
}

fn skip_project_relation_modifiers(source: &str, source_upper: &str, start: usize) -> usize {
    let mut cursor = skip_local_whitespace(source, start);
    loop {
        let mut advanced = false;
        for keyword in ["ONLY", "LATERAL"] {
            if source_upper[cursor..].starts_with(keyword)
                && local_keyword_boundary(source_upper, cursor, keyword)
            {
                cursor = skip_local_whitespace(source, cursor + keyword.len());
                advanced = true;
                break;
            }
        }
        if !advanced {
            return cursor;
        }
    }
}

fn read_project_identifier_path_after(
    source: &str,
    start: usize,
) -> Option<(String, String, usize, usize, usize)> {
    let (first, first_start, mut path_end) = read_local_identifier_after(source, start)?;
    let mut parts = vec![first];
    let (mut selection_start, mut selection_end) =
        project_identifier_selection_range(source, first_start, path_end);
    loop {
        let dot = skip_local_whitespace(source, path_end);
        if !source[dot..].starts_with('.') {
            break;
        }
        let after_dot = skip_local_whitespace(source, dot + 1);
        if source[after_dot..].starts_with('.') {
            parts.push("dbo".to_string());
            path_end = after_dot;
            continue;
        }
        let Some((part, part_start, part_end)) = read_local_identifier_after(source, after_dot)
        else {
            break;
        };
        parts.push(part);
        (selection_start, selection_end) =
            project_identifier_selection_range(source, part_start, part_end);
        path_end = part_end;
    }
    let path = parts.join(".");
    let name = parts.last()?.clone();
    Some((path, name, selection_start, selection_end, path_end))
}

fn project_identifier_selection_range(source: &str, start: usize, end: usize) -> (usize, usize) {
    let Some(first) = source[start..end].chars().next() else {
        return (start, end);
    };
    let Some(last) = source[start..end].chars().next_back() else {
        return (start, end);
    };
    if matches!((first, last), ('"', '"') | ('`', '`') | ('[', ']')) {
        (start + first.len_utf8(), end - last.len_utf8())
    } else {
        (start, end)
    }
}

fn push_project_sql_occurrence(
    source: &str,
    start: usize,
    kind: ProjectSqlSymbolKind,
    role: ProjectSqlSymbolRole,
    cte_scopes: &[(String, usize, usize)],
    occurrences: &mut Vec<ProjectSqlSymbolOccurrence>,
) {
    if occurrences.len() >= PROJECT_SQL_INDEX_MAX_OCCURRENCES || source[start..].starts_with('(') {
        return;
    }
    let Some((path, name, selection_start, selection_end, _)) =
        read_project_identifier_path_after(source, start)
    else {
        return;
    };
    if Keywords::is_keyword(&name) {
        return;
    }
    let normalized_name = SqlParser::normalize_identifier(&path).to_ascii_lowercase();
    if role == ProjectSqlSymbolRole::Reference
        && !kind.is_routine()
        && !normalized_name.contains('.')
        && cte_scopes.iter().any(|(cte, scope_start, scope_end)| {
            cte == &normalized_name
                && selection_start >= *scope_start
                && selection_start < *scope_end
        })
    {
        return;
    }
    occurrences.push(ProjectSqlSymbolOccurrence {
        name,
        normalized_name,
        kind,
        role,
        range: range_for_offsets(source, selection_start, selection_end),
    });
}

fn append_project_function_call_occurrences(
    source: &str,
    searchable: &str,
    occurrences: &mut Vec<ProjectSqlSymbolOccurrence>,
) {
    let mut cursor = 0;
    while cursor < searchable.len() && occurrences.len() < PROJECT_SQL_INDEX_MAX_OCCURRENCES {
        let Some(character) = searchable[cursor..].chars().next() else {
            break;
        };
        let previous = searchable[..cursor].chars().next_back();
        if !(character == '_' || character.is_alphabetic())
            || previous.is_some_and(|value| value == '_' || value == '$' || value.is_alphanumeric())
        {
            cursor += character.len_utf8();
            continue;
        }
        let Some((path, name, selection_start, selection_end, path_end)) =
            read_project_identifier_path_after(source, cursor)
        else {
            cursor += character.len_utf8();
            continue;
        };
        cursor = path_end.max(cursor + character.len_utf8());
        let after_path = skip_local_whitespace(source, path_end);
        if !source[after_path..].starts_with('(') || Keywords::is_keyword(&name) {
            continue;
        }
        let range = range_for_offsets(source, selection_start, selection_end);
        if occurrences.iter().any(|occurrence| {
            occurrence.range == range && occurrence.role == ProjectSqlSymbolRole::Definition
        }) {
            continue;
        }
        if occurrences.iter().any(|occurrence| {
            occurrence.range == range
                && occurrence.role == ProjectSqlSymbolRole::Reference
                && occurrence.kind.is_routine()
        }) {
            continue;
        }
        occurrences.retain(|occurrence| {
            !(occurrence.range == range
                && occurrence.role == ProjectSqlSymbolRole::Reference
                && !occurrence.kind.is_routine())
        });
        occurrences.push(ProjectSqlSymbolOccurrence {
            name,
            normalized_name: SqlParser::normalize_identifier(&path).to_ascii_lowercase(),
            kind: ProjectSqlSymbolKind::Function,
            role: ProjectSqlSymbolRole::Reference,
            range,
        });
    }
}

fn project_cte_scopes(source: &str, searchable: &str) -> Vec<(String, usize, usize)> {
    let parser = SqlParser::new();
    let mut scopes = Vec::new();
    let mut statement_start = 0;
    for statement_end in searchable
        .match_indices(';')
        .map(|(position, _)| position + 1)
        .chain(std::iter::once(source.len()))
    {
        if statement_end <= statement_start {
            continue;
        }
        for cte in parser.extract_common_table_expressions(&source[statement_start..statement_end])
        {
            scopes.push((
                SqlParser::normalize_identifier(&cte).to_ascii_lowercase(),
                statement_start,
                statement_end,
            ));
        }
        statement_start = statement_end;
    }
    scopes
}

fn project_update_is_relation_target(source_upper: &str, update_start: usize) -> bool {
    let statement_start = source_upper[..update_start]
        .rfind(';')
        .map_or(0, |position| position + 1);
    let prefix = source_upper[statement_start..update_start].trim();
    prefix.is_empty()
        || (prefix.starts_with("WITH") && !prefix.contains("ON DUPLICATE KEY"))
        || prefix.ends_with("EXPLAIN")
        || prefix.ends_with("EXPLAIN ANALYZE")
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

fn join_without_condition_span(normalized: &str) -> Option<(usize, usize)> {
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
            let relation_end = join_clause.trim_end_matches(';').trim_end().len();
            return Some((offset, offset + "JOIN".len() + relation_end));
        }
    }
    None
}

fn join_without_condition_offset(normalized: &str) -> Option<usize> {
    join_without_condition_span(normalized).map(|(offset, _)| offset)
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct RoutineCallContext {
    name: String,
    active_group: u32,
    active_parameter: u32,
    current_argument_has_content: bool,
}

fn previous_char_start(text: &str, end: usize) -> Option<usize> {
    text.get(..end)?
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}

fn matching_open_parenthesis(masked: &str, close: usize) -> Option<usize> {
    let mut nesting = 0u32;
    for (index, character) in masked.get(..=close)?.char_indices().rev() {
        match character {
            ')' => nesting += 1,
            '(' => {
                nesting = nesting.checked_sub(1)?;
                if nesting == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn routine_name_before(text: &str, end: usize) -> Option<String> {
    let mut name_end = end.min(text.len());
    while name_end > 0
        && text[..name_end]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        name_end = previous_char_start(text, name_end)?;
    }
    let token_start = identifier_path_start_before_cursor(text.get(..name_end)?)?;
    let raw = text.get(token_start..name_end)?.trim();
    if raw.is_empty() || raw.starts_with('\'') {
        return None;
    }
    let name = SqlParser::identifier_last_part(raw);
    (!name.is_empty()).then_some(name)
}

fn routine_name_and_group(text: &str, masked: &str, mut opening: usize) -> Option<(String, u32)> {
    let mut active_group = 0u32;
    loop {
        let mut before = opening;
        while before > 0
            && text[..before]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            before = previous_char_start(text, before)?;
        }
        let previous = previous_char_start(text, before)?;
        if masked.as_bytes().get(previous) == Some(&b')') {
            opening = matching_open_parenthesis(masked, previous)?;
            active_group = active_group.checked_add(1)?;
            if active_group > 3 {
                return None;
            }
            continue;
        }
        return routine_name_before(text, before).map(|name| (name, active_group));
    }
}

fn routine_call_at_position(text: &str, position: Position) -> Option<RoutineCallContext> {
    let offset = position_to_byte_offset(text, position).min(text.len());
    let mut scan_start = offset.saturating_sub(SIGNATURE_HELP_MAX_SCAN_BYTES);
    while scan_start < offset && !text.is_char_boundary(scan_start) {
        scan_start += 1;
    }
    let before = text.get(scan_start..offset)?;
    let masked = SqlParser::mask_sql_noise(before);
    let mut nesting = 0u32;
    let mut opening = None;
    for (index, character) in masked.char_indices().rev() {
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
    let (name, active_group) = routine_name_and_group(before, &masked, opening)?;

    let mut active_parameter = 0u32;
    let mut nested = 0u32;
    let mut current_argument_start = opening + 1;
    for (relative, character) in masked[opening + 1..].char_indices() {
        match character {
            '(' | '[' | '{' => nested += 1,
            ')' | ']' | '}' if nested > 0 => nested -= 1,
            ',' if nested == 0 => {
                active_parameter += 1;
                current_argument_start = opening + 1 + relative + character.len_utf8();
            }
            ';' if nested == 0 => return None,
            _ => {}
        }
    }
    let current_argument_has_content = sql_fragment_has_content(&before[current_argument_start..]);
    Some(RoutineCallContext {
        name,
        active_group,
        active_parameter,
        current_argument_has_content,
    })
}

fn sql_fragment_has_content(fragment: &str) -> bool {
    let mut offset = 0usize;
    while offset < fragment.len() {
        let rest = &fragment[offset..];
        let Some(character) = rest.chars().next() else {
            return false;
        };
        if character.is_whitespace() {
            offset += character.len_utf8();
            continue;
        }
        if rest.starts_with("--")
            || (rest.starts_with('#') && !rest.starts_with("#>") && !rest.starts_with("#-"))
        {
            let Some(newline) = rest.find('\n') else {
                return false;
            };
            offset += newline + 1;
            continue;
        }
        if rest.starts_with("/*") {
            let Some(end) = rest.get(2..).and_then(|tail| tail.find("*/")) else {
                return false;
            };
            offset += 2 + end + 2;
            continue;
        }
        return true;
    }
    false
}

fn routine_names_match(candidate: &str, requested: &str) -> bool {
    SqlParser::identifier_last_part(candidate).eq_ignore_ascii_case(requested)
}

fn live_overload_accepts_call(
    function: &crate::schema::Function,
    call: &RoutineCallContext,
) -> bool {
    if call.active_group != 0 {
        return false;
    }
    let parameters = &function.parameters;
    if parameters.is_empty() {
        return call.active_parameter == 0 && !call.current_argument_has_content;
    }
    call.active_parameter < parameters.len() as u32
}

fn builtin_group_accepts_call(parameters: &[&str], call: &RoutineCallContext) -> bool {
    if parameters.is_empty() {
        return call.active_parameter == 0 && !call.current_argument_has_content;
    }
    call.active_parameter < parameters.len() as u32
        || parameters
            .last()
            .is_some_and(|parameter| parameter.trim_start().starts_with("..."))
}

fn builtin_overload_accepts_call(signature: &BuiltinSignature, call: &RoutineCallContext) -> bool {
    signature
        .parameter_groups
        .get(call.active_group as usize)
        .is_some_and(|parameters| builtin_group_accepts_call(parameters, call))
}

fn clamped_active_parameter(parameter_count: usize, requested: u32) -> Option<u32> {
    (parameter_count > 0).then(|| requested.min(parameter_count.saturating_sub(1) as u32))
}

fn live_signature_information(
    function: &crate::schema::Function,
    call: &RoutineCallContext,
) -> SignatureInformation {
    SignatureInformation {
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
                    label: ParameterLabel::Simple(parameter.signature_label()),
                    documentation: None,
                })
                .collect(),
        ),
        active_parameter: clamped_active_parameter(
            function.parameters.len(),
            call.active_parameter,
        ),
    }
}

fn builtin_signature_information(
    signature: &BuiltinSignature,
    call: &RoutineCallContext,
    dialect: &str,
) -> Option<SignatureInformation> {
    let parameters = signature.parameter_groups.get(call.active_group as usize)?;
    Some(SignatureInformation {
        label: signature.label(),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!(
                "**Built-in signature** for the active `{dialect}` SQL profile. Live routine metadata takes precedence when available."
            ),
        })),
        parameters: Some(
            parameters
                .iter()
                .map(|parameter| ParameterInformation {
                    label: ParameterLabel::Simple((*parameter).to_string()),
                    documentation: None,
                })
                .collect(),
        ),
        active_parameter: clamped_active_parameter(parameters.len(), call.active_parameter),
    })
}

fn single_document_edit(uri: Url, range: Range, new_text: String) -> WorkspaceEdit {
    WorkspaceEdit {
        changes: Some(HashMap::from([(uri, vec![TextEdit { range, new_text }])])),
        document_changes: None,
        change_annotations: None,
    }
}

fn code_action_kind_available(context: &CodeActionContext, kind: &CodeActionKind) -> bool {
    context.only.as_ref().is_none_or(|only| {
        only.iter().any(|requested| {
            kind.as_str() == requested.as_str()
                || kind
                    .as_str()
                    .starts_with(&format!("{}.", requested.as_str()))
        })
    })
}

fn code_action_kind_explicitly_requested(
    context: &CodeActionContext,
    kind: &CodeActionKind,
) -> bool {
    context.only.as_ref().is_some_and(|only| {
        only.iter().any(|requested| {
            kind.as_str() == requested.as_str()
                || kind
                    .as_str()
                    .starts_with(&format!("{}.", requested.as_str()))
        })
    })
}

#[derive(Debug, Clone)]
struct IdentifierPathSelection {
    range: Range,
    raw_parts: Vec<String>,
    normalized_parts: Vec<String>,
}

#[derive(Debug, Clone)]
struct ColumnQualificationSource {
    relation: String,
    qualifier_label: String,
    qualifier_sql: String,
    columns: Vec<String>,
    is_aliased: bool,
}

fn identifier_path_end(text: &str, start: usize) -> usize {
    let mut quote_end: Option<char> = None;
    let mut index = start;
    while index < text.len() {
        let Some(character) = text[index..].chars().next() else {
            break;
        };
        if let Some(closing_quote) = quote_end {
            index += character.len_utf8();
            if character == closing_quote {
                if text[index..].starts_with(closing_quote) {
                    index += closing_quote.len_utf8();
                } else {
                    quote_end = None;
                }
            }
            continue;
        }
        match character {
            '"' | '`' => quote_end = Some(character),
            '[' => quote_end = Some(']'),
            _ if is_identifier_path_boundary(character) || character == ':' => break,
            _ => {}
        }
        index += character.len_utf8();
    }
    index
}

fn split_identifier_path_sql(raw: &str, dialect_name: &str) -> Option<(Vec<String>, Vec<String>)> {
    let mut raw_parts = Vec::new();
    let mut normalized_parts = Vec::new();
    let mut index = 0;
    while index < raw.len() {
        let start = index;
        let first = raw[index..].chars().next()?;
        let (normalized, end) = if matches!(first, '"' | '`' | '[') {
            if first == '"' && matches!(dialect_name, "mysql" | "hive" | "clickhouse") {
                return None;
            }
            let closing = if first == '[' { ']' } else { first };
            let mut value = String::new();
            index += first.len_utf8();
            let mut closed = false;
            while index < raw.len() {
                let character = raw[index..].chars().next()?;
                index += character.len_utf8();
                if character == closing {
                    if raw[index..].starts_with(closing) {
                        value.push(closing);
                        index += closing.len_utf8();
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    value.push(character);
                }
            }
            if !closed || value.is_empty() {
                return None;
            }
            (value, index)
        } else {
            if !SqlParser::is_identifier_char(first) {
                return None;
            }
            index += first.len_utf8();
            while index < raw.len() {
                let character = raw[index..].chars().next()?;
                if !SqlParser::is_identifier_char(character) {
                    break;
                }
                index += character.len_utf8();
            }
            (raw[start..index].to_string(), index)
        };
        raw_parts.push(raw[start..end].to_string());
        normalized_parts.push(normalized);
        if index == raw.len() {
            break;
        }
        if !raw[index..].starts_with('.') {
            return None;
        }
        index += 1;
    }
    (!raw_parts.is_empty()).then_some((raw_parts, normalized_parts))
}

fn identifier_path_selection(
    text: &str,
    request_range: Range,
    dialect_name: &str,
) -> Option<IdentifierPathSelection> {
    let request_start = position_to_byte_offset(text, request_range.start);
    let request_end = position_to_byte_offset(text, request_range.end);
    let cursor = request_end.max(request_start).min(text.len());
    let prefix = text.get(..cursor)?;
    let start = identifier_path_start_before_cursor(prefix)?;
    let end = identifier_path_end(text, start);
    if cursor < start || cursor > end || start >= end {
        return None;
    }
    let raw = text.get(start..end)?.trim();
    let trim_start = text.get(start..end)?.len() - text.get(start..end)?.trim_start().len();
    let start = start + trim_start;
    let end = start + raw.len();
    let (raw_parts, normalized_parts) = split_identifier_path_sql(raw, dialect_name)?;

    let masked = SqlParser::mask_sql_noise(text);
    let masked_selection = masked.get(start..end)?;
    let explicitly_quoted = raw_parts.iter().all(|part| {
        (part.starts_with('"') && part.ends_with('"'))
            || (part.starts_with('`') && part.ends_with('`'))
            || (part.starts_with('[') && part.ends_with(']'))
    });
    if masked_selection.trim_matches('.').trim().is_empty() && !explicitly_quoted {
        return None;
    }

    Some(IdentifierPathSelection {
        range: range_for_offsets(text, start, end),
        raw_parts,
        normalized_parts,
    })
}

fn relation_name_matches(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
        || left
            .rsplit('.')
            .next()
            .zip(right.rsplit('.').next())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn column_qualification_sources(
    parser: &SqlParser,
    tree: &tree_sitter::Tree,
    text: &str,
    position: Position,
    schema: &Schema,
    dialect_name: &str,
) -> Vec<ColumnQualificationSource> {
    let aliases = SqlParser::relation_aliases_at_position(text, position);
    let byte_position = SqlParser::lsp_position_to_byte_position(text, position);
    let references = parser.extract_row_sources_at_position(tree, text, byte_position);
    let mut sources = Vec::new();

    for alias in &aliases {
        let Some(table) = schema
            .tables
            .iter()
            .find(|table| schema_table_matches(schema, &alias.relation, table))
        else {
            continue;
        };
        sources.push(ColumnQualificationSource {
            relation: alias.relation.clone(),
            qualifier_label: alias.name.clone(),
            qualifier_sql: alias.sql.clone(),
            columns: table
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect(),
            is_aliased: true,
        });
    }
    for reference in references {
        if aliases
            .iter()
            .any(|alias| relation_name_matches(&alias.relation, &reference))
        {
            continue;
        }
        let Some(table) = schema
            .tables
            .iter()
            .find(|table| schema_table_matches(schema, &reference, table))
        else {
            continue;
        };
        let qualifier_label = SqlParser::identifier_last_part(&reference);
        sources.push(ColumnQualificationSource {
            relation: reference,
            qualifier_sql: quote_completion_identifier(&qualifier_label, dialect_name),
            qualifier_label,
            columns: table
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect(),
            is_aliased: false,
        });
    }
    sources.sort_by(|left, right| {
        left.qualifier_label
            .cmp(&right.qualifier_label)
            .then_with(|| left.qualifier_sql.cmp(&right.qualifier_sql))
    });

    // An ordinary alias is case-insensitive, while a quoted alias remains a
    // distinct SQL identifier. Do not collapse `u` and `"U"`: doing so would
    // make a qualification action silently pick a different row source.
    let mut unique_sources = Vec::with_capacity(sources.len());
    for source in sources {
        if unique_sources
            .iter()
            .any(|existing| qualification_sources_are_equivalent(existing, &source))
        {
            continue;
        }
        unique_sources.push(source);
    }
    unique_sources
}

fn sql_identifier_is_quoted(value: &str) -> bool {
    let value = value.trim();
    matches!(
        value.as_bytes(),
        [b'"', .., b'"'] | [b'`', .., b'`'] | [b'[', .., b']']
    )
}

fn qualification_sources_are_equivalent(
    left: &ColumnQualificationSource,
    right: &ColumnQualificationSource,
) -> bool {
    if left.is_aliased || right.is_aliased {
        return left.is_aliased
            && right.is_aliased
            && if sql_identifier_is_quoted(&left.qualifier_sql)
                || sql_identifier_is_quoted(&right.qualifier_sql)
            {
                left.qualifier_sql == right.qualifier_sql
            } else {
                left.qualifier_label
                    .eq_ignore_ascii_case(&right.qualifier_label)
            };
    }

    left.qualifier_label
        .eq_ignore_ascii_case(&right.qualifier_label)
}

fn source_matches_qualifier(
    source: &ColumnQualificationSource,
    raw_qualifier: &str,
    qualifier: &str,
) -> bool {
    if source.is_aliased {
        if sql_identifier_is_quoted(&source.qualifier_sql) {
            return source.qualifier_sql == raw_qualifier;
        }
        source.qualifier_label.eq_ignore_ascii_case(qualifier)
    } else {
        relation_name_matches(&source.relation, qualifier)
            || source.qualifier_label.eq_ignore_ascii_case(qualifier)
    }
}

fn previous_sql_word(source: &str, offset: usize) -> Option<&str> {
    source[..offset.min(source.len())]
        .trim_end()
        .rsplit(|character: char| !character.is_alphanumeric() && character != '_')
        .find(|word| !word.is_empty())
}

fn batch_qualify_identifier_action(
    text: &str,
    uri: &Url,
    request_range: Range,
    sources: &[ColumnQualificationSource],
    dialect_name: &str,
) -> Option<CodeAction> {
    const MAX_BATCH_QUALIFY_BYTES: usize = 256 * 1024;
    let start = position_to_byte_offset(text, request_range.start);
    let end = position_to_byte_offset(text, request_range.end);
    let (start, end) = (start.min(end), start.max(end).min(text.len()));
    if start >= end || end - start > MAX_BATCH_QUALIFY_BYTES {
        return None;
    }
    let masked = SqlParser::mask_sql_noise(text);
    let alias_names = sources
        .iter()
        .map(|source| source.qualifier_label.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let relation_names = sources
        .iter()
        .flat_map(|source| {
            [
                source.relation.to_ascii_lowercase(),
                source
                    .relation
                    .rsplit('.')
                    .next()
                    .unwrap_or(&source.relation)
                    .to_ascii_lowercase(),
            ]
        })
        .collect::<HashSet<_>>();
    let mut edits = Vec::new();
    let mut index = start;
    while index < end {
        let Some(first) = text[index..].chars().next() else {
            break;
        };
        if !SqlParser::is_identifier_char(first) && !matches!(first, '"' | '`' | '[') {
            index += first.len_utf8();
            continue;
        }
        let candidate_end = identifier_path_end(text, index).min(end);
        if candidate_end <= index {
            index += first.len_utf8();
            continue;
        }
        let raw = &text[index..candidate_end];
        let Some((raw_parts, normalized_parts)) = split_identifier_path_sql(raw, dialect_name)
        else {
            index += first.len_utf8();
            continue;
        };
        index = candidate_end;
        if normalized_parts.len() != 1 {
            continue;
        }
        let column = &normalized_parts[0];
        let normalized = column.to_ascii_lowercase();
        if Keywords::is_keyword(column)
            || alias_names.contains(&normalized)
            || relation_names.contains(&normalized)
        {
            continue;
        }
        let masked_candidate = &masked[candidate_end - raw.len()..candidate_end];
        let explicitly_quoted = raw_parts[0]
            .chars()
            .next()
            .is_some_and(|character| matches!(character, '"' | '`' | '['));
        if masked_candidate.trim().is_empty() && !explicitly_quoted {
            continue;
        }
        if previous_sql_word(&masked, candidate_end - raw.len()).is_some_and(|word| {
            matches!(
                word.to_ascii_uppercase().as_str(),
                "AS" | "FROM" | "JOIN" | "APPLY" | "UPDATE" | "INTO" | "TABLE" | "VIEW"
            )
        }) {
            continue;
        }
        if text[candidate_end..]
            .chars()
            .find(|character| !character.is_whitespace())
            == Some('(')
        {
            continue;
        }
        let matching_sources = sources
            .iter()
            .filter(|source| {
                source
                    .columns
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(column))
            })
            .collect::<Vec<_>>();
        if matching_sources.len() != 1 {
            continue;
        }
        edits.push(TextEdit {
            range: range_for_offsets(text, candidate_end - raw.len(), candidate_end),
            new_text: format!("{}.{}", matching_sources[0].qualifier_sql, raw_parts[0]),
        });
    }
    if edits.is_empty() {
        return None;
    }
    let count = edits.len();
    Some(CodeAction {
        title: format!(
            "Qualify {count} selected column{}",
            if count == 1 { "" } else { "s" }
        ),
        kind: Some(CodeActionKind::REFACTOR_REWRITE),
        edit: Some(WorkspaceEdit {
            changes: Some(HashMap::from([(uri.clone(), edits)])),
            document_changes: None,
            change_annotations: None,
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}

fn qualify_identifier_actions(
    text: &str,
    uri: &Url,
    request_range: Range,
    schema: Option<&Schema>,
    dialect_name: &str,
) -> Vec<CodeAction> {
    let Some(schema) = schema else {
        return Vec::new();
    };
    let request_start = position_to_byte_offset(text, request_range.start);
    let request_end = position_to_byte_offset(text, request_range.end);
    if request_start != request_end {
        let mut parser = SqlParser::new();
        let parsed = parser.parse(text);
        let Some(tree) = parsed.tree.as_ref() else {
            return Vec::new();
        };
        let sources = column_qualification_sources(
            &parser,
            tree,
            text,
            request_range.start,
            schema,
            dialect_name,
        );
        return batch_qualify_identifier_action(text, uri, request_range, &sources, dialect_name)
            .into_iter()
            .collect();
    }
    let Some(selection) = identifier_path_selection(text, request_range, dialect_name) else {
        return Vec::new();
    };
    let Some(column) = selection.normalized_parts.last() else {
        return Vec::new();
    };
    if Keywords::is_keyword(column) {
        return Vec::new();
    }
    let before = text[..position_to_byte_offset(text, selection.range.start)].trim_end();
    if before
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .next_back()
        .is_some_and(|word| word.eq_ignore_ascii_case("AS"))
    {
        return Vec::new();
    }

    let mut parser = SqlParser::new();
    let parsed = parser.parse(text);
    let Some(tree) = parsed.tree.as_ref() else {
        return Vec::new();
    };
    let sources = column_qualification_sources(
        &parser,
        tree,
        text,
        request_range.start,
        schema,
        dialect_name,
    );
    if sources.is_empty()
        || sources
            .iter()
            .any(|source| source.qualifier_label.eq_ignore_ascii_case(column))
    {
        return Vec::new();
    }
    let matching_sources = sources
        .iter()
        .filter(|source| {
            source
                .columns
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(column))
        })
        .collect::<Vec<_>>();
    let column_sql = selection.raw_parts.last().cloned().unwrap_or_default();

    if selection.normalized_parts.len() > 1 {
        let qualifier =
            selection.normalized_parts[..selection.normalized_parts.len() - 1].join(".");
        let raw_qualifier = selection.raw_parts[..selection.raw_parts.len() - 1].join(".");
        if matching_sources.len() != 1
            || !source_matches_qualifier(matching_sources[0], &raw_qualifier, &qualifier)
        {
            return Vec::new();
        }
        return vec![CodeAction {
            title: format!("Remove qualifier from {}", column_sql),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            edit: Some(single_document_edit(
                uri.clone(),
                selection.range,
                column_sql,
            )),
            is_preferred: Some(true),
            ..Default::default()
        }];
    }

    matching_sources
        .into_iter()
        .map(|source| {
            let replacement = format!("{}.{}", source.qualifier_sql, column_sql);
            CodeAction {
                title: format!("Qualify column as {replacement}"),
                kind: Some(CodeActionKind::REFACTOR_REWRITE),
                edit: Some(single_document_edit(
                    uri.clone(),
                    selection.range,
                    replacement,
                )),
                is_preferred: Some(sources.len() == 1),
                ..Default::default()
            }
        })
        .collect()
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

fn foreign_key_targets_table(schema: &Schema, constraint: &Constraint, target: &Table) -> bool {
    if !constraint
        .constraint_type
        .to_ascii_uppercase()
        .contains("FOREIGN")
    {
        return false;
    }
    let Some(referenced_table) = constraint.referenced_table.as_deref() else {
        return false;
    };
    let reference = constraint
        .referenced_schema
        .as_deref()
        .filter(|schema| !schema.trim().is_empty())
        .map(|referenced_schema| format!("{referenced_schema}.{referenced_table}"))
        .unwrap_or_else(|| referenced_table.to_string());
    schema_table_matches(schema, &reference, target)
}

fn join_reference_qualifier(
    schema: &Schema,
    table: &Table,
    reference: &str,
    aliases: &HashMap<String, String>,
    dialect_name: &str,
) -> Option<String> {
    let mut matching_aliases = aliases
        .iter()
        .filter(|(_, target)| schema_table_matches(schema, target, table))
        .map(|(alias, _)| alias.as_str());
    let alias = matching_aliases.next();
    if alias.is_some() && matching_aliases.next().is_some() {
        // Multiple aliases for the same relation form a self-join-like shape;
        // the alias map does not prove which occurrence owns this constraint.
        return None;
    }
    Some(quote_completion_identifier(
        alias
            .map(str::to_string)
            .unwrap_or_else(|| SqlParser::identifier_last_part(reference))
            .as_str(),
        dialect_name,
    ))
}

fn join_predicate(
    left_qualifier: &str,
    left_columns: &[String],
    right_qualifier: &str,
    right_columns: &[String],
    dialect_name: &str,
) -> Option<String> {
    if left_columns.is_empty() || left_columns.len() != right_columns.len() {
        return None;
    }
    Some(
        left_columns
            .iter()
            .zip(right_columns)
            .map(|(left, right)| {
                format!(
                    "{left_qualifier}.{} = {right_qualifier}.{}",
                    quote_completion_identifier(left, dialect_name),
                    quote_completion_identifier(right, dialect_name),
                )
            })
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

fn add_join_condition_actions(
    text: &str,
    uri: &Url,
    diagnostic: &Diagnostic,
    schema: Option<&Schema>,
    dialect_name: &str,
) -> Vec<CodeAction> {
    const MAX_INFERRED_JOIN_ACTIONS: usize = 16;

    let join_start = position_to_byte_offset(text, diagnostic.range.start).min(text.len());
    let statement_start = text[..join_start].rfind(';').map_or(0, |offset| offset + 1);
    let statement_end = text[join_start..]
        .find(';')
        .map_or(text.len(), |offset| join_start + offset + 1);
    let statement = &text[statement_start..statement_end];
    let normalized = SqlParser::mask_sql_noise(statement).to_ascii_uppercase();
    let expected_join = join_start.saturating_sub(statement_start);
    let Some((join_relative, insert_relative)) = join_without_condition_span(&normalized) else {
        return Vec::new();
    };
    if join_relative != expected_join || insert_relative > statement.len() {
        return Vec::new();
    }
    let insert_offset = statement_start + insert_relative;
    let insert_range = range_for_offsets(text, insert_offset, insert_offset);
    let fallback = || CodeAction {
        title: "Add non-matching JOIN safety guard".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(single_document_edit(
            uri.clone(),
            insert_range,
            " ON 1 = 0 /* replace join condition before executing */".to_string(),
        )),
        is_preferred: Some(false),
        ..Default::default()
    };

    let Some(schema) = schema else {
        return vec![fallback()];
    };
    let join_prefix = &statement[..insert_relative];
    let mut parser = SqlParser::new();
    let Some(tree) = parser.parse(join_prefix).tree else {
        return vec![fallback()];
    };
    let references = parser.extract_referenced_tables(&tree, join_prefix);
    let Some(candidate_reference) = references.last() else {
        return vec![fallback()];
    };
    if references.len() < 2 {
        return vec![fallback()];
    }
    let Some(candidate) = schema
        .tables
        .iter()
        .find(|table| schema_table_matches(schema, candidate_reference, table))
    else {
        return vec![fallback()];
    };
    let aliases = parser.extract_aliases(&tree, join_prefix);
    let Some(candidate_qualifier) = join_reference_qualifier(
        schema,
        candidate,
        candidate_reference,
        &aliases,
        dialect_name,
    ) else {
        return vec![fallback()];
    };

    let mut inferred = Vec::<(String, String)>::new();
    let mut seen = HashSet::new();
    for existing_reference in &references[..references.len() - 1] {
        let Some(existing) = schema
            .tables
            .iter()
            .find(|table| schema_table_matches(schema, existing_reference, table))
        else {
            continue;
        };
        if std::ptr::eq(candidate, existing) {
            continue;
        }
        let Some(existing_qualifier) =
            join_reference_qualifier(schema, existing, existing_reference, &aliases, dialect_name)
        else {
            continue;
        };

        for constraint in &candidate.constraints {
            if !foreign_key_targets_table(schema, constraint, existing) {
                continue;
            }
            if let Some(predicate) = join_predicate(
                &candidate_qualifier,
                &constraint.columns,
                &existing_qualifier,
                &constraint.referenced_columns,
                dialect_name,
            ) {
                if seen.insert(predicate.to_ascii_lowercase()) {
                    inferred.push((constraint.name.clone(), predicate));
                }
            }
        }
        for constraint in &existing.constraints {
            if !foreign_key_targets_table(schema, constraint, candidate) {
                continue;
            }
            if let Some(predicate) = join_predicate(
                &existing_qualifier,
                &constraint.columns,
                &candidate_qualifier,
                &constraint.referenced_columns,
                dialect_name,
            ) {
                if seen.insert(predicate.to_ascii_lowercase()) {
                    inferred.push((constraint.name.clone(), predicate));
                }
            }
        }
        if inferred.len() >= MAX_INFERRED_JOIN_ACTIONS {
            break;
        }
    }
    inferred.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    inferred.truncate(MAX_INFERRED_JOIN_ACTIONS);
    let unique = inferred.len() == 1;
    let mut actions = inferred
        .into_iter()
        .map(|(constraint_name, predicate)| CodeAction {
            title: if constraint_name.trim().is_empty() {
                "Add JOIN condition from foreign-key metadata".to_string()
            } else {
                format!("Add JOIN condition via {constraint_name}")
            },
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(single_document_edit(
                uri.clone(),
                insert_range,
                format!(" ON {predicate}"),
            )),
            is_preferred: Some(unique),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    actions.push(fallback());
    actions
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
    if sources.is_empty()
        || sources
            .iter()
            .any(|(table, _)| table.columns.iter().all(|column| column.hidden))
    {
        return None;
    }

    let qualified_expansion = qualifier.is_none() && sources.len() > 1;
    let mut replacement_columns = Vec::new();
    for (table, source_qualifier) in &sources {
        for (index, column) in table
            .columns
            .iter()
            .filter(|column| !column.hidden)
            .enumerate()
        {
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
fn document_symbols(
    text: &str,
    schema: Option<&Schema>,
    project_occurrences: &[ProjectSqlSymbolOccurrence],
) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    let mut seen = HashSet::new();
    for occurrence in project_occurrences
        .iter()
        .filter(|occurrence| occurrence.role == ProjectSqlSymbolRole::Definition)
    {
        let category = if occurrence.kind.is_routine() {
            "routine"
        } else {
            "relation"
        };
        if seen.insert((category, occurrence.name.to_ascii_lowercase())) {
            symbols.push(DocumentSymbol {
                name: occurrence.name.clone(),
                detail: Some(occurrence.kind.detail().to_string()),
                kind: occurrence.kind.symbol_kind(),
                tags: None,
                deprecated: None,
                range: occurrence.range,
                selection_range: occurrence.range,
                children: None,
            });
        }
    }
    if let Some(schema) = schema {
        for table in &schema.tables {
            let range = project_occurrences
                .iter()
                .find(|occurrence| {
                    !occurrence.kind.is_routine()
                        && occurrence.name.eq_ignore_ascii_case(&table.name)
                })
                .map(|occurrence| occurrence.range)
                .or_else(|| range_for_identifier_occurrence(text, &table.name));
            if let Some(range) = range {
                if seen.insert(("relation", table.name.to_ascii_lowercase())) {
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
            let range = project_occurrences
                .iter()
                .find(|occurrence| {
                    occurrence.kind.is_routine()
                        && occurrence.name.eq_ignore_ascii_case(&function.name)
                })
                .map(|occurrence| occurrence.range)
                .or_else(|| range_for_identifier_occurrence(text, &function.name));
            if let Some(range) = range {
                if seen.insert(("routine", function.name.to_ascii_lowercase())) {
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
                && local_keyword_boundary(upper, definition_start, "AS")
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
        default_value: None,
        auto_increment: false,
        generated: false,
        hidden: false,
        comment: Some("Column from a temporary table in the current console".to_string()),
        source_location: Some((uri.to_string(), line)),
    })
}

fn cte_tables_before_cursor(source: &str, upper: &str, schema: &Schema, uri: &str) -> Vec<Table> {
    let mut tables = Vec::new();
    let mut search = 0;
    while let Some(relative) = upper[search..].find("WITH") {
        let with_start = search + relative;
        if !local_keyword_boundary(upper, with_start, "WITH") {
            search = with_start + "WITH".len();
            continue;
        }
        let mut cursor = skip_local_whitespace(source, with_start + "WITH".len());
        if upper[cursor..].starts_with("RECURSIVE")
            && local_keyword_boundary(upper, cursor, "RECURSIVE")
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
            if !upper[cursor..].starts_with("AS") || !local_keyword_boundary(upper, cursor, "AS") {
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
        default_value: None,
        auto_increment: false,
        generated: false,
        hidden: false,
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

fn insert_value_hints(text: &str, range: Range, schema: &Schema) -> Vec<InlayHint> {
    let absolute_range_start = position_to_byte_offset(text, range.start);
    let absolute_range_end = position_to_byte_offset(text, range.end)
        .max(absolute_range_start)
        .min(absolute_range_start.saturating_add(INLAY_HINT_REQUEST_MAX_BYTES));
    let provisional_window_start = floor_char_boundary(
        text,
        absolute_range_start.saturating_sub(INLAY_HINT_STATEMENT_SCAN_BYTES),
    );
    let window_start = if provisional_window_start == 0 {
        0
    } else {
        text[provisional_window_start..absolute_range_start]
            .find('\n')
            .map_or(provisional_window_start, |offset| {
                provisional_window_start + offset + 1
            })
    };
    let window_end = ceil_char_boundary(
        text,
        absolute_range_end
            .saturating_add(INLAY_HINT_STATEMENT_SCAN_BYTES)
            .min(text.len()),
    );
    let text_window = &text[window_start..window_end];
    let range_start = absolute_range_start - window_start;
    let range_end = absolute_range_end - window_start;
    let masked = SqlParser::mask_sql_noise(text_window);
    let lower = masked.to_ascii_lowercase();
    let scan_start = lower[..range_start]
        .rfind(';')
        .map_or(0, |offset| offset + 1);
    let scan_end = lower[range_end..]
        .find(';')
        .map_or(text_window.len(), |offset| range_end + offset);
    let mut labels = Vec::new();
    let mut search_from = scan_start;

    while search_from < scan_end && labels.len() < INLAY_HINT_MAX_ITEMS {
        let Some(insert) = find_sql_keyword(&lower, "insert", search_from, scan_end) else {
            break;
        };
        let statement_end = lower[insert..scan_end]
            .find(';')
            .map_or(scan_end, |offset| insert + offset);
        let Some(into) = find_sql_keyword(&lower, "into", insert + "insert".len(), statement_end)
        else {
            search_from = insert + "insert".len();
            continue;
        };
        let Some((table_reference, mut cursor)) =
            read_insert_qualified_identifier(text_window, into + "into".len(), statement_end)
        else {
            search_from = into + "into".len();
            continue;
        };

        cursor = skip_sql_whitespace(text_window, cursor, statement_end);
        if sql_keyword_at(&lower, "with", cursor, statement_end) {
            let after_with = skip_sql_whitespace(text_window, cursor + "with".len(), statement_end);
            if lower[after_with..statement_end].starts_with('(') {
                if let Some(close) = matching_sql_paren(&masked, after_with, statement_end) {
                    cursor = skip_sql_whitespace(text_window, close + 1, statement_end);
                }
            }
        }
        cursor = skip_insert_target_alias(text_window, &lower, cursor, statement_end);

        let (columns, values_search_from) = if lower[cursor..statement_end].starts_with('(') {
            let Some(close) = matching_sql_paren(&masked, cursor, statement_end) else {
                search_from = statement_end.max(insert + "insert".len());
                continue;
            };
            (
                Some(insert_column_names(text_window, &masked, cursor + 1, close)),
                close + 1,
            )
        } else {
            (None, cursor)
        };
        let Some(values) = find_sql_keyword(&lower, "values", values_search_from, statement_end)
        else {
            search_from = statement_end.max(insert + "insert".len());
            continue;
        };
        if find_sql_keyword(&lower, "select", values_search_from, values).is_some() {
            search_from = statement_end.max(insert + "insert".len());
            continue;
        }

        let resolved_columns = columns.unwrap_or_else(|| {
            schema
                .tables
                .iter()
                .find(|table| schema_table_matches(schema, &table_reference, table))
                .map(|table| {
                    table
                        .columns
                        .iter()
                        .filter(|column| {
                            !column.auto_increment && !column.generated && !column.hidden
                        })
                        .map(|column| column.name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        if !resolved_columns.is_empty() {
            append_insert_row_hints(
                text_window,
                &masked,
                (values + "values".len(), statement_end),
                (range_start, range_end),
                &resolved_columns,
                &mut labels,
            );
        }
        search_from = statement_end.max(insert + "insert".len());
    }

    labels
        .into_iter()
        .map(|(offset, column)| InlayHint {
            position: position_for_offset(text, window_start + offset),
            label: InlayHintLabel::String(format!("{column}:")),
            kind: Some(InlayHintKind::PARAMETER),
            text_edits: None,
            tooltip: Some(InlayHintTooltip::String(
                "Column matched from the INSERT target and value position.".to_string(),
            )),
            padding_left: None,
            padding_right: Some(true),
            data: None,
        })
        .collect()
}

fn append_insert_row_hints(
    text: &str,
    masked: &str,
    statement: (usize, usize),
    requested_range: (usize, usize),
    columns: &[String],
    hints: &mut Vec<(usize, String)>,
) {
    let (from, statement_end) = statement;
    let (range_start, range_end) = requested_range;
    let mut cursor = skip_sql_whitespace(text, from, statement_end);
    while cursor < statement_end
        && masked[cursor..statement_end].starts_with('(')
        && hints.len() < INLAY_HINT_MAX_ITEMS
    {
        let Some((value_offsets, next)) =
            insert_value_row_offsets(text, masked, cursor, statement_end)
        else {
            break;
        };
        for (offset, column) in value_offsets.into_iter().zip(columns.iter()) {
            if offset < range_start || offset > range_end || hints.len() >= INLAY_HINT_MAX_ITEMS {
                continue;
            }
            hints.push((offset, column.clone()));
        }
        cursor = skip_sql_whitespace(text, next, statement_end);
        if !masked[cursor..statement_end].starts_with(',') {
            break;
        }
        cursor = skip_sql_whitespace(text, cursor + 1, statement_end);
    }
}

fn insert_value_row_offsets(
    text: &str,
    masked: &str,
    open: usize,
    end: usize,
) -> Option<(Vec<usize>, usize)> {
    let mut offsets = Vec::new();
    let mut depth = 1usize;
    let mut value_start = open + 1;
    let mut cursor = open + 1;
    while cursor < end {
        let character = masked[cursor..].chars().next()?;
        match character {
            '(' => depth += 1,
            ')' if depth == 1 => {
                if let Some(offset) = first_insert_value_offset(text, masked, value_start, cursor) {
                    offsets.push(offset);
                }
                return Some((offsets, cursor + 1));
            }
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 1 => {
                if let Some(offset) = first_insert_value_offset(text, masked, value_start, cursor) {
                    offsets.push(offset);
                }
                value_start = cursor + 1;
            }
            ';' if depth == 1 => return None,
            _ => {}
        }
        cursor += character.len_utf8();
    }
    None
}

fn first_insert_value_offset(text: &str, masked: &str, from: usize, to: usize) -> Option<usize> {
    let mut cursor = from;
    while cursor < to {
        let raw = text[cursor..].chars().next()?;
        let searchable = masked[cursor..].chars().next()?;
        if !searchable.is_whitespace() || matches!(raw, '\'' | '"' | '`' | '[' | '$') {
            return Some(cursor);
        }
        cursor += raw.len_utf8();
    }
    None
}

fn insert_column_names(text: &str, masked: &str, from: usize, to: usize) -> Vec<String> {
    let mut columns = Vec::new();
    let mut start = from;
    let mut depth = 0usize;
    let mut cursor = from;
    while cursor <= to {
        let character = if cursor == to {
            ','
        } else {
            masked[cursor..].chars().next().unwrap_or(',')
        };
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let Some(column) = normalize_insert_identifier(&text[start..cursor]) else {
                    return Vec::new();
                };
                columns.push(column);
                start = cursor + 1;
            }
            _ => {}
        }
        cursor += character.len_utf8();
    }
    columns
}

fn normalize_insert_identifier(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return Some(trimmed[1..trimmed.len() - 1].replace("]]", "]"));
    }
    for quote in ['"', '`'] {
        if trimmed.starts_with(quote) && trimmed.ends_with(quote) {
            return Some(
                trimmed[quote.len_utf8()..trimmed.len() - quote.len_utf8()]
                    .replace(&format!("{quote}{quote}"), &quote.to_string()),
            );
        }
    }
    trimmed
        .chars()
        .all(|character| character == '_' || character == '$' || character.is_alphanumeric())
        .then(|| trimmed.to_string())
}

fn skip_insert_target_alias(text: &str, lower: &str, from: usize, end: usize) -> usize {
    let cursor = skip_sql_whitespace(text, from, end);
    if sql_keyword_at(lower, "as", cursor, end) {
        return read_insert_identifier_part(text, cursor + "as".len(), end)
            .map_or(cursor, |(_, next)| skip_sql_whitespace(text, next, end));
    }
    if lower[cursor..end].starts_with('(')
        || ["values", "select", "default", "overriding"]
            .iter()
            .any(|keyword| sql_keyword_at(lower, keyword, cursor, end))
    {
        return cursor;
    }
    read_insert_identifier_part(text, cursor, end)
        .map_or(cursor, |(_, next)| skip_sql_whitespace(text, next, end))
}

fn read_insert_qualified_identifier(
    text: &str,
    from: usize,
    end: usize,
) -> Option<(String, usize)> {
    let (first, mut cursor) = read_insert_identifier_part(text, from, end)?;
    let mut parts = vec![first];
    loop {
        let dot = skip_sql_whitespace(text, cursor, end);
        if !text[dot..end].starts_with('.') {
            break;
        }
        let after_dot = skip_sql_whitespace(text, dot + 1, end);
        if text[after_dot..end].starts_with('.') {
            parts.push(String::new());
            cursor = after_dot;
            continue;
        }
        let (part, next) = read_insert_identifier_part(text, after_dot, end)?;
        parts.push(part);
        cursor = next;
    }
    Some((parts.join("."), cursor))
}

fn read_insert_identifier_part(text: &str, from: usize, end: usize) -> Option<(String, usize)> {
    let start = skip_sql_whitespace(text, from, end);
    let opener = text[start..end].chars().next()?;
    if matches!(opener, '"' | '`' | '[') {
        let closer = if opener == '[' { ']' } else { opener };
        let mut cursor = start + opener.len_utf8();
        while cursor < end {
            let character = text[cursor..].chars().next()?;
            cursor += character.len_utf8();
            if character != closer {
                continue;
            }
            if text[cursor..end].starts_with(closer) {
                cursor += closer.len_utf8();
                continue;
            }
            return Some((text[start..cursor].to_string(), cursor));
        }
        return None;
    }
    if !(opener == '_' || opener == '$' || opener == '#' || opener.is_alphabetic()) {
        return None;
    }
    let mut cursor = start + opener.len_utf8();
    while cursor < end {
        let character = text[cursor..].chars().next()?;
        if !(character == '_'
            || character == '$'
            || character == '#'
            || character.is_alphanumeric())
        {
            break;
        }
        cursor += character.len_utf8();
    }
    Some((text[start..cursor].to_string(), cursor))
}

fn matching_sql_paren(masked: &str, open: usize, end: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut cursor = open;
    while cursor < end {
        let character = masked[cursor..].chars().next()?;
        match character {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(cursor);
                }
            }
            _ => {}
        }
        cursor += character.len_utf8();
    }
    None
}

fn find_sql_keyword(lower: &str, keyword: &str, from: usize, end: usize) -> Option<usize> {
    let mut cursor = from;
    while cursor < end {
        let offset = lower[cursor..end].find(keyword)?;
        let candidate = cursor + offset;
        if sql_keyword_at(lower, keyword, candidate, end) {
            return Some(candidate);
        }
        cursor = candidate + keyword.len();
    }
    None
}

fn sql_keyword_at(lower: &str, keyword: &str, start: usize, end: usize) -> bool {
    let keyword_end = start.saturating_add(keyword.len());
    keyword_end <= end
        && lower[start..keyword_end].eq_ignore_ascii_case(keyword)
        && lower[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_insert_identifier_character(character))
        && lower[keyword_end..]
            .chars()
            .next()
            .is_none_or(|character| !is_insert_identifier_character(character))
}

fn is_insert_identifier_character(character: char) -> bool {
    character == '_' || character == '$' || character == '#' || character.is_alphanumeric()
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    offset
}

fn ceil_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
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
    let aliases = SqlParser::relation_aliases_at_position(text, byte_position);
    let table_reference = SqlParser::resolve_relation_alias(&aliases, &table_name)
        .map(|alias| alias.relation.as_str())
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
        add_builtin_function_completions, add_clickhouse_table_function_completions,
        add_insert_all_columns_completion, add_insert_statement_snippets,
        add_join_condition_actions, add_oracle_table_function_completions,
        add_referenced_alias_completions, add_window_function_completions,
        apply_completed_sql_context_completion_edits, apply_completion_preferences,
        apply_high_frequency_keyword_ranking, apply_qualified_identifier_completion_edits,
        augment_schema_with_local_relations, calculate_schema_match_score,
        client_supports_completion_documentation_resolve, code_action_kind_available,
        code_action_kind_explicitly_requested, completed_sql_context_keyword_at_position,
        completion_statement_prefix, deduplicate_simple_completion_items,
        defer_completion_documentation, expand_select_star_action, find_schema_by_qualifier,
        find_schema_by_table_reference, infer_dialect_from_uri_and_language,
        infer_schema_id_from_tables, insert_value_hints, live_overload_accepts_call,
        oracle_table_routine_context, position_to_byte_offset, project_sql_symbol_occurrences,
        project_sql_symbols_match, qualify_identifier_actions, range_for_offsets,
        resolve_completion_documentation, rewrite_current_document_location_uri,
        rewrite_current_document_location_uris, routine_call_at_position,
        schema_for_table_column_at_position, schema_id_for_file, schema_qualifier_at_position,
        sql_inspection_diagnostics, table_alias_initials, CompletionPreferences,
        CompletionResolveCache, FormattingPreferences, FromClauseLayout, KeywordCase,
        LogicalOperatorNewline, ProjectSqlSymbolKind, ProjectSqlSymbolOccurrence,
        ProjectSqlSymbolRole, RoutineCallContext, TableAliasStyle,
        COMPLETION_RESOLVE_CACHE_MAX_ENTRIES, COMPLETION_RESOLVE_DOCUMENTATION_MAX_BYTES,
        LOCAL_RELATION_SCAN_MAX_BYTES, PROJECT_SQL_INDEX_MAX_BYTES,
    };
    use crate::dialects::DialectRegistry;
    use crate::position::lsp_position_at_end;
    use crate::schema::{
        Column, Constraint, Function, FunctionParameter, Schema, SchemaId, SchemaManager, Table,
    };
    use dashmap::DashMap;
    use std::sync::atomic::AtomicU64;
    use std::sync::Mutex;
    use tower_lsp::lsp_types::{
        CodeAction, CodeActionContext, CodeActionKind, CompletionItem, CompletionItemKind,
        CompletionTextEdit, Documentation, InitializeParams, InlayHintLabel, InsertTextFormat,
        Location, Position, Range, TextEdit, Url,
    };

    #[test]
    fn formatting_preferences_decode_editor_layout_values() {
        let preferences: FormattingPreferences = serde_json::from_value(serde_json::json!({
            "keywordCase": "lower",
            "functionCase": "upper",
            "logicalOperatorNewline": "none",
            "fromClauseLayout": "sameLine"
        }))
        .unwrap();

        assert!(matches!(preferences.keyword_case, KeywordCase::Lower));
        assert!(matches!(preferences.function_case, KeywordCase::Upper));
        assert_eq!(
            preferences.logical_operator_newline,
            LogicalOperatorNewline::None
        );
        assert_eq!(preferences.from_clause_layout, FromClauseLayout::SameLine);
    }

    #[test]
    fn completion_documentation_is_deferred_only_for_capable_clients() {
        let capable: InitializeParams = serde_json::from_value(serde_json::json!({
            "capabilities": {
                "textDocument": {
                    "completion": {
                        "completionItem": {
                            "resolveSupport": {
                                "properties": ["documentation", "detail"]
                            }
                        }
                    }
                }
            }
        }))
        .unwrap();
        let incapable: InitializeParams = serde_json::from_value(serde_json::json!({
            "capabilities": {
                "textDocument": {
                    "completion": {
                        "completionItem": {
                            "resolveSupport": { "properties": ["detail"] }
                        }
                    }
                }
            }
        }))
        .unwrap();

        assert!(client_supports_completion_documentation_resolve(&capable));
        assert!(!client_supports_completion_documentation_resolve(
            &incapable
        ));
    }

    #[test]
    fn completion_documentation_is_deferred_and_restores_original_data() {
        let cache = Mutex::new(CompletionResolveCache::default());
        let next_id = AtomicU64::new(1);
        let original_data = serde_json::json!({ "source": "metadata" });
        let mut items = vec![CompletionItem {
            label: "customer_id".to_string(),
            documentation: Some(Documentation::String("Primary key".to_string())),
            data: Some(original_data.clone()),
            ..Default::default()
        }];

        defer_completion_documentation(&mut items, &cache, &next_id);
        assert!(items[0].documentation.is_none());
        assert_eq!(
            items[0]
                .data
                .as_ref()
                .and_then(|data| data.get("oxideSqlLspCompletionResolveId"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );

        let resolved = resolve_completion_documentation(items.remove(0), &cache);
        assert_eq!(
            resolved.documentation,
            Some(Documentation::String("Primary key".to_string()))
        );
        assert_eq!(resolved.data, Some(original_data));
    }

    #[test]
    fn completion_documentation_cache_is_bounded_and_oversize_docs_stay_inline() {
        let cache = Mutex::new(CompletionResolveCache::default());
        let next_id = AtomicU64::new(1);
        let mut oversize = vec![CompletionItem {
            label: "large".to_string(),
            documentation: Some(Documentation::String(
                "x".repeat(COMPLETION_RESOLVE_DOCUMENTATION_MAX_BYTES + 1),
            )),
            ..Default::default()
        }];
        defer_completion_documentation(&mut oversize, &cache, &next_id);
        assert!(oversize[0].documentation.is_some());
        assert!(oversize[0].data.is_none());

        let mut cache = cache.lock().unwrap();
        for id in 0..=COMPLETION_RESOLVE_CACHE_MAX_ENTRIES as u64 {
            assert!(cache.insert(id, Documentation::String("doc".to_string())));
        }
        assert_eq!(cache.entries.len(), COMPLETION_RESOLVE_CACHE_MAX_ENTRIES);
        assert!(!cache.entries.contains_key(&0));
        assert!(cache
            .entries
            .contains_key(&(COMPLETION_RESOLVE_CACHE_MAX_ENTRIES as u64)));
    }

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

    #[test]
    fn builtin_function_completion_is_expression_scoped_and_snippet_aware() {
        let sql = "SELECT con";
        let mut items = vec![CompletionItem {
            label: "CONCAT".to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("String function: concatenate".to_string()),
            ..Default::default()
        }];
        add_builtin_function_completions(sql, lsp_position_at_end(sql), "mysql", None, &mut items);
        let concat = items
            .iter()
            .find(|item| item.label == "CONCAT")
            .expect("catalog CONCAT completion");
        assert_eq!(
            concat.insert_text.as_deref(),
            Some("CONCAT(${1:value}, ${2:values})")
        );
        assert_eq!(concat.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert_eq!(
            items.iter().filter(|item| item.label == "CONCAT").count(),
            1,
            "the richer catalog item should replace a legacy hard-coded item"
        );

        let relation_sql = "SELECT * FROM con";
        let mut relation_items = Vec::new();
        add_builtin_function_completions(
            relation_sql,
            lsp_position_at_end(relation_sql),
            "mysql",
            None,
            &mut relation_items,
        );
        assert!(relation_items.is_empty());

        let literal_sql = "SELECT 'con";
        let mut literal_items = Vec::new();
        add_builtin_function_completions(
            literal_sql,
            lsp_position_at_end(literal_sql),
            "mysql",
            None,
            &mut literal_items,
        );
        assert!(literal_items.is_empty());
    }

    #[test]
    fn clickhouse_dbx_catalog_supports_aliases_combinators_and_table_functions() {
        let sql = "SELECT arraym";
        let mut items = Vec::new();
        add_builtin_function_completions(
            sql,
            lsp_position_at_end(sql),
            "clickhouse",
            None,
            &mut items,
        );
        assert_eq!(
            items
                .iter()
                .find(|item| item.label == "arrayMap")
                .and_then(|item| item.insert_text.as_deref()),
            Some("arrayMap(${1:lambda}, ${2:array}, ${3:arrays})")
        );

        let alias_sql = "SELECT uca";
        let mut alias_items = Vec::new();
        add_builtin_function_completions(
            alias_sql,
            lsp_position_at_end(alias_sql),
            "clickhouse",
            None,
            &mut alias_items,
        );
        let upper = alias_items
            .iter()
            .find(|item| item.label == "upper")
            .expect("alias lookup returns the canonical function");
        assert_eq!(upper.filter_text.as_deref(), Some("uca upper"));

        let combinator_sql = "SELECT sumarrayif";
        let mut combinator_items = Vec::new();
        add_builtin_function_completions(
            combinator_sql,
            lsp_position_at_end(combinator_sql),
            "clickhouse",
            None,
            &mut combinator_items,
        );
        assert_eq!(
            combinator_items
                .iter()
                .find(|item| item.label == "sumArrayIf")
                .and_then(|item| item.insert_text.as_deref()),
            Some("sumArrayIf(${1:array}, ${2:condition})")
        );

        let relation_sql = "SELECT * FROM num";
        let mut relation_items = Vec::new();
        add_clickhouse_table_function_completions(
            relation_sql,
            lsp_position_at_end(relation_sql),
            "clickhouse",
            &mut relation_items,
        );
        let numbers = relation_items
            .iter()
            .find(|item| item.label == "numbers")
            .expect("ClickHouse numbers table function");
        assert_eq!(numbers.kind, Some(CompletionItemKind::CLASS));
        assert_eq!(numbers.insert_text.as_deref(), Some("numbers(${1:count})"));
    }

    #[test]
    fn dbx_window_function_blocks_are_expression_scoped_and_product_aware() {
        let row_sql = "SELECT row_";
        let mut postgres_items = Vec::new();
        add_window_function_completions(
            row_sql,
            lsp_position_at_end(row_sql),
            "postgres",
            &mut postgres_items,
        );
        assert_eq!(
            postgres_items
                .iter()
                .find(|item| item.label == "ROW_NUMBER")
                .and_then(|item| item.insert_text.as_deref()),
            Some("ROW_NUMBER() OVER (PARTITION BY ${1:column} ORDER BY ${2:column})")
        );

        let lag_sql = "SELECT la";
        let mut lag_items = Vec::new();
        add_window_function_completions(
            lag_sql,
            lsp_position_at_end(lag_sql),
            "mysql",
            &mut lag_items,
        );
        assert_eq!(
            lag_items
                .iter()
                .find(|item| item.label == "LAG")
                .and_then(|item| item.insert_text.as_deref()),
            Some("LAG(${1:value}) OVER (PARTITION BY ${2:column} ORDER BY ${3:column})")
        );

        let relation_sql = "SELECT * FROM row_";
        let mut relation_items = Vec::new();
        add_window_function_completions(
            relation_sql,
            lsp_position_at_end(relation_sql),
            "postgres",
            &mut relation_items,
        );
        assert!(relation_items.is_empty());

        let mut clickhouse_items = Vec::new();
        add_window_function_completions(
            row_sql,
            lsp_position_at_end(row_sql),
            "clickhouse",
            &mut clickhouse_items,
        );
        assert!(clickhouse_items.is_empty());

        let mut mongo_items = Vec::new();
        add_window_function_completions(
            row_sql,
            lsp_position_at_end(row_sql),
            "mongodb-json",
            &mut mongo_items,
        );
        assert!(mongo_items.is_empty());
    }

    #[test]
    fn live_routine_completion_takes_precedence_over_builtin_catalog() {
        let sql = "SELECT con";
        let live = CompletionItem {
            label: "CONCAT".to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("Function: CONCAT(value text) -> text".to_string()),
            insert_text: Some("CONCAT()".to_string()),
            ..Default::default()
        };
        let mut items = vec![live.clone()];
        add_builtin_function_completions(sql, lsp_position_at_end(sql), "mysql", None, &mut items);
        assert_eq!(
            items.iter().filter(|item| item.label == "CONCAT").count(),
            1
        );
        assert_eq!(
            items
                .iter()
                .find(|item| item.label == "CONCAT")
                .and_then(|item| item.detail.as_deref()),
            live.detail.as_deref()
        );
    }

    #[test]
    fn oracle_system_value_completion_is_bare_and_expression_scoped() {
        let sql = "SELECT sys";
        let mut items = Vec::new();
        add_builtin_function_completions(sql, lsp_position_at_end(sql), "oracle", None, &mut items);
        let sysdate = items
            .iter()
            .find(|item| item.label == "SYSDATE")
            .expect("Oracle SYSDATE completion");
        assert_eq!(sysdate.kind, Some(CompletionItemKind::VALUE));
        assert_eq!(sysdate.insert_text.as_deref(), Some("SYSDATE"));
        assert_eq!(
            sysdate.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT)
        );

        let relation_sql = "SELECT * FROM sys";
        let mut relation_items = Vec::new();
        add_builtin_function_completions(
            relation_sql,
            lsp_position_at_end(relation_sql),
            "oracle",
            None,
            &mut relation_items,
        );
        assert!(relation_items.is_empty());

        let mut postgres_items = Vec::new();
        add_builtin_function_completions(
            sql,
            lsp_position_at_end(sql),
            "postgres",
            None,
            &mut postgres_items,
        );
        assert!(!postgres_items.iter().any(|item| item.label == "SYSDATE"));
    }

    #[test]
    fn oracle_relation_blocks_and_table_routine_context_align_dbx() {
        let relation_sql = "SELECT * FROM json_";
        let mut relation_items = Vec::new();
        add_oracle_table_function_completions(
            relation_sql,
            lsp_position_at_end(relation_sql),
            "oracle",
            None,
            &mut relation_items,
        );
        let json_table = relation_items
            .iter()
            .find(|item| item.label == "JSON_TABLE")
            .expect("Oracle JSON_TABLE relation block");
        assert_eq!(json_table.kind, Some(CompletionItemKind::CLASS));
        assert_eq!(
            json_table.insert_text.as_deref(),
            Some("JSON_TABLE(${1:expr}, ${2:path})")
        );

        let mut postgres_items = Vec::new();
        add_oracle_table_function_completions(
            relation_sql,
            lsp_position_at_end(relation_sql),
            "postgres",
            None,
            &mut postgres_items,
        );
        assert!(postgres_items.is_empty());

        let routine_sql = "SELECT * FROM TABLE(pkg.fe";
        let context = oracle_table_routine_context(routine_sql, lsp_position_at_end(routine_sql))
            .expect("Oracle TABLE routine context");
        assert_eq!(context.prefix, "fe");
        assert_eq!(context.qualifier.as_deref(), Some("pkg"));

        let mut schema = test_schema("APP", &[]);
        schema.functions = vec![
            Function {
                name: "fetch_rows".to_string(),
                routine_type: Some("function".to_string()),
                parameters: vec![FunctionParameter {
                    name: "limit".to_string(),
                    data_type: "NUMBER".to_string(),
                    optional: false,
                }],
                return_type: "ROWSET".to_string(),
                description: Some("Fetch rows".to_string()),
            },
            Function {
                name: "refresh_rows".to_string(),
                routine_type: Some("procedure".to_string()),
                parameters: Vec::new(),
                return_type: "void".to_string(),
                description: None,
            },
        ];
        let mut routine_items = vec![CompletionItem {
            label: "orders".to_string(),
            kind: Some(CompletionItemKind::CLASS),
            ..Default::default()
        }];
        add_oracle_table_function_completions(
            routine_sql,
            lsp_position_at_end(routine_sql),
            "oceanbase-oracle",
            Some(&schema),
            &mut routine_items,
        );
        assert_eq!(routine_items.len(), 1);
        assert_eq!(routine_items[0].label, "fetch_rows");
        assert_eq!(
            routine_items[0].insert_text.as_deref(),
            Some("pkg.fetch_rows(${1:limit})")
        );
        apply_qualified_identifier_completion_edits(
            routine_sql,
            lsp_position_at_end(routine_sql),
            &mut routine_items,
        );
        let CompletionTextEdit::Edit(edit) = routine_items[0]
            .text_edit
            .as_ref()
            .expect("qualified routine replacement edit")
        else {
            panic!("expected a plain text edit");
        };
        assert_eq!(edit.new_text, "pkg.fetch_rows(${1:limit})");
    }

    #[test]
    fn db2_registers_and_dameng_functions_preserve_product_syntax() {
        let sql = "SELECT sys";
        let mut db2_items = Vec::new();
        add_builtin_function_completions(
            sql,
            lsp_position_at_end(sql),
            "db2",
            None,
            &mut db2_items,
        );
        let db2_sysdate = db2_items
            .iter()
            .find(|item| item.label == "SYSDATE")
            .expect("Db2 SYSDATE special register");
        assert_eq!(db2_sysdate.kind, Some(CompletionItemKind::VALUE));
        assert_eq!(db2_sysdate.insert_text.as_deref(), Some("SYSDATE"));

        let mut dameng_items = Vec::new();
        add_builtin_function_completions(
            sql,
            lsp_position_at_end(sql),
            "dameng",
            None,
            &mut dameng_items,
        );
        let dameng_sysdate = dameng_items
            .iter()
            .find(|item| item.label == "SYSDATE")
            .expect("Dameng SYSDATE function");
        assert_eq!(dameng_sysdate.kind, Some(CompletionItemKind::FUNCTION));
        assert_eq!(dameng_sysdate.insert_text.as_deref(), Some("SYSDATE()"));

        let add_sql = "SELECT add_";
        let mut add_items = Vec::new();
        add_builtin_function_completions(
            add_sql,
            lsp_position_at_end(add_sql),
            "db2",
            None,
            &mut add_items,
        );
        assert_eq!(
            add_items
                .iter()
                .find(|item| item.label == "ADD_DAYS")
                .and_then(|item| item.insert_text.as_deref()),
            Some("ADD_DAYS(${1:datetime}, ${2:days})")
        );
    }

    #[test]
    fn manticore_catalog_and_cloudflare_d1_now_boundary_are_expression_scoped() {
        let poly_sql = "SELECT pol";
        let mut manticore_items = Vec::new();
        add_builtin_function_completions(
            poly_sql,
            lsp_position_at_end(poly_sql),
            "manticoresearch",
            None,
            &mut manticore_items,
        );
        let poly2d = manticore_items
            .iter()
            .find(|item| item.label == "POLY2D")
            .expect("Manticore POLY2D completion");
        assert_eq!(poly2d.insert_text.as_deref(), Some("POLY2D(${1:points})"));

        let now_sql = "SELECT now";
        let mut sqlite_items = Vec::new();
        add_builtin_function_completions(
            now_sql,
            lsp_position_at_end(now_sql),
            "sqlite",
            None,
            &mut sqlite_items,
        );
        assert!(sqlite_items
            .iter()
            .any(|item| { item.label == "NOW" && item.insert_text.as_deref() == Some("NOW()") }));

        let mut d1_items = Vec::new();
        add_builtin_function_completions(
            now_sql,
            lsp_position_at_end(now_sql),
            "cloudflare-d1",
            None,
            &mut d1_items,
        );
        assert!(!d1_items.iter().any(|item| item.label == "NOW"));
    }

    #[test]
    fn signature_call_context_ignores_nested_sql_noise() {
        let sql = "SELECT calc(inner(1, 2), 'a,''b)', $$,)$$, /* , ) */ ";
        assert_eq!(
            routine_call_at_position(sql, lsp_position_at_end(sql)),
            Some(RoutineCallContext {
                name: "calc".to_string(),
                active_group: 0,
                active_parameter: 3,
                current_argument_has_content: false,
            })
        );

        let literal = "SELECT calc('value'";
        assert!(
            routine_call_at_position(literal, lsp_position_at_end(literal))
                .is_some_and(|call| call.current_argument_has_content)
        );
    }

    #[test]
    fn signature_call_context_selects_nested_and_quoted_routines() {
        let nested = "SELECT outer(inner('),', ";
        assert_eq!(
            routine_call_at_position(nested, lsp_position_at_end(nested)),
            Some(RoutineCallContext {
                name: "inner".to_string(),
                active_group: 0,
                active_parameter: 1,
                current_argument_has_content: false,
            })
        );

        let quoted = "SELECT [dbo].[计算](";
        assert_eq!(
            routine_call_at_position(quoted, lsp_position_at_end(quoted)).map(|call| call.name),
            Some("计算".to_string())
        );
    }

    #[test]
    fn signature_call_context_supports_parametric_function_groups() {
        let sql = "SELECT quantilesTDigest(0.5, 0.9)(value, ";
        assert_eq!(
            routine_call_at_position(sql, lsp_position_at_end(sql)),
            Some(RoutineCallContext {
                name: "quantilesTDigest".to_string(),
                active_group: 1,
                active_parameter: 1,
                current_argument_has_content: false,
            })
        );
    }

    #[test]
    fn signature_overload_fit_prioritizes_a_available_parameter_slot() {
        let call = RoutineCallContext {
            name: "calculate".to_string(),
            active_group: 0,
            active_parameter: 1,
            current_argument_has_content: false,
        };
        let one_parameter = Function {
            name: "calculate".to_string(),
            routine_type: Some("function".to_string()),
            parameters: vec![FunctionParameter {
                name: "value".to_string(),
                data_type: "numeric".to_string(),
                optional: false,
            }],
            return_type: "numeric".to_string(),
            description: None,
        };
        let mut two_parameters = one_parameter.clone();
        two_parameters.parameters.push(FunctionParameter {
            name: "precision".to_string(),
            data_type: "integer".to_string(),
            optional: true,
        });

        assert!(!live_overload_accepts_call(&one_parameter, &call));
        assert!(live_overload_accepts_call(&two_parameters, &call));
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
                function_case: KeywordCase::Preserve,
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
    fn completion_preferences_case_generated_functions_without_rewriting_live_metadata() {
        let mut items = vec![
            CompletionItem {
                label: "COALESCE".to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                sort_text: Some("1:builtin:coalesce".to_string()),
                filter_text: Some("COALESCE".to_string()),
                insert_text: Some("COALESCE(${1:value})".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "ROW_NUMBER".to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                sort_text: Some("1:window-function:row_number".to_string()),
                insert_text: Some(
                    "ROW_NUMBER() OVER (PARTITION BY ${1:column} ORDER BY ${2:column})".to_string(),
                ),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: Range::default(),
                    new_text: " ROW_NUMBER() OVER (PARTITION BY ${1:column} ORDER BY ${2:column})"
                        .to_string(),
                })),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "CURRENT_DATE".to_string(),
                kind: Some(CompletionItemKind::VALUE),
                sort_text: Some("1:builtin-value:current_date".to_string()),
                insert_text: Some("CURRENT_DATE".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "TABLE".to_string(),
                kind: Some(CompletionItemKind::CLASS),
                sort_text: Some("1:oracle-table-function:table".to_string()),
                insert_text: Some("TABLE(${1:function_call})".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "JSON_TABLE".to_string(),
                kind: Some(CompletionItemKind::CLASS),
                sort_text: Some("1:oracle-table-function:json_table".to_string()),
                insert_text: Some("JSON_TABLE(${1:expr}, ${2:path})".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "MixedCaseRoutine".to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some("Function: MixedCaseRoutine()".to_string()),
                sort_text: Some("0:oracle-table-routine:mixedcaseroutine".to_string()),
                insert_text: Some("MixedCaseRoutine()".to_string()),
                ..CompletionItem::default()
            },
        ];

        apply_completion_preferences(
            "SELECT ",
            Position::new(0, 7),
            &mut items,
            &CompletionPreferences {
                keyword_case: KeywordCase::Lower,
                function_case: KeywordCase::Lower,
                table_alias: TableAliasStyle::None,
            },
        );

        assert_eq!(items[0].label, "coalesce");
        assert_eq!(
            items[0].insert_text.as_deref(),
            Some("coalesce(${1:value})")
        );
        assert_eq!(items[1].label, "row_number");
        assert_eq!(
            items[1].insert_text.as_deref(),
            Some("row_number() over (partition by ${1:column} order by ${2:column})")
        );
        let Some(CompletionTextEdit::Edit(edit)) = items[1].text_edit.as_ref() else {
            panic!("window completion should keep its text edit");
        };
        assert_eq!(
            edit.new_text,
            " row_number() over (partition by ${1:column} order by ${2:column})"
        );
        assert_eq!(items[2].label, "current_date");
        assert_eq!(items[3].label, "table");
        assert_eq!(
            items[3].insert_text.as_deref(),
            Some("table(${1:function_call})")
        );
        assert_eq!(items[4].label, "json_table");
        assert_eq!(items[5].label, "MixedCaseRoutine");
        assert_eq!(items[5].insert_text.as_deref(), Some("MixedCaseRoutine()"));
    }

    #[test]
    fn completion_preferences_default_function_case_preserves_legacy_clients() {
        let preferences: CompletionPreferences = serde_json::from_value(serde_json::json!({
            "keywordCase": "lower",
            "tableAlias": "none"
        }))
        .expect("completion preferences should deserialize");

        assert!(matches!(preferences.function_case, KeywordCase::Preserve));
    }

    #[test]
    fn completion_deduplicates_keywords_without_collapsing_overloads() {
        let mut items = vec![
            CompletionItem {
                label: "TRUE".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Boolean value for enabled".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "true".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("PostgreSQL keyword".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "calculate".to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some("calculate(integer)".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "calculate".to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some("calculate(text)".to_string()),
                ..CompletionItem::default()
            },
        ];

        deduplicate_simple_completion_items(&mut items);

        assert_eq!(
            items
                .iter()
                .filter(|item| item.kind == Some(CompletionItemKind::KEYWORD))
                .count(),
            1
        );
        assert_eq!(
            items
                .iter()
                .filter(|item| item.kind == Some(CompletionItemKind::FUNCTION))
                .count(),
            2
        );
        assert_eq!(
            items[0].detail.as_deref(),
            Some("Boolean value for enabled")
        );
    }

    #[test]
    fn completion_ranks_high_frequency_keywords_within_their_semantic_group() {
        let mut items = vec![
            CompletionItem {
                label: "WHEN".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                sort_text: Some("0:when".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "where".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                sort_text: Some("0where".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "WHERE".to_string(),
                kind: Some(CompletionItemKind::FIELD),
                sort_text: Some("2:where".to_string()),
                ..CompletionItem::default()
            },
        ];

        apply_high_frequency_keyword_ranking(&mut items);

        assert_eq!(items[0].sort_text.as_deref(), Some("0:b:when"));
        assert_eq!(items[1].sort_text.as_deref(), Some("0:a:where"));
        assert!(items[1].sort_text < items[0].sort_text);
        assert_eq!(items[2].sort_text.as_deref(), Some("2:where"));
    }

    #[test]
    fn completion_frequency_rank_does_not_cross_semantic_groups() {
        let mut items = vec![
            CompletionItem {
                label: "WHERE".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                sort_text: Some("2:where".to_string()),
                ..CompletionItem::default()
            },
            CompletionItem {
                label: "WHEN".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                sort_text: Some("1:when".to_string()),
                ..CompletionItem::default()
            },
        ];

        apply_high_frequency_keyword_ranking(&mut items);

        assert!(items[0].sort_text > items[1].sort_text);
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
    fn relation_alias_completion_keeps_quoted_collisions_and_fuzzy_matches() {
        let collision_sql = "SELECT u FROM app.users u JOIN app.audit AS \"U\" ON TRUE";
        let collision_position = Position::new(0, "SELECT u".len() as u32);
        let mut collision_items = Vec::new();

        add_referenced_alias_completions(collision_sql, collision_position, &mut collision_items);

        assert_eq!(collision_items.len(), 2);
        assert!(collision_items
            .iter()
            .any(|item| { item.label == "u" && item.insert_text.as_deref() == Some("u") }));
        assert!(collision_items
            .iter()
            .any(|item| { item.label == "U" && item.insert_text.as_deref() == Some("\"U\"") }));

        let fuzzy_sql = "SELECT cmo FROM app.customer_orders customerOrder";
        let fuzzy_position = Position::new(0, "SELECT cmo".len() as u32);
        let mut fuzzy_items = Vec::new();
        add_referenced_alias_completions(fuzzy_sql, fuzzy_position, &mut fuzzy_items);

        let alias = fuzzy_items
            .iter()
            .find(|item| item.label == "customerOrder")
            .expect("ordered-subsequence alias completion");
        assert_eq!(alias.insert_text.as_deref(), Some("customerOrder"));
        assert_eq!(alias.sort_text.as_deref(), Some("-2:alias:3:customerorder"));
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
            function_case: KeywordCase::Preserve,
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

        let mut table_function_items = vec![CompletionItem {
            label: "JSON_TABLE".to_string(),
            kind: Some(CompletionItemKind::CLASS),
            insert_text: Some("JSON_TABLE(${1:expr}, ${2:path})".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        }];
        apply_completion_preferences(
            "SELECT * FROM json_",
            Position::new(0, 19),
            &mut table_function_items,
            &preferences,
        );
        assert_eq!(
            table_function_items[0].insert_text.as_deref(),
            Some("JSON_TABLE(${1:expr}, ${2:path}) ${3:jt}")
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

    fn inlay_labels(sql: &str, schema: &Schema) -> Vec<String> {
        insert_value_hints(
            sql,
            Range {
                start: Position::new(0, 0),
                end: lsp_position_at_end(sql),
            },
            schema,
        )
        .into_iter()
        .filter_map(|hint| match hint.label {
            InlayHintLabel::String(label) => Some(label),
            InlayHintLabel::LabelParts(_) => None,
        })
        .collect()
    }

    #[test]
    fn insert_value_hints_resolve_implicit_columns_from_metadata() {
        let sql = "INSERT INTO app.orders VALUES (1, 42, 9), (2, coalesce(7, 8), 10);";

        assert_eq!(
            inlay_labels(sql, &completion_schema_with_columns()),
            vec![
                "id:",
                "customer_id:",
                "total:",
                "id:",
                "customer_id:",
                "total:"
            ]
        );
    }

    #[test]
    fn insert_value_hints_keep_explicit_quoted_columns_and_skip_database_owned_implicit_columns() {
        let mut schema = completion_schema_with_columns();
        schema.tables[0].columns[0].auto_increment = true;
        schema.tables[0].columns[2].generated = true;
        schema.tables[0].columns.push(Column {
            name: "system period".to_string(),
            hidden: true,
            ..Default::default()
        });
        let sql = concat!(
            "INSERT INTO app.orders VALUES (42);\n",
            "INSERT INTO [app].[orders] ([customer_id], \"display name\") ",
            "VALUES (7, 'Ada'), (8, json_object('name', 'Lin'));"
        );

        assert_eq!(
            inlay_labels(sql, &schema),
            vec![
                "customer_id:",
                "customer_id:",
                "display name:",
                "customer_id:",
                "display name:"
            ]
        );
    }

    #[test]
    fn insert_value_hints_ignore_comments_strings_and_insert_select() {
        let sql = concat!(
            "-- INSERT INTO app.orders VALUES (1, 2, 3);\n",
            "SELECT 'INSERT INTO app.orders VALUES (4, 5, 6)';\n",
            "INSERT INTO app.orders (id, total) SELECT id, total FROM staging;"
        );

        assert!(inlay_labels(sql, &completion_schema_with_columns()).is_empty());
    }

    #[test]
    fn insert_value_hints_match_sql_server_catalog_and_implicit_dbo() {
        let mut schema = completion_schema_with_columns();
        schema.catalog = Some("AppDb".to_string());
        schema.database = "dbo".to_string();
        let sql = "INSERT INTO [AppDb]..[orders] WITH (TABLOCK) VALUES (1, 2, 3);";

        assert_eq!(
            inlay_labels(sql, &schema),
            vec!["id:", "customer_id:", "total:"]
        );
    }

    #[test]
    fn insert_value_hints_preserve_document_utf16_positions_from_a_bounded_window() {
        let prefix = "SELECT 'emoji \u{1F680}';\n".repeat(4_000);
        let statement = "INSERT INTO app.orders VALUES (1, 2, 3);";
        let sql = format!("{prefix}{statement}");
        let statement_start = prefix.len();
        let value_start = sql[statement_start..].find('1').unwrap() + statement_start;
        let hints = insert_value_hints(
            &sql,
            range_for_offsets(&sql, statement_start, sql.len()),
            &completion_schema_with_columns(),
        );

        assert_eq!(hints.len(), 3);
        assert_eq!(
            hints[0].position,
            range_for_offsets(&sql, value_start, value_start).start
        );
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
                function_case: KeywordCase::Preserve,
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
    fn insert_completion_prefers_required_columns_and_skips_database_owned_columns() {
        let mut schema = completion_schema_with_columns();
        schema.tables[0].columns = vec![
            Column {
                name: "id".to_string(),
                data_type: "bigint".to_string(),
                auto_increment: true,
                ..Default::default()
            },
            Column {
                name: "customer_id".to_string(),
                data_type: "bigint".to_string(),
                ..Default::default()
            },
            Column {
                name: "note".to_string(),
                data_type: "text".to_string(),
                nullable: true,
                ..Default::default()
            },
            Column {
                name: "created_at".to_string(),
                data_type: "timestamp".to_string(),
                default_value: Some("CURRENT_TIMESTAMP".to_string()),
                ..Default::default()
            },
            Column {
                name: "search_vector".to_string(),
                data_type: "tsvector".to_string(),
                generated: true,
                ..Default::default()
            },
            Column {
                name: "period_start".to_string(),
                data_type: "timestamp".to_string(),
                hidden: true,
                ..Default::default()
            },
        ];

        let sql = "INSERT INTO app.orders (";
        let mut items = Vec::new();
        add_insert_all_columns_completion(
            sql,
            lsp_position_at_end(sql),
            Some(&schema),
            &mut items,
            &CompletionPreferences::default(),
            "postgres",
        );

        let required = items
            .iter()
            .find(|item| item.label == "orders.required")
            .expect("required-column completion");
        assert_eq!(
            required.insert_text.as_deref(),
            Some("customer_id) VALUES (${1:value})")
        );
        let writable = items
            .iter()
            .find(|item| item.label == "orders.*")
            .expect("writable-column completion");
        assert_eq!(
            writable.insert_text.as_deref(),
            Some("customer_id, note, created_at) VALUES (${1:value}, ${2:value}, ${3:value})")
        );
        assert!(!writable
            .insert_text
            .as_deref()
            .unwrap()
            .contains("search_vector"));
        assert!(!writable
            .insert_text
            .as_deref()
            .unwrap()
            .contains("period_start"));
    }

    #[test]
    fn insert_statement_snippets_are_table_specific_and_database_owned_safe() {
        let mut schema = completion_schema_with_columns();
        schema.tables[0].columns = vec![
            Column {
                name: "id".to_string(),
                data_type: "bigint".to_string(),
                auto_increment: true,
                ..Default::default()
            },
            Column {
                name: "customer_id".to_string(),
                data_type: "bigint".to_string(),
                ..Default::default()
            },
            Column {
                name: "order note".to_string(),
                data_type: "text".to_string(),
                nullable: true,
                ..Default::default()
            },
            Column {
                name: "created_at".to_string(),
                data_type: "timestamp".to_string(),
                default_value: Some("CURRENT_TIMESTAMP".to_string()),
                ..Default::default()
            },
            Column {
                name: "search_vector".to_string(),
                data_type: "tsvector".to_string(),
                generated: true,
                ..Default::default()
            },
        ];
        schema.tables.push(Table {
            name: "order_view".to_string(),
            object_type: Some("VIEW".to_string()),
            columns: vec![Column {
                name: "id".to_string(),
                data_type: "bigint".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        });

        let sql = "ins";
        let mut items = Vec::new();
        add_insert_statement_snippets(
            sql,
            lsp_position_at_end(sql),
            Some(&schema),
            &mut items,
            &CompletionPreferences::default(),
            "postgres",
        );

        let required = items
            .iter()
            .find(|item| item.label == "insert orders required")
            .expect("required table INSERT snippet");
        assert_eq!(
            required.insert_text.as_deref(),
            Some(
                "INSERT INTO app.orders (\n  customer_id\n)\nVALUES (\n  ${1:customer_id_value}\n);"
            )
        );
        let complete = items
            .iter()
            .find(|item| item.label == "insert orders")
            .expect("complete table INSERT snippet");
        let complete_sql = complete.insert_text.as_deref().unwrap();
        assert!(complete_sql.contains("\"order note\""));
        assert!(complete_sql.contains("${2:order_note_value}"));
        assert!(!complete_sql.contains("search_vector"));
        assert!(!complete_sql.contains("\n  id"));
        assert!(!items.iter().any(|item| item.label.contains("order_view")));

        let mut after_keyword = Vec::new();
        add_insert_statement_snippets(
            "INSERT ",
            lsp_position_at_end("INSERT "),
            Some(&schema),
            &mut after_keyword,
            &CompletionPreferences::default(),
            "postgres",
        );
        assert!(after_keyword.is_empty());
    }

    #[test]
    fn generated_column_lists_quote_reserved_and_unicode_identifiers() {
        let mut schema = completion_schema_with_columns();
        schema.tables.push(Table {
            name: "事件".to_string(),
            columns: vec![
                Column {
                    name: "id".to_string(),
                    data_type: "text".to_string(),
                    ..Default::default()
                },
                Column {
                    name: "order".to_string(),
                    data_type: "text".to_string(),
                    ..Default::default()
                },
                Column {
                    name: "显示名称".to_string(),
                    data_type: "text".to_string(),
                    ..Default::default()
                },
                Column {
                    name: "隐藏值".to_string(),
                    data_type: "text".to_string(),
                    hidden: true,
                    ..Default::default()
                },
            ],
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

    fn code_action_edits(action: &CodeAction, uri: &Url) -> Vec<TextEdit> {
        action
            .edit
            .as_ref()
            .and_then(|edit| edit.changes.as_ref())
            .and_then(|changes| changes.get(uri))
            .cloned()
            .expect("code action replacement")
    }

    fn code_action_replacement(action: &CodeAction, uri: &Url) -> String {
        code_action_edits(action, uri)[0].new_text.clone()
    }

    #[test]
    fn automatic_code_actions_skip_full_document_source_work() {
        let automatic = CodeActionContext {
            diagnostics: Vec::new(),
            only: None,
            trigger_kind: None,
        };
        assert!(code_action_kind_available(
            &automatic,
            &CodeActionKind::REFACTOR_REWRITE
        ));
        assert!(!code_action_kind_explicitly_requested(
            &automatic,
            &CodeActionKind::SOURCE
        ));

        let source_only = CodeActionContext {
            diagnostics: Vec::new(),
            only: Some(vec![CodeActionKind::SOURCE]),
            trigger_kind: None,
        };
        assert!(code_action_kind_explicitly_requested(
            &source_only,
            &CodeActionKind::SOURCE
        ));
        assert!(!code_action_kind_available(
            &source_only,
            &CodeActionKind::REFACTOR_REWRITE
        ));
    }

    #[test]
    fn missing_join_condition_quick_fix_uses_unique_foreign_key_metadata() {
        let mut schema = completion_schema_with_columns();
        schema.tables[0].constraints.push(Constraint {
            name: "orders_customer_fk".to_string(),
            constraint_type: "FOREIGN KEY".to_string(),
            columns: vec!["customer_id".to_string()],
            referenced_schema: Some("app".to_string()),
            referenced_table: Some("customers".to_string()),
            referenced_columns: vec!["id".to_string()],
            definition: None,
        });
        let sql = "SELECT o.id FROM app.orders o JOIN app.customers c WHERE c.name <> '';";
        let diagnostic = sql_inspection_diagnostics(sql, Some(&schema), "postgres")
            .into_iter()
            .find(|diagnostic| {
                diagnostic.code.as_ref().is_some_and(|code| {
                    matches!(code, tower_lsp::lsp_types::NumberOrString::String(code) if code == "OXIDE002")
                })
            })
            .expect("missing JOIN diagnostic");
        let uri = Url::parse("file:///query.sql").unwrap();

        let actions = add_join_condition_actions(sql, &uri, &diagnostic, Some(&schema), "postgres");

        assert_eq!(actions.len(), 2);
        assert_eq!(
            actions[0].title,
            "Add JOIN condition via orders_customer_fk"
        );
        assert_eq!(actions[0].is_preferred, Some(true));
        assert_eq!(
            code_action_replacement(&actions[0], &uri),
            " ON o.customer_id = c.id"
        );
        let edit = &code_action_edits(&actions[0], &uri)[0];
        assert_eq!(
            position_to_byte_offset(sql, edit.range.start),
            sql.find(" WHERE").unwrap()
        );
        assert_eq!(actions[1].title, "Add non-matching JOIN safety guard");
    }

    #[test]
    fn missing_join_condition_quick_fix_keeps_ambiguous_foreign_keys_as_choices() {
        let mut schema = completion_schema_with_columns();
        schema.tables[0].constraints.extend([
            Constraint {
                name: "orders_customer_fk".to_string(),
                constraint_type: "FOREIGN KEY".to_string(),
                columns: vec!["customer_id".to_string()],
                referenced_table: Some("customers".to_string()),
                referenced_columns: vec!["id".to_string()],
                ..Default::default()
            },
            Constraint {
                name: "orders_owner_fk".to_string(),
                constraint_type: "FOREIGN KEY".to_string(),
                columns: vec!["id".to_string()],
                referenced_table: Some("customers".to_string()),
                referenced_columns: vec!["id".to_string()],
                ..Default::default()
            },
        ]);
        let sql = "SELECT * FROM app.orders o JOIN app.customers c;";
        let diagnostic = sql_inspection_diagnostics(sql, Some(&schema), "mysql")
            .into_iter()
            .find(|diagnostic| diagnostic.message.starts_with("JOIN has no"))
            .expect("missing JOIN diagnostic");
        let uri = Url::parse("file:///query.sql").unwrap();

        let actions = add_join_condition_actions(sql, &uri, &diagnostic, Some(&schema), "mysql");

        assert_eq!(actions.len(), 3);
        assert!(actions[..2]
            .iter()
            .all(|action| action.is_preferred == Some(false)));
        assert_eq!(
            position_to_byte_offset(sql, code_action_edits(&actions[0], &uri)[0].range.start),
            sql.len() - 1,
        );
    }

    #[test]
    fn missing_join_condition_quick_fix_falls_back_without_proven_metadata() {
        let schema = completion_schema_with_columns();
        let sql = "SELECT * FROM app.orders o JOIN app.customers c";
        let diagnostic = sql_inspection_diagnostics(sql, Some(&schema), "postgres")
            .into_iter()
            .find(|diagnostic| diagnostic.message.starts_with("JOIN has no"))
            .expect("missing JOIN diagnostic");
        let uri = Url::parse("file:///query.sql").unwrap();

        let actions = add_join_condition_actions(sql, &uri, &diagnostic, Some(&schema), "postgres");

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Add non-matching JOIN safety guard");
        assert_eq!(
            code_action_replacement(&actions[0], &uri),
            " ON 1 = 0 /* replace join condition before executing */"
        );
    }

    #[test]
    fn qualify_identifier_actions_use_the_unique_visible_source() {
        let sql = "SELECT total FROM app.orders o";
        let uri = Url::parse("file:///query.sql").unwrap();
        let end = "SELECT total".len();
        let actions = qualify_identifier_actions(
            sql,
            &uri,
            range_for_offsets(sql, end, end),
            Some(&completion_schema_with_columns()),
            "postgres",
        );

        assert_eq!(actions.len(), 1);
        assert_eq!(code_action_replacement(&actions[0], &uri), "o.total");
        assert_eq!(actions[0].is_preferred, Some(true));
    }

    #[test]
    fn qualify_identifier_actions_remove_only_an_unambiguous_qualifier() {
        let uri = Url::parse("file:///query.sql").unwrap();
        let unique_sql = "SELECT o.total FROM app.orders o";
        let unique_end = "SELECT o.total".len();
        let actions = qualify_identifier_actions(
            unique_sql,
            &uri,
            range_for_offsets(unique_sql, unique_end, unique_end),
            Some(&completion_schema_with_columns()),
            "postgres",
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(code_action_replacement(&actions[0], &uri), "total");

        let ambiguous_sql =
            "SELECT o.id FROM app.orders o JOIN app.customers c ON c.id = o.customer_id";
        let ambiguous_end = "SELECT o.id".len();
        assert!(qualify_identifier_actions(
            ambiguous_sql,
            &uri,
            range_for_offsets(ambiguous_sql, ambiguous_end, ambiguous_end),
            Some(&completion_schema_with_columns()),
            "postgres",
        )
        .is_empty());
    }

    #[test]
    fn qualify_identifier_actions_offer_each_metadata_backed_ambiguous_source() {
        let sql = "SELECT id FROM app.orders o JOIN app.customers c ON c.id = o.customer_id";
        let uri = Url::parse("file:///query.sql").unwrap();
        let end = "SELECT id".len();
        let actions = qualify_identifier_actions(
            sql,
            &uri,
            range_for_offsets(sql, end, end),
            Some(&completion_schema_with_columns()),
            "postgres",
        );
        let replacements = actions
            .iter()
            .map(|action| code_action_replacement(action, &uri))
            .collect::<Vec<_>>();

        assert_eq!(replacements, vec!["c.id", "o.id"]);
        assert!(actions
            .iter()
            .all(|action| action.is_preferred == Some(false)));
    }

    #[test]
    fn qualify_identifier_actions_preserve_quoted_aliases_and_columns() {
        let sql = "SELECT \"total\" FROM app.orders AS \"Order Alias\"";
        let uri = Url::parse("file:///query.sql").unwrap();
        let end = "SELECT \"total\"".len();
        let actions = qualify_identifier_actions(
            sql,
            &uri,
            range_for_offsets(sql, end, end),
            Some(&completion_schema_with_columns()),
            "postgres",
        );

        assert_eq!(actions.len(), 1);
        assert_eq!(
            code_action_replacement(&actions[0], &uri),
            "\"Order Alias\".\"total\""
        );

        let mysql_sql = "SELECT `total` FROM app.orders AS `Order Alias`";
        let mysql_end = "SELECT `total`".len();
        let mysql_actions = qualify_identifier_actions(
            mysql_sql,
            &uri,
            range_for_offsets(mysql_sql, mysql_end, mysql_end),
            Some(&completion_schema_with_columns()),
            "mysql",
        );
        assert_eq!(mysql_actions.len(), 1);
        assert_eq!(
            code_action_replacement(&mysql_actions[0], &uri),
            "`Order Alias`.`total`"
        );

        let mysql_qualified = "SELECT `o`.`total` FROM app.orders AS o";
        let mysql_qualified_end = "SELECT `o`.`total`".len();
        let mysql_unqualify = qualify_identifier_actions(
            mysql_qualified,
            &uri,
            range_for_offsets(mysql_qualified, mysql_qualified_end, mysql_qualified_end),
            Some(&completion_schema_with_columns()),
            "mysql",
        );
        assert_eq!(mysql_unqualify.len(), 1);
        assert_eq!(
            code_action_replacement(&mysql_unqualify[0], &uri),
            "`total`"
        );
    }

    #[test]
    fn qualify_identifier_actions_keep_quoted_aliases_distinct_from_ordinary_aliases() {
        let uri = Url::parse("file:///query.sql").unwrap();
        let schema = completion_schema_with_columns();

        let collision_sql =
            "SELECT id FROM app.orders u JOIN app.customers AS \"U\" ON \"U\".id = u.customer_id";
        let collision_end = "SELECT id".len();
        let replacements = qualify_identifier_actions(
            collision_sql,
            &uri,
            range_for_offsets(collision_sql, collision_end, collision_end),
            Some(&schema),
            "postgres",
        )
        .iter()
        .map(|action| code_action_replacement(action, &uri))
        .collect::<Vec<_>>();
        assert_eq!(replacements, vec!["\"U\".id", "u.id"]);

        let wrong_quote_sql = "SELECT U.total FROM app.orders AS \"U\"";
        let wrong_quote_end = "SELECT U.total".len();
        assert!(qualify_identifier_actions(
            wrong_quote_sql,
            &uri,
            range_for_offsets(wrong_quote_sql, wrong_quote_end, wrong_quote_end),
            Some(&schema),
            "postgres",
        )
        .is_empty());

        let quoted_sql = "SELECT \"U\".total FROM app.orders AS \"U\"";
        let quoted_end = "SELECT \"U\".total".len();
        let actions = qualify_identifier_actions(
            quoted_sql,
            &uri,
            range_for_offsets(quoted_sql, quoted_end, quoted_end),
            Some(&schema),
            "postgres",
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(code_action_replacement(&actions[0], &uri), "total");
    }

    #[test]
    fn qualify_identifier_actions_ignore_alias_declarations_and_sql_noise() {
        let uri = Url::parse("file:///query.sql").unwrap();
        for (sql, end) in [
            (
                "SELECT total AS amount FROM app.orders o",
                "SELECT total AS amount".len(),
            ),
            ("SELECT 'total' FROM app.orders o", "SELECT 'total".len()),
            (
                "SELECT total -- amount\nFROM app.orders o",
                "SELECT total -- amount".len(),
            ),
        ] {
            assert!(
                qualify_identifier_actions(
                    sql,
                    &uri,
                    range_for_offsets(sql, end, end),
                    Some(&completion_schema_with_columns()),
                    "postgres",
                )
                .is_empty(),
                "unexpected intention for {sql}"
            );
        }
    }

    #[test]
    fn batch_qualify_identifier_action_edits_only_unique_metadata_columns() {
        let sql = "SELECT total, name, id, COUNT(id), 'total' FROM app.orders o JOIN app.customers c ON c.id = o.customer_id";
        let uri = Url::parse("file:///query.sql").unwrap();
        let start = "SELECT ".len();
        let end = sql.find(" FROM").unwrap();
        let actions = qualify_identifier_actions(
            sql,
            &uri,
            range_for_offsets(sql, start, end),
            Some(&completion_schema_with_columns()),
            "postgres",
        );

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Qualify 2 selected columns");
        assert_eq!(
            code_action_edits(&actions[0], &uri)
                .into_iter()
                .map(|edit| edit.new_text)
                .collect::<Vec<_>>(),
            vec!["o.total", "c.name"]
        );
    }

    #[test]
    fn qualify_identifier_actions_include_unaliased_cte_row_sources() {
        let mut schema = completion_schema_with_columns();
        schema.tables.push(Table {
            name: "recent".to_string(),
            columns: vec![Column {
                name: "total".to_string(),
                data_type: "bigint".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let sql = "WITH recent AS (SELECT total FROM app.orders) SELECT total FROM recent";
        let uri = Url::parse("file:///query.sql").unwrap();
        let end = sql.rfind("total").unwrap() + "total".len();
        let actions = qualify_identifier_actions(
            sql,
            &uri,
            range_for_offsets(sql, end, end),
            Some(&schema),
            "postgres",
        );

        assert_eq!(actions.len(), 1);
        assert_eq!(code_action_replacement(&actions[0], &uri), "recent.total");
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
    fn resolves_mixed_case_unquoted_aliases_for_column_schema_selection() {
        let manager = SchemaManager::new();
        manager.register(test_schema("app", &["users"]));
        let audit_id = manager.register(test_schema("audit", &["events"]));
        let sql = "SELECT * FROM audit.events EventAlias WHERE eventalias.";

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

    #[test]
    fn project_sql_index_tracks_ddl_calls_and_physical_relation_references() {
        let sql = r#"-- FROM ignored.orders
CREATE VIEW reporting.active_orders AS
SELECT * FROM app.orders;
SELECT 'FROM secret.orders';
WITH recent AS (SELECT * FROM app.orders)
SELECT * FROM recent;
CALL ops.refresh_orders();
CREATE FUNCTION ops.compute_total() RETURNS integer AS $$ SELECT 1 $$ LANGUAGE SQL;
SELECT ops.compute_total();
SELECT * FROM recent;"#;
        let occurrences = project_sql_symbol_occurrences(sql);

        assert!(occurrences.iter().any(|occurrence| {
            occurrence.role == ProjectSqlSymbolRole::Definition
                && occurrence.kind == ProjectSqlSymbolKind::View
                && occurrence.normalized_name == "reporting.active_orders"
        }));
        assert_eq!(
            occurrences
                .iter()
                .filter(|occurrence| {
                    occurrence.role == ProjectSqlSymbolRole::Reference
                        && occurrence.normalized_name == "app.orders"
                })
                .count(),
            2
        );
        assert!(occurrences.iter().any(|occurrence| {
            occurrence.kind == ProjectSqlSymbolKind::Procedure
                && occurrence.normalized_name == "ops.refresh_orders"
        }));
        assert!(occurrences.iter().any(|occurrence| {
            occurrence.kind == ProjectSqlSymbolKind::Function
                && occurrence.role == ProjectSqlSymbolRole::Definition
                && occurrence.normalized_name == "ops.compute_total"
        }));
        assert!(occurrences.iter().any(|occurrence| {
            occurrence.kind == ProjectSqlSymbolKind::Function
                && occurrence.role == ProjectSqlSymbolRole::Reference
                && occurrence.normalized_name == "ops.compute_total"
        }));
        assert_eq!(
            occurrences
                .iter()
                .filter(|occurrence| occurrence.normalized_name == "recent")
                .count(),
            1,
            "a CTE name must become physical again outside its statement"
        );
        assert!(!occurrences.iter().any(|occurrence| {
            matches!(
                occurrence.normalized_name.as_str(),
                "ignored.orders" | "secret.orders"
            )
        }));
    }

    #[test]
    fn project_sql_index_preserves_quoted_names_and_utf16_selection_ranges() {
        let sql = "SELECT 'emoji 馃榾', * FROM [AppDb]..[orders]";
        let occurrence = project_sql_symbol_occurrences(sql)
            .into_iter()
            .find(|occurrence| occurrence.normalized_name == "appdb.dbo.orders")
            .expect("SQL Server relation should be indexed");
        let orders_start = sql.find("orders").unwrap();

        assert_eq!(occurrence.name, "orders");
        assert_eq!(
            occurrence.range.start.character,
            sql[..orders_start].encode_utf16().count() as u32
        );
        assert_eq!(
            occurrence.range.end.character,
            sql[..orders_start + "orders".len()].encode_utf16().count() as u32
        );
    }

    #[test]
    fn project_sql_symbol_matching_uses_qualified_suffixes_and_object_families() {
        let relation = |normalized_name: &str, kind| ProjectSqlSymbolOccurrence {
            name: normalized_name.split('.').next_back().unwrap().to_string(),
            normalized_name: normalized_name.to_string(),
            kind,
            role: ProjectSqlSymbolRole::Reference,
            range: Range::default(),
        };
        let qualified = relation("catalog.app.orders", ProjectSqlSymbolKind::Table);
        let schema_qualified = relation("app.orders", ProjectSqlSymbolKind::View);
        let unqualified = relation("orders", ProjectSqlSymbolKind::Table);
        let routine = relation("orders", ProjectSqlSymbolKind::Procedure);

        assert!(project_sql_symbols_match(&qualified, &schema_qualified));
        assert!(project_sql_symbols_match(&qualified, &unqualified));
        assert!(!project_sql_symbols_match(&qualified, &routine));
    }

    #[test]
    fn project_sql_index_is_bounded_for_oversized_documents() {
        let sql = format!(
            "SELECT * FROM orders;{}",
            " ".repeat(PROJECT_SQL_INDEX_MAX_BYTES)
        );
        assert!(project_sql_symbol_occurrences(&sql).is_empty());
    }
}
