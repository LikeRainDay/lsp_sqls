//! SQL 解析器实现
//! 参考 sqls-server/sqls 的实现
//! https://github.com/sqls-server/sqls/tree/master/parser

use crate::token::{Delimiters, Keywords, Operators, Token, TokenType};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};
use tree_sitter::{Node, Parser, Tree};

/// 补全上下文类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// 在 FROM 子句中，应该补全表名
    FromClause,
    /// 在 SELECT 子句中，应该补全列名和关键字
    SelectClause,
    /// 在 WHERE 子句中，应该补全列名、操作符、关键字
    WhereClause,
    /// 在表名后（如 table.），应该补全列名
    TableColumn,
    /// 在 JOIN 子句中，应该补全表名
    JoinClause,
    /// 在 ORDER BY 子句中，应该补全列名
    OrderByClause,
    /// 在 GROUP BY 子句中，应该补全列名
    GroupByClause,
    /// 在 HAVING 子句中，应该补全列名和关键字
    HavingClause,
    /// 默认上下文，返回所有关键字
    Default,
}

/// SQL 解析结果
#[derive(Debug, Clone)]
pub struct ParseResult {
    /// 解析后的 AST Tree
    pub tree: Option<Tree>,
    /// 诊断信息
    pub diagnostics: Vec<Diagnostic>,
    /// 解析是否成功（Tree-sitter 总是能生成树，即使有错误）
    pub success: bool,
    /// 原始 SQL 文本
    pub source: String,
}

/// SQL 解析器（基于 Tree-sitter）
pub struct SqlParser {
    parser: Parser,
    source: String, // 存储当前解析的 SQL 文本
}

impl SqlParser {
    /// 创建 SQL 解析器
    pub fn new() -> Self {
        let language = tree_sitter::Language::from(tree_sitter_sequel::LANGUAGE);
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .expect("Failed to set SQL language");

        Self {
            parser,
            source: String::new(),
        }
    }

