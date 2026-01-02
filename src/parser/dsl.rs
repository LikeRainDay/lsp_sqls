//! Elasticsearch DSL 解析器
//! DSL 是基于 JSON 的查询语言，使用 tree-sitter-json 进行解析
//! 参考 sqls-server/sqls 的实现方式，保持与 SQL 解析器的一致性

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};
use tree_sitter::{Node, Parser, Tree};

/// DSL 补全上下文类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DslCompletionContext {
    /// 顶级字段（query, aggs, sort 等）
    TopLevel,
    /// 在 query 对象内，应该补全查询类型
    QueryObject,
    /// 在 aggs/aggregations 对象内，应该补全聚合类型
    AggsObject,
    /// 在 bool 查询内，应该补全 must/must_not/should/filter
    BoolQuery,
    /// 在 sort 对象内
    SortObject,
    /// 默认上下文
    Default,
}

/// Elasticsearch DSL 解析结果
#[derive(Debug, Clone)]
pub struct DslParseResult {
    /// 解析后的 AST Tree
    pub tree: Option<Tree>,
    /// 诊断信息
    pub diagnostics: Vec<Diagnostic>,
    /// 解析是否成功（Tree-sitter 总是能生成树，即使有错误）
    pub success: bool,
    /// 原始 DSL 文本
    pub source: String,
}

/// Elasticsearch DSL 解析器（基于 Tree-sitter JSON）
/// 注意：DSL 是 JSON 格式，使用 tree-sitter-json 解析
pub struct DslParser {
    parser: Parser,
    source: String, // 存储当前解析的 DSL 文本
}

impl DslParser {
    pub fn new() -> Self {
        let language = tree_sitter::Language::from(tree_sitter_json::LANGUAGE);
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .expect("Failed to set JSON language");

        Self {
            parser,
            source: String::new(),
        }
    }

    /// 解析 Elasticsearch DSL（JSON 格式）
    pub fn parse(&mut self, dsl: &str) -> Vec<Diagnostic> {
        // 存储 source 以便后续使用
        self.source = dsl.to_string();
        let (_, diagnostics) = self.parse_with_tree(dsl);
        diagnostics
    }

