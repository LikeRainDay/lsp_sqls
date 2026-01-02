use crate::dialect::Dialect;
use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, Hover, Location,
    MarkedString, NumberOrString, Position, Range,
};

/// Elasticsearch EQL (Event Query Language) 方言
pub struct ElasticsearchEqlDialect;

impl Default for ElasticsearchEqlDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl ElasticsearchEqlDialect {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Dialect for ElasticsearchEqlDialect {
    fn name(&self) -> &str {
        "elasticsearch-eql"
    }

    async fn parse(&self, sql: &str, _schema: Option<&Schema>) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        if sql.trim().is_empty() {
            return diagnostics;
        }

        // EQL 特定的语法检查
        // EQL 查询通常以 sequence, join, event 等关键字开始
        let upper_sql = sql.to_uppercase();
        if upper_sql.contains("SEQUENCE")
            || upper_sql.contains("JOIN")
            || upper_sql.contains("EVENT")
        {
            // 检查基本的 EQL 语法结构
            if upper_sql.contains("SEQUENCE") && !upper_sql.contains("WHERE") {
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: sql.len() as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("EQL001".to_string())),
                    code_description: None,
                    source: Some("elasticsearch-eql".to_string()),
                    message: "EQL SEQUENCE query should typically include a WHERE clause"
                        .to_string(),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }

        diagnostics
    }

    async fn completion(
        &self,
        _sql: &str,
        _position: Position,
        schema: Option<&Schema>,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        // EQL 关键字和命令
        let keywords = vec![
            "sequence",
            "join",
            "event",
            "where",
            "with",
            "maxspan",
            "until",
            "by",
            "any",
            "all",
            "in",
            "not",
            "and",
            "or",
            "true",
            "false",
            "null",
            "length",
            "start",
            "end",
            "key",
            "value",
            "count",
            "head",
            "tail",
            "array",
            "array_length",
            "array_search",
            "array_sort",
            "array_unique",
            "cidr_match",
            "concat",
            "ends_with",
            "index_of",
            "length",
            "match",
            "number",
            "starts_with",
            "string",
            "string_contains",
            "string_ends_with",
            "string_starts_with",
            "substring",
            "wildcard",
            "between",
            "in",
            "is_null",
            "is_not_null",
            "like",
            "not_like",
            "regex",
            "not_regex",
        ];

        for keyword in keywords {
            items.push(CompletionItem {
                label: keyword.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some(format!("EQL keyword: {}", keyword)),
                documentation: None,
                deprecated: None,
                preselect: None,
                sort_text: Some(format!("0{}", keyword)),
                filter_text: None,
                insert_text: Some(keyword.to_string()),
                insert_text_format: None,
                text_edit: None,
                additional_text_edits: None,
                commit_characters: None,
                command: None,
                data: None,
                tags: None,
                insert_text_mode: None,
                label_details: None,
            });
        }

        if let Some(schema) = schema {
            for table in &schema.tables {
                items.push(CompletionItem {
                    label: table.name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(format!("Elasticsearch Index: {}", table.name)),
                    documentation: table
                        .comment
                        .clone()
                        .map(tower_lsp::lsp_types::Documentation::String),
                    deprecated: None,
                    preselect: None,
                    sort_text: Some(format!("1{}", table.name)),
                    filter_text: None,
                    insert_text: Some(table.name.clone()),
                    insert_text_format: None,
                    text_edit: None,
                    additional_text_edits: None,
                    commit_characters: None,
                    command: None,
                    data: None,
                    tags: None,
                    insert_text_mode: None,
                    label_details: None,
                });
            }
        }

        items
    }

    async fn hover(
        &self,
        sql: &str,
        _position: Position,
        schema: Option<&Schema>,
    ) -> Option<Hover> {
        if let Some(schema) = schema {
            for table in &schema.tables {
                if sql.contains(&table.name) {
                    return Some(Hover {
                        contents: tower_lsp::lsp_types::HoverContents::Scalar(
                            MarkedString::String(format!(
                                "Elasticsearch EQL Index: {}\n{}",
                                table.name,
                                table.comment.as_deref().unwrap_or("No description")
                            )),
                        ),
                        range: None,
                    });
                }
            }
        }
        None
    }

    async fn goto_definition(
        &self,
        _sql: &str,
        _position: Position,
        _schema: Option<&Schema>,
    ) -> Option<Location> {
        None
    }

    async fn references(
        &self,
        _sql: &str,
        _position: Position,
        _schema: Option<&Schema>,
    ) -> Vec<Location> {
        Vec::new()
    }

    async fn format(&self, sql: &str) -> String {
        // EQL 格式化：保持基本格式
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(sql, schema).await
    }
}
