use crate::dialect::Dialect;
use crate::dialects::common;
use crate::parser::SqlParser;
use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, Hover, Location, MarkedString, Position,
};

pub struct ClickHouseDialect {
    parser: std::sync::Mutex<SqlParser>,
}

impl Default for ClickHouseDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl ClickHouseDialect {
    pub fn new() -> Self {
        Self {
            parser: std::sync::Mutex::new(SqlParser::new()),
        }
    }

    fn create_keyword_item(&self, keyword: &str) -> CompletionItem {
        CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(format!("ClickHouse keyword: {}", keyword)),
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
}

#[async_trait]
impl Dialect for ClickHouseDialect {
    fn name(&self) -> &str {
        "clickhouse"
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
        let position = SqlParser::lsp_position_to_byte_position(sql, position);
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

        let mut items = Vec::new();
        let keywords = &[
            "SELECT",
            "FROM",
            "WHERE",
            "INSERT",
            "INTO",
            "VALUES",
            "CREATE",
            "DROP",
            "ALTER",
            "TABLE",
            "DATABASE",
            "ENGINE",
            "MergeTree",
            "ReplacingMergeTree",
            "SummingMergeTree",
            "AggregatingMergeTree",
            "CollapsingMergeTree",
            "VersionedCollapsingMergeTree",
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
            "BETWEEN",
            "IS",
            "NULL",
            "CAST",
            "ARRAY",
            "TUPLE",
            "MAP",
            "Nested",
            "AggregateFunction",
            "Array",
            "String",
            "Int8",
            "Int16",
            "Int32",
            "Int64",
            "UInt8",
            "UInt16",
            "UInt32",
            "UInt64",
            "Float32",
            "Float64",
            "Date",
            "DateTime",
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
            }
            crate::parser::CompletionContext::WhereClause => {
                let predicate_operator_expected =
                    common::predicate_operator_expected(sql, position);
                let where_keywords: Vec<&str> = keywords
                    .iter()
                    .filter(|&&k| {
                        if predicate_operator_expected {
                            matches!(
                                k,
                                "AND"
                                    | "OR"
                                    | "NOT"
                                    | "IN"
                                    | "LIKE"
                                    | "ILIKE"
                                    | "BETWEEN"
                                    | "IS"
                                    | "NULL"
                            )
                        } else {
                            matches!(k, "NOT" | "NULL")
                        }
                    })
                    .copied()
                    .collect();

                if !predicate_operator_expected {
                    if let Some(schema) = schema {
                        for table in &schema.tables {
                            for column in &table.columns {
                                let mut item = self.create_column_item(
                                    column,
                                    Some(&format!("{}.{}", schema.database, table.name)),
                                );
                                item.sort_text = Some(format!("0{}", column.name));
                                items.push(item);
                            }
                        }
                    }
                }

                for keyword in where_keywords {
                    let mut item = self.create_keyword_item(keyword);
                    item.sort_text = Some(format!("1{}", keyword));
                    items.push(item);
                }

                if predicate_operator_expected {
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
                }
            }
            crate::parser::CompletionContext::OrderByClause => {
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
            crate::parser::CompletionContext::GroupByClause => {
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
            crate::parser::CompletionContext::GroupByContinuationClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in [",", "HAVING", "ORDER BY", "LIMIT", "OFFSET", "WITH TOTALS"] {
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
                    false,
                    &[",", "LIMIT", "OFFSET", "WITH FILL"],
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
                let having_keywords: Vec<&str> = keywords
                    .iter()
                    .filter(|&&k| {
                        if predicate_operator_expected {
                            matches!(
                                k,
                                "AND"
                                    | "OR"
                                    | "NOT"
                                    | "IN"
                                    | "LIKE"
                                    | "ILIKE"
                                    | "BETWEEN"
                                    | "IS"
                                    | "NULL"
                            )
                        } else {
                            matches!(k, "NOT" | "NULL")
                        }
                    })
                    .copied()
                    .collect();

                if !predicate_operator_expected {
                    let aggregate_functions = vec!["COUNT", "SUM", "AVG", "MIN", "MAX"];
                    for func in aggregate_functions {
                        if !prefix.is_empty() && !func.to_lowercase().starts_with(&prefix) {
                            continue;
                        }
                        items.push(self.create_keyword_item(func));
                    }
                    if let Some(schema) = schema {
                        for table in &schema.tables {
                            for column in &table.columns {
                                if !prefix.is_empty()
                                    && !column.name.to_lowercase().starts_with(&prefix)
                                {
                                    continue;
                                }
                                items.push(self.create_column_item(
                                    column,
                                    Some(&format!("{}.{}", schema.database, table.name)),
                                ));
                            }
                        }
                    }
                }

                for keyword in having_keywords {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(keyword));
                }

                if predicate_operator_expected {
                    let operators = vec!["=", "<>", "!=", ">", "<", ">=", "<="];
                    common::add_operator_items(&mut items, &operators, &prefix, "1");
                }
            }
            crate::parser::CompletionContext::UsingClause => {
                let prefix = common::cursor_prefix_excluding_keywords(sql, position, &["using"]);
                if let (Some(schema), Some(tree)) = (schema, &parse_result.tree) {
                    let referenced_tables =
                        common::referenced_table_names_at_position(&parser, tree, sql, position);
                    common::add_schema_using_columns(
                        &mut items,
                        schema,
                        &referenced_tables,
                        &prefix,
                        "0",
                    );
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
                for keyword in ["(", "ON DELETE", "ON UPDATE"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(keyword));
                }
            }
            crate::parser::CompletionContext::ReferenceRuleClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["CASCADE", "RESTRICT", "NO ACTION", "SET NULL"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(keyword));
                }
            }
            crate::parser::CompletionContext::ColumnTargetClause => {
                let prefix = common::cursor_prefix_excluding_keywords(sql, position, &["column"]);
                if let (Some(schema), Some(tree)) = (schema, &parse_result.tree) {
                    let referenced_tables =
                        common::referenced_table_names_at_position(&parser, tree, sql, position);
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
                        common::referenced_table_names_at_position(&parser, tree, sql, position);
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
                    "RENAME COLUMN",
                    "RENAME TO",
                ] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(keyword));
                }
            }
            crate::parser::CompletionContext::InsertActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["(", "VALUES", "SELECT", "FORMAT"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(keyword));
                }
            }
            crate::parser::CompletionContext::UpdateActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["SET", "WHERE"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(keyword));
                }
            }
            crate::parser::CompletionContext::DeleteActionClause => {
                let prefix = common::cursor_prefix(sql, position);
                for keyword in ["WHERE"] {
                    if !prefix.is_empty() && !keyword.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(keyword));
                }
            }
            crate::parser::CompletionContext::IndexTargetClause => {
                let prefix = common::cursor_prefix_excluding_keywords(sql, position, &["index"]);
                if let (Some(schema), Some(tree)) = (schema, &parse_result.tree) {
                    let referenced_tables =
                        common::referenced_table_names_at_position(&parser, tree, sql, position);
                    common::add_schema_indexes(
                        &mut items,
                        schema,
                        &referenced_tables,
                        &prefix,
                        "0",
                        false,
                    );
                }
            }
            crate::parser::CompletionContext::DataTypeClause => {
                let prefix = common::cursor_prefix(sql, position);
                for data_type in [
                    "String", "Int32", "Int64", "UInt32", "UInt64", "Float32", "Float64", "Bool",
                    "Date", "DateTime", "Decimal", "Array()", "Map()", "Tuple()",
                ] {
                    if !prefix.is_empty() && !data_type.to_lowercase().starts_with(&prefix) {
                        continue;
                    }
                    items.push(self.create_keyword_item(data_type));
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
                                common::find_table_by_reference(schema, real_table_name)
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
                                "ClickHouse Table: {}.{}\n{}",
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
}
