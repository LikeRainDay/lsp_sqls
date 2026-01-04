use crate::dialect::Dialect;
use crate::dialects::DialectRegistry;
use crate::schema::{Schema, SchemaId, SchemaManager};
use dashmap::DashMap;
use std::sync::Arc;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

/// 文档管理器，用于存储和管理打开的文档内容
#[derive(Clone)]
struct DocumentManager {
    documents: Arc<DashMap<String, String>>,
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
    /// 文件到方言的映射
    file_dialects: Arc<DashMap<String, String>>,
    /// 文件到 Schema ID 的映射
    file_schemas: Arc<DashMap<String, SchemaId>>,
    /// 文档管理器
    document_manager: DocumentManager,
}

impl SqlLspServer {
    pub fn new(client: Client) -> Self {
        tracing::info!("Creating new SQL LSP server instance");
        Self {
            client,
            dialect_registry: Arc::new(DialectRegistry::new()),
            schema_manager: Arc::new(SchemaManager::new()),
            file_dialects: Arc::new(DashMap::new()),
            file_schemas: Arc::new(DashMap::new()),
            document_manager: DocumentManager::new(),
        }
    }

    /// 获取文件的方言
    fn get_dialect_for_file(&self, uri: &str) -> Option<Arc<dyn Dialect>> {
        self.file_dialects
            .get(uri)
            .and_then(|dialect_name| self.dialect_registry.get_by_name(dialect_name.value()))
    }

    /// 获取文件的 Schema
    /// 如果文件没有显式关联的 Schema，则根据 SQL 内容自动推断最佳匹配的 Schema
    fn get_schema_for_file(&self, uri: &str) -> Option<Schema> {
        // 1. 先检查是否有显式关联的 Schema
        if let Some(schema_id) = self.file_schemas.get(uri) {
            return self.schema_manager.get(*schema_id.value());
        }

        // 2. 从文档内容推断 Schema
        if let Some(text) = self.document_manager.get(uri) {
            use crate::parser::SqlParser;
            let mut parser = SqlParser::new();
            let parse_result = parser.parse(&text);

            if let Some(tree) = parse_result.tree {
                let tables = parser.extract_tables(&tree, &text);

                if !tables.is_empty() {
                    // 3. 查找最佳匹配的 Schema
                    let best_match = self
                        .schema_manager
                        .list_ids()
                        .iter()
                        .filter_map(|&schema_id| {
                            let schema = self.schema_manager.get(schema_id)?;
                            let score = self.calculate_schema_match_score(&tables, &schema);
                            if score > 0 {
                                Some((schema_id, score))
                            } else {
                                None
                            }
                        })
                        .max_by_key(|(_, score)| *score);

                    if let Some((schema_id, _score)) = best_match {
                        // 缓存推断结果
                        self.file_schemas.insert(uri.to_string(), schema_id);
                        return self.schema_manager.get(schema_id);
                    }
                }
            }
        }

        None
    }

    /// 计算 Schema 匹配分数
    /// 返回匹配分数，分数越高表示匹配度越高
    fn calculate_schema_match_score(&self, tables: &[String], schema: &Schema) -> i32 {
        let mut score = 0;

        for table_name in tables {
            // 完全匹配：+10 分
            if schema.tables.iter().any(|t| t.name == *table_name) {
                score += 10;
            } else {
                // 模糊匹配：+5 分
                for schema_table in &schema.tables {
                    if schema_table.name.contains(table_name)
                        || table_name.contains(&schema_table.name)
                    {
                        score += 5;
                        break; // 每个表名只匹配一次
                    }
                }
            }
        }

        // 如果匹配的表数量越多，额外加分
        let matched_count = tables
            .iter()
            .filter(|table_name| schema.tables.iter().any(|t| t.name == **table_name))
            .count();

        if matched_count > 1 {
            score += matched_count as i32 * 2; // 多表匹配额外加分
        }

        score
    }

