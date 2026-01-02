use crate::dialect::Dialect;
use crate::parser::SqlParser;
use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, Hover, Location, MarkedString, Position,
};

pub struct MysqlDialect {
    parser: std::sync::Mutex<SqlParser>,
}

impl Default for MysqlDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl MysqlDialect {
    pub fn new() -> Self {
        Self {
            parser: std::sync::Mutex::new(SqlParser::new()),
        }
    }

    /// 创建关键字补全项
    fn create_keyword_item(&self, keyword: &str) -> CompletionItem {
        CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(format!("MySQL keyword: {}", keyword)),
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

    /// 创建表补全项
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

    /// 创建列补全项
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

        let detail = if let Some(table) = table_name {
            format!("Column: {}.{} ({})", table, column.name, column.data_type)
        } else {
            format!("Column: {} ({})", column.name, column.data_type)
        };

        CompletionItem {
            label,
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(detail),
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
}

#[async_trait]
impl Dialect for MysqlDialect {
    fn name(&self) -> &str {
        "mysql"
    }

    async fn parse(&self, sql: &str, _schema: Option<&Schema>) -> Vec<Diagnostic> {
        // 使用 Tree-sitter 进行容错 SQL 解析
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
        let mut parser = self.parser.lock().unwrap();
        let parse_result = parser.parse(sql);

        // 分析补全上下文
        let context = if let Some(tree) = &parse_result.tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                parser.analyze_completion_context(node, sql)
            } else {
                crate::parser::CompletionContext::Default
            }
        } else {
            crate::parser::CompletionContext::Default
        };

        let mut items = Vec::new();

        // 根据上下文提供不同的补全
        match context {
            crate::parser::CompletionContext::FromClause
            | crate::parser::CompletionContext::JoinClause => {
                // FROM/JOIN 子句：只补全表名和 JOIN 相关关键字
                let join_keywords = vec!["JOIN", "INNER", "LEFT", "RIGHT", "OUTER", "ON"];
                for keyword in join_keywords {
                    items.push(self.create_keyword_item(keyword));
                }

                // 添加表名补全
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        items.push(self.create_table_item(table));
                    }
                }
            }

