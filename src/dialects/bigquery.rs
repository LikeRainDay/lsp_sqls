use crate::dialect::Dialect;
use crate::parser::SqlParser;
use crate::schema::Schema;
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, Hover, Location, MarkedString, Position,
};

pub struct BigQueryDialect {
    parser: std::sync::Mutex<SqlParser>,
    schema_cache: std::sync::Arc<DashMap<String, crate::schema::Table>>,
}

impl Default for BigQueryDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl BigQueryDialect {
    pub fn new() -> Self {
        Self {
            parser: std::sync::Mutex::new(SqlParser::new()),
            schema_cache: std::sync::Arc::new(DashMap::new()),
        }
    }

    /// 手动添加表 Schema 到缓存（用于测试或预加载）
    pub fn add_to_cache(&self, table_name: String, table: crate::schema::Table) {
        self.schema_cache.insert(table_name, table);
    }

    fn create_keyword_item(&self, keyword: &str) -> CompletionItem {
        CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(format!("BigQuery keyword: {}", keyword)),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("0{}", keyword)),
            filter_text: None,
            insert_text: Some(keyword.to_string()),
            insert_text_format: None,
            insert_text_mode: None,
            text_edit: None,
            additional_text_edits: None,
            commit_characters: None,
            command: None,
            data: None,
            tags: None,
            label_details: None,
        }
    }

    fn create_table_item(&self, table: &crate::schema::Table) -> CompletionItem {
        CompletionItem {
            label: table.name.clone(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some(format!("Table: {}", table.name)),
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
            insert_text_mode: None,
            text_edit: None,
            additional_text_edits: None,
            commit_characters: None,
            command: None,
            data: None,
            tags: None,
            label_details: None,
        }
    }

    fn create_column_item(
        &self,
        column: &crate::schema::Column,
        table_name: Option<&str>,
    ) -> CompletionItem {
        let label = if let Some(table) = table_name {
            format!("{}.{}", table, column.name)
        } else {
            column.name.clone()
        };

        CompletionItem {
            label,
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(format!("Column: {} ({})", column.name, column.data_type)),
            documentation: column
                .comment
                .clone()
                .map(tower_lsp::lsp_types::Documentation::String),
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("2{}", column.name)),
            filter_text: None,
            insert_text: Some(column.name.clone()),
            insert_text_format: None,
            insert_text_mode: None,
            text_edit: None,
            additional_text_edits: None,
            commit_characters: None,
            command: None,
            data: None,
            tags: None,
            label_details: None,
        }
    }

    async fn get_or_fetch_table_schema(&self, table_name: &str) -> Option<crate::schema::Table> {
        if let Some(table) = self.schema_cache.get(table_name) {
            return Some(table.clone());
        }

        let parts: Vec<&str> = table_name.split('.').collect();

        let project = if parts.len() == 2 {
            let output = tokio::process::Command::new("gcloud")
                .args(["config", "get", "project"])
                .output()
                .await
                .ok()?;
            let p = String::from_utf8(output.stdout).ok()?;
            p.trim().to_string()
        } else {
            String::new()
        };

        let (project_id, dataset_id, table_id) = match parts.len() {
            3 => (parts[0], parts[1], parts[2]),
            2 => (project.as_str(), parts[0], parts[1]),
            1 => return None,
            _ => return None,
        };

        let token = BigQueryClient::get_access_token().await?;

        match BigQueryClient::fetch_table_schema(project_id, dataset_id, table_id, &token).await {
            Ok(table) => {
                self.schema_cache
                    .insert(table_name.to_string(), table.clone());
                Some(table)
            }
            Err(e) => {
                tracing::error!("Failed to fetch schema for {}: {}", table_name, e);
                None
            }
        }
    }
}

#[async_trait]
impl Dialect for BigQueryDialect {
    fn name(&self) -> &str {
        "bigquery"
    }

    async fn parse(&self, sql: &str, _schema: Option<&Schema>) -> Vec<Diagnostic> {
        let mut parser = self.parser.lock().unwrap();
        let parse_result = parser.parse(sql);
        parse_result.diagnostics
    }

