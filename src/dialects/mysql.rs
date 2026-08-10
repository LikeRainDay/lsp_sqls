use crate::dialect::Dialect;
use crate::dialects::common;
use crate::parser::SqlParser;
use crate::placeholder::SqlPlaceholderDialect;
use crate::schema::{Function, Schema};
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
            parser: std::sync::Mutex::new(SqlParser::new_with_placeholder_dialect(
                SqlPlaceholderDialect::Mysql,
            )),
        }
    }

    /// 创建关键字补全项
    fn create_keyword_item(&self, keyword: &str) -> CompletionItem {
        common::create_keyword_item("MySQL", keyword)
    }

    /// 创建列补全项
    fn create_column_item(
        &self,
        column: &crate::schema::Column,
        table_name: Option<&str>,
    ) -> CompletionItem {
        common::create_column_item(column, table_name)
    }

    fn add_schema_functions(
        &self,
        items: &mut Vec<CompletionItem>,
        schema: &Schema,
        prefix: &str,
        sort_prefix: &str,
        qualify_with_database: bool,
    ) {
        common::add_schema_functions(items, schema, prefix, sort_prefix, qualify_with_database);
    }

    fn table_matches(schema: &Schema, table: &crate::schema::Table, reference: &str) -> bool {
        common::table_matches(schema, table, reference)
    }

    fn find_table_by_reference<'a>(
        schema: &'a Schema,
        reference: &str,
    ) -> Option<&'a crate::schema::Table> {
        common::find_table_by_reference(schema, reference)
    }

    fn referenced_table_names_at_position(
        parser: &SqlParser,
        tree: &tree_sitter::Tree,
        sql: &str,
        position: Position,
    ) -> Vec<String> {
        common::referenced_table_names_at_position(parser, tree, sql, position)
    }

    fn find_function_by_reference<'a>(schema: &'a Schema, reference: &str) -> Option<&'a Function> {
        common::find_function_by_reference(schema, reference)
    }

    fn is_function_reference(node: tree_sitter::Node, sql: &str) -> bool {
        common::is_function_reference(node, sql)
    }

    fn cursor_prefix(sql: &str, position: Position) -> String {
        common::cursor_prefix(sql, position)
    }

    fn relation_reference_prefix(sql: &str, position: Position) -> String {
        common::cursor_prefix_excluding_keywords(
            sql,
            position,
            &[
                "from", "join", "inner", "left", "right", "full", "outer", "cross", "insert",
                "into", "update", "delete", "truncate", "alter", "drop", "table", "view", "on",
            ],
        )
    }

    fn cursor_has_identifier_qualifier(sql: &str, position: Position) -> bool {
        common::cursor_has_identifier_qualifier(sql, position)
    }

    fn add_schema_columns(
        &self,
        items: &mut Vec<CompletionItem>,
        schema: &Schema,
        referenced_tables: &[String],
        use_table_prefix: bool,
        prefix: &str,
        sort_prefix: &str,
    ) {
        common::add_schema_columns(
            items,
            schema,
            referenced_tables,
            use_table_prefix,
            prefix,
            sort_prefix,
        );
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
        let position = SqlParser::lsp_position_to_byte_position(sql, position);
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
        let is_join_clause = context == crate::parser::CompletionContext::JoinClause;

        let mut items = Vec::new();

        // 根据上下文提供不同的补全
        match context {
            crate::parser::CompletionContext::FromClause
            | crate::parser::CompletionContext::JoinClause => {
                let prefix = Self::relation_reference_prefix(sql, position);
                if let Some(schema) = schema {
                    let qualify_with_database =
                        Self::cursor_has_identifier_qualifier(sql, position);
                    common::add_schema_tables(&mut items, schema, &prefix, qualify_with_database);
                    if is_join_clause {
                        if let Some(tree) = &parse_result.tree {
                            let referenced_tables =
                                parser.extract_referenced_tables_at_position(tree, sql, position);
                            let aliases = parser.extract_aliases_at_position(tree, sql, position);
                            common::add_foreign_key_join_snippets(
                                &mut items,
                                schema,
                                &referenced_tables,
                                &aliases,
                                &prefix,
                                qualify_with_database,
                            );
                        }
                    }
                }
            }

            crate::parser::CompletionContext::FromContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in [
                    "AS",
                    "JOIN",
                    "INNER JOIN",
                    "LEFT JOIN",
                    "RIGHT JOIN",
                    "CROSS JOIN",
                    "WHERE",
                    "GROUP BY",
                    "ORDER BY",
                    "LIMIT",
                    "OFFSET",
                ] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::JoinConditionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["AS", "ON", "USING"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::SelectClause => {
                let prefix = common::cursor_prefix_excluding_keywords(
                    sql,
                    position,
                    &["select", "distinct"],
                );
                if let Some(schema) = schema {
                    let referenced_tables = parse_result
                        .tree
                        .as_ref()
                        .map(|tree| {
                            Self::referenced_table_names_at_position(&parser, tree, sql, position)
                        })
                        .unwrap_or_default();
                    let use_table_prefix = referenced_tables.len() != 1;

                    self.add_schema_columns(
                        &mut items,
                        schema,
                        &referenced_tables,
                        use_table_prefix,
                        &prefix,
                        "0",
                    );
                    self.add_schema_functions(
                        &mut items,
                        schema,
                        &prefix,
                        "1",
                        Self::cursor_has_identifier_qualifier(sql, position),
                    );
                }

                // 添加聚合函数和常用函数（优先级中等）
                let functions = vec![
                    ("COUNT", "Aggregate function: count rows"),
                    ("SUM", "Aggregate function: sum values"),
                    ("AVG", "Aggregate function: average"),
                    ("MIN", "Aggregate function: minimum"),
                    ("MAX", "Aggregate function: maximum"),
                    ("CONCAT", "String function: concatenate"),
                    ("UPPER", "String function: uppercase"),
                    ("LOWER", "String function: lowercase"),
                    ("NOW", "Date function: current timestamp"),
                    ("DATE", "Date function: extract date"),
                ];

                for (func, desc) in functions {
                    // Apply prefix filter to functions
                    if !prefix.is_empty() && !func.to_lowercase().starts_with(&prefix) {
                        continue;
                    }

                    items.push(CompletionItem {
                        label: func.to_string(),
                        kind: Some(CompletionItemKind::FUNCTION),
                        detail: Some(desc.to_string()),
                        documentation: None,
                        deprecated: None,
                        preselect: None,
                        sort_text: Some(common::completion_sort_text("1", func)), // Functions after columns
                        filter_text: None,
                        insert_text: Some(format!("{}()", func)),
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

                // 然后添加 SELECT 相关关键字（优先级最低）
                let select_keywords = vec!["DISTINCT", "AS", "FROM"];
                for keyword in select_keywords {
                    // Apply prefix filter to keywords
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }

                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "2", keyword); // Keywords last
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::SelectContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in common::select_continuation_keywords(sql, position) {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::WhereClause => {
                let prefix = common::cursor_prefix_excluding_keywords(
                    sql,
                    position,
                    &["where", "and", "or", "not"],
                );
                let predicate_operator_expected =
                    common::predicate_operator_expected(sql, position);
                let latest_predicate_clause = common::latest_predicate_clause(sql, position);
                if !predicate_operator_expected {
                    if let Some(schema) = schema {
                        if let Some(tree) = &parse_result.tree {
                            let referenced_tables = Self::referenced_table_names_at_position(
                                &parser, tree, sql, position,
                            );
                            let use_table_prefix =
                                !matches!(latest_predicate_clause, Some("SET" | "UPDATE"))
                                    && referenced_tables.len() > 1;
                            common::add_column_value_items(
                                &mut items,
                                schema,
                                &referenced_tables,
                                sql,
                                position,
                                &prefix,
                            );
                            self.add_schema_columns(
                                &mut items,
                                schema,
                                &referenced_tables,
                                use_table_prefix,
                                &prefix,
                                "0",
                            );
                        }
                        self.add_schema_functions(
                            &mut items,
                            schema,
                            &prefix,
                            "1",
                            Self::cursor_has_identifier_qualifier(sql, position),
                        );
                    }
                }

                if predicate_operator_expected {
                    let operators = vec![
                        "=",
                        "<>",
                        "!=",
                        ">",
                        "<",
                        ">=",
                        "<=",
                        "LIKE",
                        "IN",
                        "BETWEEN",
                        "IS NULL",
                        "IS NOT NULL",
                    ];
                    common::add_operator_items(&mut items, &operators, &prefix, "1");
                }
            }

            crate::parser::CompletionContext::OrderByClause => {
                let prefix =
                    common::cursor_prefix_excluding_keywords(sql, position, &["order", "by"]);
                // ORDER BY：补全列名和排序关键字
                // 添加列名补全 (优先级高)
                if let Some(schema) = schema {
                    // Check if query has multiple tables (to decide whether to use table prefix)
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables =
                            Self::referenced_table_names_at_position(&parser, tree, sql, position);
                        let use_table_prefix = referenced_tables.len() != 1;
                        self.add_schema_columns(
                            &mut items,
                            schema,
                            &referenced_tables,
                            use_table_prefix,
                            &prefix,
                            "0",
                        );
                    } else {
                        // 如果没有解析树，默认返回所有列，不带前缀
                        self.add_schema_columns(&mut items, schema, &[], false, &prefix, "0");
                    }
                }

                // 添加 ORDER BY 排序关键字 (优先级低)
                let keywords = vec!["ASC", "DESC"];
                for keyword in keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "1", keyword); // Keywords after columns
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::OrderDirectionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in common::order_direction_keywords(
                    sql,
                    position,
                    false,
                    &[",", "LIMIT", "OFFSET"],
                ) {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(
                        &mut item,
                        common::order_direction_sort_prefix(keyword),
                        keyword,
                    );
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::GroupByClause => {
                let prefix =
                    common::cursor_prefix_excluding_keywords(sql, position, &["group", "by"]);
                // GROUP BY：只补全列名，不需要排序关键字
                // 添加列名补全
                if let Some(schema) = schema {
                    // Check if query has multiple tables (to decide whether to use table prefix)
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables =
                            Self::referenced_table_names_at_position(&parser, tree, sql, position);
                        let use_table_prefix = referenced_tables.len() != 1;
                        self.add_schema_columns(
                            &mut items,
                            schema,
                            &referenced_tables,
                            use_table_prefix,
                            &prefix,
                            "0",
                        );
                    } else {
                        // 如果没有解析树，默认返回所有列，不带前缀
                        self.add_schema_columns(&mut items, schema, &[], false, &prefix, "0");
                    }
                }
                // GROUP BY 不添加任何关键字
            }

            crate::parser::CompletionContext::GroupByContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in [",", "HAVING", "ORDER BY", "LIMIT", "OFFSET", "WITH ROLLUP"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(
                        &mut item,
                        common::group_by_continuation_sort_prefix(keyword),
                        keyword,
                    );
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::HavingClause => {
                let prefix = common::cursor_prefix_excluding_keywords(
                    sql,
                    position,
                    &["having", "and", "or", "not"],
                );
                let predicate_operator_expected =
                    common::predicate_operator_expected(sql, position);

                if !predicate_operator_expected {
                    if let Some(schema) = schema {
                        if let Some(tree) = &parse_result.tree {
                            let referenced_tables = Self::referenced_table_names_at_position(
                                &parser, tree, sql, position,
                            );
                            let use_table_prefix = referenced_tables.len() != 1;
                            self.add_schema_columns(
                                &mut items,
                                schema,
                                &referenced_tables,
                                use_table_prefix,
                                &prefix,
                                "0",
                            );
                        }
                        self.add_schema_functions(
                            &mut items,
                            schema,
                            &prefix,
                            "1",
                            Self::cursor_has_identifier_qualifier(sql, position),
                        );
                    }

                    let aggregate_functions = vec!["COUNT", "SUM", "AVG", "MIN", "MAX"];
                    for func in aggregate_functions {
                        if !prefix.is_empty() && !func.to_lowercase().starts_with(&prefix) {
                            continue;
                        }
                        let mut item = self.create_keyword_item(func);
                        item.kind = Some(CompletionItemKind::FUNCTION);
                        common::set_completion_sort_text(&mut item, "1", func);
                        items.push(item);
                    }
                }

                let having_keywords = if predicate_operator_expected {
                    vec![
                        "AND", "OR", "NOT", "IN", "LIKE", "BETWEEN", "IS", "NULL", "TRUE", "FALSE",
                    ]
                } else {
                    vec!["NOT", "TRUE", "FALSE"]
                };
                for keyword in having_keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "2", keyword);
                    items.push(item);
                }

                if predicate_operator_expected {
                    let operators = vec![
                        "=",
                        "<>",
                        "!=",
                        ">",
                        "<",
                        ">=",
                        "<=",
                        "LIKE",
                        "IN",
                        "BETWEEN",
                        "IS NULL",
                        "IS NOT NULL",
                    ];
                    common::add_operator_items(&mut items, &operators, &prefix, "1");
                }
            }

            crate::parser::CompletionContext::UsingClause => {
                let prefix = common::cursor_prefix_excluding_keywords(sql, position, &["using"]);
                if let Some(schema) = schema {
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables =
                            Self::referenced_table_names_at_position(&parser, tree, sql, position);
                        common::add_schema_using_columns(
                            &mut items,
                            schema,
                            &referenced_tables,
                            &prefix,
                            "0",
                        );
                    }
                }
            }

            crate::parser::CompletionContext::ReferenceColumnClause => {
                let prefix = common::cursor_prefix(sql, position);
                if let Some(schema) = schema {
                    if let Some(table_reference) =
                        SqlParser::reference_table_at_position(sql, position)
                    {
                        common::add_reference_table_columns(
                            &mut items,
                            schema,
                            &table_reference,
                            &prefix,
                            "0",
                        );
                    }
                }
            }

            crate::parser::CompletionContext::ReferenceActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in [
                    "(",
                    "MATCH FULL",
                    "MATCH PARTIAL",
                    "MATCH SIMPLE",
                    "ON DELETE",
                    "ON UPDATE",
                ] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::ReferenceRuleClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["CASCADE", "RESTRICT", "NO ACTION", "SET NULL"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::ColumnTargetClause => {
                let prefix = common::cursor_prefix_excluding_keywords(sql, position, &["column"]);
                if let (Some(schema), Some(tree)) = (schema, &parse_result.tree) {
                    let referenced_tables =
                        Self::referenced_table_names_at_position(&parser, tree, sql, position);
                    common::add_schema_columns(
                        &mut items,
                        schema,
                        &referenced_tables,
                        false,
                        &prefix,
                        "0",
                    );
                }
            }

            crate::parser::CompletionContext::ConstraintTargetClause => {
                let prefix =
                    common::cursor_prefix_excluding_keywords(sql, position, &["constraint"]);
                if let (Some(schema), Some(tree)) = (schema, &parse_result.tree) {
                    let referenced_tables =
                        Self::referenced_table_names_at_position(&parser, tree, sql, position);
                    common::add_schema_constraints(
                        &mut items,
                        schema,
                        &referenced_tables,
                        &prefix,
                        "0",
                    );
                }
            }

            crate::parser::CompletionContext::AlterTableActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in [
                    "ADD COLUMN",
                    "DROP COLUMN",
                    "MODIFY COLUMN",
                    "CHANGE COLUMN",
                    "RENAME COLUMN",
                    "ADD CONSTRAINT",
                    "DROP CONSTRAINT",
                    "DROP FOREIGN KEY",
                    "ADD INDEX",
                    "DROP INDEX",
                    "RENAME INDEX",
                    "RENAME TO",
                ] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::InsertActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["(", "VALUES", "VALUE", "SELECT", "SET"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::InsertValueClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in [
                    "DEFAULT",
                    "NULL",
                    "TRUE",
                    "FALSE",
                    "CURRENT_DATE",
                    "CURRENT_TIMESTAMP",
                    "NOW()",
                ] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::InsertContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["ON DUPLICATE KEY UPDATE"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::InsertConflictTargetClause
            | crate::parser::CompletionContext::InsertConflictConstraintClause
            | crate::parser::CompletionContext::InsertConflictActionClause => {}

            crate::parser::CompletionContext::ExpressionValueClause => {
                let prefix = common::cursor_prefix(sql, position);
                if let Some(schema) = schema {
                    let referenced_tables = parse_result
                        .tree
                        .as_ref()
                        .map(|tree| {
                            Self::referenced_table_names_at_position(&parser, tree, sql, position)
                        })
                        .unwrap_or_default();
                    common::add_column_value_items(
                        &mut items,
                        schema,
                        &referenced_tables,
                        sql,
                        position,
                        &prefix,
                    );
                }
                let mut keywords = vec![
                    "NULL",
                    "TRUE",
                    "FALSE",
                    "CURRENT_DATE",
                    "CURRENT_TIMESTAMP",
                    "NOW()",
                ];
                if common::expression_value_allows_default(sql, position) {
                    keywords.insert(0, "DEFAULT");
                }
                for keyword in keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::CaseResultClause => {
                let prefix =
                    common::cursor_prefix_excluding_keywords(sql, position, &["then", "else"]);
                if let Some(schema) = schema {
                    let referenced_tables = parse_result
                        .tree
                        .as_ref()
                        .map(|tree| {
                            Self::referenced_table_names_at_position(&parser, tree, sql, position)
                        })
                        .unwrap_or_default();
                    let use_table_prefix = referenced_tables.len() != 1;

                    self.add_schema_columns(
                        &mut items,
                        schema,
                        &referenced_tables,
                        use_table_prefix,
                        &prefix,
                        "0",
                    );
                    self.add_schema_functions(
                        &mut items,
                        schema,
                        &prefix,
                        "1",
                        Self::cursor_has_identifier_qualifier(sql, position),
                    );
                }

                for keyword in [
                    "NULL",
                    "TRUE",
                    "FALSE",
                    "CURRENT_DATE",
                    "CURRENT_TIMESTAMP",
                    "NOW()",
                ] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "1", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::CaseWhenValueContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["THEN"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::CaseContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in common::case_continuation_keywords(sql, position) {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::PredicateContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in common::predicate_continuation_keywords(sql, position, false) {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::UpdateActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["SET"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::DeleteActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["WHERE", "ORDER BY", "LIMIT"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::IndexTargetClause => {
                let prefix = common::cursor_prefix_excluding_keywords(sql, position, &["index"]);
                if let Some(schema) = schema {
                    let referenced_tables = parse_result
                        .tree
                        .as_ref()
                        .map(|tree| {
                            Self::referenced_table_names_at_position(&parser, tree, sql, position)
                        })
                        .unwrap_or_default();
                    common::add_schema_indexes(
                        &mut items,
                        schema,
                        &referenced_tables,
                        &prefix,
                        "0",
                        Self::cursor_has_identifier_qualifier(sql, position),
                    );
                }
            }

            crate::parser::CompletionContext::DataTypeClause => {
                let prefix = common::cursor_prefix(sql, position);
                for data_type in [
                    "VARCHAR",
                    "TEXT",
                    "INT",
                    "BIGINT",
                    "BOOLEAN",
                    "DATETIME",
                    "TIMESTAMP",
                    "DATE",
                    "DECIMAL",
                    "JSON",
                    "BLOB",
                ] {
                    if !prefix.is_empty() && !data_type.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(data_type);
                    common::set_completion_sort_text(&mut item, "0", data_type);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::TableColumn => {
                // 表名.列名：只补全特定表的列名
                if let Some(tree) = &parse_result.tree {
                    if let Some(table_name) =
                        common::table_column_reference_at_position(&parser, tree, sql, position)
                    {
                        if let Some(schema) = schema {
                            let aliases = parser.extract_aliases_at_position(tree, sql, position);

                            // Resolve alias to real table name
                            let real_table_name = aliases.get(&table_name).unwrap_or(&table_name);

                            if let Some(table) =
                                Self::find_table_by_reference(schema, real_table_name)
                            {
                                for column in &table.columns {
                                    items.push(self.create_column_item(column, None));
                                }
                            }
                        }
                    }
                }
            }

            crate::parser::CompletionContext::Default => {
                let prefix = Self::cursor_prefix(sql, position);

                let keywords = vec![
                    "SELECT", "FROM", "WHERE", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP",
                    "ALTER", "TABLE", "INDEX", "DATABASE", "SHOW", "DESCRIBE", "EXPLAIN", "JOIN",
                    "INNER", "LEFT", "RIGHT", "OUTER", "ON", "GROUP", "BY", "ORDER", "HAVING",
                    "LIMIT", "OFFSET", "UNION", "ALL", "DISTINCT", "AS", "AND", "OR", "NOT", "IN",
                    "LIKE", "BETWEEN", "IS", "NULL", "TRUE", "FALSE",
                ];

                for keyword in keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(keyword));
                }

                if let Some(schema) = schema {
                    common::add_schema_tables(
                        &mut items,
                        schema,
                        &prefix,
                        Self::cursor_has_identifier_qualifier(sql, position),
                    );
                    self.add_schema_functions(
                        &mut items,
                        schema,
                        &prefix,
                        "1",
                        Self::cursor_has_identifier_qualifier(sql, position),
                    );
                }
            }
        }

        if let (Some(schema), Some(_)) = (schema, parse_result.tree.as_ref()) {
            let aliases = SqlParser::relation_aliases_at_position(sql, position);
            common::apply_column_aliases(&mut items, schema, &aliases);
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
                        let table_ref = SqlParser::normalize_identifier(&node_text);
                        if let Some(table) = Self::find_table_by_reference(schema, &table_ref) {
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
                                if !Self::table_matches(schema, table, tname) {
                                    continue;
                                }
                            }

                            let column_name = SqlParser::identifier_last_part(&node_text);
                            if let Some(column) =
                                table.columns.iter().find(|c| c.name == column_name)
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
                    if Self::is_function_reference(node, sql) {
                        if let Some(func) = Self::find_function_by_reference(schema, &node_text) {
                            return Some(Hover {
                                contents: tower_lsp::lsp_types::HoverContents::Markup(
                                    tower_lsp::lsp_types::MarkupContent {
                                        kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                                        value: func.markdown_documentation(),
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
                        let table_ref = SqlParser::normalize_identifier(&node_text);
                        if let Some(table) = Self::find_table_by_reference(schema, &table_ref) {
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
                                (
                                    Some(table_name),
                                    SqlParser::identifier_last_part(&node_text),
                                )
                            } else {
                                // 查找列所属的表
                                let tables = parser.extract_tables(tree, sql);
                                let table_name = tables.first().cloned();
                                (table_name, SqlParser::identifier_last_part(&node_text))
                            };

                        // 在 Schema 中查找列
                        for table in &schema.tables {
                            if let Some(ref tname) = table_name {
                                if Self::table_matches(schema, table, tname) {
                                    if let Some(column) = table
                                        .columns
                                        .iter()
                                        .find(|column| column.name == column_name)
                                    {
                                        return common::metadata_location(
                                            column
                                                .source_location
                                                .as_ref()
                                                .or(table.source_location.as_ref()),
                                            schema.source_uri.as_ref(),
                                            "file:///schema.sql",
                                        );
                                    }
                                }
                            } else if let Some(column) = table
                                .columns
                                .iter()
                                .find(|column| column.name == column_name)
                            {
                                return common::metadata_location(
                                    column
                                        .source_location
                                        .as_ref()
                                        .or(table.source_location.as_ref()),
                                    schema.source_uri.as_ref(),
                                    "file:///schema.sql",
                                );
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
                                            + token.text.encode_utf16().count() as u32,
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
        common::format_sql_pretty(sql)
    }

    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(sql, schema).await
    }
}