            crate::parser::CompletionContext::SelectClause => {
                // SELECT 子句：补全列名和 SELECT 相关关键字
                let select_keywords = vec!["SELECT", "DISTINCT", "AS", "FROM"];
                for keyword in select_keywords {
                    items.push(self.create_keyword_item(keyword));
                }

                // 添加列名补全
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_column_item(column, Some(&table.name)));
                        }
                    }
                }
            }

            crate::parser::CompletionContext::WhereClause => {
                // WHERE 子句：补全列名、操作符、关键字
                let where_keywords = vec![
                    "AND", "OR", "NOT", "IN", "LIKE", "BETWEEN", "IS", "NULL", "TRUE", "FALSE",
                ];
                for keyword in where_keywords {
                    items.push(self.create_keyword_item(keyword));
                }

                // 添加操作符
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

                // 添加列名补全
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_column_item(column, Some(&table.name)));
                        }
                    }
                }
            }

            crate::parser::CompletionContext::OrderByClause
            | crate::parser::CompletionContext::GroupByClause => {
                // ORDER BY / GROUP BY：补全列名和关键字
                let keywords = vec!["ASC", "DESC", "BY"];
                for keyword in keywords {
                    items.push(self.create_keyword_item(keyword));
                }

                // 添加列名补全
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_column_item(column, Some(&table.name)));
                        }
                    }
                }
            }

            crate::parser::CompletionContext::HavingClause => {
                // HAVING 子句：类似 WHERE，补全列名、操作符、关键字
                let having_keywords =
                    vec!["AND", "OR", "NOT", "IN", "LIKE", "BETWEEN", "IS", "NULL"];
                for keyword in having_keywords {
                    items.push(self.create_keyword_item(keyword));
                }

                // 添加聚合函数
                let aggregate_functions = vec!["COUNT", "SUM", "AVG", "MIN", "MAX"];
                for func in aggregate_functions {
                    items.push(self.create_keyword_item(func));
                }

                // 添加列名补全
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_column_item(column, Some(&table.name)));
                        }
                    }
                }
            }

            crate::parser::CompletionContext::TableColumn => {
                // 表名.列名：只补全特定表的列名
                if let Some(tree) = &parse_result.tree {
                    if let Some(node) = parser.get_node_at_position(tree, position) {
                        if let Some(table_name) = parser.get_table_name_for_column(node, sql) {
                            if let Some(schema) = schema {
                                if let Some(table) =
                                    schema.tables.iter().find(|t| t.name == table_name)
                                {
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
                // 默认：返回所有关键字
                let keywords = vec![
                    "SELECT", "FROM", "WHERE", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP",
                    "ALTER", "TABLE", "INDEX", "DATABASE", "SHOW", "DESCRIBE", "EXPLAIN", "JOIN",
                    "INNER", "LEFT", "RIGHT", "OUTER", "ON", "GROUP", "BY", "ORDER", "HAVING",
                    "LIMIT", "OFFSET", "UNION", "ALL", "DISTINCT", "AS", "AND", "OR", "NOT", "IN",
                    "LIKE", "BETWEEN", "IS", "NULL", "TRUE", "FALSE",
                ];

                for keyword in keywords {
                    items.push(self.create_keyword_item(keyword));
                }

                // 如果提供了 schema，添加表和列补全
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        items.push(self.create_table_item(table));
                    }
                }
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
        // 简单的悬停实现
        if let Some(schema) = schema {
            for table in &schema.tables {
                if sql.contains(&table.name) {
                    return Some(Hover {
                        contents: tower_lsp::lsp_types::HoverContents::Scalar(
                            MarkedString::String(format!(
                                "Table: {}\n{}",
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
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Option<Location> {
        let mut parser = self.parser.lock().unwrap();
        let parse_result = parser.parse(sql);

        // 获取光标位置的节点
        if let Some(tree) = &parse_result.tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                let node_text = parser.node_text(node, sql);
                let node_kind = node.kind();

                // 过滤关键字、操作符、分隔符
                if crate::token::Keywords::is_keyword(&node_text)
                    || crate::token::Operators::is_operator(&node_text)
                    || crate::token::Delimiters::is_delimiter(&node_text)
                {
                    return None;
                }

                // 判断是表名还是列名
                let is_table = node_kind == "table_name"
                    || node_kind == "table_reference"
                    || node_kind == "table_identifier"
                    || (node_kind == "identifier" && parser.is_in_from_context(node, sql));

                let is_column = node_kind == "column_name"
                    || node_kind == "column_reference"
                    || node_kind == "column_identifier"
                    || (node_kind == "identifier" && parser.is_in_column_context(node, sql));

                // 如果是表名，查找表定义
                if is_table {
                    if let Some(schema) = schema {
                        if schema.tables.iter().any(|t| t.name == node_text) {
                            // 返回当前文档中表名第一次出现的位置（作为定义位置）
                            // 在实际应用中，这应该是 Schema 文件中的位置
                            return Some(Location {
                                uri: tower_lsp::lsp_types::Url::parse("file:///schema.sql")
                                    .unwrap_or_else(|_| {
                                        tower_lsp::lsp_types::Url::parse("file:///").unwrap()
                                    }),
                                range: parser.node_range(node),
                            });
                        }
                    }
                }

                // 如果是列名，查找列定义
                if is_column {
                    if let Some(schema) = schema {
                        // 检查是否是 table.column 格式
                        let (table_name, column_name) =
                            if let Some(table_name) = parser.get_table_name_for_column(node, sql) {
                                (Some(table_name), node_text.clone())
                            } else {
                                // 查找列所属的表
                                let tables = parser.extract_tables(tree, sql);
                                let table_name = tables.first().cloned();
                                (table_name, node_text.clone())
                            };

                        // 在 Schema 中查找列
                        for table in &schema.tables {
                            if let Some(ref tname) = table_name {
                                if table.name == *tname
                                    && table.columns.iter().any(|c| c.name == column_name)
                                {
                                    // 返回当前文档中列名第一次出现的位置
                                    return Some(Location {
                                        uri: tower_lsp::lsp_types::Url::parse("file:///schema.sql")
                                            .unwrap_or_else(|_| {
                                                tower_lsp::lsp_types::Url::parse("file:///")
                                                    .unwrap()
                                            }),
                                        range: parser.node_range(node),
                                    });
                                }
                            } else if table.columns.iter().any(|c| c.name == column_name) {
                                // 在所有表中查找列
                                return Some(Location {
                                    uri: tower_lsp::lsp_types::Url::parse("file:///schema.sql")
                                        .unwrap_or_else(|_| {
                                            tower_lsp::lsp_types::Url::parse("file:///").unwrap()
                                        }),
                                    range: parser.node_range(node),
                                });
                            }
                        }
                    }
                }
            }
        }

        None
    }

    async fn references(
        &self,
        sql: &str,
        position: Position,
        _schema: Option<&Schema>,
    ) -> Vec<Location> {
        let mut parser = self.parser.lock().unwrap();
        let parse_result = parser.parse(sql);

        let mut locations = Vec::new();

        // 获取光标位置的标识符
        if let Some(tree) = &parse_result.tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                let identifier = parser.node_text(node, sql);
                let node_kind = node.kind();

                // 过滤关键字、操作符、分隔符
                if crate::token::Keywords::is_keyword(&identifier)
                    || crate::token::Operators::is_operator(&identifier)
                    || crate::token::Delimiters::is_delimiter(&identifier)
                {
                    return locations;
                }

                // 判断是表名还是列名
                let is_table = node_kind == "table_name"
                    || node_kind == "table_reference"
                    || node_kind == "table_identifier"
                    || (node_kind == "identifier" && parser.is_in_from_context(node, sql));

                let is_column = node_kind == "column_name"
                    || node_kind == "column_reference"
                    || node_kind == "column_identifier"
                    || (node_kind == "identifier" && parser.is_in_column_context(node, sql));

                if is_table || is_column {
                    // 在当前文档中查找所有引用
                    let tokens = parser.tokenize(tree, sql);
                    let current_uri = tower_lsp::lsp_types::Url::parse("file:///current.sql")
                        .unwrap_or_else(|_| tower_lsp::lsp_types::Url::parse("file:///").unwrap());

                    for token in tokens {
                        // 匹配标识符（忽略大小写）
                        if token.text.eq_ignore_ascii_case(&identifier)
                            && !crate::token::Keywords::is_keyword(&token.text)
                            && !crate::token::Operators::is_operator(&token.text)
                            && !crate::token::Delimiters::is_delimiter(&token.text)
                        {
                            // 检查 token 类型，确保是标识符而不是关键字
                            locations.push(Location {
                                uri: current_uri.clone(),
                                range: tower_lsp::lsp_types::Range {
                                    start: token.position,
                                    end: tower_lsp::lsp_types::Position {
                                        line: token.position.line,
                                        character: token.position.character
                                            + token.text.len() as u32,
                                    },
                                },
                            });
                        }
                    }
                }
            }
        }

        locations
    }

    async fn format(&self, sql: &str) -> String {
        // 简单的格式化：去除多余空格
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(sql, schema).await
    }
}