    /// 解析并返回 Tree（用于补全等功能）
    pub fn parse_with_tree(&mut self, dsl: &str) -> (Option<Tree>, Vec<Diagnostic>) {
        let tree = self.parser.parse(dsl, None);

        let mut diagnostics = Vec::new();

        if let Some(tree) = &tree {
            // Tree-sitter 即使有错误也能生成部分树
            // 检查是否有错误节点
            self.collect_errors(tree.root_node(), dsl, &mut diagnostics);
        } else {
            // 完全无法解析
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: dsl.len() as u32,
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("DSL_PARSE_ERROR".to_string())),
                code_description: None,
                source: Some("tree-sitter-json".to_string()),
                message: "Failed to parse JSON".to_string(),
                related_information: None,
                tags: None,
                data: None,
            });
        }

        // 如果 JSON 结构有效，检查 Elasticsearch DSL 特定的字段
        if diagnostics
            .iter()
            .all(|d| d.severity != Some(DiagnosticSeverity::ERROR))
        {
            self.validate_dsl_structure(tree.as_ref(), dsl, &mut diagnostics);
        }

        (tree, diagnostics)
    }

    /// 收集错误节点（参考 SQL 解析器的实现）
    fn collect_errors(&self, node: Node, source: &str, diagnostics: &mut Vec<Diagnostic>) {
        // 检查是否是错误节点
        if node.is_error() || node.is_missing() {
            let start_byte = node.start_byte();
            let end_byte = node.end_byte();
            let start_point = node.start_position();
            let end_point = node.end_position();

            // 获取节点文本
            let node_text = if start_byte < source.len() && end_byte <= source.len() {
                &source[start_byte..end_byte]
            } else {
                ""
            };

            // 过滤空白字符错误（格式问题，不是语法错误）
            if node_text.trim().is_empty() && !node.is_missing() {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.collect_errors(child, source, diagnostics);
                }
                return;
            }

            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position {
                        line: start_point.row as u32,
                        character: start_point.column as u32,
                    },
                    end: Position {
                        line: end_point.row as u32,
                        character: end_point.column as u32,
                    },
                },
                severity: Some(if node.is_error() {
                    DiagnosticSeverity::ERROR
                } else {
                    DiagnosticSeverity::WARNING
                }),
                code: Some(NumberOrString::String("DSL_SYNTAX_ERROR".to_string())),
                code_description: None,
                source: Some("tree-sitter-json".to_string()),
                message: if node.is_error() {
                    format!("JSON syntax error: {}", node_text)
                } else {
                    "Missing JSON element".to_string()
                },
                related_information: None,
                tags: None,
                data: None,
            });
        }

        // 递归检查子节点
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_errors(child, source, diagnostics);
        }
    }

    /// 验证 Elasticsearch DSL 结构
    /// 检查是否包含常见的 Elasticsearch DSL 字段
    fn validate_dsl_structure(
        &self,
        tree: Option<&Tree>,
        json: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if let Some(tree) = tree {
            // 检查是否包含常见的 Elasticsearch DSL 顶级字段
            let has_query = json.contains("\"query\"") || json.contains("'query'");
            let has_aggs = json.contains("\"aggs\"") || json.contains("\"aggregations\"");
            let has_sort = json.contains("\"sort\"");

            // 如果都没有，给出提示（不是错误）
            if !has_query && !has_aggs && !has_sort {
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: json.len() as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::HINT),
                    code: Some(NumberOrString::String("DSL_HINT".to_string())),
                    code_description: None,
                    source: Some("elasticsearch-dsl".to_string()),
                    message:
                        "Elasticsearch DSL typically includes 'query', 'aggs', or 'sort' fields"
                            .to_string(),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }

            // 验证 query 对象的结构
            self.validate_query_structure(tree, json, diagnostics);
        }
    }

    /// 验证 query 结构（如果存在）
    /// 遍历 AST，检查 query 对象的结构
    fn validate_query_structure(&self, tree: &Tree, json: &str, diagnostics: &mut Vec<Diagnostic>) {
        let root = tree.root_node();

        // Elasticsearch DSL 有效的查询类型
        let valid_query_types = vec![
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

        // 查找 "query" 字段
        if let Some(query_node) = self.find_field_in_object(root, json, "query") {
            // 检查 query 对象是否包含有效的查询类型
            let query_value = self.get_node_text(query_node, json);

            // 检查是否是对象类型（query 应该是一个对象）
            if query_node.kind() == "object" {
                // 查找 query 对象中的第一个键（应该是查询类型）
                let mut found_valid_query = false;
                self.check_query_types_recursive(
                    query_node,
                    json,
                    &valid_query_types,
                    &mut found_valid_query,
                );

                if !found_valid_query {
                    // 如果 query 对象存在但没有找到有效的查询类型，给出警告
                    let range = self.node_range(query_node);
                    diagnostics.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::WARNING),
                        code: Some(NumberOrString::String("DSL_QUERY_TYPE".to_string())),
                        code_description: None,
                        source: Some("elasticsearch-dsl".to_string()),
                        message: "Query object should contain a valid query type (match, term, bool, etc.)".to_string(),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            } else if query_value.trim().is_empty() {
                // query 字段存在但值为空
                let range = self.node_range(query_node);
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String("DSL_EMPTY_QUERY".to_string())),
                    code_description: None,
                    source: Some("elasticsearch-dsl".to_string()),
                    message: "Query field should not be empty".to_string(),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }
    }

    /// 在 JSON 对象中查找指定字段
    fn find_field_in_object<'a>(
        &self,
        object_node: Node<'a>,
        source: &str,
        field_name: &str,
    ) -> Option<Node<'a>> {
        if object_node.kind() != "object" {
            return None;
        }

        let mut cursor = object_node.walk();
        for child in object_node.children(&mut cursor) {
            if child.kind() == "pair" {
                // pair 的第一个子节点是 key（string）
                if let Some(key_node) = child.child(0) {
                    if let Some(key_text) = key_node.utf8_text(source.as_bytes()).ok() {
                        let key = key_text.trim_matches('"').trim_matches('\'');
                        if key == field_name {
                            // 返回 pair 的第二个子节点（value）
                            return child.child(1);
                        }
                    }
                }
            }
        }
        None
    }

    /// 递归检查查询类型
    fn check_query_types_recursive<'a>(
        &self,
        node: Node<'a>,
        source: &str,
        valid_types: &[&str],
        found: &mut bool,
    ) {
        if *found {
            return;
        }

        let node_kind = node.kind();

        // 如果是 pair，检查 key 是否是有效的查询类型
        if node_kind == "pair" {
            if let Some(key_node) = node.child(0) {
                if let Some(key_text) = key_node.utf8_text(source.as_bytes()).ok() {
                    let key = key_text.trim_matches('"').trim_matches('\'');
                    if valid_types.iter().any(|&t| t == key) {
                        *found = true;
                        return;
                    }
                }
            }
        }

        // 递归检查子节点
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.check_query_types_recursive(child, source, valid_types, found);
        }
    }

    /// 获取节点的文本内容（辅助方法）
    fn get_node_text<'a>(&self, node: Node<'a>, source: &str) -> String {
        node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
    }

    /// 提取 JSON 中的字段名（用于代码补全）
    pub fn extract_fields(&self, tree: &Tree, source: &str) -> Vec<String> {
        let mut fields = Vec::new();
        self.extract_fields_recursive(tree.root_node(), source, &mut fields);
        fields
    }

    /// 递归提取字段名
    fn extract_fields_recursive<'a>(&self, node: Node<'a>, source: &str, fields: &mut Vec<String>) {
        let node_kind = node.kind();

        // 查找 JSON 对象中的键（field names）
        if node_kind == "pair" {
            // pair 节点包含 key 和 value
            if let Some(key_node) = node.child(0) {
                if key_node.kind() == "string" {
                    if let Some(text) = key_node.utf8_text(source.as_bytes()).ok() {
                        // 移除引号
                        let field_name = text.trim_matches('"').trim_matches('\'');
                        if !field_name.is_empty() && !fields.contains(&field_name.to_string()) {
                            fields.push(field_name.to_string());
                        }
                    }
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_fields_recursive(child, source, fields);
        }
    }

    /// 获取指定位置的节点
    pub fn get_node_at_position<'a>(&self, tree: &'a Tree, position: Position) -> Option<Node<'a>> {
        let root = tree.root_node();
        let point = tree_sitter::Point {
            row: position.line as usize,
            column: position.character as usize,
        };
        root.descendant_for_point_range(point, point)
    }

    /// 获取节点的文本内容
    pub fn node_text(&self, node: Node, source: &str) -> String {
        node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
    }

    /// 获取节点的范围
    pub fn node_range(&self, node: Node) -> Range {
        let start = node.start_position();
        let end = node.end_position();
        Range {
            start: Position {
                line: start.row as u32,
                character: start.column as u32,
            },
            end: Position {
                line: end.row as u32,
                character: end.column as u32,
            },
        }
    }

    /// 分析补全上下文
    /// 根据光标位置的 AST 节点，判断应该提供什么类型的补全
    pub fn analyze_completion_context(&self, node: Node, source: &str) -> DslCompletionContext {
        let mut current = Some(node);

        // 向上遍历 AST，查找上下文
        while let Some(n) = current {
            let kind = n.kind();

            // 检查是否在 pair 节点中（JSON 键值对）
            if kind == "pair" {
                // 检查 key 是否是 "query", "aggs", "bool" 等
                if let Some(key_node) = n.child(0) {
                    if let Some(key_text) = key_node.utf8_text(source.as_bytes()).ok() {
                        let key = key_text.trim_matches('"').trim_matches('\'');

                        // 检查 value 节点
                        if let Some(value_node) = n.child(1) {
                            if value_node.kind() == "object" {
                                match key {
                                    "query" => return DslCompletionContext::QueryObject,
                                    "aggs" | "aggregations" => {
                                        return DslCompletionContext::AggsObject
                                    }
                                    "bool" => return DslCompletionContext::BoolQuery,
                                    "sort" => return DslCompletionContext::SortObject,
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }

            // 检查是否在对象内，查找父对象的 key
            if kind == "object" {
                // 查找父 pair 的 key
                if let Some(parent) = n.parent() {
                    if parent.kind() == "pair" {
                        if let Some(key_node) = parent.child(0) {
                            if let Some(key_text) = key_node.utf8_text(source.as_bytes()).ok() {
                                let key = key_text.trim_matches('"').trim_matches('\'');
                                match key {
                                    "query" => return DslCompletionContext::QueryObject,
                                    "aggs" | "aggregations" => {
                                        return DslCompletionContext::AggsObject
                                    }
                                    "bool" => return DslCompletionContext::BoolQuery,
                                    "sort" => return DslCompletionContext::SortObject,
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                // 检查是否是根对象（顶级）
                if n.parent().is_none()
                    || (n.parent().is_some() && n.parent().unwrap().kind() == "document")
                {
                    return DslCompletionContext::TopLevel;
                }
            }

            current = n.parent();
        }

        DslCompletionContext::Default
    }

    /// 检查节点是否在指定字段的对象内
    pub fn is_in_field_object(&self, node: Node, source: &str, field_name: &str) -> bool {
        let mut current = Some(node);

        while let Some(n) = current {
            if n.kind() == "object" {
                if let Some(parent) = n.parent() {
                    if parent.kind() == "pair" {
                        if let Some(key_node) = parent.child(0) {
                            if let Some(key_text) = key_node.utf8_text(source.as_bytes()).ok() {
                                let key = key_text.trim_matches('"').trim_matches('\'');
                                if key == field_name {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
            current = n.parent();
        }

        false
    }

    /// 提取字段名（用于跳转定义和查找引用）
    pub fn extract_field_name(&self, node: Node, source: &str) -> Option<String> {
        // 如果节点是 pair，提取 key
        if node.kind() == "pair" {
            if let Some(key_node) = node.child(0) {
                if let Some(key_text) = key_node.utf8_text(source.as_bytes()).ok() {
                    let key = key_text.trim_matches('"').trim_matches('\'');
                    return Some(key.to_string());
                }
            }
        }

        // 如果节点是 string（可能是 key），提取文本
        if node.kind() == "string" {
            if let Some(text) = node.utf8_text(source.as_bytes()).ok() {
                let key = text.trim_matches('"').trim_matches('\'');
                // 检查是否是 key（在 pair 的第一个子节点）
                if let Some(parent) = node.parent() {
                    if parent.kind() == "pair" && parent.child(0) == Some(node) {
                        return Some(key.to_string());
                    }
                }
            }
        }

        None
    }
}

impl Default for DslParser {
    fn default() -> Self {
        Self::new()
    }
}
