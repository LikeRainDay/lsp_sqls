use crate::dialect::Dialect;
use crate::parser::dsl::DslParser;
use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, Hover, Location,
    MarkedString, NumberOrString, Position, Range,
};

/// Elasticsearch DSL (Domain Specific Language) 方言
/// 注意：DSL 是基于 JSON 的查询语言，使用 tree-sitter-json 解析
pub struct ElasticsearchDslDialect {
    dsl_parser: std::sync::Mutex<DslParser>,
}

impl Default for ElasticsearchDslDialect {
    fn default() -> Self {
        Self::new()
    }
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

    fn add_schema_field_items(&self, items: &mut Vec<CompletionItem>, schema: Option<&Schema>) {
        let Some(schema) = schema else {
            return;
        };

        for table in &schema.tables {
            for column in &table.columns {
                items.push(self.create_field_item(
                    &column.name,
                    &format!("Elasticsearch field in {}", table.name),
                ));
            }
        }
    }

    /// 递归查找字段引用
    #[allow(clippy::only_used_in_recursion)]
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
                if let Ok(key_text) = key_node.utf8_text(source.as_bytes()) {
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
        match extract_http_request_bodies(dsl) {
            HttpRequestBodies::Bodies(bodies) => {
                let mut parser = self.dsl_parser.lock().unwrap();
                let mut diagnostics = Vec::new();
                for body in bodies {
                    let mut body_diagnostics = parser.parse(&body.text);
                    offset_diagnostics(&mut body_diagnostics, body.start_line);
                    diagnostics.extend(body_diagnostics);
                }
                return diagnostics;
            }
            HttpRequestBodies::Diagnostics(diagnostics) => return diagnostics,
            HttpRequestBodies::NotHttp => {}
        }

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
                self.add_schema_field_items(&mut items, schema);
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
                self.add_schema_field_items(&mut items, schema);
            }

            crate::parser::DslCompletionContext::BoolQuery => {
                // bool 查询的子字段
                let bool_fields = vec!["must", "must_not", "should", "filter"];

                for field in bool_fields {
                    items.push(self.create_field_item(field, "Bool query field"));
                }
                self.add_schema_field_items(&mut items, schema);
            }

