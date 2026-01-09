use crate::dialect::Dialect;
use crate::parser::SqlParser;
use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, Hover, Location, Position,
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
                parser.analyze_completion_context(node, sql, position)
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
                // FROM/JOIN 子句：只补全表名，不要关键字
                // 添加表名补全
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        items.push(self.create_table_item(table));
                    }
                }
            }

            crate::parser::CompletionContext::SelectClause => {
                // SELECT 子句：优先补全列名，然后是 SELECT 相关关键字
                // Extract prefix from cursor position
                let prefix = {
                    let lines: Vec<&str> = sql.lines().collect();
                    let line_text = lines.get(position.line as usize).unwrap_or(&"");
                    let text_before =
                        &line_text[..position.character.min(line_text.len() as u32) as usize];

                    // Extract last word/identifier before cursor
                    text_before
                        .split(|c: char| c.is_whitespace() || c == ',' || c == '(')
                        .next_back()
                        .unwrap_or("")
                        .to_lowercase()
                };

                // 先添加列名补全（优先级更高）
                if let Some(schema) = schema {
                    // Check if query has multiple tables (to decide whether to use table prefix)
                    let use_table_prefix = if let Some(tree) = &parse_result.tree {
                        let referenced_tables = parser.extract_referenced_tables(tree, sql);
                        referenced_tables.len() > 1
                    } else {
                        false
                    };

                    for table in &schema.tables {
                        for column in &table.columns {
                            // 单表查询时不使用表名前缀，多表查询时使用前缀避免歧义
                            let table_name = if use_table_prefix {
                                Some(table.name.as_str())
                            } else {
                                None
                            };
                            let mut item = self.create_column_item(column, table_name);

                            // Smart sorting based on prefix match
                            if !prefix.is_empty() && column.name.to_lowercase().starts_with(&prefix)
                            {
                                // Prefix match: highest priority
                                item.sort_text = Some(format!("00{}", column.name));
                            } else {
                                // No match: normal column priority
                                item.sort_text = Some(format!("01{}", column.name));
                            }

                            items.push(item);
                        }
                    }
                }

                // 然后添加 SELECT 相关关键字（优先级较低）
                let select_keywords = vec!["SELECT", "DISTINCT", "AS", "FROM"];
                for keyword in select_keywords {
                    let mut item = self.create_keyword_item(keyword);
                    item.sort_text = Some(format!("1{}", keyword));
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::WhereClause => {
                // WHERE 子句:优先补全列名,然后是操作符,不要关键字
                // 先添加列名 (优先级更高)
                if let Some(schema) = schema {
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables = parser.extract_referenced_tables(tree, sql);
                        let aliases = parser.extract_aliases(tree, sql);

                        // Resolve aliases to real table names
                        let mut real_table_names: Vec<String> = referenced_tables
                            .iter()
                            .map(|t| aliases.get(t).unwrap_or(t).clone())
                            .collect();
                        real_table_names.dedup();

                        // 单表查询时不使用表名前缀，多表查询时使用前缀避免歧义
                        let use_table_prefix = real_table_names.len() > 1;

                        for table in &schema.tables {
                            if real_table_names.contains(&table.name) {
                                for column in &table.columns {
                                    let table_name = if use_table_prefix {
                                        Some(table.name.as_str())
                                    } else {
                                        None
                                    };
                                    let mut item = self.create_column_item(column, table_name);
                                    item.sort_text = Some(format!("0{}", column.name)); // Columns first
                                    items.push(item);
                                }
                            }
                        }
                    }
                }

                // 然后添加操作符 (优先级较低)
                // 只添加关键字形式的运算符，不添加符号运算符
                let operators = vec!["LIKE", "IN", "BETWEEN", "IS NULL", "IS NOT NULL"];
                for op in operators {
                    items.push(CompletionItem {
                        label: op.to_string(),
                        kind: Some(CompletionItemKind::OPERATOR),
                        detail: Some(format!("Operator: {}", op)),
                        documentation: None,
                        deprecated: None,
                        preselect: None,
                        sort_text: Some(format!("1{}", op)), // Operators after columns
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
            }

            crate::parser::CompletionContext::OrderByClause => {
                // ORDER BY：补全列名和排序关键字
                // 添加列名补全 (优先级高)
                if let Some(schema) = schema {
                    // Check if query has multiple tables (to decide whether to use table prefix)
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables = parser.extract_referenced_tables(tree, sql);
                        let aliases = parser.extract_aliases(tree, sql);

                        // Resolve aliases to real table names
                        let mut real_table_names: Vec<String> = referenced_tables
                            .iter()
                            .map(|t| aliases.get(t).unwrap_or(t).clone())
                            .collect();
                        real_table_names.dedup();

                        // 单表查询时不使用表名前缀，多表查询时使用前缀避免歧义
                        let use_table_prefix = real_table_names.len() > 1;

                        for table in &schema.tables {
                            // 只添加查询中引用的表的列
                            if real_table_names.is_empty() || real_table_names.contains(&table.name)
                            {
                                for column in &table.columns {
                                    let table_name = if use_table_prefix {
                                        Some(table.name.as_str())
                                    } else {
                                        None
                                    };
                                    let mut item = self.create_column_item(column, table_name);
                                    item.sort_text = Some(format!("0{}", column.name)); // Columns first
                                    items.push(item);
                                }
                            }
                        }
                    } else {
                        // 如果没有解析树，默认返回所有列，不带前缀
                        for table in &schema.tables {
                            for column in &table.columns {
                                let mut item = self.create_column_item(column, None);
                                item.sort_text = Some(format!("0{}", column.name)); // Columns first
                                items.push(item);
                            }
                        }
                    }
                }

                // 添加 ORDER BY 排序关键字 (优先级低)
                let keywords = vec!["ASC", "DESC"];
                for keyword in keywords {
                    let mut item = self.create_keyword_item(keyword);
                    item.sort_text = Some(format!("1{}", keyword)); // Keywords after columns
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::GroupByClause => {
                // GROUP BY：只补全列名，不需要排序关键字
                // 添加列名补全
                if let Some(schema) = schema {
                    // Check if query has multiple tables (to decide whether to use table prefix)
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables = parser.extract_referenced_tables(tree, sql);
                        let aliases = parser.extract_aliases(tree, sql);

                        // Resolve aliases to real table names
                        let mut real_table_names: Vec<String> = referenced_tables
                            .iter()
                            .map(|t| aliases.get(t).unwrap_or(t).clone())
                            .collect();
                        real_table_names.dedup();

                        // 单表查询时不使用表名前缀，多表查询时使用前缀避免歧义
                        let use_table_prefix = real_table_names.len() > 1;

                        for table in &schema.tables {
                            // 只添加查询中引用的表的列
                            if real_table_names.is_empty() || real_table_names.contains(&table.name)
                            {
                                for column in &table.columns {
                                    let table_name = if use_table_prefix {
                                        Some(table.name.as_str())
                                    } else {
                                        None
                                    };
                                    let mut item = self.create_column_item(column, table_name);
                                    item.sort_text = Some(format!("0{}", column.name)); // Columns first
                                    items.push(item);
                                }
                            }
                        }
                    } else {
                        // 如果没有解析树，默认返回所有列，不带前缀
                        for table in &schema.tables {
                            for column in &table.columns {
                                let mut item = self.create_column_item(column, None);
                                item.sort_text = Some(format!("0{}", column.name)); // Columns first
                                items.push(item);
                            }
                        }
                    }
                }
                // GROUP BY 不添加任何关键字
            }

            crate::parser::CompletionContext::HavingClause => {
                // HAVING 子句：列名(优先) > 聚合函数 > 操作符 > 关键字

                // 1. 添加列名补全 (优先级最高 "0")
                if let Some(schema) = schema {
                    // Check if query has multiple tables (to decide whether to use table prefix)
                    // Same logic as WHERE/ORDER BY
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables = parser.extract_referenced_tables(tree, sql);
                        let aliases = parser.extract_aliases(tree, sql);
                        let mut real_table_names: Vec<String> = referenced_tables
                            .iter()
                            .map(|t| aliases.get(t).unwrap_or(t).clone())
                            .collect();
                        real_table_names.dedup();
                        let use_table_prefix = real_table_names.len() > 1;

                        for table in &schema.tables {
                            if real_table_names.is_empty() || real_table_names.contains(&table.name)
                            {
                                for column in &table.columns {
                                    let table_name = if use_table_prefix {
                                        Some(table.name.as_str())
                                    } else {
                                        None
                                    };
                                    let mut item = self.create_column_item(column, table_name);
                                    item.sort_text = Some(format!("0{}", column.name));
                                    items.push(item);
                                }
                            }
                        }
                    }
                }

                // 2. 添加聚合函数 (优先级中 "1")
                let aggregate_functions = vec!["COUNT", "SUM", "AVG", "MIN", "MAX"];
                for func in aggregate_functions {
                    let mut item = self.create_keyword_item(func);
                    item.kind = Some(CompletionItemKind::FUNCTION);
                    item.sort_text = Some(format!("1{}", func));
                    items.push(item);
                }

                // 3. 添加逻辑关键字和关键字形式的运算符 (优先级 \"2\")
                // 只添加关键字形式的运算符，不添加符号运算符
                let having_keywords =
                    vec!["AND", "OR", "NOT", "IN", "LIKE", "BETWEEN", "IS", "NULL"];
                for keyword in having_keywords {
                    let mut item = self.create_keyword_item(keyword);
                    item.sort_text = Some(format!("2{}", keyword)); // Keywords after aggregate functions
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::TableColumn => {
                // 表名.列名：只补全特定表的列名
                if let Some(tree) = &parse_result.tree {
                    if let Some(node) = parser.get_node_at_position(tree, position) {
                        if let Some(table_name) = parser.get_table_name_for_column(node, sql) {
                            if let Some(schema) = schema {
                                let aliases = parser.extract_aliases(tree, sql);

                                // Resolve alias to real table name
                                let real_table_name =
                                    aliases.get(&table_name).unwrap_or(&table_name);

                                if let Some(table) =
                                    schema.tables.iter().find(|t| t.name == *real_table_name)
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

    async fn hover(&self, sql: &str, position: Position, schema: Option<&Schema>) -> Option<Hover> {
        let mut parser = self.parser.lock().unwrap();
        let parse_result = parser.parse(sql);

        // 获取光标位置的节点
        if let Some(tree) = &parse_result.tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                let node_text = parser.node_text(node, sql);
                let node_kind = node.kind();
                let node_range = parser.node_range(node);

                // 过滤关键字、操作符、分隔符
                if crate::token::Keywords::is_keyword(&node_text)
                    || crate::token::Operators::is_operator(&node_text)
                    || crate::token::Delimiters::is_delimiter(&node_text)
                {
                    return None;
                }

                if let Some(schema) = schema {
                    // 检查是否是表名
                    let is_table = node_kind == "table_name"
                        || node_kind == "table_reference"
                        || node_kind == "table_identifier"
                        || (node_kind == "identifier" && parser.is_in_from_context(node, sql));

                    if is_table {
                        if let Some(table) = schema.tables.iter().find(|t| t.name == node_text) {
                            let mut info = format!("**Table**: `{}`\n\n", table.name);
                            if let Some(comment) = &table.comment {
                                info.push_str(&format!("{}\n\n", comment));
                            }
                            info.push_str(&format!("**Columns** ({})\n", table.columns.len()));
                            for (idx, col) in table.columns.iter().take(10).enumerate() {
                                info.push_str(&format!(
                                    "- `{}`: {} {}\n",
                                    col.name,
                                    col.data_type,
                                    if col.nullable { "" } else { "NOT NULL" }
                                ));
                                if idx == 9 && table.columns.len() > 10 {
                                    info.push_str(&format!(
                                        "- ... and {} more\n",
                                        table.columns.len() - 10
                                    ));
                                    break;
                                }
                            }

                            return Some(Hover {
                                contents: tower_lsp::lsp_types::HoverContents::Markup(
                                    tower_lsp::lsp_types::MarkupContent {
                                        kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                                        value: info,
                                    },
                                ),
                                range: Some(node_range),
                            });
                        }
                    }

                    // 检查是否是列名
                    let is_column = node_kind == "column_name"
                        || node_kind == "column_reference"
                        || node_kind == "column_identifier"
                        || (node_kind == "identifier" && parser.is_in_column_context(node, sql));

                    if is_column {
                        // 尝试获取表名（如果是 table.column 格式）
                        let table_name = parser.get_table_name_for_column(node, sql);

                        for table in &schema.tables {
                            // 如果有明确的表名，只在该表中查找
                            if let Some(ref tname) = table_name {
                                if table.name != *tname {
                                    continue;
                                }
                            }

                            if let Some(column) = table.columns.iter().find(|c| c.name == node_text)
                            {
                                let mut info =
                                    format!("**Column**: `{}.{}`\n\n", table.name, column.name);
                                info.push_str(&format!("**Type**: `{}`\n", column.data_type));
                                info.push_str(&format!(
                                    "**Nullable**: {}\n",
                                    if column.nullable { "Yes" } else { "No" }
                                ));
                                if let Some(comment) = &column.comment {
                                    info.push_str(&format!("\n{}\n", comment));
                                }

                                return Some(Hover {
                                    contents: tower_lsp::lsp_types::HoverContents::Markup(
                                        tower_lsp::lsp_types::MarkupContent {
                                            kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                                            value: info,
                                        },
                                    ),
                                    range: Some(node_range),
                                });
                            }
                        }
                    }

                    // 检查是否是函数名
                    if node_kind == "function_name" || node_kind.contains("function") {
                        if let Some(func) = schema.functions.iter().find(|f| f.name == node_text) {
                            let mut info = format!("**Function**: `{}`\n\n", func.name);
                            if let Some(desc) = &func.description {
                                info.push_str(&format!("{}\n\n", desc));
                            }
                            info.push_str(&format!("**Returns**: `{}`\n", func.return_type));
                            if !func.parameters.is_empty() {
                                info.push_str("\n**Parameters**:\n");
                                for param in &func.parameters {
                                    info.push_str(&format!(
                                        "- `{}`: `{}`{}\n",
                                        param.name,
                                        param.data_type,
                                        if param.optional { " (optional)" } else { "" }
                                    ));
                                }
                            }

                            return Some(Hover {
                                contents: tower_lsp::lsp_types::HoverContents::Markup(
                                    tower_lsp::lsp_types::MarkupContent {
                                        kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                                        value: info,
                                    },
                                ),
                                range: Some(node_range),
                            });
                        }
                    }
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
                        if let Some(table) = schema.tables.iter().find(|t| t.name == node_text) {
                            // 使用表的源位置（如果有）
                            let (uri, line) = if let Some((ref source_uri, source_line)) =
                                table.source_location
                            {
                                (
                                    tower_lsp::lsp_types::Url::parse(source_uri).unwrap_or_else(
                                        |_| {
                                            tower_lsp::lsp_types::Url::parse("file:///schema.sql")
                                                .unwrap()
                                        },
                                    ),
                                    source_line.saturating_sub(1), // 转换为0-indexed
                                )
                            } else if let Some(ref schema_uri) = schema.source_uri {
                                // 回退到 schema 的源文件
                                (
                                    tower_lsp::lsp_types::Url::parse(schema_uri).unwrap_or_else(
                                        |_| {
                                            tower_lsp::lsp_types::Url::parse("file:///schema.sql")
                                                .unwrap()
                                        },
                                    ),
                                    0,
                                )
                            } else {
                                // 默认虚拟位置
                                (
                                    tower_lsp::lsp_types::Url::parse("file:///schema.sql").unwrap(),
                                    0,
                                )
                            };

                            return Some(Location {
                                uri,
                                range: tower_lsp::lsp_types::Range {
                                    start: tower_lsp::lsp_types::Position { line, character: 0 },
                                    end: tower_lsp::lsp_types::Position {
                                        line,
                                        character: 100,
                                    },
                                },
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
        use sqlformat::{FormatOptions, Indent, QueryParams};
        let options = FormatOptions {
            indent: Indent::Spaces(2),
            uppercase: true,
            lines_between_queries: 1,
        };
        sqlformat::format(sql, &QueryParams::None, options)
    }

    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(sql, schema).await
    }
}