    async fn completion(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Vec<CompletionItem> {
        let (parse_result, context) = {
            let mut parser = self.parser.lock().unwrap();
            let parse_result = parser.parse(sql);
            let context = if let Some(tree) = &parse_result.tree {
                if let Some(node) = parser.get_node_at_position(tree, position) {
                    parser.analyze_completion_context(node, sql, position)
                } else {
                    crate::parser::CompletionContext::Default
                }
            } else {
                crate::parser::CompletionContext::Default
            };
            (parse_result, context)
        };

        let mut items = Vec::new();
        let keywords = &[
            "SELECT",
            "FROM",
            "WHERE",
            "INSERT",
            "INTO",
            "UPDATE",
            "DELETE",
            "CREATE",
            "DROP",
            "ALTER",
            "TABLE",
            "VIEW",
            "JOIN",
            "INNER",
            "LEFT",
            "RIGHT",
            "FULL",
            "OUTER",
            "ON",
            "GROUP",
            "BY",
            "ORDER",
            "HAVING",
            "LIMIT",
            "OFFSET",
            "UNION",
            "ALL",
            "DISTINCT",
            "WITH",
            "EXCEPT",
            "AND",
            "OR",
            "NOT",
            "IN",
            "LIKE",
            "BETWEEN",
            "IS",
            "NULL",
            "CASE",
            "WHEN",
            "THEN",
            "ELSE",
            "END",
            "CAST",
            "MERGE",
            "WINDOW",
            "QUALIFY",
            "UNNEST",
            "STRUCT",
            "ARRAY",
            "PARTITION",
            "CLUSTER",
            "OPTIONS",
            "SYSTEM_TIME",
            "OF",
        ];

        match context {
            crate::parser::CompletionContext::FromClause
            | crate::parser::CompletionContext::JoinClause => {
                let join_keywords: Vec<&str> = keywords
                    .iter()
                    .filter(|&&k| matches!(k, "JOIN" | "INNER" | "LEFT" | "RIGHT" | "OUTER" | "ON"))
                    .copied()
                    .collect();
                for keyword in join_keywords {
                    items.push(self.create_keyword_item(keyword));
                }
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        items.push(self.create_table_item(table));
                    }
                }
            }
            crate::parser::CompletionContext::SelectClause => {
                let select_keywords: Vec<&str> = keywords
                    .iter()
                    .filter(|&&k| matches!(k, "SELECT" | "DISTINCT" | "AS" | "FROM"))
                    .copied()
                    .collect();
                for keyword in select_keywords {
                    items.push(self.create_keyword_item(keyword));
                }
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_column_item(
                                column,
                                Some(&format!("{}.{}", schema.database, table.name)),
                            ));
                        }
                    }
                }

                // Add columns from dynamically fetched tables
                if let Some(tree) = &parse_result.tree {
                    let referenced_tables = {
                        let parser = self.parser.lock().unwrap();
                        parser.extract_tables(tree, sql)
                    };

                    for table_name in &referenced_tables {
                        if let Some(table) = self.get_or_fetch_table_schema(table_name).await {
                            for column in &table.columns {
                                items.push(self.create_column_item(column, Some(table_name)));
                            }
                        }
                    }
                }
            }
            crate::parser::CompletionContext::WhereClause => {
                let where_keywords: Vec<&str> = keywords
                    .iter()
                    .filter(|&&k| {
                        matches!(
                            k,
                            "AND" | "OR" | "NOT" | "IN" | "LIKE" | "BETWEEN" | "IS" | "NULL"
                        )
                    })
                    .copied()
                    .collect();
                for keyword in where_keywords {
                    items.push(self.create_keyword_item(keyword));
                }
                let operators = vec!["=", "<>", "!=", ">", "<", ">=", "<="];
                for op in operators {
                    items.push(CompletionItem {
                        label: op.to_string(),
                        kind: Some(CompletionItemKind::OPERATOR),
                        detail: Some(format!("Operator: {}", op)),
                        documentation: None,
                        deprecated: None,
                        preselect: None,
                        sort_text: Some(format!("1{}", op)),
                        filter_text: None,
                        insert_text: Some(op.to_string()),
                        insert_text_format: None,
                        insert_text_mode: None,
                        text_edit: None,
                        additional_text_edits: None,
                        commit_characters: None,
                        command: None,
                        data: None,
                        tags: None,
                        label_details: None,
                    });
                }
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_column_item(
                                column,
                                Some(&format!("{}.{}", schema.database, table.name)),
                            ));
                        }
                    }
                }

                // Add columns from dynamically fetched tables
                if let Some(tree) = &parse_result.tree {
                    let referenced_tables = {
                        let parser = self.parser.lock().unwrap();
                        parser.extract_tables(tree, sql)
                    };
                    for table_name in &referenced_tables {
                        if let Some(table) = self.get_or_fetch_table_schema(table_name).await {
                            for column in &table.columns {
                                items.push(self.create_column_item(column, Some(table_name)));
                            }
                        }
                    }
                }
            }
            crate::parser::CompletionContext::OrderByClause
            | crate::parser::CompletionContext::GroupByClause => {
                let keywords_list: Vec<&str> = keywords
                    .iter()
                    .filter(|&&k| matches!(k, "ASC" | "DESC" | "BY"))
                    .copied()
                    .collect();
                for keyword in keywords_list {
                    items.push(self.create_keyword_item(keyword));
                }
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_column_item(
                                column,
                                Some(&format!("{}.{}", schema.database, table.name)),
                            ));
                        }
                    }
                }
            }
            crate::parser::CompletionContext::HavingClause => {
                let having_keywords: Vec<&str> = keywords
                    .iter()
                    .filter(|&&k| {
                        matches!(
                            k,
                            "AND" | "OR" | "NOT" | "IN" | "LIKE" | "BETWEEN" | "IS" | "NULL"
                        )
                    })
                    .copied()
                    .collect();
                for keyword in having_keywords {
                    items.push(self.create_keyword_item(keyword));
                }
                let aggregate_functions = vec!["COUNT", "SUM", "AVG", "MIN", "MAX"];
                for func in aggregate_functions {
                    items.push(self.create_keyword_item(func));
                }
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_column_item(
                                column,
                                Some(&format!("{}.{}", schema.database, table.name)),
                            ));
                        }
                    }
                }
            }
            crate::parser::CompletionContext::TableColumn => {
                if let Some(tree) = &parse_result.tree {
                    let parser = self.parser.lock().unwrap();
                    if let Some(node) = parser.get_node_at_position(tree, position) {
                        if let Some(table_name) = parser.get_table_name_for_column(node, sql) {
                            if let Some(schema) = schema {
                                if let Some(table) = schema.tables.iter().find(|t| {
                                    t.name == table_name
                                        || format!("{}.{}", schema.database, t.name) == table_name
                                }) {
                                    for column in &table.columns {
                                        items.push(self.create_column_item(column, None));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            crate::parser::CompletionContext::Default => {
                for keyword in keywords {
                    items.push(self.create_keyword_item(keyword));
                }
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        items.push(self.create_table_item(table));
                    }
                }
            }
        }

        items
    }

    async fn hover(&self, sql: &str, position: Position, schema: Option<&Schema>) -> Option<Hover> {
        // 1. Check static schema first
        if let Some(schema) = schema {
            for table in &schema.tables {
                if sql.contains(&table.name) {
                    return Some(Hover {
                        contents: tower_lsp::lsp_types::HoverContents::Scalar(
                            MarkedString::String(format!(
                                "BigQuery Table: {}.{}\n{}",
                                schema.database,
                                table.name,
                                table.comment.as_deref().unwrap_or("No description")
                            )),
                        ),
                        range: None,
                    });
                }
            }
        }

        // 2. Check dynamic schema
        let (node_text, node_range) = {
            let mut parser = self.parser.lock().unwrap();
            let parse_result = parser.parse(sql);
            if let Some(tree) = &parse_result.tree {
                if let Some(node) = parser.get_node_at_position(tree, position) {
                    (
                        Some(parser.node_text(node, sql)),
                        Some(parser.node_range(node)),
                    )
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            }
        };

        if let Some(node_text) = node_text {
            if let Some(table) = self.get_or_fetch_table_schema(&node_text).await {
                let mut info = format!("**BigQuery Table**: `{}`\n\n", table.name);
                if let Some(comment) = &table.comment {
                    info.push_str(&format!("{}\n\n", comment));
                }
                info.push_str(&format!("**Columns** ({})\n", table.columns.len()));
                for col in &table.columns {
                    info.push_str(&format!(
                        "- `{}`: {} {}\n",
                        col.name,
                        col.data_type,
                        if col.nullable { "" } else { "NOT NULL" }
                    ));
                }

                return Some(Hover {
                    contents: tower_lsp::lsp_types::HoverContents::Markup(
                        tower_lsp::lsp_types::MarkupContent {
                            kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                            value: info,
                        },
                    ),
                    range: node_range,
                });
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
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(sql, schema).await
    }

    async fn trigger_background_fetch(&self, sql: &str) {
        let mut parser = self.parser.lock().unwrap();
        let parse_result = parser.parse(sql);
        if let Some(tree) = &parse_result.tree {
            let referenced_tables = parser.extract_tables(tree, sql);
            for table_name in referenced_tables {
                let cache = self.schema_cache.clone();
                let t_name = table_name.clone();

                if cache.contains_key(&t_name) {
                    continue;
                }

                tokio::spawn(async move {
                    let parts: Vec<&str> = t_name.split('.').collect();
                    let project = if parts.len() == 2 {
                        let output = tokio::process::Command::new("gcloud")
                            .args(["config", "get", "project"])
                            .output()
                            .await
                            .ok();
                        if let Some(output) = output {
                            if output.status.success() {
                                let p = String::from_utf8(output.stdout).unwrap_or_default();
                                p.trim().to_string()
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                    let (project_id, dataset_id, table_id) = match parts.len() {
                        3 => (parts[0], parts[1], parts[2]),
                        2 => (project.as_str(), parts[0], parts[1]),
                        _ => return,
                    };

                    if let Some(token) = BigQueryClient::get_access_token().await {
                        match BigQueryClient::fetch_table_schema(
                            project_id, dataset_id, table_id, &token,
                        )
                        .await
                        {
                            Ok(table) => {
                                tracing::info!("Successfully pre-fetched schema for {}", t_name);
                                cache.insert(t_name, table);
                            }
                            Err(e) => {
                                tracing::error!("Failed to pre-fetch schema for {}: {}", t_name, e);
                            }
                        }
                    }
                });
            }
        }
    }
}

/// BigQuery API 客户端
pub struct BigQueryClient;

impl BigQueryClient {
    /// 获取访问令牌（通过 shell 调用 gcloud）
    pub async fn get_access_token() -> Option<String> {
        let output = tokio::process::Command::new("gcloud")
            .args(["auth", "print-access-token"])
            .output()
            .await
            .ok()?;

        if output.status.success() {
            let token = String::from_utf8(output.stdout).ok()?;
            return Some(token.trim().to_string());
        }
        None
    }

    /// 获取表 Schema
    pub async fn fetch_table_schema(
        project_id: &str,
        dataset_id: &str,
        table_id: &str,
        token: &str,
    ) -> Result<crate::schema::Table, anyhow::Error> {
        let url = format!(
            "https://bigquery.googleapis.com/bigquery/v2/projects/{}/datasets/{}/tables/{}",
            project_id, dataset_id, table_id
        );

        let client = reqwest::Client::new();
        let res = client.get(&url).bearer_auth(token).send().await?;

        if res.status().is_success() {
            let json: Value = res.json().await?;

            let mut columns = Vec::new();
            if let Some(fields) = json["schema"]["fields"].as_array() {
                for field in fields {
                    let name = field["name"].as_str().unwrap_or("").to_string();
                    let data_type = field["type"].as_str().unwrap_or("").to_string();
                    let mode = field["mode"].as_str().unwrap_or("NULLABLE");
                    let nullable = mode != "REQUIRED";
                    let comment = field["description"].as_str().map(|s| s.to_string());

                    columns.push(crate::schema::Column {
                        name,
                        data_type,
                        nullable,
                        comment,
                        source_location: None,
                    });
                }
            }

            let comment = json["description"].as_str().map(|s| s.to_string());

            Ok(crate::schema::Table {
                name: table_id.to_string(),
                columns,
                comment,
                source_location: None,
            })
        } else {
            anyhow::bail!("Failed to fetch schema: {}", res.status());
        }
    }
}
