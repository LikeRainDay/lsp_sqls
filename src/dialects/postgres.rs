use crate::dialect::Dialect;
use crate::dialects::common;
use crate::parser::SqlParser;
use crate::schema::{Function, Schema};
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, Hover, Location, MarkedString, Position,
};

pub struct PostgresDialect {
    parser: std::sync::Mutex<SqlParser>,
}

impl Default for PostgresDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl PostgresDialect {
    pub fn new() -> Self {
        Self {
            parser: std::sync::Mutex::new(SqlParser::new()),
        }
    }

    /// 创建关键字补全项
    fn create_keyword_item(&self, keyword: &str) -> CompletionItem {
        common::create_keyword_item("PostgreSQL", keyword)
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

    fn find_function_by_reference<'a>(schema: &'a Schema, reference: &str) -> Option<&'a Function> {
        common::find_function_by_reference(schema, reference)
    }

    fn is_function_reference(node: tree_sitter::Node, sql: &str) -> bool {
        common::is_function_reference(node, sql)
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

    fn referenced_table_names_at_position(
        parser: &SqlParser,
        tree: &tree_sitter::Tree,
        sql: &str,
        position: Position,
    ) -> Vec<String> {
        common::referenced_table_names_at_position(parser, tree, sql, position)
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
impl Dialect for PostgresDialect {
    fn name(&self) -> &str {
        "postgres"
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

        let mut items = Vec::new();

        // 根据上下文提供不同的补全
        match context {
            crate::parser::CompletionContext::FromClause
            | crate::parser::CompletionContext::JoinClause => {
                let prefix = Self::relation_reference_prefix(sql, position);

                if let Some(schema) = schema {
                    common::add_schema_tables(&mut items, schema, &prefix, true);
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
                    "FULL JOIN",
                    "CROSS JOIN",
                    "WHERE",
                    "GROUP BY",
                    "ORDER BY",
                    "LIMIT",
                    "OFFSET",
                    "FETCH",
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
                    let use_table_prefix = referenced_tables.len() > 1;

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

                let select_keywords = vec!["DISTINCT", "AS", "FROM"];
                for keyword in select_keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "2", keyword);
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

                let where_keywords = if predicate_operator_expected {
                    vec![
                        "AND", "OR", "NOT", "IN", "LIKE", "ILIKE", "SIMILAR", "BETWEEN", "IS",
                        "NULL", "TRUE", "FALSE",
                    ]
                } else {
                    vec!["NOT", "TRUE", "FALSE"]
                };
                for keyword in where_keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "1", keyword);
                    items.push(item);
                }

                let operators = vec!["=", "<>", "!=", ">", "<", ">=", "<="];
                if predicate_operator_expected {
                    for op in operators {
                        if !prefix.is_empty() && !op.to_lowercase().starts_with(&prefix) {
                            continue;
                        }
                        items.push(CompletionItem {
                            label: op.to_string(),
                            kind: Some(CompletionItemKind::OPERATOR),
                            detail: Some(format!("Operator: {}", op)),
                            documentation: None,
                            deprecated: None,
                            preselect: None,
                            sort_text: Some(common::completion_sort_text("1", op)),
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
            }

            crate::parser::CompletionContext::OrderByClause => {
                let prefix =
                    common::cursor_prefix_excluding_keywords(sql, position, &["order", "by"]);
                if let Some(schema) = schema {
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables =
                            Self::referenced_table_names_at_position(&parser, tree, sql, position);
                        let use_table_prefix = referenced_tables.len() > 1;
                        self.add_schema_columns(
                            &mut items,
                            schema,
                            &referenced_tables,
                            use_table_prefix,
                            &prefix,
                            "0",
                        );
                    }
                }

                let keywords = vec!["ASC", "DESC", "BY"];
                for keyword in keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "1", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::GroupByClause => {
                let prefix =
                    common::cursor_prefix_excluding_keywords(sql, position, &["group", "by"]);
                if let Some(schema) = schema {
                    if let Some(tree) = &parse_result.tree {
                        let referenced_tables =
                            Self::referenced_table_names_at_position(&parser, tree, sql, position);
                        let use_table_prefix = referenced_tables.len() > 1;
                        self.add_schema_columns(
                            &mut items,
                            schema,
                            &referenced_tables,
                            use_table_prefix,
                            &prefix,
                            "0",
                        );
                    }
                }
            }

            crate::parser::CompletionContext::GroupByContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in [",", "HAVING", "ORDER BY", "LIMIT", "OFFSET", "FETCH"] {
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

            crate::parser::CompletionContext::OrderDirectionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in common::order_direction_keywords(
                    sql,
                    position,
                    true,
                    &[",", "LIMIT", "OFFSET", "FETCH"],
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
                            let use_table_prefix = referenced_tables.len() > 1;
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
                        "AND", "OR", "NOT", "IN", "LIKE", "ILIKE", "SIMILAR", "BETWEEN", "IS",
                        "NULL", "TRUE", "FALSE",
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
                    let operators = vec!["=", "<>", "!=", ">", "<", ">=", "<="];
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
                    "MATCH SIMPLE",
                    "ON DELETE",
                    "ON UPDATE",
                    "DEFERRABLE",
                    "NOT DEFERRABLE",
                    "INITIALLY DEFERRED",
                    "INITIALLY IMMEDIATE",
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
                for keyword in [
                    "CASCADE",
                    "RESTRICT",
                    "NO ACTION",
                    "SET NULL",
                    "SET DEFAULT",
                ] {
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
                    "ALTER COLUMN",
                    "RENAME COLUMN",
                    "ADD CONSTRAINT",
                    "DROP CONSTRAINT",
                    "RENAME CONSTRAINT",
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
                for keyword in [
                    "(",
                    "VALUES",
                    "SELECT",
                    "DEFAULT VALUES",
                    "OVERRIDING SYSTEM VALUE",
                    "OVERRIDING USER VALUE",
                ] {
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
                for keyword in ["ON CONFLICT", "RETURNING"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::InsertConflictTargetClause => {
                let prefix =
                    common::cursor_prefix_excluding_keywords(sql, position, &["on", "conflict"]);
                if let Some(schema) = schema {
                    let referenced_tables = parse_result
                        .tree
                        .as_ref()
                        .map(|tree| {
                            Self::referenced_table_names_at_position(&parser, tree, sql, position)
                        })
                        .unwrap_or_default();
                    self.add_schema_columns(
                        &mut items,
                        schema,
                        &referenced_tables,
                        false,
                        &prefix,
                        "0",
                    );
                }
            }

            crate::parser::CompletionContext::InsertConflictConstraintClause => {
                let prefix =
                    common::cursor_prefix_excluding_keywords(sql, position, &["constraint"]);
                if let Some(schema) = schema {
                    let referenced_tables = parse_result
                        .tree
                        .as_ref()
                        .map(|tree| {
                            Self::referenced_table_names_at_position(&parser, tree, sql, position)
                        })
                        .unwrap_or_default();
                    common::add_schema_conflict_constraints(
                        &mut items,
                        schema,
                        &referenced_tables,
                        &prefix,
                        "0",
                    );
                }
            }

            crate::parser::CompletionContext::InsertConflictActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                let text_before = common::text_before_position(sql, position).to_ascii_uppercase();
                let searchable = SqlParser::mask_sql_noise(&text_before);
                let in_do_tail = searchable
                    .rfind("ON CONFLICT")
                    .and_then(|conflict_position| searchable.get(conflict_position..))
                    .is_some_and(|conflict_segment| {
                        conflict_segment.trim_end().ends_with(" DO")
                            || conflict_segment.contains(" DO ")
                    });
                let keywords: &[&str] = if in_do_tail {
                    &["NOTHING", "UPDATE SET"]
                } else {
                    &["(", "ON CONSTRAINT", "DO NOTHING", "DO UPDATE SET"]
                };
                for keyword in keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    let mut item = self.create_keyword_item(keyword);
                    common::set_completion_sort_text(&mut item, "0", keyword);
                    items.push(item);
                }
            }

            crate::parser::CompletionContext::ExpressionValueClause => {
                let prefix = common::cursor_prefix(sql, position);
                let mut keywords =
                    vec!["NULL", "TRUE", "FALSE", "CURRENT_DATE", "CURRENT_TIMESTAMP"];
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
                    let use_table_prefix = referenced_tables.len() > 1;

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

                for keyword in ["NULL", "TRUE", "FALSE", "CURRENT_DATE", "CURRENT_TIMESTAMP"] {
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
                for keyword in common::predicate_continuation_keywords(sql, position, true) {
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
                for keyword in ["WHERE", "USING", "RETURNING"] {
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
                        true,
                    );
                }
            }

            crate::parser::CompletionContext::DataTypeClause => {
                let prefix = common::cursor_prefix(sql, position);
                for data_type in [
                    "VARCHAR",
                    "TEXT",
                    "INTEGER",
                    "BIGINT",
                    "BOOLEAN",
                    "TIMESTAMP",
                    "TIMESTAMPTZ",
                    "DATE",
                    "NUMERIC",
                    "UUID",
                    "JSONB",
                    "BYTEA",
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
                if let Some(tree) = &parse_result.tree {
                    if let Some(table_name) =
                        common::table_column_reference_at_position(&parser, tree, sql, position)
                    {
                        if let Some(schema) = schema {
                            let aliases = parser.extract_aliases_at_position(tree, sql, position);
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
                let keywords = vec![
                    "SELECT",
                    "FROM",
                    "WHERE",
                    "INSERT",
                    "UPDATE",
                    "DELETE",
                    "CREATE",
                    "DROP",
                    "ALTER",
                    "TABLE",
                    "INDEX",
                    "DATABASE",
                    "SCHEMA",
                    "VIEW",
                    "TRIGGER",
                    "FUNCTION",
                    "PROCEDURE",
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
                    "AS",
                    "AND",
                    "OR",
                    "NOT",
                    "IN",
                    "LIKE",
                    "ILIKE",
                    "SIMILAR",
                    "BETWEEN",
                    "IS",
                    "NULL",
                    "TRUE",
                    "FALSE",
                    "CAST",
                    "::",
                    "ARRAY",
                    "JSONB",
                ];

                for keyword in keywords {
                    items.push(self.create_keyword_item(keyword));
                }

                if let Some(schema) = schema {
                    common::add_schema_tables(&mut items, schema, "", true);
                    self.add_schema_functions(
                        &mut items,
                        schema,
                        "",
                        "1",
                        Self::cursor_has_identifier_qualifier(sql, position),
                    );
                }
            }
        }

        items
    }

    async fn hover(&self, sql: &str, position: Position, schema: Option<&Schema>) -> Option<Hover> {
        let mut parser = self.parser.lock().unwrap();
        let parse_result = parser.parse(sql);

        if let (Some(schema), Some(tree)) = (schema, &parse_result.tree) {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                let node_text = parser.node_text(node, sql);
                let node_kind = node.kind();

                if crate::token::Keywords::is_keyword(&node_text)
                    || crate::token::Operators::is_operator(&node_text)
                    || crate::token::Delimiters::is_delimiter(&node_text)
                {
                    return None;
                }

                let is_table = node_kind == "table_name"
                    || node_kind == "table_reference"
                    || node_kind == "table_identifier"
                    || (node_kind == "identifier" && parser.is_in_from_context(node, sql));

                if is_table {
                    let table_ref = SqlParser::normalize_identifier(&node_text);
                    if let Some(table) = Self::find_table_by_reference(schema, &table_ref) {
                        return Some(Hover {
                            contents: tower_lsp::lsp_types::HoverContents::Scalar(
                                MarkedString::String(format!(
                                    "PostgreSQL Table: {}.{}\n{}",
                                    schema.database,
                                    table.name,
                                    table
                                        .documentation()
                                        .unwrap_or_else(|| "No description".to_string())
                                )),
                            ),
                            range: Some(parser.node_range(node)),
                        });
                    }
                }

                let is_column = node_kind == "column_name"
                    || node_kind == "column_reference"
                    || node_kind == "column_identifier"
                    || (node_kind == "identifier" && parser.is_in_column_context(node, sql));

                if is_column {
                    let table_name = parser.get_table_name_for_column(node, sql);
                    let column_name = SqlParser::identifier_last_part(&node_text);

                    for table in &schema.tables {
                        if let Some(ref table_ref) = table_name {
                            if !Self::table_matches(schema, table, table_ref) {
                                continue;
                            }
                        }

                        if let Some(column) = table
                            .columns
                            .iter()
                            .find(|column| column.name == column_name)
                        {
                            return Some(Hover {
                                contents: tower_lsp::lsp_types::HoverContents::Scalar(
                                    MarkedString::String(format!(
                                        "PostgreSQL Column: {}.{}\n{}",
                                        table.name,
                                        column.name,
                                        column
                                            .documentation()
                                            .unwrap_or_else(|| "No description".to_string())
                                    )),
                                ),
                                range: Some(parser.node_range(node)),
                            });
                        }
                    }
                }

                if Self::is_function_reference(node, sql) {
                    if let Some(function) = Self::find_function_by_reference(schema, &node_text) {
                        return Some(Hover {
                            contents: tower_lsp::lsp_types::HoverContents::Markup(
                                tower_lsp::lsp_types::MarkupContent {
                                    kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                                    value: function.markdown_documentation(),
                                },
                            ),
                            range: Some(parser.node_range(node)),
                        });
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

        if let Some(tree) = &parse_result.tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                let node_text = parser.node_text(node, sql);
                let node_kind = node.kind();

                if crate::token::Keywords::is_keyword(&node_text)
                    || crate::token::Operators::is_operator(&node_text)
                    || crate::token::Delimiters::is_delimiter(&node_text)
                {
                    return None;
                }

                let is_table = node_kind == "table_name"
                    || node_kind == "table_reference"
                    || node_kind == "table_identifier"
                    || (node_kind == "identifier" && parser.is_in_from_context(node, sql));

                let is_column = node_kind == "column_name"
                    || node_kind == "column_reference"
                    || node_kind == "column_identifier"
                    || (node_kind == "identifier" && parser.is_in_column_context(node, sql));

                if is_table {
                    if let Some(schema) = schema {
                        let table_ref = SqlParser::normalize_identifier(&node_text);
                        if Self::find_table_by_reference(schema, &table_ref).is_some() {
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

                if is_column {
                    if let Some(schema) = schema {
                        let (table_name, column_name) =
                            if let Some(table_name) = parser.get_table_name_for_column(node, sql) {
                                (
                                    Some(table_name),
                                    SqlParser::identifier_last_part(&node_text),
                                )
                            } else {
                                let tables = parser.extract_tables(tree, sql);
                                (
                                    tables.first().cloned(),
                                    SqlParser::identifier_last_part(&node_text),
                                )
                            };

                        for table in &schema.tables {
                            if let Some(ref tname) = table_name {
                                if Self::table_matches(schema, table, tname)
                                    && table.columns.iter().any(|c| c.name == column_name)
                                {
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

        if let Some(tree) = &parse_result.tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                let identifier = parser.node_text(node, sql);
                let node_kind = node.kind();

                if crate::token::Keywords::is_keyword(&identifier)
                    || crate::token::Operators::is_operator(&identifier)
                    || crate::token::Delimiters::is_delimiter(&identifier)
                {
                    return locations;
                }

                let is_table = node_kind == "table_name"
                    || node_kind == "table_reference"
                    || node_kind == "table_identifier"
                    || (node_kind == "identifier" && parser.is_in_from_context(node, sql));

                let is_column = node_kind == "column_name"
                    || node_kind == "column_reference"
                    || node_kind == "column_identifier"
                    || (node_kind == "identifier" && parser.is_in_column_context(node, sql));

                if is_table || is_column {
                    let tokens = parser.tokenize(tree, sql);
                    let current_uri = tower_lsp::lsp_types::Url::parse("file:///current.sql")
                        .unwrap_or_else(|_| tower_lsp::lsp_types::Url::parse("file:///").unwrap());

                    for token in tokens {
                        if token.text.eq_ignore_ascii_case(&identifier)
                            && !crate::token::Keywords::is_keyword(&token.text)
                            && !crate::token::Operators::is_operator(&token.text)
                            && !crate::token::Delimiters::is_delimiter(&token.text)
                        {
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
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(sql, schema).await
    }
}
