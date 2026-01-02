use crate::dialect::Dialect;
use crate::parser::dsl::DslParser;
use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, Hover, Location, MarkedString, Position,
};

/// Elasticsearch DSL (Domain Specific Language) 方言
/// 注意：DSL 是基于 JSON 的查询语言，使用 tree-sitter-json 解析
pub struct ElasticsearchDslDialect {
    dsl_parser: std::sync::Mutex<DslParser>,
}

impl ElasticsearchDslDialect {
    pub fn new() -> Self {
        Self {
            dsl_parser: std::sync::Mutex::new(DslParser::new()),
        }
    }

    /// 创建字段补全项
    fn create_field_item(&self, field: &str, detail_prefix: &str) -> CompletionItem {
        CompletionItem {
            label: field.to_string(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(format!("{}: {}", detail_prefix, field)),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("1{}", field)),
            filter_text: None,
            insert_text: Some(format!("\"{}\"", field)),
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

    /// 创建查询类型补全项
    fn create_query_type_item(&self, query_type: &str) -> CompletionItem {
        CompletionItem {
            label: query_type.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(format!("Elasticsearch DSL query type: {}", query_type)),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("0{}", query_type)),
            filter_text: None,
            insert_text: Some(format!("\"{}\"", query_type)),
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

    /// 创建聚合类型补全项
    fn create_agg_type_item(&self, agg_type: &str) -> CompletionItem {
        CompletionItem {
            label: agg_type.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("Elasticsearch aggregation: {}", agg_type)),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("2{}", agg_type)),
            filter_text: None,
            insert_text: Some(format!("\"{}\"", agg_type)),
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

    /// 递归查找字段引用
    fn find_field_references_recursive(
        &self,
        node: tree_sitter::Node,
        source: &str,
        field_name: &str,
        uri: &tower_lsp::lsp_types::Url,
        locations: &mut Vec<Location>,
        parser: &crate::parser::dsl::DslParser,
    ) {
        if node.kind() == "pair" {
            if let Some(key_node) = node.child(0) {
                if let Some(key_text) = key_node.utf8_text(source.as_bytes()).ok() {
                    let key = key_text.trim_matches('"').trim_matches('\'');
                    if key == field_name {
                        locations.push(Location {
                            uri: uri.clone(),
                            range: parser.node_range(key_node),
                        });
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.find_field_references_recursive(child, source, field_name, uri, locations, parser);
        }
    }
}

#[async_trait]
impl Dialect for ElasticsearchDslDialect {
    fn name(&self) -> &str {
        "elasticsearch-dsl"
    }

    async fn parse(&self, dsl: &str, _schema: Option<&Schema>) -> Vec<Diagnostic> {
        // 使用 tree-sitter-json 解析 DSL（保持与 SQL 解析器的一致性）
        let mut parser = self.dsl_parser.lock().unwrap();
        parser.parse(dsl)
    }

    async fn completion(
        &self,
        dsl: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Vec<CompletionItem> {
        let mut parser = self.dsl_parser.lock().unwrap();
        let (tree, _) = parser.parse_with_tree(dsl);

        // 分析补全上下文
        let context = if let Some(ref tree) = tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                parser.analyze_completion_context(node, dsl)
            } else {
                crate::parser::DslCompletionContext::Default
            }
        } else {
            crate::parser::DslCompletionContext::Default
        };

        let mut items = Vec::new();

        // 根据上下文提供不同的补全
        match context {
            crate::parser::DslCompletionContext::TopLevel => {
                // 顶级字段
                let top_level_fields = vec![
                    "query",
                    "aggs",
                    "aggregations",
                    "sort",
                    "from",
                    "size",
                    "source",
                    "_source",
                    "fields",
                    "highlight",
                    "suggest",
                    "script_fields",
                    "docvalue_fields",
                    "stored_fields",
                    "post_filter",
                    "min_score",
                    "timeout",
                    "terminate_after",
                ];

                for field in top_level_fields {
                    items.push(self.create_field_item(field, "Elasticsearch DSL field"));
                }
            }

            crate::parser::DslCompletionContext::QueryObject => {
                // 查询类型
                let query_types = vec![
                    "match",
                    "match_all",
                    "match_none",
                    "match_phrase",
                    "match_phrase_prefix",
                    "multi_match",
                    "common",
                    "query_string",
                    "simple_query_string",
                    "term",
                    "terms",
                    "range",
                    "exists",
                    "prefix",
                    "wildcard",
                    "regexp",
                    "fuzzy",
                    "type",
                    "ids",
                    "constant_score",
                    "bool",
                    "boosting",
                    "dis_max",
                    "function_score",
                    "script_score",
                    "percolate",
                ];

                for query_type in query_types {
                    items.push(self.create_query_type_item(query_type));
                }
            }

            crate::parser::DslCompletionContext::AggsObject => {
                // 聚合类型
                let agg_types = vec![
                    "terms",
                    "range",
                    "date_range",
                    "ip_range",
                    "histogram",
                    "date_histogram",
                    "geo_distance",
                    "geohash_grid",
                    "geotile_grid",
                    "filters",
                    "adjacency_matrix",
                    "sampler",
                    "diversified_sampler",
                    "global",
                    "filter",
                    "missing",
                    "nested",
                    "reverse_nested",
                    "children",
                    "parent",
                    "cardinality",
                    "avg",
                    "sum",
                    "min",
                    "max",
                    "stats",
                    "extended_stats",
                    "percentiles",
                    "percentile_ranks",
                    "top_hits",
                    "scripted_metric",
                    "matrix_stats",
                    "bucket_script",
                    "bucket_selector",
                    "bucket_sort",
                    "serial_diff",
                    "moving_avg",
                ];

                for agg_type in agg_types {
                    items.push(self.create_agg_type_item(agg_type));
                }
            }

            crate::parser::DslCompletionContext::BoolQuery => {
                // bool 查询的子字段
                let bool_fields = vec!["must", "must_not", "should", "filter"];

                for field in bool_fields {
                    items.push(self.create_field_item(field, "Bool query field"));
                }
            }

            crate::parser::DslCompletionContext::SortObject => {
                // sort 字段（可以是字段名或特殊值）
                if let Some(schema) = schema {
                    for table in &schema.tables {
                        for column in &table.columns {
                            items.push(self.create_field_item(&column.name, "Sort field"));
                        }
                    }
                }

                // 排序方向
                items.push(self.create_field_item("_score", "Sort by score"));
                items.push(self.create_field_item("_doc", "Sort by document order"));
            }

            crate::parser::DslCompletionContext::Default => {
                // 默认：返回所有类型
                let query_types = vec![
                    "match",
                    "match_all",
                    "match_none",
                    "match_phrase",
                    "match_phrase_prefix",
                    "multi_match",
                    "common",
                    "query_string",
                    "simple_query_string",
                    "term",
                    "terms",
                    "range",
                    "exists",
                    "prefix",
                    "wildcard",
                    "regexp",
                    "fuzzy",
                    "type",
                    "ids",
                    "constant_score",
                    "bool",
                    "boosting",
                    "dis_max",
                    "function_score",
                    "script_score",
                    "percolate",
                ];

                for query_type in query_types {
                    items.push(self.create_query_type_item(query_type));
                }

                let top_level_fields = vec![
                    "query",
                    "aggs",
                    "aggregations",
                    "sort",
                    "from",
                    "size",
                    "source",
                    "_source",
                    "fields",
                    "highlight",
                    "suggest",
                ];

                for field in top_level_fields {
                    items.push(self.create_field_item(field, "Elasticsearch DSL field"));
                }
            }
        }

        // 如果提供了 schema，添加索引名补全
        if let Some(schema) = schema {
            for table in &schema.tables {
                items.push(CompletionItem {
                    label: table.name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(format!("Elasticsearch Index: {}", table.name)),
                    documentation: table
                        .comment
                        .clone()
                        .map(|c| tower_lsp::lsp_types::Documentation::String(c)),
                    deprecated: None,
                    preselect: None,
                    sort_text: Some(format!("3{}", table.name)),
                    filter_text: None,
                    insert_text: Some(format!("\"{}\"", table.name)),
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
                                "Elasticsearch DSL Index: {}\n{}",
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
        dsl: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Option<Location> {
        let mut parser = self.dsl_parser.lock().unwrap();
        let (tree, _) = parser.parse_with_tree(dsl);

        if let Some(ref tree) = tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                // 提取字段名
                if let Some(field_name) = parser.extract_field_name(node, dsl) {
                    // 如果是索引名，在 schema 中查找
                    if let Some(schema) = schema {
                        if schema.tables.iter().any(|t| t.name == field_name) {
                            return Some(Location {
                                uri: tower_lsp::lsp_types::Url::parse("file:///schema.json")
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

        None
    }

    async fn references(
        &self,
        dsl: &str,
        position: Position,
        _schema: Option<&Schema>,
    ) -> Vec<Location> {
        let mut parser = self.dsl_parser.lock().unwrap();
        let (tree, _) = parser.parse_with_tree(dsl);
        let mut locations = Vec::new();

        if let Some(ref tree) = tree {
            if let Some(node) = parser.get_node_at_position(tree, position) {
                // 提取字段名
                if let Some(field_name) = parser.extract_field_name(node, dsl) {
                    // 在当前文档中查找所有引用
                    let current_uri = tower_lsp::lsp_types::Url::parse("file:///current.json")
                        .unwrap_or_else(|_| tower_lsp::lsp_types::Url::parse("file:///").unwrap());

                    // 遍历所有字段，查找匹配的
                    let root = tree.root_node();
                    let mut cursor = root.walk();
                    for child in root.children(&mut cursor) {
                        self.find_field_references_recursive(
                            child,
                            dsl,
                            &field_name,
                            &current_uri,
                            &mut locations,
                            &parser,
                        );
                    }
                }
            }
        }

        locations
    }

    async fn format(&self, sql: &str) -> String {
        // DSL 格式化：尝试美化 JSON
        // 这里简化处理，实际应该使用 JSON 格式化库
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.parse(sql, schema).await
    }
}