    /// 将 LSP Position 转换为字符串字节偏移
    fn position_to_offset(&self, text: &str, position: tower_lsp::lsp_types::Position) -> usize {
        let mut offset = 0;
        for (line_idx, line) in text.lines().enumerate() {
            if line_idx < position.line as usize {
                offset += line.len() + 1; // +1 for newline
            } else {
                offset += position.character.min(line.len() as u32) as usize;
                break;
            }
        }
        offset.min(text.len())
    }
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
                if let Some(file_schemas_obj) = file_schemas_value.as_object() {
                    for (uri, schema_id_str) in file_schemas_obj {
                        if let Some(id_str) = schema_id_str.as_str() {
                            if let Ok(schema_id) = id_str.parse::<crate::schema::SchemaId>() {
                                self.file_schemas.insert(uri.clone(), schema_id);
                            }
                        }
                    }
                    self.client
                        .log_message(MessageType::INFO, "Updated file-schema mappings")
                        .await;
                }
            }
        }
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        let text = params.text_document.text.clone();
        let language_id = params.text_document.language_id.clone();

        // 存储文档内容
        self.document_manager.update(uri.clone(), text.clone());

        // 尝试从 URI 和 languageId 推断方言
        // 优先级：URI 扩展名 > languageId > 默认 MySQL
        let dialect_name = infer_dialect_from_uri_and_language(&uri, &language_id);
        self.file_dialects.insert(uri.clone(), dialect_name.clone());

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
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_file(&uri);
            let items = dialect.completion(&text, position, schema.as_ref()).await;
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
            let schema = self.get_schema_for_file(&uri);
            return Ok(dialect.hover(&text, position, schema.as_ref()).await);
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_file(&uri);
            if let Some(location) = dialect
                .goto_definition(&text, position, schema.as_ref())
                .await
            {
                return Ok(Some(GotoDefinitionResponse::Scalar(location)));
            }
        }

        Ok(None)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;

        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let schema = self.get_schema_for_file(&uri);
            let locations = dialect.references(&text, position, schema.as_ref()).await;
            return Ok(Some(locations));
        }

        Ok(None)
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.to_string();
        let text = self.document_manager.get(&uri).unwrap_or_default();

        if let Some(dialect) = self.get_dialect_for_file(&uri) {
            let formatted = dialect.format(&text).await;
            let line_count = if text.is_empty() {
                0
            } else {
                text.lines().count() as u32
            };
            let range = Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: line_count.saturating_sub(1),
                    character: 0,
                },
            };
            return Ok(Some(vec![TextEdit {
                range,
                new_text: formatted,
            }]));
        }

        Ok(None)
    }
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
/// 3. 默认 MySQL
fn infer_dialect_from_uri_and_language(uri: &str, language_id: &str) -> String {
    // 首先尝试从 URI 扩展名推断
    let uri_lower = uri.to_lowercase();

    if uri_lower.ends_with(".mysql.sql") || uri_lower.ends_with(".mysql") {
        return "mysql".to_string();
    } else if uri_lower.ends_with(".postgres.sql") || uri_lower.ends_with(".pgsql") {
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
    }

    // 如果 URI 无法推断，尝试从 languageId 推断
    let lang_lower = language_id.to_lowercase();
    match lang_lower.as_str() {
        "mysql" | "mysql-sql" => "mysql".to_string(),
        "postgresql" | "postgres" | "pgsql" => "postgres".to_string(),
        "hive" | "hql" => "hive".to_string(),
        "elasticsearch-eql" | "eql" => "elasticsearch-eql".to_string(),
        "elasticsearch-dsl" | "es-dsl" | "json" if uri_lower.contains("elasticsearch") => {
            "elasticsearch-dsl".to_string()
        }
        "clickhouse" | "ch" => "clickhouse".to_string(),
        "redis" => "redis".to_string(),
        _ => "mysql".to_string(), // 默认使用 MySQL
    }
}