    /// 解析 SQL 语句
    pub fn parse(&mut self, sql: &str) -> ParseResult {
        // 存储 source 以便后续使用
        self.source = sql.to_string();
        let tree = self.parser.parse(sql, None);

        let mut diagnostics = Vec::new();

        if let Some(tree) = &tree {
            // Tree-sitter 即使有错误也能生成部分树
            // 检查是否有错误节点
            self.collect_errors(tree.root_node(), sql, &mut diagnostics);
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
                        character: sql.len() as u32,
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("PARSE_ERROR".to_string())),
                code_description: None,
                source: Some("tree-sitter-sql".to_string()),
                message: "Failed to parse SQL".to_string(),
                related_information: None,
                tags: None,
                data: None,
            });
        }

        ParseResult {
            tree,
            diagnostics,
            success: true, // Tree-sitter 总是能生成树
            source: sql.to_string(),
        }
    }

    /// 收集错误节点
    /// 参考 sqls 的错误处理逻辑：过滤误报，只报告真正的语法错误
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

            // 参考 sqls：过滤常见的误报情况

            // 1. SELECT * 中的 * 是有效的
            if node_text.trim() == "*" && self.is_in_select_context(node, source) {
                // 跳过这个错误，* 在 SELECT 中是有效的
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.collect_errors(child, source, diagnostics);
                }
                return;
            }

            // 2. 过滤空白字符错误（格式问题，不是语法错误）
            if node_text.trim().is_empty() && !node.is_missing() {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.collect_errors(child, source, diagnostics);
                }
                return;
            }

            // 3. 过滤已知的有效语法模式
            // 例如：某些方言的特殊语法可能被 tree-sitter-sql 误判
            if self.is_valid_syntax_pattern(node, source) {
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
                code: Some(NumberOrString::String("SYNTAX_ERROR".to_string())),
                code_description: None,
                source: Some("tree-sitter-sql".to_string()),
                message: if node.is_error() {
                    format!("Syntax error: {}", node_text)
                } else {
                    "Missing syntax element".to_string()
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

    /// 检查节点是否在 SELECT 上下文中
    fn is_in_select_context(&self, node: Node, source: &str) -> bool {
        let mut current = Some(node);
        while let Some(n) = current {
            let kind = n.kind();
            if kind == "select_list"
                || kind == "select_expression_list"
                || kind == "select_statement"
                || kind == "select"
                || kind == "query"
            {
                return true;
            }
            if let Ok(text) = n.utf8_text(source.as_bytes()) {
                if text.to_uppercase().contains("SELECT") {
                    return true;
                }
            }
            current = n.parent();
        }
        false
    }

    /// 检查是否是有效的语法模式（参考 sqls 的容错处理）
    fn is_valid_syntax_pattern(&self, node: Node, source: &str) -> bool {
        // 检查是否是已知的有效语法模式
        // 例如：某些方言的特殊语法

        // 检查节点类型和上下文
        let node_kind = node.kind();

        // 某些节点类型即使被标记为错误，也可能是有效的
        // 这取决于具体的 SQL 方言
        match node_kind {
            // 这些节点类型在某些情况下可能是有效的
            "identifier" | "expression" | "literal" => {
                // 检查上下文，如果是合理的语法位置，可能是误报
                self.has_reasonable_context(node, source)
            }
            _ => false,
        }
    }

    /// 检查节点是否有合理的上下文（不是真正的语法错误）
    fn has_reasonable_context(&self, node: Node, _source: &str) -> bool {
        // 检查父节点和兄弟节点，判断是否是合理的语法位置
        if let Some(parent) = node.parent() {
            let parent_kind = parent.kind();
            // 如果父节点是合理的容器节点，可能是误报
            matches!(
                parent_kind,
                "select_list"
                    | "expression"
                    | "where_clause"
                    | "order_by_clause"
                    | "group_by_clause"
                    | "having_clause"
                    | "table_reference"
                    | "column_reference"
            )
        } else {
            false
        }
    }

    /// 提取所有 Token（参考 sqls 的 tokenizer）
    pub fn tokenize(&self, tree: &Tree, source: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        self.tokenize_recursive(tree.root_node(), source, &mut tokens);
        tokens
    }

    /// 递归提取 Token
    fn tokenize_recursive(&self, node: Node, source: &str, tokens: &mut Vec<Token>) {
        let node_kind = node.kind();
        let start_point = node.start_position();

        if let Ok(text) = node.utf8_text(source.as_bytes()) {
            let text = text.trim();
            if !text.is_empty() {
                let token_type = self.classify_token(node_kind, text);
                let position = Position {
                    line: start_point.row as u32,
                    character: start_point.column as u32,
                };
                tokens.push(Token::new(token_type, text.to_string(), position));
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.tokenize_recursive(child, source, tokens);
        }
    }

    /// 分类 Token 类型（参考 sqls 的 token 分类逻辑）
    fn classify_token(&self, node_kind: &str, text: &str) -> TokenType {
        // 检查是否是关键字
        if Keywords::is_keyword(text) {
            return TokenType::Keyword;
        }

        // 检查是否是操作符
        if Operators::is_operator(text) {
            return TokenType::Operator;
        }

        // 检查是否是分隔符
        if Delimiters::is_delimiter(text) {
            return TokenType::Delimiter;
        }

        // 根据节点类型分类
        match node_kind {
            "string" | "string_literal" => TokenType::String,
            "number" | "numeric_literal" => TokenType::Number,
            "identifier" | "table_name" | "column_name" => TokenType::Identifier,
            "comment" => TokenType::Comment,
            _ => TokenType::Unknown,
        }
    }

    /// 获取指定位置的节点
    pub fn get_node_at_position<'a>(&self, tree: &'a Tree, position: Position) -> Option<Node<'a>> {
        let root = tree.root_node();
        let row = position.line as usize;
        let col = position.character as usize;

        // Try exact position
        let point = tree_sitter::Point { row, column: col };
        let node = root.descendant_for_point_range(point, point);

        // If we got the root node (and we are not at 0,0), it usually means we are at the end of a token or file
        // and missed the specific node. Try moving back 1 char.
        if let Some(n) = node {
            if n.kind() == "program" && col > 0 {
                let point_prev = tree_sitter::Point {
                    row,
                    column: col - 1,
                };
                return root.descendant_for_point_range(point_prev, point_prev);
            }
            return Some(n);
        }

        node
    }

    /// 提取查询中的表名
    pub fn extract_tables(&self, tree: &Tree, source: &str) -> Vec<String> {
        let mut tables = Vec::new();
        self.extract_tables_recursive(tree.root_node(), source, &mut tables);
        tables
    }

    /// 递归提取表名
    /// 参考 sqls 的实现：查找 FROM/JOIN 子句中的表名
    fn extract_tables_recursive(&self, node: Node, source: &str, tables: &mut Vec<String>) {
        let node_kind = node.kind();

        // 参考 sqls：查找 table_name, table_reference, table_identifier 等节点
        if node_kind == "table_name"
            || node_kind == "table_reference"
            || node_kind == "table_identifier"
            || node_kind == "table"
            || (node_kind == "identifier" && self.is_in_from_context(node, source))
        {
            if let Ok(text) = node.utf8_text(source.as_bytes()) {
                let text = text.trim();
                // 过滤关键字和操作符
                if !text.is_empty()
                    && !Keywords::is_keyword(text)
                    && !Operators::is_operator(text)
                    && !Delimiters::is_delimiter(text)
                    && !tables.contains(&text.to_string())
                {
                    tables.push(text.to_string());
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_tables_recursive(child, source, tables);
        }
    }

    /// 检查节点是否在 FROM/JOIN 上下文中
    pub fn is_in_from_context(&self, node: Node, source: &str) -> bool {
        let mut current = Some(node);
        while let Some(n) = current {
            let kind = n.kind();
            // 检查是否是 FROM/JOIN 相关的节点
            if kind == "from_clause"
                || kind == "join_clause"
                || kind == "table_reference"
                || kind == "table_expression"
            {
                return true;
            }
            // 检查父节点文本是否包含 FROM/JOIN
            if let Ok(text) = n.utf8_text(source.as_bytes()) {
                let upper = text.to_uppercase();
                if upper.contains("FROM") || upper.contains("JOIN") {
                    return true;
                }
            }
            current = n.parent();
        }
        false
    }

    /// 提取查询中的列名
    pub fn extract_columns(&self, tree: &Tree, source: &str) -> Vec<String> {
        let mut columns = Vec::new();
        self.extract_columns_recursive(tree.root_node(), source, &mut columns);
        columns
    }

    /// 递归提取列名
    /// 参考 sqls 的实现：查找 SELECT/WHERE/ORDER BY 等子句中的列名
    fn extract_columns_recursive(&self, node: Node, source: &str, columns: &mut Vec<String>) {
        let node_kind = node.kind();

        // 参考 sqls：查找 column_name, column_reference, column_identifier 等节点
        if node_kind == "column_name"
            || node_kind == "column_reference"
            || node_kind == "column_identifier"
            || node_kind == "column"
            || (node_kind == "identifier" && self.is_in_column_context(node, source))
        {
            if let Ok(text) = node.utf8_text(source.as_bytes()) {
                let text = text.trim();
                // 过滤关键字和操作符
                if !text.is_empty()
                    && !Keywords::is_keyword(text)
                    && !Operators::is_operator(text)
                    && !Delimiters::is_delimiter(text)
                    && text != "*"  // 排除通配符
                    && !columns.contains(&text.to_string())
                {
                    columns.push(text.to_string());
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_columns_recursive(child, source, columns);
        }
    }

    /// 检查节点是否在列上下文中（SELECT/WHERE/ORDER BY 等）
    pub fn is_in_column_context(&self, node: Node, source: &str) -> bool {
        let mut current = Some(node);
        while let Some(n) = current {
            let kind = n.kind();
            // 检查是否是列相关的节点
            if kind == "select_list"
                || kind == "select_expression"
                || kind == "where_clause"
                || kind == "order_by_clause"
                || kind == "group_by_clause"
                || kind == "having_clause"
                || kind == "column_reference"
            {
                return true;
            }
            // 检查父节点文本是否包含 SELECT/WHERE/ORDER 等
            if let Ok(text) = n.utf8_text(source.as_bytes()) {
                let upper = text.to_uppercase();
                if upper.contains("SELECT")
                    || upper.contains("WHERE")
                    || upper.contains("ORDER")
                    || upper.contains("GROUP")
                    || upper.contains("HAVING")
                {
                    return true;
                }
            }
            current = n.parent();
        }
        false
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

    /// 分析补全上下文 (Text-based heuristics for reliability)
    /// 分析补全上下文 (AST-based)
    /// Uses Tree-sitter AST node traversal for robust context detection
    pub fn analyze_completion_context(
        &self,
        node: Node,
        source: &str,
        _position: Position,
    ) -> CompletionContext {
        let mut current_node = Some(node);

        // First, check if we are inside a specific node type that dictates context directly
        // Usually we want to find the clause we are in (SELECT, FROM, WHERE, etc.)
        while let Some(n) = current_node {
            match n.kind() {
                // SELECT clause
                "select_clause" | "select_list" => {
                    // Check if we are in a column position or after a dot
                    // (This might need refinement, but select_clause generally means column completion)
                    return CompletionContext::SelectClause;
                }
                // FROM clause
                "from_clause" | "table_references" => {
                    return CompletionContext::FromClause;
                }
                // JOIN clause
                // Tree-sitter sql often structures joins inside table_references or as specific join nodes
                // Depending on the exact grammar structure.
                // Assuming "join_clause" or similar if available, or fallback to heuristics if tree-sitter is murky here.
                // Note: tree-sitter-sql often puts joins in `table_expression` or `joined_table`
                "joined_table" => {
                    // Verify if we are at the ON part or table part
                    // For now, treat as JoinClause
                    return CompletionContext::JoinClause;
                }
                // WHERE clause
                "where_clause" => {
                    return CompletionContext::WhereClause;
                }
                // ORDER BY clause
                "order_by_clause" => {
                    return CompletionContext::OrderByClause;
                }
                // GROUP BY clause
                "group_by_clause" => {
                    return CompletionContext::GroupByClause;
                }
                // HAVING clause
                "having_clause" => {
                    return CompletionContext::HavingClause;
                }
                // If we hit the statement level, we might be in a specific position
                "select_statement" => {
                    // If we traversed up to statement without hitting a clause,
                    // we might be in an empty space between clauses or at the end.
                    // Fallback or check children?
                    // For robustness, let's keep searching up or break if root.
                }
                _ => {}
            }
            current_node = n.parent();
        }

        // Fallback: Use simple heuristics if AST traversal didn't find a specific clause
        // This handles cases where Syntax is broken (common during typing) and AST is incomplete
        self.analyze_completion_context_fallback(source, _position)
    }

    /// Fallback heuristics for context analysis when AST is incomplete
    fn analyze_completion_context_fallback(
        &self,
        source: &str,
        position: Position,
    ) -> CompletionContext {
        // Convert LSP position (line, character) to byte offset
        let lines: Vec<&str> = source.lines().collect();
        let mut cursor_offset = 0;

        // Add bytes for all complete lines before cursor
        for (line_idx, line) in lines.iter().enumerate() {
            if line_idx < position.line as usize {
                cursor_offset += line.len() + 1; // +1 for newline
            } else if line_idx == position.line as usize {
                // Add characters up to cursor position in target line
                cursor_offset += position.character.min(line.len() as u32) as usize;
                break;
            }
        }

        // Extract text before cursor
        let text_before = if cursor_offset <= source.len() {
            &source[..cursor_offset]
        } else {
            source
        };
        let text_upper = text_before.to_uppercase();

        // Priority 1: Check for table/alias column access (ends with .)
        if text_before.trim_end().ends_with('.') {
            return CompletionContext::TableColumn;
        }

        // Priority 2: Find the last keyword to determine context

        // Check for WHERE clause
        if let Some(where_pos) = text_upper.rfind("WHERE") {
            let has_later_keyword = text_upper[where_pos..]
                .find("ORDER BY")
                .or_else(|| text_upper[where_pos..].find("GROUP BY"))
                .or_else(|| text_upper[where_pos..].find("LIMIT"))
                .or_else(|| text_upper[where_pos..].find("HAVING"));

            if has_later_keyword.is_none() {
                return CompletionContext::WhereClause;
            }
        }

        // Check for JOIN clause (basic check)
        if let Some(join_pos) = text_upper.rfind("JOIN") {
            let after_join = &text_upper[join_pos + 4..].trim_start();
            if !after_join.starts_with("ON") && !after_join.contains(" ON ") {
                return CompletionContext::JoinClause;
            }
        }

        // IMPORTANT: Check for HAVING before GROUP BY and ORDER BY
        // because HAVING comes after GROUP BY in SQL syntax
        // When both exist, we want to detect the later one
        if text_upper.rfind("HAVING").is_some() {
            return CompletionContext::HavingClause;
        }

        // Check for ORDER BY clause
        if text_upper.rfind("ORDER BY").is_some() {
            return CompletionContext::OrderByClause;
        }

        // Check for GROUP BY clause
        if text_upper.rfind("GROUP BY").is_some() {
            return CompletionContext::GroupByClause;
        }

        // Check for FROM clause
        if let Some(from_pos) = text_upper.rfind("FROM") {
            let after_from = &text_upper[from_pos + 4..].trim_start();
            if !after_from.contains("WHERE")
                && !after_from.contains("JOIN")
                && !after_from.contains("ORDER")
                && !after_from.contains("GROUP")
                && !after_from.contains("LIMIT")
            {
                return CompletionContext::FromClause;
            }
        }

        // Check for SELECT clause
        if let Some(select_pos) = text_upper.rfind("SELECT") {
            let after_select = &text_upper[select_pos + 6..].trim_start();
            if !after_select.contains("FROM") {
                return CompletionContext::SelectClause;
            }
        }

        CompletionContext::Default
    }

    /// 获取表名（用于 TableColumn 上下文）
    /// 如果光标在 table.column 的位置，返回表名
    pub fn get_table_name_for_column(&self, node: Node, source: &str) -> Option<String> {
        let mut current = Some(node);

        while let Some(n) = current {
            let kind = n.kind();

            // Try to extract from text directly first, if it looks like "table." or "table.col"
            if let Ok(text) = n.utf8_text(source.as_bytes()) {
                if let Some(dot_pos) = text.find('.') {
                    let table_name = text[..dot_pos].trim();
                    if !table_name.is_empty() && !Keywords::is_keyword(table_name) {
                        return Some(table_name.to_string());
                    }
                }
            }

            // 查找 member_expression 或 dotted_name
            if kind == "member_expression" || kind == "dotted_name" {
                if let Ok(text) = n.utf8_text(source.as_bytes()) {
                    if let Some(dot_pos) = text.find('.') {
                        let table_name = text[..dot_pos].trim();
                        if !table_name.is_empty() && !Keywords::is_keyword(table_name) {
                            return Some(table_name.to_string());
                        }
                    }
                }
            }

            // 检查父节点
            if let Some(parent) = n.parent() {
                if let Ok(text) = parent.utf8_text(source.as_bytes()) {
                    if let Some(dot_pos) = text.find('.') {
                        let table_name = text[..dot_pos].trim();
                        if !table_name.is_empty() && !Keywords::is_keyword(table_name) {
                            return Some(table_name.to_string());
                        }
                    }
                }
            }

            current = n.parent();
        }

        None
    }
}

impl Default for SqlParser {
    fn default() -> Self {
        Self::new()
    }
}

/// AST 节点信息
#[derive(Debug, Clone)]
pub struct AstNode {
    pub node_type: String,
    pub position: Range,
    pub text: String,
}

impl SqlParser {
    /// 提取表别名映射 (Alias -> Table Name)
    /// Uses text-based extraction for reliability
    pub fn extract_aliases(
        &self,
        _tree: &Tree,
        source: &str,
    ) -> std::collections::HashMap<String, String> {
        let mut aliases = std::collections::HashMap::new();
        let source_upper = source.to_uppercase();

        // Pattern: FROM/JOIN table_name alias
        // Look for FROM/JOIN keywords followed by identifiers
        let keywords = ["FROM", "JOIN", "INNER JOIN", "LEFT JOIN", "RIGHT JOIN"];

        for keyword in keywords {
            let mut search_pos = 0;
            while let Some(keyword_pos) = source_upper[search_pos..].find(keyword) {
                let abs_pos = search_pos + keyword_pos + keyword.len();

                // Extract text after keyword
                let after_keyword = &source[abs_pos..].trim_start();

                // Try to extract "table_name alias" pattern
                // Split by whitespace and take first two tokens
                let tokens: Vec<&str> = after_keyword
                    .split_whitespace()
                    .take(3) // table, optional AS, alias
                    .collect();

                if tokens.len() >= 2 {
                    let table_name = tokens[0];
                    let alias_candidate =
                        if tokens.len() >= 3 && tokens[1].eq_ignore_ascii_case("AS") {
                            tokens[2]
                        } else if !tokens[1].eq_ignore_ascii_case("WHERE")
                            && !tokens[1].eq_ignore_ascii_case("ON")
                            && !tokens[1].eq_ignore_ascii_case("JOIN")
                            && !tokens[1].eq_ignore_ascii_case("INNER")
                            && !tokens[1].eq_ignore_ascii_case("LEFT")
                            && !tokens[1].eq_ignore_ascii_case("RIGHT")
                        {
                            tokens[1]
                        } else {
                            ""
                        };

                    if !alias_candidate.is_empty() && !Keywords::is_keyword(alias_candidate) {
                        aliases.insert(alias_candidate.to_string(), table_name.to_string());
                    }
                }

                search_pos = abs_pos + 1;
            }
        }

        aliases
    }

    /// 提取SQL中引用的表名（从FROM和JOIN子句）
    pub fn extract_referenced_tables(&self, _tree: &Tree, source: &str) -> Vec<String> {
        let mut tables = Vec::new();
        let source_upper = source.to_uppercase();

        let keywords = ["FROM", "JOIN", "INNER JOIN", "LEFT JOIN", "RIGHT JOIN"];

        for keyword in keywords {
            let mut search_pos = 0;
            while let Some(keyword_pos) = source_upper[search_pos..].find(keyword) {
                let abs_pos = search_pos + keyword_pos + keyword.len();
                let after_keyword = &source[abs_pos..].trim_start();

                // Extract first token (table name)
                if let Some(first_token) = after_keyword.split_whitespace().next() {
                    if !Keywords::is_keyword(first_token)
                        && !tables.contains(&first_token.to_string())
                    {
                        tables.push(first_token.to_string());
                    }
                }

                search_pos = abs_pos + 1;
            }
        }

        tables
    }

    /// 将 Tree-sitter Node 转换为 AstNode
    pub fn node_to_ast_node(&self, node: Node, source: &str) -> AstNode {
        AstNode {
            node_type: node.kind().to_string(),
            position: self.node_range(node),
            text: self.node_text(node, source),
        }
    }
}