            crate::parser::DslCompletionContext::SortObject => {
                // sort 字段（可以是字段名或特殊值）
                self.add_schema_field_items(&mut items, schema);

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
                self.add_schema_field_items(&mut items, schema);
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
                        .map(tower_lsp::lsp_types::Documentation::String),
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

    async fn hover(&self, dsl: &str, position: Position, schema: Option<&Schema>) -> Option<Hover> {
        let token = token_at_position(dsl, position);
        let schema = schema?;

        for table in &schema.tables {
            if table.name == token {
                return Some(Hover {
                    contents: tower_lsp::lsp_types::HoverContents::Scalar(MarkedString::String(
                        format!(
                            "Elasticsearch DSL Index: {}\n{}",
                            table.name,
                            table.comment.as_deref().unwrap_or("No description")
                        ),
                    )),
                    range: None,
                });
            }

            if let Some(column) = table.columns.iter().find(|column| column.name == token) {
                return Some(Hover {
                    contents: tower_lsp::lsp_types::HoverContents::Scalar(MarkedString::String(
                        format!(
                            "Elasticsearch field: {}.{}\nType: {}",
                            table.name, column.name, column.data_type
                        ),
                    )),
                    range: None,
                });
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
                let field_name = parser
                    .extract_field_name(node, dsl)
                    .unwrap_or_else(|| token_at_position(dsl, position));
                if !field_name.is_empty() {
                    // 如果是索引名或字段名，在 schema 中查找
                    if let Some(schema) = schema {
                        if schema.tables.iter().any(|table| {
                            table.name == field_name
                                || table.columns.iter().any(|column| column.name == field_name)
                        }) {
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

fn token_at_position(text: &str, position: Position) -> String {
    let line = text.lines().nth(position.line as usize).unwrap_or("");
    let byte_index = position.character.min(line.len() as u32) as usize;
    let bytes = line.as_bytes();
    let mut start = byte_index.min(bytes.len());
    while start > 0 && is_token_char(bytes[start - 1] as char) {
        start -= 1;
    }
    let mut end = byte_index.min(bytes.len());
    while end < bytes.len() && is_token_char(bytes[end] as char) {
        end += 1;
    }

    line[start..end]
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':')
}

#[derive(Debug)]
struct HttpBody {
    text: String,
    start_line: u32,
}

enum HttpRequestBodies {
    NotHttp,
    Bodies(Vec<HttpBody>),
    Diagnostics(Vec<Diagnostic>),
}

fn extract_http_request_bodies(input: &str) -> HttpRequestBodies {
    let mut saw_request = false;
    let mut bodies = Vec::new();
    let mut current_body = Vec::<String>::new();
    let mut current_start_line = 0u32;

    for (line_index, line) in input.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match parse_http_request_line(trimmed, line_index as u32) {
            Ok(Some(inline_body)) => {
                saw_request = true;
                push_http_body(&mut bodies, &mut current_body, current_start_line);
                if let Some(body) = inline_body {
                    current_start_line = line_index as u32;
                    current_body.push(body);
                }
            }
            Ok(None) if saw_request => {
                if current_body.is_empty() {
                    current_start_line = line_index as u32;
                }
                current_body.push(trimmed.to_string());
            }
            Ok(None) => return HttpRequestBodies::NotHttp,
            Err(diagnostic) => return HttpRequestBodies::Diagnostics(vec![diagnostic]),
        }
    }

    if !saw_request {
        return HttpRequestBodies::NotHttp;
    }

    push_http_body(&mut bodies, &mut current_body, current_start_line);
    HttpRequestBodies::Bodies(bodies)
}

fn parse_http_request_line(
    line: &str,
    line_index: u32,
) -> Result<Option<Option<String>>, Diagnostic> {
    let Some((method, rest)) = line.split_once(char::is_whitespace) else {
        return Ok(None);
    };
    if !matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "HEAD" | "PATCH"
    ) {
        return Ok(None);
    }

    let rest = rest.trim();
    if rest.is_empty() {
        return Err(http_request_diagnostic(
            line_index,
            line,
            "Elasticsearch request path is required",
        ));
    }

    let mut split = rest.splitn(2, char::is_whitespace);
    let path = split.next().unwrap_or("").trim();
    if !path.starts_with('/') {
        return Err(http_request_diagnostic(
            line_index,
            line,
            "Elasticsearch request path must start with '/'",
        ));
    }

    Ok(Some(
        split
            .next()
            .map(str::trim)
            .filter(|body| !body.is_empty())
            .map(ToString::to_string),
    ))
}

fn push_http_body(
    bodies: &mut Vec<HttpBody>,
    current_body: &mut Vec<String>,
    current_start_line: u32,
) {
    if current_body.is_empty() {
        return;
    }

    bodies.push(HttpBody {
        text: current_body.join("\n"),
        start_line: current_start_line,
    });
    current_body.clear();
}

fn offset_diagnostics(diagnostics: &mut [Diagnostic], line_offset: u32) {
    for diagnostic in diagnostics {
        diagnostic.range.start.line += line_offset;
        diagnostic.range.end.line += line_offset;
    }
}

fn http_request_diagnostic(line: u32, text: &str, message: &str) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position { line, character: 0 },
            end: Position {
                line,
                character: text.len() as u32,
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(
            "ELASTICSEARCH_HTTP_REQUEST".to_string(),
        )),
        code_description: None,
        source: Some("elasticsearch-dsl".to_string()),
        message: message.to_string(),
        related_information: None,
        tags: None,
        data: None,
    }
}
