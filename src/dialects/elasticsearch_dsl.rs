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
        self.create_field_item_with_insert(field, detail_prefix, true)
    }

    fn create_field_item_with_insert(
        &self,
        field: &str,
        detail_prefix: &str,
        quoted_insert: bool,
    ) -> CompletionItem {
        CompletionItem {
            label: field.to_string(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(format!("{}: {}", detail_prefix, field)),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("1{}", field)),
            filter_text: Some(field.to_string()),
            insert_text: Some(if quoted_insert {
                format!("\"{}\"", field)
            } else {
                field.to_string()
            }),
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

    fn create_query_type_item_with_insert(
        &self,
        query_type: &str,
        quoted_insert: bool,
    ) -> CompletionItem {
        CompletionItem {
            label: query_type.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(format!("Elasticsearch DSL query type: {}", query_type)),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("0{}", query_type)),
            filter_text: Some(query_type.to_string()),
            insert_text: Some(if quoted_insert {
                format!("\"{}\"", query_type)
            } else {
                query_type.to_string()
            }),
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

    fn create_agg_type_item_with_insert(
        &self,
        agg_type: &str,
        quoted_insert: bool,
    ) -> CompletionItem {
        CompletionItem {
            label: agg_type.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("Elasticsearch aggregation: {}", agg_type)),
            documentation: None,
            deprecated: None,
            preselect: None,
            sort_text: Some(format!("2{}", agg_type)),
            filter_text: Some(agg_type.to_string()),
            insert_text: Some(if quoted_insert {
                format!("\"{}\"", agg_type)
            } else {
                agg_type.to_string()
            }),
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

    fn create_index_item(
        &self,
        table: &crate::schema::Table,
        quoted_insert: bool,
    ) -> CompletionItem {
        CompletionItem {
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
            filter_text: Some(table.name.clone()),
            insert_text: Some(if quoted_insert {
                format!("\"{}\"", table.name)
            } else {
                table.name.clone()
            }),
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

    fn add_schema_field_items(
        &self,
        items: &mut Vec<CompletionItem>,
        schema: Option<&Schema>,
        prefix: &str,
    ) {
        self.add_schema_field_items_with_insert(items, schema, prefix, true);
    }

    fn add_schema_field_items_with_insert(
        &self,
        items: &mut Vec<CompletionItem>,
        schema: Option<&Schema>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        let Some(schema) = schema else {
            return;
        };

        for table in &schema.tables {
            for column in &table.columns {
                if !prefix.is_empty() && !column.name.to_ascii_lowercase().starts_with(prefix) {
                    continue;
                }

                items.push(self.create_field_item_with_insert(
                    &column.name,
                    &format!("Elasticsearch field in {}", table.name),
                    quoted_insert,
                ));
            }
        }
    }

    fn add_schema_index_items(
        &self,
        items: &mut Vec<CompletionItem>,
        schema: Option<&Schema>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        let Some(schema) = schema else {
            return;
        };

        for table in &schema.tables {
            if !prefix.is_empty() && !table.name.to_ascii_lowercase().starts_with(prefix) {
                continue;
            }

            items.push(self.create_index_item(table, quoted_insert));
        }
    }

    fn add_top_level_items(
        &self,
        items: &mut Vec<CompletionItem>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        for field in ES_TOP_LEVEL_FIELDS {
            if !prefix.is_empty() && !field.to_ascii_lowercase().starts_with(prefix) {
                continue;
            }

            items.push(self.create_field_item_with_insert(
                field,
                "Elasticsearch DSL field",
                quoted_insert,
            ));
        }
    }

    fn add_query_type_items(
        &self,
        items: &mut Vec<CompletionItem>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        for query_type in ES_QUERY_TYPES {
            if !prefix.is_empty() && !query_type.to_ascii_lowercase().starts_with(prefix) {
                continue;
            }

            items.push(self.create_query_type_item_with_insert(query_type, quoted_insert));
        }
    }

    fn add_agg_type_items(
        &self,
        items: &mut Vec<CompletionItem>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        for agg_type in ES_AGG_TYPES {
            if !prefix.is_empty() && !agg_type.to_ascii_lowercase().starts_with(prefix) {
                continue;
            }

            items.push(self.create_agg_type_item_with_insert(agg_type, quoted_insert));
        }
    }

    fn add_bool_field_items(
        &self,
        items: &mut Vec<CompletionItem>,
        prefix: &str,
        quoted_insert: bool,
    ) {
        for field in ES_BOOL_FIELDS {
            if !prefix.is_empty() && !field.to_ascii_lowercase().starts_with(prefix) {
                continue;
            }

            items.push(self.create_field_item_with_insert(
                field,
                "Bool query field",
                quoted_insert,
            ));
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
        let prefix = crate::position::cursor_token_prefix(dsl, position, is_token_char);
        let hint = elasticsearch_completion_context(dsl, position);
        let quoted_insert = !hint.inside_string;
        let mut items = Vec::new();

        match hint.kind {
            EsCompletionKind::TopLevel => {
                self.add_top_level_items(&mut items, &prefix, quoted_insert);
                return items;
            }
            EsCompletionKind::QueryType => {
                self.add_query_type_items(&mut items, &prefix, quoted_insert);
                return items;
            }
            EsCompletionKind::AggType => {
                self.add_agg_type_items(&mut items, &prefix, quoted_insert);
                return items;
            }
            EsCompletionKind::BoolField => {
                self.add_bool_field_items(&mut items, &prefix, quoted_insert);
                return items;
            }
            EsCompletionKind::FieldName => {
                self.add_schema_field_items_with_insert(&mut items, schema, &prefix, quoted_insert);
                return items;
            }
            EsCompletionKind::IndexValue => {
                self.add_schema_index_items(&mut items, schema, &prefix, quoted_insert);
                return items;
            }
            EsCompletionKind::Broad => {}
        }

        let byte_position = crate::position::lsp_position_to_byte_position(dsl, position);
        let mut parser = self.dsl_parser.lock().unwrap();
        let (tree, _) = parser.parse_with_tree(dsl);

        // 分析补全上下文
        let context = if let Some(ref tree) = tree {
            if let Some(node) = parser.get_node_at_position(tree, byte_position) {
                parser.analyze_completion_context(node, dsl)
            } else {
                crate::parser::DslCompletionContext::Default
            }
        } else {
            crate::parser::DslCompletionContext::Default
        };

        // 根据上下文提供不同的补全
        match context {
            crate::parser::DslCompletionContext::TopLevel => {
                self.add_top_level_items(&mut items, &prefix, true);
            }

            crate::parser::DslCompletionContext::QueryObject => {
                self.add_query_type_items(&mut items, &prefix, true);
                self.add_schema_field_items(&mut items, schema, &prefix);
            }

            crate::parser::DslCompletionContext::AggsObject => {
                self.add_agg_type_items(&mut items, &prefix, true);
                self.add_schema_field_items(&mut items, schema, &prefix);
            }

            crate::parser::DslCompletionContext::BoolQuery => {
                self.add_bool_field_items(&mut items, &prefix, true);
                self.add_schema_field_items(&mut items, schema, &prefix);
            }

            crate::parser::DslCompletionContext::SortObject => {
                // sort 字段（可以是字段名或特殊值）
                self.add_schema_field_items(&mut items, schema, &prefix);

                // 排序方向
                if prefix.is_empty() || "_score".starts_with(&prefix) {
                    items.push(self.create_field_item("_score", "Sort by score"));
                }
                if prefix.is_empty() || "_doc".starts_with(&prefix) {
                    items.push(self.create_field_item("_doc", "Sort by document order"));
                }
            }

            crate::parser::DslCompletionContext::Default => {
                self.add_query_type_items(&mut items, &prefix, true);
                self.add_top_level_items(&mut items, &prefix, true);
                self.add_schema_field_items(&mut items, schema, &prefix);
            }
        }

        // 如果提供了 schema，添加索引名补全
        self.add_schema_index_items(&mut items, schema, &prefix, true);

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
        let byte_position = crate::position::lsp_position_to_byte_position(dsl, position);
        let mut parser = self.dsl_parser.lock().unwrap();
        let (tree, _) = parser.parse_with_tree(dsl);

        if let Some(ref tree) = tree {
            if let Some(node) = parser.get_node_at_position(tree, byte_position) {
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
        let position = crate::position::lsp_position_to_byte_position(dsl, position);
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

const ES_TOP_LEVEL_FIELDS: &[&str] = &[
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

const ES_QUERY_TYPES: &[&str] = &[
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

const ES_AGG_TYPES: &[&str] = &[
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

const ES_BOOL_FIELDS: &[&str] = &["must", "must_not", "should", "filter"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EsCompletionKind {
    Broad,
    TopLevel,
    QueryType,
    AggType,
    BoolField,
    FieldName,
    IndexValue,
}

#[derive(Debug, Clone, Copy)]
struct EsCompletionContext {
    kind: EsCompletionKind,
    inside_string: bool,
}

#[derive(Debug, Clone, Default)]
struct EsJsonObjectFrame {
    owner_key: Option<String>,
    last_key: Option<String>,
    after_colon: bool,
}

#[derive(Debug, Clone, Default)]
struct EsJsonScanState {
    frames: Vec<EsJsonObjectFrame>,
    array_owner_keys: Vec<Option<String>>,
    pending_array_object_owner: Option<String>,
    previous_significant: Option<char>,
}

fn elasticsearch_completion_context(source: &str, position: Position) -> EsCompletionContext {
    let byte_offset = byte_offset_at_position(source, position);
    let before = &source[..byte_offset.min(source.len())];
    let open_string_start = current_open_string_start(before);
    let context_source = open_string_start
        .map(|start| &before[..start])
        .unwrap_or(before);
    let state = scan_json_context(context_source);
    let inside_string = open_string_start.is_some();
    let previous = state.previous_significant;
    let current_key = state
        .frames
        .last()
        .and_then(|frame| frame.last_key.as_deref());
    let owner_key = state
        .frames
        .last()
        .and_then(|frame| frame.owner_key.as_deref());

    let kind = if matches!(previous, Some(':')) {
        match current_key {
            Some(key) if is_elasticsearch_index_key(key) => EsCompletionKind::IndexValue,
            Some(key) if is_elasticsearch_field_value_key(key) => EsCompletionKind::FieldName,
            _ => EsCompletionKind::Broad,
        }
    } else if is_json_key_position(previous) {
        match owner_key {
            None => EsCompletionKind::TopLevel,
            Some("query") => EsCompletionKind::QueryType,
            Some("aggs" | "aggregations") => EsCompletionKind::AggType,
            Some("bool") => EsCompletionKind::BoolField,
            Some(key) if is_elasticsearch_field_object_key(key) => EsCompletionKind::FieldName,
            _ => EsCompletionKind::Broad,
        }
    } else {
        EsCompletionKind::Broad
    };

    EsCompletionContext {
        kind,
        inside_string,
    }
}

fn byte_offset_at_position(source: &str, position: Position) -> usize {
    let position = crate::position::lsp_position_to_byte_position(source, position);
    let mut offset = 0usize;

    for (line_index, line) in source.split('\n').enumerate() {
        if line_index == position.line as usize {
            return offset + (position.character as usize).min(line.len());
        }
        offset += line.len() + 1;
    }

    source.len()
}

fn current_open_string_start(source: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escaping = false;
    let mut start = 0usize;

    for (index, ch) in source.char_indices() {
        if in_string {
            if escaping {
                escaping = false;
            } else if ch == '\\' {
                escaping = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            start = index;
        }
    }

    in_string.then_some(start)
}

fn scan_json_context(source: &str) -> EsJsonScanState {
    let mut state = EsJsonScanState::default();
    let mut in_string = false;
    let mut escaping = false;
    let mut string_start = 0usize;
    let mut previous_significant = None;

    for (index, ch) in source.char_indices() {
        if in_string {
            if escaping {
                escaping = false;
            } else if ch == '\\' {
                escaping = true;
            } else if ch == '"' {
                in_string = false;
                let value = source[string_start..index].to_string();
                if previous_significant != Some(':') {
                    if let Some(frame) = state.frames.last_mut() {
                        frame.last_key = Some(value);
                    }
                }
                previous_significant = Some('"');
            }
            continue;
        }

        if ch.is_whitespace() {
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                escaping = false;
                string_start = index + ch.len_utf8();
            }
            '{' => {
                let owner_key = if previous_significant == Some(':') {
                    state.frames.last().and_then(|frame| frame.last_key.clone())
                } else if previous_significant == Some('[') {
                    state.array_owner_keys.last().cloned().flatten()
                } else if previous_significant == Some(',') {
                    state.pending_array_object_owner.take()
                } else {
                    None
                };
                if let Some(frame) = state.frames.last_mut() {
                    if previous_significant == Some(':') {
                        frame.after_colon = false;
                    }
                }
                state.frames.push(EsJsonObjectFrame {
                    owner_key,
                    last_key: None,
                    after_colon: false,
                });
                previous_significant = Some('{');
            }
            '}' => {
                state.frames.pop();
                previous_significant = Some('}');
            }
            '[' => {
                let owner_key = if previous_significant == Some(':') {
                    state.frames.last().and_then(|frame| frame.last_key.clone())
                } else {
                    None
                };
                if let Some(frame) = state.frames.last_mut() {
                    if previous_significant == Some(':') {
                        frame.after_colon = false;
                    }
                }
                state.array_owner_keys.push(owner_key);
                state.pending_array_object_owner = None;
                previous_significant = Some('[');
            }
            ']' => {
                state.array_owner_keys.pop();
                state.pending_array_object_owner = None;
                previous_significant = Some(']');
            }
            ':' => {
                if let Some(frame) = state.frames.last_mut() {
                    frame.after_colon = true;
                }
                previous_significant = Some(':');
            }
            ',' => {
                let comma_in_array = matches!(previous_significant, Some('}' | ']'))
                    && !state.array_owner_keys.is_empty();
                if comma_in_array {
                    state.pending_array_object_owner =
                        state.array_owner_keys.last().cloned().flatten();
                } else if let Some(frame) = state.frames.last_mut() {
                    frame.last_key = None;
                    frame.after_colon = false;
                    state.pending_array_object_owner = None;
                }
                previous_significant = Some(',');
            }
            _ => {
                if let Some(frame) = state.frames.last_mut() {
                    if frame.after_colon {
                        frame.after_colon = false;
                    }
                }
                previous_significant = Some(ch);
            }
        }
    }

    state.previous_significant = previous_significant;
    state
}

fn is_json_key_position(previous: Option<char>) -> bool {
    matches!(previous, Some('{') | Some(',') | Some('"'))
}

fn is_elasticsearch_index_key(key: &str) -> bool {
    matches!(key, "index" | "_index")
}

fn is_elasticsearch_field_value_key(key: &str) -> bool {
    matches!(key, "field" | "fields")
}

fn is_elasticsearch_field_object_key(key: &str) -> bool {
    matches!(
        key,
        "match"
            | "match_phrase"
            | "match_phrase_prefix"
            | "term"
            | "terms"
            | "range"
            | "exists"
            | "prefix"
            | "wildcard"
            | "regexp"
            | "fuzzy"
            | "sort"
    )
}

fn token_at_position(text: &str, position: Position) -> String {
    let position = crate::position::lsp_position_to_byte_position(text, position);
    let line = text.split('\n').nth(position.line as usize).unwrap_or("");
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
            Err(diagnostic) => return HttpRequestBodies::Diagnostics(vec![*diagnostic]),
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
) -> Result<Option<Option<String>>, Box<Diagnostic>> {
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
        return Err(Box::new(http_request_diagnostic(
            line_index,
            line,
            "Elasticsearch request path is required",
        )));
    }

    let mut split = rest.splitn(2, char::is_whitespace);
    let path = split.next().unwrap_or("").trim();
    if !path.starts_with('/') {
        return Err(Box::new(http_request_diagnostic(
            line_index,
            line,
            "Elasticsearch request path must start with '/'",
        )));
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
