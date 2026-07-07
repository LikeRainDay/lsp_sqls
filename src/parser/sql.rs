//! SQL 解析器实现
//! 参考 sqls-server/sqls 的实现
//! https://github.com/sqls-server/sqls/tree/master/parser

use crate::token::{Delimiters, Keywords, Operators, Token, TokenType};
use std::collections::{HashMap, HashSet};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};
use tree_sitter::{Node, Parser, Tree};

/// 补全上下文类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// 在 FROM 子句中，应该补全表名
    FromClause,
    /// 在 FROM 关系目标之后，应该补全 JOIN/WHERE/GROUP 等后续结构
    FromContinuationClause,
    /// 在 SELECT 子句中，应该补全列名和关键字
    SelectClause,
    /// 在 SELECT 表达式之后，应该补全 FROM/AS/逗号
    SelectContinuationClause,
    /// 在 WHERE 子句中，应该补全列名、操作符、关键字
    WhereClause,
    /// 在谓词或赋值操作符之后，应该补全值表达式
    ExpressionValueClause,
    /// 在完整谓词或赋值之后，应该补全连接词或后续子句
    PredicateContinuationClause,
    /// 在 CASE THEN/ELSE 之后，应该补全结果表达式
    CaseResultClause,
    /// 在 CASE 结果表达式之后，应该补全 WHEN/ELSE/END
    CaseContinuationClause,
    /// 在表名后（如 table.），应该补全列名
    TableColumn,
    /// 在 JOIN 子句中，应该补全表名
    JoinClause,
    /// 在 JOIN 关系目标之后，应该补全 ON/USING 条件
    JoinConditionClause,
    /// 在 ORDER BY 子句中，应该补全列名
    OrderByClause,
    /// 在 ORDER BY 表达式之后，应该补全排序方向和 NULLS 规则
    OrderDirectionClause,
    /// 在 GROUP BY 子句中，应该补全列名
    GroupByClause,
    /// 在 GROUP BY 表达式之后，应该补全后续分组子句
    GroupByContinuationClause,
    /// 在 HAVING 子句中，应该补全列名和关键字
    HavingClause,
    /// 在 JOIN ... USING (...) 子句中，应该补全可共享的列名
    UsingClause,
    /// 在 REFERENCES table (...) 子句中，应该补全被引用表的列名
    ReferenceColumnClause,
    /// 在 REFERENCES table 之后，应该补全外键引用动作
    ReferenceActionClause,
    /// 在 REFERENCES table ON DELETE/UPDATE 之后，应该补全外键规则
    ReferenceRuleClause,
    /// 在 ALTER TABLE 的列目标位置，应该补全当前表列名
    ColumnTargetClause,
    /// 在 ALTER TABLE 的约束目标位置，应该补全当前表约束名
    ConstraintTargetClause,
    /// 在 ALTER TABLE 表名之后，应该补全结构操作
    AlterTableActionClause,
    /// 在 INSERT INTO 表名之后，应该补全插入动作
    InsertActionClause,
    /// 在 INSERT ... VALUES (...) 值位置，应该补全值相关关键字
    InsertValueClause,
    /// 在 UPDATE 表名之后，应该补全更新动作
    UpdateActionClause,
    /// 在 DELETE FROM 表名之后，应该补全删除动作
    DeleteActionClause,
    /// 在索引名目标位置，应该补全 schema 或当前表的索引名
    IndexTargetClause,
    /// 在列定义的数据类型位置，应该补全当前方言的数据类型
    DataTypeClause,
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
            Self::filter_trailing_incomplete_diagnostics(sql, &mut diagnostics);
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

    /// Convert an LSP UTF-16 position to the byte-based column expected by tree-sitter.
    pub fn lsp_position_to_byte_position(source: &str, position: Position) -> Position {
        crate::position::lsp_position_to_byte_position(source, position)
    }

    fn filter_trailing_incomplete_diagnostics(source: &str, diagnostics: &mut Vec<Diagnostic>) {
        if !Self::is_trailing_incomplete_statement(source) {
            return;
        }

        let eof = Self::position_at_end(source);
        diagnostics.retain(|diagnostic| !Self::diagnostic_reaches_position(diagnostic, eof));
    }

    fn position_at_end(source: &str) -> Position {
        let mut line = 0;
        let mut character = 0;

        for ch in source.chars() {
            if ch == '\n' {
                line += 1;
                character = 0;
            } else {
                character += ch.len_utf8() as u32;
            }
        }

        Position { line, character }
    }

    fn diagnostic_reaches_position(diagnostic: &Diagnostic, position: Position) -> bool {
        diagnostic.range.end.line > position.line
            || (diagnostic.range.end.line == position.line
                && diagnostic.range.end.character >= position.character.saturating_sub(1))
            || (diagnostic.range.start.line == position.line
                && diagnostic.range.start.character >= position.character.saturating_sub(1))
    }

    fn is_trailing_incomplete_statement(source: &str) -> bool {
        let searchable_source = Self::mask_sql_noise(source);
        let trimmed = searchable_source.trim_end();
        if trimmed.is_empty() || trimmed.ends_with(';') {
            return false;
        }

        let source_upper = trimmed.to_ascii_uppercase();
        let statement_start = source_upper
            .rfind(';')
            .map(|position| position + 1)
            .unwrap_or(0);
        let statement = source_upper[statement_start..].trim();
        if statement.is_empty() {
            return false;
        }

        let words = Self::statement_words(statement);
        let incomplete_phrases: &[&[&str]] = &[
            &["SELECT"],
            &["SELECT", "DISTINCT"],
            &["WITH"],
            &["FROM"],
            &["JOIN"],
            &["INNER", "JOIN"],
            &["LEFT", "JOIN"],
            &["RIGHT", "JOIN"],
            &["FULL", "JOIN"],
            &["CROSS", "JOIN"],
            &["ON"],
            &["USING"],
            &["WHERE"],
            &["AND"],
            &["OR"],
            &["NOT"],
            &["GROUP", "BY"],
            &["ORDER", "BY"],
            &["HAVING"],
            &["LIMIT"],
            &["OFFSET"],
            &["INSERT"],
            &["INSERT", "INTO"],
            &["UPDATE"],
            &["DELETE", "FROM"],
            &["CREATE", "TABLE"],
            &["ALTER", "TABLE"],
            &["DROP", "TABLE"],
            &["SET"],
            &["VALUES"],
        ];

        if incomplete_phrases
            .iter()
            .any(|phrase| Self::words_end_with(&words, phrase))
        {
            return true;
        }

        let last_char = statement.chars().rev().find(|ch| !ch.is_whitespace());
        matches!(
            last_char,
            Some(',' | '.' | '(' | '=' | '<' | '>' | '!' | '+' | '-' | '/' | '%')
        )
    }

    fn statement_words(statement_upper: &str) -> Vec<&str> {
        statement_upper
            .split(|ch: char| !Self::is_identifier_char(ch))
            .filter(|word| !word.is_empty())
            .collect()
    }

    fn words_end_with(words: &[&str], phrase: &[&str]) -> bool {
        words.len() >= phrase.len()
            && words[words.len() - phrase.len()..]
                .iter()
                .zip(phrase.iter())
                .all(|(word, expected)| word == expected)
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
                let table_name = Self::normalize_identifier(text);
                // 过滤关键字和操作符
                if !text.is_empty()
                    && !Keywords::is_keyword(text)
                    && !Operators::is_operator(text)
                    && !Delimiters::is_delimiter(text)
                    && !table_name.is_empty()
                    && !tables.contains(&table_name)
                {
                    tables.push(table_name);
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
                let column_name = Self::normalize_identifier(text);
                // 过滤关键字和操作符
                if !text.is_empty()
                    && !Keywords::is_keyword(text)
                    && !Operators::is_operator(text)
                    && !Delimiters::is_delimiter(text)
                    && text != "*"  // 排除通配符
                    && !column_name.is_empty()
                    && !columns.contains(&column_name)
                {
                    columns.push(column_name);
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
        position: Position,
    ) -> CompletionContext {
        if Self::expression_value_context_at_position(source, position) {
            return CompletionContext::ExpressionValueClause;
        }

        if Self::predicate_continuation_context_at_position(source, position) {
            return CompletionContext::PredicateContinuationClause;
        }

        if Self::case_when_condition_context_at_position(source, position) {
            return CompletionContext::WhereClause;
        }

        if Self::case_result_context_at_position(source, position) {
            return CompletionContext::CaseResultClause;
        }

        if Self::case_continuation_context_at_position(source, position) {
            return CompletionContext::CaseContinuationClause;
        }

        if Self::select_continuation_context_at_position(source, position) {
            return CompletionContext::SelectContinuationClause;
        }

        if let Some(context) = Self::analyze_completed_keyword_context(source, position) {
            return context;
        }

        if Self::join_using_column_context_at_position(source, position) {
            return CompletionContext::UsingClause;
        }

        if Self::order_direction_context_at_position(source, position) {
            return CompletionContext::OrderDirectionClause;
        }

        if Self::group_by_continuation_context_at_position(source, position) {
            return CompletionContext::GroupByContinuationClause;
        }

        if Self::reference_column_context_at_position(source, position) {
            return CompletionContext::ReferenceColumnClause;
        }

        if Self::reference_rule_context_at_position(source, position) {
            return CompletionContext::ReferenceRuleClause;
        }

        if Self::reference_action_context_at_position(source, position) {
            return CompletionContext::ReferenceActionClause;
        }

        if Self::reference_relation_target_context_at_position(source, position) {
            return CompletionContext::FromClause;
        }

        if Self::data_type_context_at_position(source, position) {
            return CompletionContext::DataTypeClause;
        }

        if let Some(context) = Self::analyze_ddl_target_context_at_position(source, position) {
            return context;
        }

        if Self::alter_table_action_context_at_position(source, position) {
            return CompletionContext::AlterTableActionClause;
        }

        if let Some(context) = Self::analyze_dml_action_context_at_position(source, position) {
            return context;
        }

        if Self::insert_value_context_at_position(source, position) {
            return CompletionContext::InsertValueClause;
        }

        if let Some(context) =
            Self::analyze_relation_continuation_context_at_position(source, position)
        {
            return context;
        }

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
                "using_clause" => {
                    return CompletionContext::UsingClause;
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
        self.analyze_completion_context_fallback(source, position)
    }

    fn byte_offset_for_position(source: &str, position: Position) -> usize {
        let lines: Vec<&str> = source.lines().collect();
        let mut cursor_offset = 0;

        for (line_idx, line) in lines.iter().enumerate() {
            if line_idx < position.line as usize {
                cursor_offset += line.len() + 1;
            } else if line_idx == position.line as usize {
                cursor_offset += position.character.min(line.len() as u32) as usize;
                break;
            }
        }

        cursor_offset.min(source.len())
    }

    fn analyze_completed_keyword_context(
        source: &str,
        position: Position,
    ) -> Option<CompletionContext> {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);

        if text_before.trim_end().ends_with('.') {
            return Some(CompletionContext::TableColumn);
        }

        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement = text_upper[statement_start..].trim_end();
        if statement.is_empty() {
            return None;
        }

        let words = Self::statement_words(statement);
        if Self::words_end_with(&words, &["SELECT"])
            || Self::words_end_with(&words, &["SELECT", "DISTINCT"])
        {
            return Some(CompletionContext::SelectClause);
        }
        if Self::words_end_with(&words, &["FROM"]) {
            return Some(CompletionContext::FromClause);
        }
        if Self::words_end_with(&words, &["INSERT", "INTO"])
            || Self::words_end_with(&words, &["UPDATE"])
            || Self::words_end_with(&words, &["DELETE", "FROM"])
            || Self::words_end_with(&words, &["TRUNCATE", "TABLE"])
            || Self::words_end_with(&words, &["ALTER", "TABLE"])
            || Self::words_end_with(&words, &["DROP", "TABLE"])
            || Self::words_end_with(&words, &["DROP", "VIEW"])
        {
            return Some(CompletionContext::FromClause);
        }
        if Self::words_end_with(&words, &["JOIN"])
            || Self::words_end_with(&words, &["INNER", "JOIN"])
            || Self::words_end_with(&words, &["LEFT", "JOIN"])
            || Self::words_end_with(&words, &["RIGHT", "JOIN"])
            || Self::words_end_with(&words, &["FULL", "JOIN"])
            || Self::words_end_with(&words, &["CROSS", "JOIN"])
        {
            return Some(CompletionContext::JoinClause);
        }
        if Self::words_end_with(&words, &["ON"]) {
            if Self::ddl_on_relation_target_at_position(source, position) {
                return Some(CompletionContext::FromClause);
            }
            return Some(CompletionContext::WhereClause);
        }
        if Self::words_end_with(&words, &["USING"]) && Self::is_join_using_column_context(statement)
        {
            return Some(CompletionContext::UsingClause);
        }
        if Self::words_end_with(&words, &["WHERE"])
            || Self::words_end_with(&words, &["AND"])
            || Self::words_end_with(&words, &["OR"])
            || Self::words_end_with(&words, &["NOT"])
            || Self::words_end_with(&words, &["WHEN"])
            || Self::words_end_with(&words, &["SET"])
        {
            return Some(CompletionContext::WhereClause);
        }
        if Self::words_end_with(&words, &["RETURNING"]) {
            return Some(CompletionContext::SelectClause);
        }
        if Self::words_end_with(&words, &["ORDER", "BY"]) {
            return Some(CompletionContext::OrderByClause);
        }
        if Self::words_end_with(&words, &["GROUP", "BY"]) {
            return Some(CompletionContext::GroupByClause);
        }
        if Self::words_end_with(&words, &["HAVING"]) {
            return Some(CompletionContext::HavingClause);
        }
        if Self::words_end_with(&words, &["THEN"]) || Self::words_end_with(&words, &["ELSE"]) {
            return Some(CompletionContext::CaseResultClause);
        }

        None
    }

    /// Fallback heuristics for context analysis when AST is incomplete
    fn analyze_completion_context_fallback(
        &self,
        source: &str,
        position: Position,
    ) -> CompletionContext {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        // Extract text before cursor
        let text_before = if cursor_offset <= source.len() {
            &source[..cursor_offset]
        } else {
            source
        };
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let raw_text_upper = text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];
        let raw_statement_upper = &raw_text_upper[statement_start..];

        // Priority 1: Check for table/alias column access (ends with .)
        if text_before.trim_end().ends_with('.') {
            return CompletionContext::TableColumn;
        }

        if Self::is_insert_column_context(statement_upper) {
            return CompletionContext::SelectClause;
        }

        if Self::is_ddl_on_column_context(statement_upper) {
            return CompletionContext::SelectClause;
        }

        if Self::is_join_using_column_context(statement_upper) {
            return CompletionContext::UsingClause;
        }

        if Self::is_order_direction_context(statement_upper) {
            return CompletionContext::OrderDirectionClause;
        }

        if Self::is_group_by_continuation_context(statement_upper) {
            return CompletionContext::GroupByContinuationClause;
        }

        if Self::is_reference_column_context(statement_upper) {
            return CompletionContext::ReferenceColumnClause;
        }

        if Self::is_reference_rule_context(statement_upper) {
            return CompletionContext::ReferenceRuleClause;
        }

        if Self::is_reference_action_context(statement_upper) {
            return CompletionContext::ReferenceActionClause;
        }

        if Self::is_reference_relation_target_context(statement_upper) {
            return CompletionContext::FromClause;
        }

        if Self::is_data_type_context(statement_upper) {
            return CompletionContext::DataTypeClause;
        }

        if let Some(context) = Self::analyze_ddl_target_context(statement_upper) {
            return context;
        }

        if Self::is_alter_table_action_context(statement_upper) {
            return CompletionContext::AlterTableActionClause;
        }

        if let Some(context) = Self::analyze_dml_action_context(statement_upper) {
            return context;
        }

        if Self::is_insert_value_context(statement_upper) {
            return CompletionContext::InsertValueClause;
        }

        if Self::is_expression_value_context(statement_upper, raw_statement_upper) {
            return CompletionContext::ExpressionValueClause;
        }

        if Self::is_predicate_continuation_context(statement_upper, raw_statement_upper) {
            return CompletionContext::PredicateContinuationClause;
        }

        if Self::is_case_result_context(raw_statement_upper) {
            return CompletionContext::CaseResultClause;
        }

        if Self::is_case_continuation_context(raw_statement_upper) {
            return CompletionContext::CaseContinuationClause;
        }

        if Self::is_update_set_context(statement_upper) {
            return CompletionContext::WhereClause;
        }

        if let Some(context) = Self::analyze_relation_continuation_context(statement_upper) {
            return context;
        }

        if Self::is_select_continuation_context(raw_statement_upper) {
            return CompletionContext::SelectContinuationClause;
        }

        if Self::is_ddl_on_relation_target_context(statement_upper) {
            return CompletionContext::FromClause;
        }

        if Self::is_relation_target_context(statement_upper) {
            return CompletionContext::FromClause;
        }

        // Priority 2: Find the last complete keyword to determine context

        // Check for WHERE clause
        if let Some(where_pos) = Self::previous_keyword_position(&text_upper, "WHERE") {
            let has_later_keyword = Self::statement_has_any_keyword(
                &text_upper,
                where_pos + "WHERE".len(),
                text_upper.len(),
                &["ORDER BY", "GROUP BY", "LIMIT", "HAVING"],
            );

            if !has_later_keyword {
                return CompletionContext::WhereClause;
            }
        }

        // Check for JOIN clause (basic check)
        if let Some(join_pos) = Self::previous_keyword_position(&text_upper, "JOIN") {
            let after_join = &text_upper[join_pos + 4..].trim_start();
            if !after_join.starts_with("ON") && !after_join.contains(" ON ") {
                return CompletionContext::JoinClause;
            }
        }

        let latest_grouping_clause = [
            Self::previous_keyword_position(&text_upper, "GROUP BY")
                .map(|position| (position, CompletionContext::GroupByClause)),
            Self::previous_keyword_position(&text_upper, "HAVING")
                .map(|position| (position, CompletionContext::HavingClause)),
            Self::previous_keyword_position(&text_upper, "ORDER BY")
                .map(|position| (position, CompletionContext::OrderByClause)),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|(position, _)| *position);

        if let Some((_, context)) = latest_grouping_clause {
            return context;
        }

        // Check for FROM clause
        if let Some(from_pos) = Self::previous_keyword_position(&text_upper, "FROM") {
            let has_later_keyword = Self::statement_has_any_keyword(
                &text_upper,
                from_pos + "FROM".len(),
                text_upper.len(),
                &["WHERE", "JOIN", "ORDER BY", "GROUP BY", "LIMIT"],
            );

            if !has_later_keyword {
                return CompletionContext::FromClause;
            }
        }

        // Check for SELECT clause
        if let Some(select_pos) = Self::previous_keyword_position(&text_upper, "SELECT") {
            if !Self::contains_keyword_between(
                &text_upper,
                "FROM",
                select_pos + "SELECT".len(),
                text_upper.len(),
            ) {
                return CompletionContext::SelectClause;
            }
        }

        CompletionContext::Default
    }

    fn is_relation_target_context(statement_upper: &str) -> bool {
        let relation_targets = [
            ("INSERT INTO", &["VALUES", "SELECT", "RETURNING"][..]),
            ("UPDATE", &["SET", "WHERE", "RETURNING"][..]),
            ("DELETE FROM", &["WHERE", "RETURNING", "USING"][..]),
            ("TRUNCATE TABLE", &[][..]),
            (
                "ALTER TABLE",
                &[
                    "ADD", "ALTER", "DROP", "RENAME", "OWNER", "SET", "RESET", "VALIDATE",
                    "ENABLE", "DISABLE",
                ][..],
            ),
            ("DROP TABLE", &[][..]),
            ("DROP VIEW", &[][..]),
        ];

        relation_targets.iter().any(|(phrase, terminators)| {
            let Some(position) = Self::previous_keyword_position(statement_upper, phrase) else {
                return false;
            };
            let after_phrase = position + phrase.len();
            !Self::statement_has_any_keyword(
                statement_upper,
                after_phrase,
                statement_upper.len(),
                terminators,
            )
        })
    }

    pub fn ddl_on_relation_target_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = text_upper[statement_start..].trim_end();

        Self::statement_words(statement_upper)
            .last()
            .is_some_and(|word| *word == "ON")
            && Self::is_ddl_on_relation_target_context(statement_upper)
    }

    fn is_ddl_on_relation_target_context(statement_upper: &str) -> bool {
        let Some(on_position) = Self::previous_keyword_position(statement_upper, "ON") else {
            return false;
        };
        if !Self::should_read_on_relation(statement_upper, on_position) {
            return false;
        }

        let after_on = on_position + "ON".len();
        let after_on_text = &statement_upper[after_on..];
        !after_on_text.contains('(')
            && !Self::statement_has_any_keyword(
                statement_upper,
                after_on,
                statement_upper.len(),
                &["WHERE", "EXECUTE", "FOR"],
            )
    }

    fn is_ddl_on_column_context(statement_upper: &str) -> bool {
        let Some(on_position) = Self::previous_keyword_position(statement_upper, "ON") else {
            return false;
        };
        if !Self::should_read_on_relation(statement_upper, on_position) {
            return false;
        }

        statement_upper[on_position + "ON".len()..].contains('(')
    }

    fn is_insert_column_context(statement_upper: &str) -> bool {
        let Some(into_position) = Self::previous_keyword_position(statement_upper, "INSERT INTO")
        else {
            return false;
        };
        let after_into = into_position + "INSERT INTO".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_into,
            statement_upper.len(),
            &["VALUES", "SELECT", "RETURNING"],
        ) {
            return false;
        }

        statement_upper[after_into..].contains('(')
    }

    fn is_insert_value_context(statement_upper: &str) -> bool {
        let Some(into_position) = Self::previous_keyword_position(statement_upper, "INSERT INTO")
        else {
            return false;
        };
        let Some(values_position) = Self::previous_keyword_position(statement_upper, "VALUES")
            .or_else(|| Self::previous_keyword_position(statement_upper, "VALUE"))
        else {
            return false;
        };
        if values_position < into_position {
            return false;
        }

        let after_values = values_position
            + if statement_upper[values_position..].starts_with("VALUES") {
                "VALUES".len()
            } else {
                "VALUE".len()
            };
        if Self::statement_has_any_keyword(
            statement_upper,
            after_values,
            statement_upper.len(),
            &["RETURNING", "ON CONFLICT", "ON DUPLICATE", "SELECT"],
        ) {
            return false;
        }

        let segment = &statement_upper[after_values..];
        let Some(last_open) = segment.rfind('(') else {
            return false;
        };
        let after_open = &segment[last_open + 1..];
        if after_open.contains(')') {
            return false;
        }

        let trimmed = after_open.trim_start();
        let trimmed_end = trimmed.trim_end();
        if trimmed_end.is_empty() || trimmed_end.ends_with(',') {
            return true;
        }

        let words = Self::statement_words(trimmed_end);
        if words.is_empty() || trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return false;
        }

        words
            .last()
            .is_some_and(|word| Self::is_value_keyword_prefix(word))
    }

    fn latest_predicate_clause(statement_upper: &str) -> Option<(usize, &'static str)> {
        [
            (
                "WHERE",
                Self::previous_keyword_position(statement_upper, "WHERE"),
            ),
            (
                "HAVING",
                Self::previous_keyword_position(statement_upper, "HAVING"),
            ),
            ("ON", Self::previous_keyword_position(statement_upper, "ON")),
            (
                "WHEN",
                Self::previous_keyword_position(statement_upper, "WHEN"),
            ),
            (
                "SET",
                Self::previous_keyword_position(statement_upper, "SET"),
            ),
        ]
        .into_iter()
        .filter_map(|(clause, position)| position.map(|position| (position, clause)))
        .max_by_key(|(position, _)| *position)
    }

    fn predicate_clause_has_later_terminator(
        statement_upper: &str,
        after_clause: usize,
        clause: &str,
    ) -> bool {
        let terminators = match clause {
            "SET" => &["WHERE", "RETURNING"][..],
            "WHERE" | "ON" => &[
                "GROUP BY",
                "ORDER BY",
                "HAVING",
                "LIMIT",
                "RETURNING",
                "UNION",
            ][..],
            "HAVING" => &["ORDER BY", "LIMIT", "UNION"][..],
            "WHEN" => &["THEN", "ELSE", "END"][..],
            _ => &[][..],
        };

        Self::statement_has_any_keyword(
            statement_upper,
            after_clause,
            statement_upper.len(),
            terminators,
        )
    }

    fn is_expression_value_context(statement_upper: &str, raw_statement_upper: &str) -> bool {
        let Some((clause_position, clause)) = Self::latest_predicate_clause(statement_upper) else {
            return false;
        };

        let after_clause = clause_position + clause.len();
        if Self::predicate_clause_has_later_terminator(statement_upper, after_clause, clause) {
            return false;
        }

        let segment = &statement_upper[after_clause..];
        let raw_segment = raw_statement_upper.get(after_clause..).unwrap_or(segment);
        let trimmed = segment.trim_end();
        if trimmed.is_empty() || trimmed.ends_with('.') {
            return false;
        }

        let Some(operator_end) = Self::latest_value_operator_end(trimmed) else {
            return false;
        };
        let raw_after_operator = raw_segment.get(operator_end..).unwrap_or("");
        let value_text = raw_after_operator.trim_start();
        let value_text_trimmed = value_text.trim_end();
        if value_text_trimmed.is_empty() {
            return true;
        }

        if raw_after_operator
            .chars()
            .last()
            .is_some_and(|ch| ch.is_whitespace())
        {
            if value_text_trimmed.ends_with(',')
                && Self::is_in_list_value_operator(trimmed, operator_end)
            {
                return true;
            }
            return false;
        }

        let value_text_upper = value_text_trimmed.to_ascii_uppercase();
        let words = Self::statement_words(&value_text_upper);
        let Some(prefix) = words.last().copied() else {
            return false;
        };
        if !Self::is_value_keyword_prefix(prefix) {
            return false;
        }

        let Some(prefix_start) = value_text_upper.rfind(prefix) else {
            return false;
        };
        let before_prefix = value_text_trimmed[..prefix_start].trim_end();
        before_prefix.trim().is_empty()
            || (before_prefix.ends_with(',')
                && Self::is_in_list_value_operator(trimmed, operator_end))
    }

    fn latest_value_operator_end(segment_upper: &str) -> Option<usize> {
        let trimmed_len = segment_upper.trim_end().len();
        let raw_segment = &segment_upper[..trimmed_len];
        let searchable_segment = Self::mask_nested_parenthesized_regions(raw_segment);
        let segment = searchable_segment.as_str();
        let mut best: Option<(usize, usize)> = None;

        let mut consider = |position: usize, end: usize| {
            if best.is_none_or(|(best_position, best_end)| {
                position > best_position || (position == best_position && end > best_end)
            }) {
                best = Some((position, end));
            }
        };

        [
            "!=", "<>", "<=", ">=", "=", "<", ">", "+", "-", "*", "/", "%",
        ]
        .iter()
        .filter_map(|operator| {
            segment
                .rfind(operator)
                .map(|position| (position, position + operator.len()))
        })
        .for_each(|(position, end)| consider(position, end));

        for operator in [
            "LIKE", "ILIKE", "RLIKE", "REGEXP", "IS NOT", "IS", "NOT IN", "IN", "BETWEEN",
        ] {
            if let Some(position) = Self::previous_keyword_position(segment, operator) {
                let mut end = position + operator.len();
                if matches!(operator, "IN" | "NOT IN") {
                    end = Self::extend_operator_end_through_open_paren(segment, end);
                }
                consider(position, end);
            }
        }

        if let Some(between_position) = Self::previous_keyword_position(segment, "BETWEEN") {
            let after_between = between_position + "BETWEEN".len();
            if let Some(and_relative) =
                Self::previous_keyword_position(&segment[after_between..], "AND")
            {
                let and_position = after_between + and_relative;
                consider(and_position, and_position + "AND".len());
            }
        }

        best.map(|(_, end)| end)
    }

    fn extend_operator_end_through_open_paren(segment_upper: &str, operator_end: usize) -> usize {
        let Some(after_operator) = segment_upper.get(operator_end..) else {
            return operator_end;
        };
        let leading_whitespace = after_operator
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>();
        if after_operator[leading_whitespace..].starts_with('(') {
            operator_end + leading_whitespace + 1
        } else {
            operator_end
        }
    }

    fn is_in_list_value_operator(segment_upper: &str, operator_end: usize) -> bool {
        let trimmed_len = segment_upper.trim_end().len();
        let raw_segment = &segment_upper[..trimmed_len];
        let searchable_segment = Self::mask_nested_parenthesized_regions(raw_segment);
        let Some(before_operator_end) = searchable_segment.get(..operator_end) else {
            return false;
        };
        let before_operator_end = before_operator_end.trim_end();
        if !before_operator_end.ends_with('(') {
            return false;
        }

        let before_open = before_operator_end[..before_operator_end.len() - 1].trim_end();
        let words = Self::statement_words(before_open);
        matches!(words.as_slice(), [.., "IN"])
    }

    fn is_predicate_continuation_context(statement_upper: &str, raw_statement_upper: &str) -> bool {
        let Some((clause_position, clause)) = Self::latest_predicate_clause(statement_upper) else {
            return false;
        };

        let after_clause = clause_position + clause.len();
        if Self::predicate_clause_has_later_terminator(statement_upper, after_clause, clause) {
            return false;
        }

        let segment = &statement_upper[after_clause..];
        let raw_segment = raw_statement_upper.get(after_clause..).unwrap_or(segment);
        let trimmed = segment.trim_end();
        if trimmed.is_empty() || trimmed.ends_with('.') {
            return false;
        }

        let Some(operator_end) = Self::latest_value_operator_end(trimmed) else {
            return false;
        };
        let raw_after_operator = raw_segment.get(operator_end..).unwrap_or("");
        let Some(value_segment) =
            Self::predicate_continuation_value_segment(raw_after_operator, clause)
        else {
            return false;
        };

        Self::is_completed_value_expression(value_segment)
    }

    fn predicate_continuation_value_segment<'a>(
        raw_after_operator: &'a str,
        clause: &str,
    ) -> Option<&'a str> {
        let trimmed_end = raw_after_operator.trim_end();
        if trimmed_end.trim().is_empty() {
            return None;
        }

        if raw_after_operator
            .chars()
            .last()
            .is_some_and(|ch| ch.is_whitespace())
        {
            let value_segment = trimmed_end.trim();
            return (!value_segment.ends_with(',')).then_some(value_segment);
        }

        let upper = trimmed_end.to_ascii_uppercase();
        let words = Self::statement_words(&upper);
        let prefix = words.last().copied()?;
        if !Self::is_predicate_continuation_prefix(prefix, clause) {
            return None;
        }

        let prefix_start = upper.rfind(prefix)?;
        let before_prefix = &trimmed_end[..prefix_start];
        if !before_prefix
            .chars()
            .last()
            .is_some_and(|ch| ch.is_whitespace())
        {
            return None;
        }

        let value_segment = before_prefix.trim();
        (!value_segment.ends_with(',')).then_some(value_segment)
    }

    fn is_completed_value_expression(value_segment: &str) -> bool {
        let value = value_segment.trim();
        if value.is_empty() || value.ends_with(',') {
            return false;
        }
        if value.ends_with(')') {
            return true;
        }
        if value.ends_with('\'') || value.ends_with('"') {
            return true;
        }

        let token = value
            .split(|ch: char| ch.is_whitespace() || ch == ',')
            .filter(|token| !token.is_empty())
            .next_back()
            .unwrap_or("")
            .trim_matches(|ch| matches!(ch, '\'' | '"' | '`' | '[' | ']' | ';'));
        if token.is_empty() {
            return false;
        }

        let token_upper = token.to_ascii_uppercase();
        matches!(
            token_upper.as_str(),
            "DEFAULT"
                | "NULL"
                | "TRUE"
                | "FALSE"
                | "CURRENT_DATE"
                | "CURRENT_TIMESTAMP"
                | "NOW"
                | "NOW()"
                | "TODAY"
                | "TODAY()"
        ) || token.parse::<f64>().is_ok()
            || token.starts_with('$')
            || token.starts_with(':')
            || token == "?"
    }

    fn is_predicate_continuation_prefix(prefix: &str, clause: &str) -> bool {
        matches!(
            prefix,
            "A" | "AN" | "AND" | "O" | "OR" | "L" | "LI" | "LIM" | "LIMI" | "LIMIT"
        ) || matches!(
            (clause, prefix),
            (
                "WHERE" | "ON",
                "G" | "GR"
                    | "GRO"
                    | "GROU"
                    | "GROUP"
                    | "H"
                    | "HA"
                    | "HAV"
                    | "HAVI"
                    | "HAVIN"
                    | "HAVING"
                    | "ORD"
                    | "ORDE"
                    | "ORDER"
            ) | ("HAVING", "ORD" | "ORDE" | "ORDER")
                | ("WHEN", "T" | "TH" | "THE" | "THEN")
                | ("ON" | "SET", "W" | "WH" | "WHE" | "WHER" | "WHERE")
                | (
                    "SET",
                    "R" | "RE"
                        | "RET"
                        | "RETU"
                        | "RETUR"
                        | "RETURN"
                        | "RETURNI"
                        | "RETURNIN"
                        | "RETURNING"
                )
        )
    }

    fn latest_open_case_segments<'a>(
        raw_statement_upper: &'a str,
        searchable_statement_upper: &'a str,
    ) -> Option<(&'a str, &'a str)> {
        let case_position = Self::previous_keyword_position(searchable_statement_upper, "CASE")?;
        let after_case = case_position + "CASE".len();
        if Self::statement_has_any_keyword(
            searchable_statement_upper,
            after_case,
            searchable_statement_upper.len(),
            &["END"],
        ) {
            return None;
        }

        Some((
            raw_statement_upper.get(after_case..)?,
            searchable_statement_upper.get(after_case..)?,
        ))
    }

    fn latest_case_result_marker(searchable_segment_upper: &str) -> Option<(usize, &'static str)> {
        ["THEN", "ELSE"]
            .into_iter()
            .filter_map(|marker| {
                Self::previous_keyword_position(searchable_segment_upper, marker)
                    .map(|position| (position, marker))
            })
            .max_by_key(|(position, _)| *position)
    }

    fn is_case_result_context(statement_upper: &str) -> bool {
        let searchable_statement_upper = Self::mask_sql_noise(statement_upper);
        let Some((raw_case_segment, searchable_case_segment)) =
            Self::latest_open_case_segments(statement_upper, &searchable_statement_upper)
        else {
            return false;
        };
        let Some((marker_position, marker)) =
            Self::latest_case_result_marker(searchable_case_segment)
        else {
            return false;
        };
        let after_marker = marker_position + marker.len();
        let Some(result_segment) = raw_case_segment.get(after_marker..) else {
            return false;
        };
        let trimmed = result_segment.trim_end();
        if trimmed.is_empty() || Self::case_result_needs_more_expression(trimmed) {
            return true;
        }
        if result_segment
            .chars()
            .last()
            .is_some_and(|ch| ch.is_whitespace())
        {
            return false;
        }

        let words = Self::statement_words(trimmed);
        let Some(prefix) = words.last().copied() else {
            return false;
        };
        let Some(prefix_start) = trimmed.rfind(prefix) else {
            return false;
        };

        trimmed[..prefix_start].trim_end().is_empty()
    }

    fn is_case_continuation_context(statement_upper: &str) -> bool {
        let searchable_statement_upper = Self::mask_sql_noise(statement_upper);
        let Some((raw_case_segment, searchable_case_segment)) =
            Self::latest_open_case_segments(statement_upper, &searchable_statement_upper)
        else {
            return false;
        };
        let Some((marker_position, marker)) =
            Self::latest_case_result_marker(searchable_case_segment)
        else {
            return false;
        };
        let after_marker = marker_position + marker.len();
        let Some(result_segment) = raw_case_segment.get(after_marker..) else {
            return false;
        };
        let trimmed = result_segment.trim_end();
        if trimmed.is_empty() || Self::case_result_needs_more_expression(trimmed) {
            return false;
        }

        if result_segment
            .chars()
            .last()
            .is_some_and(|ch| ch.is_whitespace())
        {
            return Self::is_completed_case_result_expression(trimmed);
        }

        let words = Self::statement_words(trimmed);
        let Some(prefix) = words.last().copied() else {
            return false;
        };
        if !Self::is_case_continuation_prefix(prefix, marker) {
            return false;
        }
        let Some(prefix_start) = trimmed.rfind(prefix) else {
            return false;
        };
        let before_prefix = trimmed[..prefix_start].trim_end();

        !before_prefix.is_empty() && Self::is_completed_case_result_expression(before_prefix)
    }

    fn case_result_needs_more_expression(segment: &str) -> bool {
        segment
            .chars()
            .last()
            .is_some_and(|ch| matches!(ch, '(' | ',' | '.' | '+' | '-' | '*' | '/' | '%' | '='))
    }

    fn is_completed_case_result_expression(segment: &str) -> bool {
        let value = segment.trim();
        if value.is_empty() || Self::case_result_needs_more_expression(value) {
            return false;
        }
        if Self::is_completed_value_expression(value) {
            return true;
        }

        let token = value
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | ',' | ';'))
            .filter(|token| !token.is_empty())
            .next_back()
            .unwrap_or("")
            .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | '[' | ']'));
        if token.is_empty() {
            return false;
        }

        !matches!(
            token.to_ascii_uppercase().as_str(),
            "CASE" | "WHEN" | "THEN" | "ELSE" | "END"
        )
    }

    fn is_case_continuation_prefix(prefix: &str, marker: &str) -> bool {
        matches!(
            (marker, prefix),
            ("THEN", "W" | "WH" | "WHE" | "WHEN")
                | ("THEN", "E" | "EL" | "ELS" | "ELSE" | "EN" | "END")
                | ("ELSE", "E" | "EN" | "END")
        )
    }

    fn is_value_keyword_prefix(word: &str) -> bool {
        matches!(
            word,
            "D" | "DE"
                | "DEF"
                | "DEFA"
                | "DEFAU"
                | "DEFAUL"
                | "DEFAULT"
                | "N"
                | "NO"
                | "NOW"
                | "NU"
                | "NUL"
                | "NULL"
                | "T"
                | "TR"
                | "TRU"
                | "TRUE"
                | "F"
                | "FA"
                | "FAL"
                | "FALS"
                | "FALSE"
                | "TO"
                | "TOD"
                | "TODA"
                | "TODAY"
                | "C"
                | "CU"
                | "CUR"
                | "CURR"
                | "CURRE"
                | "CURREN"
                | "CURRENT"
                | "CURRENT_DATE"
                | "CURRENT_TIMESTAMP"
        )
    }

    fn is_update_set_context(statement_upper: &str) -> bool {
        let Some(update_position) = Self::previous_keyword_position(statement_upper, "UPDATE")
        else {
            return false;
        };
        let after_update = update_position + "UPDATE".len();
        let Some(set_position) = Self::next_keyword_position(statement_upper, "SET", after_update)
        else {
            return false;
        };

        !Self::statement_has_any_keyword(
            statement_upper,
            set_position + "SET".len(),
            statement_upper.len(),
            &["WHERE", "RETURNING"],
        )
    }

    fn join_using_column_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = text_upper[statement_start..].trim_end();

        Self::is_join_using_column_context(statement_upper)
    }

    fn order_direction_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_order_direction_context(statement_upper)
    }

    fn group_by_continuation_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_group_by_continuation_context(statement_upper)
    }

    fn case_when_condition_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = text_upper[statement_start..].trim_end();
        let words = Self::statement_words(statement_upper);

        Self::words_end_with(&words, &["WHEN"])
    }

    fn case_result_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let text_upper = text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_case_result_context(statement_upper)
    }

    fn case_continuation_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let text_upper = text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_case_continuation_context(statement_upper)
    }

    fn analyze_relation_continuation_context_at_position(
        source: &str,
        position: Position,
    ) -> Option<CompletionContext> {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::analyze_relation_continuation_context(statement_upper)
    }

    fn select_continuation_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let text_upper = text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_select_continuation_context(statement_upper)
    }

    fn insert_value_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_insert_value_context(statement_upper)
    }

    fn expression_value_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let raw_text_upper = text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];
        let raw_statement_upper = &raw_text_upper[statement_start..];

        Self::is_expression_value_context(statement_upper, raw_statement_upper)
    }

    fn predicate_continuation_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let raw_text_upper = text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];
        let raw_statement_upper = &raw_text_upper[statement_start..];

        Self::is_predicate_continuation_context(statement_upper, raw_statement_upper)
    }

    fn reference_column_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = text_upper[statement_start..].trim_end();

        Self::is_reference_column_context(statement_upper)
    }

    fn reference_relation_target_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_reference_relation_target_context(statement_upper)
    }

    fn reference_action_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_reference_action_context(statement_upper)
    }

    fn reference_rule_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_reference_rule_context(statement_upper)
    }

    fn is_join_using_column_context(statement_upper: &str) -> bool {
        let Some(using_position) = Self::previous_keyword_position(statement_upper, "USING") else {
            return false;
        };
        let Some(join_position) =
            Self::previous_keyword_position(&statement_upper[..using_position], "JOIN")
        else {
            return false;
        };
        if join_position > using_position {
            return false;
        }

        let after_using = using_position + "USING".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_using,
            statement_upper.len(),
            &["WHERE", "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "UNION"],
        ) {
            return false;
        }

        let text_after_using = &statement_upper[after_using..];
        let Some(open_position) = text_after_using.find('(') else {
            return text_after_using.trim().is_empty();
        };

        let text_after_open = &text_after_using[open_position..];
        let mut depth = 0isize;
        for ch in text_after_open.chars() {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            }
        }

        depth > 0 || text_after_open.trim_end().ends_with(',')
    }

    fn is_order_direction_context(statement_upper: &str) -> bool {
        let Some(order_position) = Self::previous_keyword_position(statement_upper, "ORDER BY")
        else {
            return false;
        };
        let after_order = order_position + "ORDER BY".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_order,
            statement_upper.len(),
            &["LIMIT", "OFFSET", "FETCH", "UNION", "HAVING"],
        ) {
            return false;
        }

        let segment_start = statement_upper[after_order..]
            .rfind(',')
            .map(|position| after_order + position + 1)
            .unwrap_or(after_order);
        let segment = &statement_upper[segment_start..];
        let trimmed = segment.trim_start();
        if trimmed.is_empty() || trimmed.ends_with('.') || trimmed.ends_with('(') {
            return false;
        }

        let words = Self::statement_words(trimmed);
        if words.is_empty() {
            return false;
        }

        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return true;
        }

        if words.len() < 2 {
            return false;
        }

        let Some(prefix) = words.last().copied() else {
            return false;
        };
        let is_order_modifier_prefix = matches!(
            prefix,
            "A" | "AS"
                | "ASC"
                | "D"
                | "DE"
                | "DES"
                | "DESC"
                | "N"
                | "NU"
                | "NUL"
                | "NULL"
                | "NULLS"
                | "F"
                | "FI"
                | "FIR"
                | "FIRS"
                | "FIRST"
                | "FE"
                | "FET"
                | "FETC"
                | "FETCH"
                | "L"
                | "LI"
                | "LIM"
                | "LIMI"
                | "LIMIT"
                | "LA"
                | "LAS"
                | "LAST"
                | "O"
                | "OF"
                | "OFF"
                | "OFFS"
                | "OFFSE"
                | "OFFSET"
                | "W"
                | "WI"
                | "WIT"
                | "WITH"
        );
        if is_order_modifier_prefix {
            return true;
        }

        match words.as_slice() {
            [_expression, prefix] => matches!(
                *prefix,
                "A" | "AS"
                    | "ASC"
                    | "D"
                    | "DE"
                    | "DES"
                    | "DESC"
                    | "N"
                    | "NU"
                    | "NUL"
                    | "NULL"
                    | "NULLS"
            ),
            [_expression, direction, prefix] if matches!(*direction, "ASC" | "DESC") => {
                matches!(*prefix, "N" | "NU" | "NUL" | "NULL" | "NULLS")
                    || matches!(
                        *prefix,
                        "F" | "FI" | "FIR" | "FIRS" | "FIRST" | "L" | "LA" | "LAS" | "LAST"
                    )
            }
            [_expression, "NULLS", prefix] => matches!(
                *prefix,
                "F" | "FI" | "FIR" | "FIRS" | "FIRST" | "L" | "LA" | "LAS" | "LAST"
            ),
            _ => false,
        }
    }

    fn is_group_by_continuation_context(statement_upper: &str) -> bool {
        let Some(group_position) = Self::previous_keyword_position(statement_upper, "GROUP BY")
        else {
            return false;
        };
        let after_group = group_position + "GROUP BY".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_group,
            statement_upper.len(),
            &[
                "HAVING", "ORDER BY", "LIMIT", "OFFSET", "FETCH", "UNION", "WHERE",
            ],
        ) {
            return false;
        }

        let segment_start = statement_upper[after_group..]
            .rfind(',')
            .map(|position| after_group + position + 1)
            .unwrap_or(after_group);
        let segment = &statement_upper[segment_start..];
        let trimmed = segment.trim_start();
        if trimmed.is_empty() || trimmed.ends_with('.') || trimmed.ends_with('(') {
            return false;
        }

        let words = Self::statement_words(trimmed);
        if words.is_empty() {
            return false;
        }

        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return true;
        }

        if words.len() < 2 {
            return false;
        }

        let Some(prefix) = words.last().copied() else {
            return false;
        };
        matches!(
            prefix,
            "H" | "HA"
                | "HAV"
                | "HAVI"
                | "HAVIN"
                | "HAVING"
                | "O"
                | "OR"
                | "ORD"
                | "ORDE"
                | "ORDER"
                | "L"
                | "LI"
                | "LIM"
                | "LIMI"
                | "LIMIT"
                | "OF"
                | "OFF"
                | "OFFS"
                | "OFFSE"
                | "OFFSET"
                | "F"
                | "FE"
                | "FET"
                | "FETC"
                | "FETCH"
                | "W"
                | "WI"
                | "WIT"
                | "WITH"
                | "S"
                | "SO"
                | "SOR"
                | "SORT"
                | "C"
                | "CL"
                | "CLU"
                | "CLUS"
                | "CLUST"
                | "CLUSTE"
                | "CLUSTER"
                | "D"
                | "DI"
                | "DIS"
                | "DIST"
                | "DISTR"
                | "DISTRI"
                | "DISTRIB"
                | "DISTRIBU"
                | "DISTRIBUT"
                | "DISTRIBUTE"
        )
    }

    fn analyze_relation_continuation_context(statement_upper: &str) -> Option<CompletionContext> {
        if Self::is_join_condition_context(statement_upper) {
            return Some(CompletionContext::JoinConditionClause);
        }

        if Self::is_from_continuation_context(statement_upper) {
            return Some(CompletionContext::FromContinuationClause);
        }

        None
    }

    fn is_join_condition_context(statement_upper: &str) -> bool {
        let Some(join_position) = Self::previous_keyword_position(statement_upper, "JOIN") else {
            return false;
        };
        let after_join = join_position + "JOIN".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_join,
            statement_upper.len(),
            &[
                "ON", "USING", "WHERE", "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "UNION",
            ],
        ) {
            return false;
        }

        Self::relation_target_completed_or_prefixed(
            &statement_upper[after_join..],
            &["A", "AS", "O", "ON", "U", "US", "USI", "USIN", "USING"],
        )
    }

    fn is_from_continuation_context(statement_upper: &str) -> bool {
        let Some(from_position) = Self::previous_keyword_position(statement_upper, "FROM") else {
            return false;
        };
        let after_from = from_position + "FROM".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_from,
            statement_upper.len(),
            &[
                "JOIN", "WHERE", "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "OFFSET", "FETCH",
                "UNION", "ON", "USING",
            ],
        ) {
            return false;
        }

        let segment = statement_upper[after_from..]
            .rsplit(',')
            .next()
            .unwrap_or(&statement_upper[after_from..]);
        Self::relation_target_completed_or_prefixed(
            segment,
            &[
                "A", "AS", "J", "JO", "JOI", "JOIN", "I", "IN", "INN", "INNE", "INNER", "L", "LE",
                "LEF", "LEFT", "R", "RI", "RIG", "RIGH", "RIGHT", "F", "FU", "FUL", "FULL", "C",
                "CR", "CRO", "CROS", "CROSS", "W", "WH", "WHE", "WHER", "WHERE", "G", "GR", "GRO",
                "GROU", "GROUP", "O", "OR", "ORD", "ORDE", "ORDER", "H", "HA", "HAV", "HAVI",
                "HAVIN", "HAVING", "LI", "LIM", "LIMI", "LIMIT", "OF", "OFF", "OFFS", "OFFSE",
                "OFFSET", "FE", "FET", "FETC", "FETCH",
            ],
        )
    }

    fn relation_target_completed_or_prefixed(
        segment: &str,
        continuation_prefixes: &[&str],
    ) -> bool {
        let trimmed = segment.trim_start();
        let trimmed_end = trimmed.trim_end();
        if trimmed_end.is_empty() || trimmed_end.ends_with('.') || trimmed_end.ends_with('(') {
            return false;
        }

        let words = Self::statement_words(trimmed_end);
        if words.is_empty() {
            return false;
        }

        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return words.last().is_some_and(|word| *word != "AS");
        }

        if words.len() < 2 {
            return false;
        }

        let Some(prefix) = words.last().copied() else {
            return false;
        };
        continuation_prefixes.contains(&prefix)
    }

    fn is_select_continuation_context(statement_upper: &str) -> bool {
        let Some(select_position) = Self::previous_keyword_position(statement_upper, "SELECT")
        else {
            return false;
        };
        let after_select = select_position + "SELECT".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_select,
            statement_upper.len(),
            &[
                "FROM", "WHERE", "GROUP BY", "ORDER BY", "HAVING", "LIMIT", "UNION",
            ],
        ) {
            return false;
        }

        let segment = statement_upper[after_select..]
            .rsplit(',')
            .next()
            .unwrap_or(&statement_upper[after_select..]);
        Self::select_item_completed_or_prefixed(segment, &["A", "AS", "F", "FR", "FRO", "FROM"])
    }

    fn select_item_completed_or_prefixed(segment: &str, continuation_prefixes: &[&str]) -> bool {
        let trimmed = segment.trim_start();
        let trimmed_end = trimmed.trim_end();
        if trimmed_end.is_empty() || trimmed_end.ends_with('.') || trimmed_end.ends_with('(') {
            return false;
        }

        let words = Self::statement_words(trimmed_end);
        let meaningful_words = words
            .iter()
            .filter(|word| **word != "DISTINCT")
            .copied()
            .collect::<Vec<_>>();
        let has_wildcard = trimmed_end.contains('*');

        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            if words.last().is_some_and(|word| *word == "AS") {
                return false;
            }
            return has_wildcard || !meaningful_words.is_empty();
        }

        let Some(prefix) = words.last().copied() else {
            return false;
        };
        if !continuation_prefixes.contains(&prefix) {
            return false;
        }

        if words.len() >= 2 {
            return true;
        }

        let Some((before_prefix, _)) = trimmed.rsplit_once(prefix) else {
            return false;
        };
        has_wildcard
            || Self::statement_words(before_prefix)
                .iter()
                .any(|word| *word != "DISTINCT")
    }

    fn is_reference_relation_target_context(statement_upper: &str) -> bool {
        let Some(references_position) =
            Self::previous_keyword_position(statement_upper, "REFERENCES")
        else {
            return false;
        };
        let after_references = references_position + "REFERENCES".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_references,
            statement_upper.len(),
            &[
                "MATCH",
                "ON DELETE",
                "ON UPDATE",
                "DEFERRABLE",
                "NOT DEFERRABLE",
                "INITIALLY",
            ],
        ) {
            return false;
        }

        if statement_upper[after_references..].contains('(') {
            return false;
        }

        let text_after_references = &statement_upper[after_references..];
        if text_after_references.trim_start().is_empty() {
            return true;
        }

        let Some((_, after_relation)) = Self::read_relation_reference_after_preserving_trailing(
            statement_upper,
            after_references,
        ) else {
            return true;
        };
        let remainder = &statement_upper[after_relation..];

        if remainder.is_empty() {
            return true;
        }
        if remainder.trim().is_empty() {
            return false;
        }

        remainder.trim_start().starts_with('.')
    }

    fn is_reference_action_context(statement_upper: &str) -> bool {
        let Some(references_position) =
            Self::previous_keyword_position(statement_upper, "REFERENCES")
        else {
            return false;
        };
        let after_references = references_position + "REFERENCES".len();
        let Some((_, after_relation)) = Self::read_relation_reference_after_preserving_trailing(
            statement_upper,
            after_references,
        ) else {
            return false;
        };
        let after_relation_text = &statement_upper[after_relation..];
        if after_relation_text.contains('(') {
            return false;
        }
        if after_relation_text.trim_start().is_empty() {
            return !after_relation_text.is_empty();
        }

        if Self::statement_has_any_keyword(
            statement_upper,
            after_relation,
            statement_upper.len(),
            &[
                "MATCH",
                "ON DELETE",
                "ON UPDATE",
                "DEFERRABLE",
                "NOT DEFERRABLE",
                "INITIALLY",
            ],
        ) {
            return false;
        }

        let words = Self::statement_words(after_relation_text);
        !after_relation_text
            .chars()
            .last()
            .is_some_and(|ch| ch.is_whitespace())
            && words.len() <= 2
    }

    fn is_reference_rule_context(statement_upper: &str) -> bool {
        let Some(references_position) =
            Self::previous_keyword_position(statement_upper, "REFERENCES")
        else {
            return false;
        };
        let after_references = references_position + "REFERENCES".len();
        let Some((_, after_relation)) = Self::read_relation_reference_after_preserving_trailing(
            statement_upper,
            after_references,
        ) else {
            return false;
        };
        let after_relation_text = &statement_upper[after_relation..];
        if after_relation_text.contains('(') {
            return false;
        }

        let delete_position = Self::previous_keyword_position(statement_upper, "ON DELETE");
        let update_position = Self::previous_keyword_position(statement_upper, "ON UPDATE");
        let Some((action_position, action_keyword)) = [
            delete_position.map(|position| (position, "ON DELETE")),
            update_position.map(|position| (position, "ON UPDATE")),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|(position, _)| *position) else {
            return false;
        };
        if action_position < after_relation {
            return false;
        }

        let after_action = action_position + action_keyword.len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_action,
            statement_upper.len(),
            &[
                "ON DELETE",
                "ON UPDATE",
                "MATCH",
                "DEFERRABLE",
                "NOT DEFERRABLE",
                "INITIALLY",
            ],
        ) {
            return false;
        }

        let after_action_text = &statement_upper[after_action..];
        if after_action_text.trim_start().is_empty() {
            return !after_action_text.is_empty();
        }

        if after_action_text
            .chars()
            .last()
            .is_some_and(|ch| ch.is_whitespace())
        {
            return false;
        }

        Self::statement_words(after_action_text).len() <= 2
    }

    fn is_reference_column_context(statement_upper: &str) -> bool {
        let Some(references_position) =
            Self::previous_keyword_position(statement_upper, "REFERENCES")
        else {
            return false;
        };
        let after_references = references_position + "REFERENCES".len();
        let Some((_, after_relation)) = Self::read_relation_reference_after_preserving_trailing(
            statement_upper,
            after_references,
        ) else {
            return false;
        };

        let open_position = Self::skip_whitespace(statement_upper, after_relation);
        if !statement_upper[open_position..].starts_with('(') {
            return false;
        }

        Self::matching_paren_end(statement_upper, open_position).is_none()
            || statement_upper[open_position..].trim_end().ends_with(',')
    }

    pub fn reference_table_at_position(source: &str, position: Position) -> Option<String> {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = text_upper.get(statement_start..)?.trim_end();
        let statement_source = text_before.get(statement_start..)?;

        let references_position = Self::previous_keyword_position(statement_upper, "REFERENCES")?;
        let after_references = references_position + "REFERENCES".len();
        let (table_name, _) =
            Self::read_relation_reference_after(statement_source, after_references)?;

        Some(table_name)
    }

    fn analyze_ddl_target_context_at_position(
        source: &str,
        position: Position,
    ) -> Option<CompletionContext> {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = text_upper[statement_start..].trim_end();

        Self::analyze_ddl_target_context(statement_upper)
    }

    fn alter_table_action_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_alter_table_action_context(statement_upper)
    }

    fn analyze_dml_action_context_at_position(
        source: &str,
        position: Position,
    ) -> Option<CompletionContext> {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::analyze_dml_action_context(statement_upper)
    }

    fn data_type_context_at_position(source: &str, position: Position) -> bool {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset).unwrap_or(source);
        let searchable_text_before = Self::mask_sql_noise(text_before);
        let text_upper = searchable_text_before.to_ascii_uppercase();
        let statement_start = Self::previous_statement_start(&text_upper, text_upper.len());
        let statement_upper = &text_upper[statement_start..];

        Self::is_data_type_context(statement_upper)
    }

    fn analyze_dml_action_context(statement_upper: &str) -> Option<CompletionContext> {
        if Self::is_relation_action_context(statement_upper, "INSERT INTO") {
            return Some(CompletionContext::InsertActionClause);
        }
        if Self::is_relation_action_context(statement_upper, "UPDATE") {
            return Some(CompletionContext::UpdateActionClause);
        }
        if Self::is_relation_action_context(statement_upper, "DELETE FROM") {
            return Some(CompletionContext::DeleteActionClause);
        }

        None
    }

    fn is_data_type_context(statement_upper: &str) -> bool {
        Self::is_create_table_column_type_context(statement_upper)
            || Self::is_alter_table_add_column_type_context(statement_upper)
            || Self::is_alter_table_modify_column_type_context(statement_upper)
            || Self::is_alter_table_change_column_type_context(statement_upper)
            || Self::is_alter_table_alter_column_type_context(statement_upper)
    }

    fn is_create_table_column_type_context(statement_upper: &str) -> bool {
        let Some(create_table_position) =
            Self::previous_keyword_position(statement_upper, "CREATE TABLE")
        else {
            return false;
        };
        let Some(open_position) = statement_upper[create_table_position..]
            .find('(')
            .map(|position| create_table_position + position)
        else {
            return false;
        };

        if Self::matching_paren_end(statement_upper, open_position).is_some() {
            return false;
        }

        let segment_start = statement_upper[open_position + 1..]
            .rfind(',')
            .map(|position| open_position + 1 + position + 1)
            .unwrap_or(open_position + 1);
        Self::is_column_definition_type_segment(&statement_upper[segment_start..])
    }

    fn is_alter_table_add_column_type_context(statement_upper: &str) -> bool {
        if Self::is_alter_table_column_definition_type_after_phrase(statement_upper, "ADD COLUMN") {
            return true;
        }

        Self::is_alter_table_column_definition_type_after_phrase(statement_upper, "ADD")
    }

    fn is_alter_table_modify_column_type_context(statement_upper: &str) -> bool {
        if Self::is_alter_table_column_definition_type_after_phrase(
            statement_upper,
            "MODIFY COLUMN",
        ) {
            return true;
        }

        Self::is_alter_table_column_definition_type_after_phrase(statement_upper, "MODIFY")
    }

    fn is_alter_table_column_definition_type_after_phrase(
        statement_upper: &str,
        phrase: &str,
    ) -> bool {
        let Some(alter_table_position) =
            Self::previous_keyword_position(statement_upper, "ALTER TABLE")
        else {
            return false;
        };
        let Some(phrase_position) = Self::previous_keyword_position(statement_upper, phrase) else {
            return false;
        };
        if phrase_position < alter_table_position {
            return false;
        }

        let after_phrase = phrase_position + phrase.len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_phrase,
            statement_upper.len(),
            &[
                "NOT NULL",
                "NULL",
                "DEFAULT",
                "PRIMARY KEY",
                "UNIQUE",
                "CHECK",
                "REFERENCES",
                "CONSTRAINT",
            ],
        ) {
            return false;
        }

        Self::is_column_definition_type_segment(&statement_upper[after_phrase..])
    }

    fn is_alter_table_change_column_type_context(statement_upper: &str) -> bool {
        if Self::is_alter_table_change_column_type_after_phrase(statement_upper, "CHANGE COLUMN") {
            return true;
        }

        Self::is_alter_table_change_column_type_after_phrase(statement_upper, "CHANGE")
    }

    fn is_alter_table_change_column_type_after_phrase(statement_upper: &str, phrase: &str) -> bool {
        let Some(alter_table_position) =
            Self::previous_keyword_position(statement_upper, "ALTER TABLE")
        else {
            return false;
        };
        let Some(phrase_position) = Self::previous_keyword_position(statement_upper, phrase) else {
            return false;
        };
        if phrase_position < alter_table_position {
            return false;
        }

        let after_phrase = phrase_position + phrase.len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_phrase,
            statement_upper.len(),
            &[
                "NOT NULL",
                "NULL",
                "DEFAULT",
                "PRIMARY KEY",
                "UNIQUE",
                "CHECK",
                "REFERENCES",
                "CONSTRAINT",
            ],
        ) {
            return false;
        }

        Self::is_column_change_type_segment(&statement_upper[after_phrase..])
    }

    fn is_alter_table_alter_column_type_context(statement_upper: &str) -> bool {
        let Some(alter_table_position) =
            Self::previous_keyword_position(statement_upper, "ALTER TABLE")
        else {
            return false;
        };
        let Some(alter_column_position) =
            Self::previous_keyword_position(statement_upper, "ALTER COLUMN")
        else {
            return false;
        };
        if alter_column_position < alter_table_position {
            return false;
        }

        let Some(type_position) = Self::previous_keyword_position(statement_upper, "TYPE") else {
            return false;
        };
        if type_position < alter_column_position {
            return false;
        }

        let after_type = type_position + "TYPE".len();
        if Self::statement_has_any_keyword(
            statement_upper,
            after_type,
            statement_upper.len(),
            &[
                "USING",
                "COLLATE",
                "NOT NULL",
                "NULL",
                "DEFAULT",
                "PRIMARY KEY",
                "UNIQUE",
                "CHECK",
                "REFERENCES",
            ],
        ) {
            return false;
        }

        Self::is_data_type_name_segment(&statement_upper[after_type..])
    }

    fn is_column_definition_type_segment(segment: &str) -> bool {
        let trimmed = segment.trim_start();
        if trimmed.is_empty() {
            return false;
        }

        let first_word = Self::statement_words(trimmed)
            .first()
            .copied()
            .unwrap_or("");
        if matches!(
            first_word,
            "PRIMARY"
                | "FOREIGN"
                | "UNIQUE"
                | "CHECK"
                | "CONSTRAINT"
                | "COLUMN"
                | "KEY"
                | "INDEX"
                | "FULLTEXT"
                | "SPATIAL"
                | "EXCLUDE"
                | "LIKE"
        ) {
            return false;
        }

        let has_separator_after_column_name = trimmed
            .split_once(|ch: char| ch.is_whitespace())
            .is_some_and(|(_, rest)| {
                !rest.is_empty() || segment.chars().last().is_some_and(|ch| ch.is_whitespace())
            });
        if !has_separator_after_column_name {
            return false;
        }

        let words = Self::statement_words(trimmed);
        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return words.len() == 1;
        }

        words.len() == 2
    }

    fn is_column_change_type_segment(segment: &str) -> bool {
        let trimmed = segment.trim_start();
        if trimmed.is_empty() {
            return false;
        }

        let words = Self::statement_words(trimmed);
        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return words.len() == 2;
        }

        words.len() == 3
    }

    fn is_data_type_name_segment(segment: &str) -> bool {
        let trimmed = segment.trim_start();
        if trimmed.is_empty() {
            return !segment.is_empty();
        }

        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return false;
        }

        Self::statement_words(trimmed).len() == 1
    }

    fn analyze_ddl_target_context(statement_upper: &str) -> Option<CompletionContext> {
        if Self::is_alter_table_target_context(
            statement_upper,
            &[
                "DROP COLUMN",
                "ALTER COLUMN",
                "RENAME COLUMN",
                "MODIFY COLUMN",
                "MODIFY",
                "CHANGE COLUMN",
                "CHANGE",
            ],
            &[
                "TYPE",
                "SET",
                "DROP",
                "RESTART",
                "TO",
                "CASCADE",
                "RESTRICT",
                "ADD",
                "ALTER",
                "RENAME",
                "NOT NULL",
                "NULL",
                "DEFAULT",
                "PRIMARY KEY",
                "UNIQUE",
                "CHECK",
                "REFERENCES",
            ],
        ) {
            return Some(CompletionContext::ColumnTargetClause);
        }

        if Self::is_alter_table_target_context(
            statement_upper,
            &["DROP CONSTRAINT", "RENAME CONSTRAINT"],
            &[
                "TO", "CASCADE", "RESTRICT", "ADD", "ALTER", "DROP", "RENAME",
            ],
        ) {
            return Some(CompletionContext::ConstraintTargetClause);
        }

        if Self::is_global_index_target_context(statement_upper)
            || Self::is_alter_table_target_context(
                statement_upper,
                &["DROP INDEX", "RENAME INDEX"],
                &["TO", "ADD", "ALTER", "DROP", "RENAME"],
            )
        {
            return Some(CompletionContext::IndexTargetClause);
        }

        None
    }

    fn is_global_index_target_context(statement_upper: &str) -> bool {
        ["DROP INDEX", "ALTER INDEX", "REINDEX INDEX"]
            .iter()
            .any(|phrase| {
                let Some(position) = Self::previous_keyword_position(statement_upper, phrase)
                else {
                    return false;
                };
                let after_phrase = position + phrase.len();
                if Self::statement_has_any_keyword(
                    statement_upper,
                    after_phrase,
                    statement_upper.len(),
                    &["ON", "TO", "SET", "RESET", "ALTER", "CASCADE", "RESTRICT"],
                ) {
                    return false;
                }

                let target_text = statement_upper[after_phrase..].trim_start();
                if target_text.is_empty() {
                    return true;
                }
                if target_text
                    .chars()
                    .last()
                    .is_some_and(|ch| ch.is_whitespace())
                {
                    return false;
                }

                Self::statement_words(target_text).len() <= 1
            })
    }

    fn is_alter_table_action_context(statement_upper: &str) -> bool {
        let Some(alter_table_position) =
            Self::previous_keyword_position(statement_upper, "ALTER TABLE")
        else {
            return false;
        };
        let after_alter_table = alter_table_position + "ALTER TABLE".len();
        let Some((_, after_table)) = Self::read_relation_reference_after_preserving_trailing(
            statement_upper,
            after_alter_table,
        ) else {
            return false;
        };

        let trailing = &statement_upper[after_table..];
        let trimmed = trailing.trim_start();
        if trimmed.is_empty() {
            return !trailing.is_empty();
        }
        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return false;
        }

        Self::statement_words(trimmed).len() == 1
    }

    fn is_relation_action_context(statement_upper: &str, phrase: &str) -> bool {
        let Some(phrase_position) = Self::previous_keyword_position(statement_upper, phrase) else {
            return false;
        };
        let after_phrase = phrase_position + phrase.len();
        let Some((_, after_table)) =
            Self::read_relation_reference_after_preserving_trailing(statement_upper, after_phrase)
        else {
            return false;
        };

        let trailing = &statement_upper[after_table..];
        let trimmed = trailing.trim_start();
        if trimmed.is_empty() {
            return !trailing.is_empty();
        }
        if trimmed.chars().last().is_some_and(|ch| ch.is_whitespace()) {
            return false;
        }

        Self::statement_words(trimmed).len() == 1
    }

    fn is_alter_table_target_context(
        statement_upper: &str,
        target_phrases: &[&str],
        terminators: &[&str],
    ) -> bool {
        let Some(alter_table_position) =
            Self::previous_keyword_position(statement_upper, "ALTER TABLE")
        else {
            return false;
        };

        target_phrases.iter().any(|phrase| {
            let Some(target_position) = Self::previous_keyword_position(statement_upper, phrase)
            else {
                return false;
            };
            if target_position < alter_table_position {
                return false;
            }

            let after_phrase = target_position + phrase.len();
            if Self::statement_has_any_keyword(
                statement_upper,
                after_phrase,
                statement_upper.len(),
                terminators,
            ) {
                return false;
            }

            let target_text = statement_upper[after_phrase..].trim_start();
            if target_text.is_empty() {
                return true;
            }
            if target_text
                .chars()
                .last()
                .is_some_and(|ch| ch.is_whitespace())
            {
                return false;
            }

            Self::statement_words(target_text).len() <= 1
        })
    }

    /// 获取表名（用于 TableColumn 上下文）
    /// 如果光标在 table.column 的位置，返回表名
    pub fn get_table_name_for_column(&self, node: Node, source: &str) -> Option<String> {
        let mut current = Some(node);

        while let Some(n) = current {
            let kind = n.kind();

            // Try to extract from text directly first, if it looks like "table." or "table.col"
            if let Ok(text) = n.utf8_text(source.as_bytes()) {
                if let Some(dot_pos) = text.rfind('.') {
                    let table_name = text[..dot_pos].trim();
                    if !table_name.is_empty() && !Keywords::is_keyword(table_name) {
                        return Some(Self::normalize_identifier(table_name));
                    }
                }
            }

            // 查找 member_expression 或 dotted_name
            if kind == "member_expression" || kind == "dotted_name" {
                if let Ok(text) = n.utf8_text(source.as_bytes()) {
                    if let Some(dot_pos) = text.rfind('.') {
                        let table_name = text[..dot_pos].trim();
                        if !table_name.is_empty() && !Keywords::is_keyword(table_name) {
                            return Some(Self::normalize_identifier(table_name));
                        }
                    }
                }
            }

            // 检查父节点
            if let Some(parent) = n.parent() {
                if let Ok(text) = parent.utf8_text(source.as_bytes()) {
                    if let Some(dot_pos) = text.rfind('.') {
                        let table_name = text[..dot_pos].trim();
                        if !table_name.is_empty() && !Keywords::is_keyword(table_name) {
                            return Some(Self::normalize_identifier(table_name));
                        }
                    }
                }
            }

            current = n.parent();
        }

        None
    }

    /// Return the table/schema qualifier immediately before the cursor.
    ///
    /// This is more reliable than AST lookup while a user is typing an incomplete
    /// member access such as `alias.` or `alias.col`.
    pub fn column_qualifier_before_position(source: &str, position: Position) -> Option<String> {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let text_before = source.get(..cursor_offset.min(source.len()))?;
        let token = Self::identifier_path_token_before_cursor(text_before)?;

        if token.ends_with('.') {
            let qualifier = Self::normalize_identifier(token.trim_end_matches('.'));
            return (!qualifier.is_empty()).then_some(qualifier);
        }

        Self::identifier_qualifier(token).map(|qualifier| Self::normalize_identifier(&qualifier))
    }

    /// Normalize an identifier path for cross-dialect matching.
    pub fn normalize_identifier(identifier: &str) -> String {
        Self::identifier_parts(identifier).join(".")
    }

    /// Return the last segment of a possibly schema-qualified identifier.
    pub fn identifier_last_part(identifier: &str) -> String {
        Self::identifier_parts(identifier)
            .pop()
            .unwrap_or_else(|| identifier.trim().to_string())
    }

    /// Return the qualifier before the last segment of a possibly schema-qualified identifier.
    pub fn identifier_qualifier(identifier: &str) -> Option<String> {
        let mut parts = Self::identifier_parts(identifier);
        if parts.len() < 2 {
            return None;
        }

        parts.pop();
        let qualifier = parts.join(".");
        if qualifier.is_empty() {
            None
        } else {
            Some(qualifier)
        }
    }

    /// Compare a referenced table name with a schema table.
    pub fn table_name_matches(reference: &str, database: &str, table_name: &str) -> bool {
        let reference_parts = Self::identifier_parts(reference);
        let table = Self::normalize_identifier(table_name);
        let database = Self::normalize_identifier(database);

        match reference_parts.as_slice() {
            [name] => Self::identifier_eq(name, &table),
            parts if parts.len() >= 2 => {
                let referenced_table = parts.last().unwrap();
                let referenced_database = &parts[parts.len() - 2];
                Self::identifier_eq(referenced_table, &table)
                    && (database.is_empty() || Self::identifier_eq(referenced_database, &database))
            }
            _ => false,
        }
    }

    fn identifier_parts(identifier: &str) -> Vec<String> {
        identifier
            .trim()
            .trim_matches(|ch: char| matches!(ch, ',' | ';' | '(' | ')'))
            .split('.')
            .map(Self::unquote_identifier_part)
            .filter(|part| !part.is_empty())
            .collect()
    }

    fn unquote_identifier_part(part: &str) -> String {
        let trimmed = part
            .trim()
            .trim_matches(|ch: char| matches!(ch, ',' | ';' | '(' | ')'));

        if trimmed.len() >= 2 {
            let bytes = trimmed.as_bytes();
            let first = bytes[0] as char;
            let last = bytes[bytes.len() - 1] as char;

            if (first == '"' && last == '"')
                || (first == '`' && last == '`')
                || (first == '\'' && last == '\'')
                || (first == '[' && last == ']')
            {
                return trimmed[1..trimmed.len() - 1]
                    .replace("\"\"", "\"")
                    .replace("``", "`")
                    .replace("''", "'");
            }
        }

        trimmed.to_string()
    }

    fn identifier_eq(left: &str, right: &str) -> bool {
        left == right || left.eq_ignore_ascii_case(right)
    }

    fn identifier_path_token_before_cursor(text_before: &str) -> Option<&str> {
        let mut token_start = 0;
        let mut quote_end: Option<char> = None;
        let mut chars = text_before.char_indices().peekable();

        while let Some((index, ch)) = chars.next() {
            if let Some(end_quote) = quote_end {
                if ch == end_quote {
                    if chars.peek().is_some_and(|(_, next)| *next == end_quote) {
                        chars.next();
                        continue;
                    }
                    quote_end = None;
                }
                continue;
            }

            match ch {
                '"' | '\'' | '`' => quote_end = Some(ch),
                '[' => quote_end = Some(']'),
                _ if Self::is_identifier_token_boundary(ch) => {
                    token_start = index + ch.len_utf8();
                }
                _ => {}
            }
        }

        let token = text_before[token_start..].trim();
        (!token.is_empty()).then_some(token)
    }

    fn is_identifier_token_boundary(ch: char) -> bool {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '(' | ')' | ';' | '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '%'
            )
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
    fn is_identifier_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$')
    }

    fn push_masked_char(output: &mut String, ch: char) {
        if matches!(ch, '\n' | '\r') {
            output.push(ch);
        } else {
            for _ in 0..ch.len_utf8() {
                output.push(' ');
            }
        }
    }

    fn push_masked_range(source: &str, output: &mut String, mut index: usize, end: usize) -> usize {
        while index < end {
            let Some(ch) = source[index..].chars().next() else {
                break;
            };
            Self::push_masked_char(output, ch);
            index += ch.len_utf8();
        }
        index
    }

    fn starts_hash_comment(source: &str, index: usize) -> bool {
        if !source[index..].starts_with('#') {
            return false;
        }

        index == 0
            || source[..index]
                .chars()
                .next_back()
                .is_none_or(|ch| ch.is_whitespace())
    }

    fn mask_quoted_region(source: &str, output: &mut String, index: usize, quote: char) -> usize {
        let mut index = Self::push_masked_range(source, output, index, index + quote.len_utf8());

        while index < source.len() {
            let Some(ch) = source[index..].chars().next() else {
                break;
            };
            index = Self::push_masked_range(source, output, index, index + ch.len_utf8());

            if ch == '\\' && quote == '\'' && index < source.len() {
                let Some(escaped) = source[index..].chars().next() else {
                    break;
                };
                index = Self::push_masked_range(source, output, index, index + escaped.len_utf8());
                continue;
            }

            if ch == quote {
                if source[index..].starts_with(quote) {
                    index =
                        Self::push_masked_range(source, output, index, index + quote.len_utf8());
                    continue;
                }
                break;
            }
        }

        index
    }

    fn mask_bracketed_identifier(source: &str, output: &mut String, index: usize) -> usize {
        let mut index = Self::push_masked_range(source, output, index, index + 1);

        while index < source.len() {
            let Some(ch) = source[index..].chars().next() else {
                break;
            };
            index = Self::push_masked_range(source, output, index, index + ch.len_utf8());

            if ch == ']' {
                if source[index..].starts_with(']') {
                    index = Self::push_masked_range(source, output, index, index + 1);
                    continue;
                }
                break;
            }
        }

        index
    }

    fn dollar_quote_tag_at(source: &str, index: usize) -> Option<&str> {
        let rest = source.get(index..)?;
        let bytes = rest.as_bytes();
        if bytes.first() != Some(&b'$') {
            return None;
        }

        let mut end = 1;
        while end < bytes.len() {
            let byte = bytes[end];
            if byte == b'$' {
                return Some(&rest[..=end]);
            }
            if end == 1 && !(byte == b'_' || byte.is_ascii_alphabetic()) {
                return None;
            }
            if end > 1 && !(byte == b'_' || byte.is_ascii_alphanumeric()) {
                return None;
            }
            end += 1;
        }

        None
    }

    fn mask_dollar_quoted_region(source: &str, output: &mut String, index: usize) -> Option<usize> {
        let tag = Self::dollar_quote_tag_at(source, index)?;
        let body_start = index + tag.len();
        let end = source[body_start..]
            .find(tag)
            .map(|body_end| body_start + body_end + tag.len())
            .unwrap_or(source.len());

        Some(Self::push_masked_range(source, output, index, end))
    }

    pub(crate) fn mask_sql_noise(source: &str) -> String {
        let mut output = String::with_capacity(source.len());
        let mut index = 0;

        while index < source.len() {
            if source[index..].starts_with("--") {
                while index < source.len() {
                    let Some(ch) = source[index..].chars().next() else {
                        break;
                    };
                    index =
                        Self::push_masked_range(source, &mut output, index, index + ch.len_utf8());
                    if ch == '\n' {
                        break;
                    }
                }
                continue;
            }

            if Self::starts_hash_comment(source, index) {
                while index < source.len() {
                    let Some(ch) = source[index..].chars().next() else {
                        break;
                    };
                    index =
                        Self::push_masked_range(source, &mut output, index, index + ch.len_utf8());
                    if ch == '\n' {
                        break;
                    }
                }
                continue;
            }

            if source[index..].starts_with("/*") {
                index = Self::push_masked_range(source, &mut output, index, index + 2);
                while index < source.len() {
                    if source[index..].starts_with("*/") {
                        index = Self::push_masked_range(source, &mut output, index, index + 2);
                        break;
                    }
                    let Some(ch) = source[index..].chars().next() else {
                        break;
                    };
                    index =
                        Self::push_masked_range(source, &mut output, index, index + ch.len_utf8());
                }
                continue;
            }

            let Some(ch) = source[index..].chars().next() else {
                break;
            };

            if ch == '$' {
                if let Some(next_index) =
                    Self::mask_dollar_quoted_region(source, &mut output, index)
                {
                    index = next_index;
                    continue;
                }
            }

            if matches!(ch, '\'' | '"' | '`') {
                index = Self::mask_quoted_region(source, &mut output, index, ch);
                continue;
            }

            if ch == '[' {
                index = Self::mask_bracketed_identifier(source, &mut output, index);
                continue;
            }

            output.push(ch);
            index += ch.len_utf8();
        }

        output
    }

    fn completion_scope_source(source: &str, position: Position) -> String {
        let cursor_offset = Self::byte_offset_for_position(source, position);
        let prefix = source
            .get(..cursor_offset.min(source.len()))
            .unwrap_or(source);
        let searchable_prefix = Self::mask_sql_noise(prefix);
        let searchable_source = Self::mask_sql_noise(source);
        let (scope_start, scope_open) = Self::innermost_query_scope_start(&searchable_prefix);
        let scope_end = scope_open
            .and_then(|open| Self::matching_paren_end(&searchable_source, open))
            .unwrap_or_else(|| Self::next_statement_end(&searchable_source, cursor_offset));
        let scoped = source
            .get(scope_start..scope_end.min(source.len()))
            .unwrap_or(prefix);

        Self::mask_nested_parenthesized_regions(scoped)
    }

    fn innermost_query_scope_start(searchable_prefix: &str) -> (usize, Option<usize>) {
        let mut open_parens = Vec::new();
        for (index, ch) in searchable_prefix.char_indices() {
            match ch {
                '(' => open_parens.push(index),
                ')' => {
                    open_parens.pop();
                }
                _ => {}
            }
        }

        let upper = searchable_prefix.to_ascii_uppercase();
        for open in open_parens.into_iter().rev() {
            let start = open + 1;
            let segment = &upper[start..];
            if Self::next_keyword_position(segment, "SELECT", 0).is_some()
                || Self::next_keyword_position(segment, "WITH", 0).is_some()
            {
                return (start, Some(open));
            }
        }

        (Self::previous_statement_start(&upper, upper.len()), None)
    }

    fn matching_paren_end(searchable_source: &str, open_index: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (index, ch) in searchable_source[open_index..].char_indices() {
            let absolute = open_index + index;
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(absolute);
                    }
                }
                _ => {}
            }
        }

        None
    }

    fn next_statement_end(searchable_source: &str, cursor_offset: usize) -> usize {
        searchable_source[cursor_offset.min(searchable_source.len())..]
            .find(';')
            .map(|relative| cursor_offset + relative)
            .unwrap_or(searchable_source.len())
    }

    fn mask_nested_parenthesized_regions(source: &str) -> String {
        let searchable = Self::mask_sql_noise(source);
        let mut output = String::with_capacity(source.len());
        let mut depth = 0usize;

        for (index, ch) in source.char_indices() {
            let searchable_ch = searchable[index..].chars().next().unwrap_or(ch);
            match searchable_ch {
                '(' => {
                    if depth == 0 {
                        output.push(ch);
                    } else {
                        Self::push_masked_char(&mut output, ch);
                    }
                    depth += 1;
                }
                ')' => {
                    if depth > 0 {
                        depth -= 1;
                        if depth == 0 {
                            output.push(ch);
                        } else {
                            Self::push_masked_char(&mut output, ch);
                        }
                    } else {
                        output.push(ch);
                    }
                }
                _ if depth > 0 => Self::push_masked_char(&mut output, ch),
                _ => output.push(ch),
            }
        }

        output
    }

    fn is_keyword_at(source_upper: &str, start: usize, keyword: &str) -> bool {
        let end = start + keyword.len();
        if end > source_upper.len() {
            return false;
        }

        let before_is_boundary = if start == 0 {
            true
        } else {
            source_upper[..start]
                .chars()
                .next_back()
                .is_none_or(|ch| !Self::is_identifier_char(ch))
        };
        let after_is_boundary = if end >= source_upper.len() {
            true
        } else {
            source_upper[end..]
                .chars()
                .next()
                .is_none_or(|ch| !Self::is_identifier_char(ch))
        };

        before_is_boundary && after_is_boundary
    }

    fn next_keyword_position(source_upper: &str, keyword: &str, from: usize) -> Option<usize> {
        let mut search_pos = from;
        while let Some(relative_pos) = source_upper[search_pos..].find(keyword) {
            let absolute_pos = search_pos + relative_pos;
            if Self::is_keyword_at(source_upper, absolute_pos, keyword) {
                return Some(absolute_pos);
            }
            search_pos = absolute_pos + keyword.len();
        }
        None
    }

    fn previous_keyword_position(source_upper: &str, keyword: &str) -> Option<usize> {
        let mut search_pos = 0;
        let mut previous = None;

        while let Some(position) = Self::next_keyword_position(source_upper, keyword, search_pos) {
            previous = Some(position);
            search_pos = position + keyword.len();
        }

        previous
    }

    fn previous_statement_start(source_upper: &str, index: usize) -> usize {
        source_upper[..index]
            .rfind(';')
            .map(|position| position + 1)
            .unwrap_or(0)
    }

    fn contains_keyword_between(
        source_upper: &str,
        keyword: &str,
        start: usize,
        end: usize,
    ) -> bool {
        Self::next_keyword_position(source_upper, keyword, start)
            .is_some_and(|keyword_pos| keyword_pos < end)
    }

    fn statement_has_any_keyword(
        source_upper: &str,
        start: usize,
        end: usize,
        keywords: &[&str],
    ) -> bool {
        keywords
            .iter()
            .any(|keyword| Self::contains_keyword_between(source_upper, keyword, start, end))
    }

    fn should_read_on_relation(source_upper: &str, on_position: usize) -> bool {
        let statement_start = Self::previous_statement_start(source_upper, on_position);
        let has_ddl_action = Self::statement_has_any_keyword(
            source_upper,
            statement_start,
            on_position,
            &["CREATE", "DROP"],
        );
        let has_on_relation_object = Self::statement_has_any_keyword(
            source_upper,
            statement_start,
            on_position,
            &["INDEX", "TRIGGER", "POLICY"],
        );

        has_ddl_action && has_on_relation_object
    }

    fn push_table_reference(tables: &mut Vec<String>, table_name: &str) {
        let table_name = Self::normalize_identifier(table_name);
        if !table_name.is_empty() && !tables.contains(&table_name) {
            tables.push(table_name);
        }
    }

    fn extract_on_relation_references(source: &str, source_upper: &str, tables: &mut Vec<String>) {
        let mut search_pos = 0;
        while let Some(on_position) = Self::next_keyword_position(source_upper, "ON", search_pos) {
            let after_on = on_position + "ON".len();
            if Self::should_read_on_relation(source_upper, on_position) {
                if let Some((table_name, _)) = Self::read_relation_reference_after(source, after_on)
                {
                    Self::push_table_reference(tables, &table_name);
                }
            }
            search_pos = after_on;
        }
    }

    fn skip_whitespace(source: &str, mut index: usize) -> usize {
        while index < source.len() {
            let Some(ch) = source[index..].chars().next() else {
                break;
            };
            if !ch.is_whitespace() {
                break;
            }
            index += ch.len_utf8();
        }
        index
    }

    fn consume_word(source: &str, index: usize, word: &str) -> Option<usize> {
        let index = Self::skip_whitespace(source, index);
        let end = index + word.len();
        if end > source.len() || !source[index..end].eq_ignore_ascii_case(word) {
            return None;
        }

        let after_is_boundary = if end >= source.len() {
            true
        } else {
            source[end..]
                .chars()
                .next()
                .is_none_or(|ch| !Self::is_identifier_char(ch))
        };

        after_is_boundary.then_some(end)
    }

    fn skip_relation_modifiers(source: &str, mut index: usize) -> usize {
        if let Some(next_index) = Self::consume_word(source, index, "ONLY") {
            index = next_index;
        }

        let after_if = Self::consume_word(source, index, "IF");
        if let Some(after_if) = after_if {
            let after_not = Self::consume_word(source, after_if, "NOT").unwrap_or(after_if);
            if let Some(after_exists) = Self::consume_word(source, after_not, "EXISTS") {
                index = after_exists;
            }
        }

        Self::skip_whitespace(source, index)
    }

    fn read_identifier_part(source: &str, index: usize) -> Option<(String, usize)> {
        let index = Self::skip_whitespace(source, index);
        let first = source[index..].chars().next()?;

        if matches!(first, '"' | '`' | '\'') {
            let quote = first;
            let mut end = index + quote.len_utf8();
            let mut part = String::new();
            while end < source.len() {
                let ch = source[end..].chars().next()?;
                end += ch.len_utf8();
                if ch == quote {
                    if source[end..].starts_with(quote) {
                        part.push(quote);
                        end += quote.len_utf8();
                        continue;
                    }
                    return Some((part, end));
                }
                part.push(ch);
            }
            return None;
        }

        if first == '[' {
            let mut end = index + 1;
            let mut part = String::new();
            while end < source.len() {
                let ch = source[end..].chars().next()?;
                end += ch.len_utf8();
                if ch == ']' {
                    return Some((part, end));
                }
                part.push(ch);
            }
            return None;
        }

        if !Self::is_identifier_char(first) {
            return None;
        }

        let mut end = index;
        while end < source.len() {
            let Some(ch) = source[end..].chars().next() else {
                break;
            };
            if !Self::is_identifier_char(ch) {
                break;
            }
            end += ch.len_utf8();
        }

        Some((source[index..end].to_string(), end))
    }

    fn read_identifier_path(source: &str, index: usize) -> Option<(String, usize)> {
        let mut parts = Vec::new();
        let (first_part, mut index) = Self::read_identifier_part(source, index)?;
        parts.push(first_part);

        loop {
            index = Self::skip_whitespace(source, index);
            if !source[index..].starts_with('.') {
                break;
            }
            index += 1;
            let Some((part, next_index)) = Self::read_identifier_part(source, index) else {
                break;
            };
            parts.push(part);
            index = next_index;
        }

        Some((parts.join("."), index))
    }

    fn skip_parenthesized_region(source: &str, index: usize) -> usize {
        let mut index = Self::skip_whitespace(source, index);
        if !source[index..].starts_with('(') {
            return index;
        }

        let mut depth = 0usize;
        while index < source.len() {
            let Some(ch) = source[index..].chars().next() else {
                break;
            };
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth = depth.saturating_sub(1);
                index += ch.len_utf8();
                if depth == 0 {
                    break;
                }
                continue;
            }
            index += ch.len_utf8();
        }

        index
    }

    fn read_relation_reference_after(source: &str, index: usize) -> Option<(String, usize)> {
        let index = Self::skip_relation_modifiers(source, index);
        Self::read_identifier_path(source, index)
    }

    fn read_relation_reference_after_preserving_trailing(
        source: &str,
        index: usize,
    ) -> Option<(String, usize)> {
        let index = Self::skip_relation_modifiers(source, index);
        Self::read_identifier_path_preserving_trailing(source, index)
    }

    fn read_identifier_path_preserving_trailing(
        source: &str,
        index: usize,
    ) -> Option<(String, usize)> {
        let mut parts = Vec::new();
        let (first_part, mut index) = Self::read_identifier_part(source, index)?;
        parts.push(first_part);

        loop {
            let after_part = index;
            let dot_index = Self::skip_whitespace(source, index);
            if !source[dot_index..].starts_with('.') {
                return Some((parts.join("."), after_part));
            }
            index = dot_index + 1;
            let Some((part, next_index)) = Self::read_identifier_part(source, index) else {
                return Some((parts.join("."), after_part));
            };
            parts.push(part);
            index = next_index;
        }
    }

    fn read_relation_alias_after(source: &str, index: usize) -> Option<(String, usize)> {
        let mut index = Self::skip_whitespace(source, index);
        if let Some(after_as) = Self::consume_word(source, index, "AS") {
            index = after_as;
        }

        let (alias, next_index) = Self::read_identifier_part(source, index)?;
        if Keywords::is_keyword(&alias)
            || matches!(
                alias.to_ascii_uppercase().as_str(),
                "ON" | "USING" | "WHERE" | "SET" | "VALUES" | "RETURNING" | "GROUP" | "ORDER"
            )
        {
            return None;
        }

        Some((alias, next_index))
    }

    fn skip_cte_materialization_hint(source: &str, index: usize) -> usize {
        let index = Self::skip_whitespace(source, index);
        if let Some(after_not) = Self::consume_word(source, index, "NOT") {
            if let Some(after_materialized) = Self::consume_word(source, after_not, "MATERIALIZED")
            {
                return after_materialized;
            }
        }

        Self::consume_word(source, index, "MATERIALIZED").unwrap_or(index)
    }

    fn extract_cte_names(
        source: &str,
        searchable_source: &str,
        source_upper: &str,
    ) -> HashSet<String> {
        let mut names = HashSet::new();
        let mut search_pos = 0;

        while let Some(with_pos) = Self::next_keyword_position(source_upper, "WITH", search_pos) {
            let mut index = with_pos + "WITH".len();
            index = Self::consume_word(searchable_source, index, "RECURSIVE").unwrap_or(index);

            loop {
                index = Self::skip_whitespace(searchable_source, index);
                let Some((cte_name, after_name)) = Self::read_identifier_part(source, index) else {
                    break;
                };
                names.insert(Self::normalize_identifier(&cte_name));
                index = Self::skip_whitespace(searchable_source, after_name);

                if searchable_source[index..].starts_with('(') {
                    index = Self::skip_parenthesized_region(searchable_source, index);
                }

                let Some(after_as) = Self::consume_word(searchable_source, index, "AS") else {
                    break;
                };
                index = Self::skip_cte_materialization_hint(searchable_source, after_as);
                index = Self::skip_whitespace(searchable_source, index);
                if !searchable_source[index..].starts_with('(') {
                    break;
                }

                index = Self::skip_parenthesized_region(searchable_source, index);
                index = Self::skip_whitespace(searchable_source, index);
                if searchable_source[index..].starts_with(',') {
                    index += 1;
                    continue;
                }
                break;
            }

            search_pos = index.max(with_pos + "WITH".len());
        }

        names
    }

    fn is_cte_reference(table_name: &str, cte_names: &HashSet<String>) -> bool {
        let normalized = Self::normalize_identifier(table_name);
        let parts = Self::identifier_parts(&normalized);
        parts.len() == 1 && cte_names.contains(&normalized)
    }

    /// 提取表别名映射 (Alias -> Table Name)
    /// Uses text-based extraction for reliability
    pub fn extract_aliases(&self, _tree: &Tree, source: &str) -> HashMap<String, String> {
        Self::extract_aliases_from_source(source)
    }

    pub fn extract_aliases_at_position(
        &self,
        _tree: &Tree,
        source: &str,
        position: Position,
    ) -> HashMap<String, String> {
        let scoped_source = Self::completion_scope_source(source, position);
        Self::extract_aliases_from_source(&scoped_source)
    }

    fn extract_aliases_from_source(source: &str) -> HashMap<String, String> {
        let mut aliases = HashMap::new();
        let searchable_source = Self::mask_sql_noise(source);
        let source_upper = searchable_source.to_ascii_uppercase();

        // Pattern: FROM/JOIN/UPDATE table_name alias
        let keywords = ["FROM", "JOIN", "UPDATE"];

        for keyword in keywords {
            let mut search_pos = 0;
            while let Some(abs_pos) =
                Self::next_keyword_position(&source_upper, keyword, search_pos)
            {
                let after_keyword = abs_pos + keyword.len();

                if let Some((table_name, after_table)) =
                    Self::read_relation_reference_after(source, after_keyword)
                {
                    if let Some((alias, _)) = Self::read_relation_alias_after(source, after_table) {
                        aliases.insert(
                            Self::normalize_identifier(&alias),
                            Self::normalize_identifier(&table_name),
                        );
                    }
                }

                search_pos = after_keyword;
            }
        }

        aliases
    }

    /// 提取SQL中引用的表名（从FROM和JOIN子句）
    pub fn extract_referenced_tables(&self, _tree: &Tree, source: &str) -> Vec<String> {
        Self::extract_referenced_tables_from_source(source)
    }

    pub fn extract_referenced_tables_at_position(
        &self,
        _tree: &Tree,
        source: &str,
        position: Position,
    ) -> Vec<String> {
        let scoped_source = Self::completion_scope_source(source, position);
        Self::extract_referenced_tables_from_source(&scoped_source)
    }

    fn extract_referenced_tables_from_source(source: &str) -> Vec<String> {
        let mut tables = Vec::new();
        let searchable_source = Self::mask_sql_noise(source);
        let source_upper = searchable_source.to_ascii_uppercase();
        let cte_names = Self::extract_cte_names(source, &searchable_source, &source_upper);

        let keywords = ["FROM", "JOIN", "UPDATE", "INTO", "TABLE", "VIEW"];

        for keyword in keywords {
            let mut search_pos = 0;
            while let Some(abs_pos) =
                Self::next_keyword_position(&source_upper, keyword, search_pos)
            {
                let after_keyword = abs_pos + keyword.len();

                if let Some((table_name, _)) =
                    Self::read_relation_reference_after(source, after_keyword)
                {
                    if !Self::is_cte_reference(&table_name, &cte_names) {
                        Self::push_table_reference(&mut tables, &table_name);
                    }
                }

                search_pos = after_keyword;
            }
        }

        Self::extract_on_relation_references(source, &source_upper, &mut tables);

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

#[cfg(test)]
mod tests {
    use super::{CompletionContext, SqlParser};
    use tower_lsp::lsp_types::{DiagnosticSeverity, Position};

    fn position_at_end(source: &str) -> Position {
        let mut line = 0;
        let mut character = 0;

        for ch in source.chars() {
            if ch == '\n' {
                line += 1;
                character = 0;
            } else {
                character += ch.len_utf8() as u32;
            }
        }

        Position { line, character }
    }

    fn analyzed_context_at_end(sql: &str) -> CompletionContext {
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should produce a tree");
        let position = position_at_end(sql);
        let node = parser
            .get_node_at_position(tree, position)
            .expect("cursor should map to an AST node");

        parser.analyze_completion_context(node, sql, position)
    }

    #[test]
    fn normalizes_quoted_identifier_paths() {
        assert_eq!(
            SqlParser::normalize_identifier(r#""public"."users""#),
            "public.users"
        );
        assert_eq!(
            SqlParser::normalize_identifier("`app`.`orders`"),
            "app.orders"
        );
        assert_eq!(
            SqlParser::normalize_identifier("[dbo].[customers];"),
            "dbo.customers"
        );
        assert_eq!(
            SqlParser::identifier_qualifier(r#""public"."calculate_score""#).as_deref(),
            Some("public")
        );
        assert_eq!(
            SqlParser::identifier_qualifier("catalog.public.calculate_score").as_deref(),
            Some("catalog.public")
        );
        assert_eq!(SqlParser::identifier_qualifier("calculate_score"), None);
    }

    #[test]
    fn extracts_column_qualifier_before_cursor() {
        let alias_dot = "SELECT * FROM users u WHERE u.";
        assert_eq!(
            SqlParser::column_qualifier_before_position(alias_dot, position_at_end(alias_dot))
                .as_deref(),
            Some("u")
        );

        let alias_prefix = "SELECT * FROM users u WHERE u.na";
        assert_eq!(
            SqlParser::column_qualifier_before_position(
                alias_prefix,
                position_at_end(alias_prefix)
            )
            .as_deref(),
            Some("u")
        );

        let qualified_dot = "SELECT * FROM public.users.";
        assert_eq!(
            SqlParser::column_qualifier_before_position(
                qualified_dot,
                position_at_end(qualified_dot)
            )
            .as_deref(),
            Some("public.users")
        );

        let no_qualifier = "SELECT name FROM users";
        assert_eq!(
            SqlParser::column_qualifier_before_position(
                no_qualifier,
                position_at_end(no_qualifier)
            ),
            None
        );
    }

    #[test]
    fn matches_schema_qualified_table_names() {
        assert!(SqlParser::table_name_matches("users", "public", "users"));
        assert!(SqlParser::table_name_matches(
            r#""public"."users""#,
            "public",
            "users"
        ));
        assert!(SqlParser::table_name_matches(
            "`app`.`orders`",
            "app",
            "orders"
        ));
        assert!(!SqlParser::table_name_matches(
            "archive.users",
            "public",
            "users"
        ));
    }

    #[test]
    fn completion_context_ignores_keywords_inside_comments_and_literals() {
        let parser = SqlParser::new();

        let from_sql = "SELECT * -- WHERE hidden\nFROM ";
        assert_eq!(
            parser.analyze_completion_context_fallback(from_sql, position_at_end(from_sql)),
            CompletionContext::FromClause
        );

        let where_sql =
            "SELECT '-- FROM hidden' AS note FROM users WHERE name = 'ORDER BY hidden' AND ";
        assert_eq!(
            parser.analyze_completion_context_fallback(where_sql, position_at_end(where_sql)),
            CompletionContext::WhereClause
        );
    }

    #[test]
    fn completion_context_prefers_completed_clause_keywords_over_ast_noise() {
        assert_eq!(
            analyzed_context_at_end("SELECT"),
            CompletionContext::SelectClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT owner "),
            CompletionContext::SelectContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * "),
            CompletionContext::SelectContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT owner F"),
            CompletionContext::SelectContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT owner, "),
            CompletionContext::SelectClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * from"),
            CompletionContext::FromClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where "),
            CompletionContext::WhereClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner = "),
            CompletionContext::ExpressionValueClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner = N"),
            CompletionContext::ExpressionValueClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner = 'app' "),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner = 'app' O"),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner = 'app' AND "),
            CompletionContext::WhereClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner BETWEEN 1 AND "),
            CompletionContext::ExpressionValueClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner BETWEEN 1 "),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner BETWEEN 1 A"),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner BETWEEN 1 AND 5 "),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner IN ("),
            CompletionContext::ExpressionValueClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner IN ('app', "),
            CompletionContext::ExpressionValueClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner IN ('app', N"),
            CompletionContext::ExpressionValueClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users where owner IN ('app') "),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN "),
            CompletionContext::WhereClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = "),
            CompletionContext::ExpressionValueClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' "),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' T"),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner BETWEEN 1 "),
            CompletionContext::PredicateContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' THEN "),
            CompletionContext::CaseResultClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' THEN N"),
            CompletionContext::CaseResultClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' THEN 'yes' "),
            CompletionContext::CaseContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' THEN 'end' "),
            CompletionContext::CaseContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' THEN 'yes' E"),
            CompletionContext::CaseContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' THEN 'yes' ELSE "),
            CompletionContext::CaseResultClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' THEN 'yes' ELSE 'no' "),
            CompletionContext::CaseContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT CASE WHEN owner = 'app' THEN 'yes' END "),
            CompletionContext::SelectContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users "),
            CompletionContext::FromContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users J"),
            CompletionContext::FromContinuationClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users, "),
            CompletionContext::FromClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users JOIN orders "),
            CompletionContext::JoinConditionClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users JOIN orders O"),
            CompletionContext::JoinConditionClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users ORDER BY"),
            CompletionContext::OrderByClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT name, count(*) FROM users GROUP BY"),
            CompletionContext::GroupByClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT name, count(*) FROM users GROUP BY name HAVING"),
            CompletionContext::HavingClause
        );
        assert_eq!(
            analyzed_context_at_end("INSERT INTO"),
            CompletionContext::FromClause
        );
        assert_eq!(
            analyzed_context_at_end("UPDATE"),
            CompletionContext::FromClause
        );
        assert_eq!(
            analyzed_context_at_end("DELETE FROM"),
            CompletionContext::FromClause
        );
        assert_eq!(
            analyzed_context_at_end("TRUNCATE TABLE"),
            CompletionContext::FromClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users u JOIN orders o ON"),
            CompletionContext::WhereClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users u JOIN orders o USING ("),
            CompletionContext::UsingClause
        );
        assert_eq!(
            analyzed_context_at_end("SELECT * FROM users u JOIN orders o USING (id, "),
            CompletionContext::UsingClause
        );
        assert_eq!(
            analyzed_context_at_end("CREATE INDEX users_email_idx ON"),
            CompletionContext::FromClause
        );
    }

    #[test]
    fn completion_context_ignores_keywords_inside_dollar_quotes() {
        let parser = SqlParser::new();

        let sql = r#"
            CREATE FUNCTION app.hidden_lookup()
            RETURNS integer
            LANGUAGE SQL
            AS $$ SELECT id FROM hidden.internal WHERE active = true; $$;
            SELECT
        "#;

        assert_eq!(
            parser.analyze_completion_context_fallback(sql, position_at_end(sql)),
            CompletionContext::SelectClause
        );
    }

    #[test]
    fn completion_context_respects_keyword_boundaries() {
        let parser = SqlParser::new();
        let sql = "SELECT * FROM order_items ";

        assert_eq!(
            parser.analyze_completion_context_fallback(sql, position_at_end(sql)),
            CompletionContext::FromContinuationClause
        );
    }

    #[test]
    fn fallback_completion_context_uses_latest_grouping_clause() {
        let parser = SqlParser::new();

        let having_sql = "SELECT user_id, count(*) FROM orders GROUP BY user_id HAVING count(*) > ";
        assert_eq!(
            parser.analyze_completion_context_fallback(having_sql, position_at_end(having_sql)),
            CompletionContext::ExpressionValueClause
        );

        let having_continuation_sql =
            "SELECT user_id, count(*) FROM orders GROUP BY user_id HAVING count(*) > 1 ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                having_continuation_sql,
                position_at_end(having_continuation_sql)
            ),
            CompletionContext::PredicateContinuationClause
        );

        let order_sql =
            "SELECT user_id, count(*) FROM orders GROUP BY user_id HAVING count(*) > 1 ORDER BY ";
        assert_eq!(
            parser.analyze_completion_context_fallback(order_sql, position_at_end(order_sql)),
            CompletionContext::OrderByClause
        );

        let group_sql = "SELECT user_id, count(*) FROM orders GROUP BY ";
        assert_eq!(
            parser.analyze_completion_context_fallback(group_sql, position_at_end(group_sql)),
            CompletionContext::GroupByClause
        );

        let group_continuation_sql = "SELECT user_id, count(*) FROM orders GROUP BY user_id ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                group_continuation_sql,
                position_at_end(group_continuation_sql)
            ),
            CompletionContext::GroupByContinuationClause
        );

        let group_continuation_prefix_sql =
            "SELECT user_id, count(*) FROM orders GROUP BY user_id H";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                group_continuation_prefix_sql,
                position_at_end(group_continuation_prefix_sql)
            ),
            CompletionContext::GroupByContinuationClause
        );

        let group_after_comma_sql = "SELECT user_id, count(*) FROM orders GROUP BY user_id, ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                group_after_comma_sql,
                position_at_end(group_after_comma_sql)
            ),
            CompletionContext::GroupByClause
        );

        let order_direction_sql = "SELECT * FROM users ORDER BY name ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                order_direction_sql,
                position_at_end(order_direction_sql)
            ),
            CompletionContext::OrderDirectionClause
        );

        let order_direction_prefix_sql = "SELECT * FROM users ORDER BY name D";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                order_direction_prefix_sql,
                position_at_end(order_direction_prefix_sql)
            ),
            CompletionContext::OrderDirectionClause
        );

        let order_nulls_prefix_sql = "SELECT * FROM users ORDER BY name DESC N";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                order_nulls_prefix_sql,
                position_at_end(order_nulls_prefix_sql)
            ),
            CompletionContext::OrderDirectionClause
        );

        let order_after_direction_sql = "SELECT * FROM users ORDER BY name DESC ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                order_after_direction_sql,
                position_at_end(order_after_direction_sql)
            ),
            CompletionContext::OrderDirectionClause
        );

        let order_after_nulls_sql = "SELECT * FROM users ORDER BY name DESC NULLS FIRST ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                order_after_nulls_sql,
                position_at_end(order_after_nulls_sql)
            ),
            CompletionContext::OrderDirectionClause
        );
    }

    #[test]
    fn fallback_completion_context_handles_dml_table_and_column_positions() {
        let parser = SqlParser::new();

        let insert_table_sql = "INSERT INTO app.us";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                insert_table_sql,
                position_at_end(insert_table_sql)
            ),
            CompletionContext::FromClause
        );

        let insert_action_sql = "INSERT INTO app.users ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                insert_action_sql,
                position_at_end(insert_action_sql)
            ),
            CompletionContext::InsertActionClause
        );

        let insert_action_prefix_sql = "INSERT INTO app.users VAL";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                insert_action_prefix_sql,
                position_at_end(insert_action_prefix_sql)
            ),
            CompletionContext::InsertActionClause
        );

        let insert_column_sql = "INSERT INTO app.users (";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                insert_column_sql,
                position_at_end(insert_column_sql)
            ),
            CompletionContext::SelectClause
        );

        let insert_value_sql = "INSERT INTO app.users (name) VALUES (";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                insert_value_sql,
                position_at_end(insert_value_sql)
            ),
            CompletionContext::InsertValueClause
        );

        let insert_value_prefix_sql = "INSERT INTO app.users (name) VALUES (NU";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                insert_value_prefix_sql,
                position_at_end(insert_value_prefix_sql)
            ),
            CompletionContext::InsertValueClause
        );

        let insert_value_after_comma_sql = "INSERT INTO app.users (owner, name) VALUES ('app', ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                insert_value_after_comma_sql,
                position_at_end(insert_value_after_comma_sql)
            ),
            CompletionContext::InsertValueClause
        );

        let update_value_sql = "UPDATE app.users SET name = ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                update_value_sql,
                position_at_end(update_value_sql)
            ),
            CompletionContext::ExpressionValueClause
        );

        let update_value_prefix_sql = "UPDATE app.users SET name = NU";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                update_value_prefix_sql,
                position_at_end(update_value_prefix_sql)
            ),
            CompletionContext::ExpressionValueClause
        );

        let update_value_continuation_sql = "UPDATE app.users SET name = 'app' ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                update_value_continuation_sql,
                position_at_end(update_value_continuation_sql)
            ),
            CompletionContext::PredicateContinuationClause
        );

        let update_next_assignment_sql = "UPDATE app.users SET name = 'app', ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                update_next_assignment_sql,
                position_at_end(update_next_assignment_sql)
            ),
            CompletionContext::WhereClause
        );

        let update_table_sql = "UPDATE app.us";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                update_table_sql,
                position_at_end(update_table_sql)
            ),
            CompletionContext::FromClause
        );

        let update_action_sql = "UPDATE app.users ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                update_action_sql,
                position_at_end(update_action_sql)
            ),
            CompletionContext::UpdateActionClause
        );

        let update_action_prefix_sql = "UPDATE app.users S";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                update_action_prefix_sql,
                position_at_end(update_action_prefix_sql)
            ),
            CompletionContext::UpdateActionClause
        );

        let update_set_sql = "UPDATE app.users SET ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                update_set_sql,
                position_at_end(update_set_sql)
            ),
            CompletionContext::WhereClause
        );

        let delete_table_sql = "DELETE FROM app.se";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                delete_table_sql,
                position_at_end(delete_table_sql)
            ),
            CompletionContext::FromClause
        );

        let delete_action_sql = "DELETE FROM app.sessions ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                delete_action_sql,
                position_at_end(delete_action_sql)
            ),
            CompletionContext::DeleteActionClause
        );

        let delete_action_prefix_sql = "DELETE FROM app.sessions WH";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                delete_action_prefix_sql,
                position_at_end(delete_action_prefix_sql)
            ),
            CompletionContext::DeleteActionClause
        );

        let create_index_table_sql = "CREATE INDEX users_email_idx ON app.us";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                create_index_table_sql,
                position_at_end(create_index_table_sql)
            ),
            CompletionContext::FromClause
        );

        let create_index_column_sql = "CREATE INDEX users_email_idx ON app.users (";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                create_index_column_sql,
                position_at_end(create_index_column_sql)
            ),
            CompletionContext::SelectClause
        );

        let references_table_sql = "CREATE TABLE app.orders (user_id INT REFERENCES ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                references_table_sql,
                position_at_end(references_table_sql)
            ),
            CompletionContext::FromClause
        );

        let references_table_prefix_sql = "CREATE TABLE app.orders (user_id INT REFERENCES app.us";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                references_table_prefix_sql,
                position_at_end(references_table_prefix_sql)
            ),
            CompletionContext::FromClause
        );

        let references_column_sql = "CREATE TABLE app.orders (user_id INT REFERENCES app.users (";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                references_column_sql,
                position_at_end(references_column_sql)
            ),
            CompletionContext::ReferenceColumnClause
        );

        let references_column_prefix_sql =
            "CREATE TABLE app.orders (user_id INT REFERENCES app.users (id";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                references_column_prefix_sql,
                position_at_end(references_column_prefix_sql)
            ),
            CompletionContext::ReferenceColumnClause
        );

        let references_completed_table_sql =
            "CREATE TABLE app.orders (user_id INT REFERENCES app.users ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                references_completed_table_sql,
                position_at_end(references_completed_table_sql)
            ),
            CompletionContext::ReferenceActionClause
        );

        let references_action_prefix_sql =
            "CREATE TABLE app.orders (user_id INT REFERENCES app.users O";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                references_action_prefix_sql,
                position_at_end(references_action_prefix_sql)
            ),
            CompletionContext::ReferenceActionClause
        );

        let references_on_delete_sql =
            "CREATE TABLE app.orders (user_id INT REFERENCES app.users ON DELETE ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                references_on_delete_sql,
                position_at_end(references_on_delete_sql)
            ),
            CompletionContext::ReferenceRuleClause
        );

        let references_on_update_prefix_sql =
            "CREATE TABLE app.orders (user_id INT REFERENCES app.users ON UPDATE C";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                references_on_update_prefix_sql,
                position_at_end(references_on_update_prefix_sql)
            ),
            CompletionContext::ReferenceRuleClause
        );

        let references_on_delete_rule_sql =
            "CREATE TABLE app.orders (user_id INT REFERENCES app.users ON DELETE CASCADE ";
        assert_ne!(
            parser.analyze_completion_context_fallback(
                references_on_delete_rule_sql,
                position_at_end(references_on_delete_rule_sql)
            ),
            CompletionContext::ReferenceRuleClause
        );

        let references_completed_column_sql =
            "CREATE TABLE app.orders (user_id INT REFERENCES app.users (id)";
        assert_ne!(
            parser.analyze_completion_context_fallback(
                references_completed_column_sql,
                position_at_end(references_completed_column_sql)
            ),
            CompletionContext::ReferenceColumnClause
        );

        let drop_index_sql = "DROP INDEX ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                drop_index_sql,
                position_at_end(drop_index_sql)
            ),
            CompletionContext::IndexTargetClause
        );

        let drop_index_prefix_sql = "DROP INDEX users_email";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                drop_index_prefix_sql,
                position_at_end(drop_index_prefix_sql)
            ),
            CompletionContext::IndexTargetClause
        );

        let alter_drop_column_sql = "ALTER TABLE app.users DROP COLUMN ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_drop_column_sql,
                position_at_end(alter_drop_column_sql)
            ),
            CompletionContext::ColumnTargetClause
        );

        let alter_modify_column_sql = "ALTER TABLE app.users MODIFY COLUMN ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_modify_column_sql,
                position_at_end(alter_modify_column_sql)
            ),
            CompletionContext::ColumnTargetClause
        );

        let alter_modify_column_prefix_sql = "ALTER TABLE app.users MODIFY ow";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_modify_column_prefix_sql,
                position_at_end(alter_modify_column_prefix_sql)
            ),
            CompletionContext::ColumnTargetClause
        );

        let alter_change_column_sql = "ALTER TABLE app.users CHANGE COLUMN ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_change_column_sql,
                position_at_end(alter_change_column_sql)
            ),
            CompletionContext::ColumnTargetClause
        );

        let alter_change_column_prefix_sql = "ALTER TABLE app.users CHANGE ow";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_change_column_prefix_sql,
                position_at_end(alter_change_column_prefix_sql)
            ),
            CompletionContext::ColumnTargetClause
        );

        let alter_action_sql = "ALTER TABLE app.users ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_action_sql,
                position_at_end(alter_action_sql)
            ),
            CompletionContext::AlterTableActionClause
        );

        let alter_action_prefix_sql = "ALTER TABLE app.users DR";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_action_prefix_sql,
                position_at_end(alter_action_prefix_sql)
            ),
            CompletionContext::AlterTableActionClause
        );

        let alter_drop_constraint_sql = "ALTER TABLE app.users DROP CONSTRAINT ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_drop_constraint_sql,
                position_at_end(alter_drop_constraint_sql)
            ),
            CompletionContext::ConstraintTargetClause
        );

        let alter_drop_index_sql = "ALTER TABLE app.users DROP INDEX ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_drop_index_sql,
                position_at_end(alter_drop_index_sql)
            ),
            CompletionContext::IndexTargetClause
        );

        let create_table_type_sql = "CREATE TABLE app.users (name ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                create_table_type_sql,
                position_at_end(create_table_type_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let create_table_type_prefix_sql = "CREATE TABLE app.users (name var";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                create_table_type_prefix_sql,
                position_at_end(create_table_type_prefix_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_add_column_type_sql = "ALTER TABLE app.users ADD COLUMN status ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_add_column_type_sql,
                position_at_end(alter_add_column_type_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_add_column_type_prefix_sql = "ALTER TABLE app.users ADD COLUMN status var";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_add_column_type_prefix_sql,
                position_at_end(alter_add_column_type_prefix_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_add_without_column_type_sql = "ALTER TABLE app.users ADD status ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_add_without_column_type_sql,
                position_at_end(alter_add_without_column_type_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_modify_column_type_sql = "ALTER TABLE app.users MODIFY COLUMN status ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_modify_column_type_sql,
                position_at_end(alter_modify_column_type_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_modify_type_prefix_sql = "ALTER TABLE app.users MODIFY status var";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_modify_type_prefix_sql,
                position_at_end(alter_modify_type_prefix_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_change_column_type_sql = "ALTER TABLE app.users CHANGE COLUMN status state ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_change_column_type_sql,
                position_at_end(alter_change_column_type_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_change_type_prefix_sql = "ALTER TABLE app.users CHANGE status state var";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_change_type_prefix_sql,
                position_at_end(alter_change_type_prefix_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_column_type_sql = "ALTER TABLE app.users ALTER COLUMN status TYPE ";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_column_type_sql,
                position_at_end(alter_column_type_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_column_type_prefix_sql = "ALTER TABLE app.users ALTER COLUMN status TYPE var";
        assert_eq!(
            parser.analyze_completion_context_fallback(
                alter_column_type_prefix_sql,
                position_at_end(alter_column_type_prefix_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let create_table_constraint_sql = "CREATE TABLE app.users (PRIMARY ";
        assert_ne!(
            parser.analyze_completion_context_fallback(
                create_table_constraint_sql,
                position_at_end(create_table_constraint_sql)
            ),
            CompletionContext::DataTypeClause
        );

        let alter_add_index_sql = "ALTER TABLE app.users ADD INDEX users_name_idx";
        assert_ne!(
            parser.analyze_completion_context_fallback(
                alter_add_index_sql,
                position_at_end(alter_add_index_sql)
            ),
            CompletionContext::DataTypeClause
        );
    }

    #[test]
    fn converts_lsp_utf16_position_to_tree_sitter_byte_position() {
        let sql = "SELECT '😀' FROM users WHERE ";
        let lsp_position = Position {
            line: 0,
            character: sql.encode_utf16().count() as u32,
        };

        assert_eq!(
            SqlParser::lsp_position_to_byte_position(sql, lsp_position),
            Position {
                line: 0,
                character: sql.len() as u32,
            }
        );

        let inside_surrogate_pair = Position {
            line: 0,
            character: "SELECT '😀".encode_utf16().count() as u32 - 1,
        };
        let byte_position = SqlParser::lsp_position_to_byte_position(sql, inside_surrogate_pair);
        assert_eq!(byte_position.character as usize, "SELECT '".len());
    }

    #[test]
    fn suppresses_diagnostics_for_interactive_trailing_sql() {
        let samples = [
            "SELECT",
            "SELECT * FROM",
            "SELECT * FROM public.users WHERE",
            "SELECT * FROM public.users WHERE id =",
            "SELECT id,",
            "SELECT * FROM public.",
        ];

        for sql in samples {
            let mut parser = SqlParser::new();
            let result = parser.parse(sql);
            assert!(
                result.diagnostics.is_empty(),
                "interactive SQL should not report diagnostics for {sql:?}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn keeps_diagnostics_for_closed_invalid_sql() {
        let mut parser = SqlParser::new();
        let result = parser.parse("SELECT * FROM users WHERE id = )");

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR)),
            "closed invalid SQL should retain an error diagnostic: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn extracts_aliases_and_references_from_qualified_selects() {
        let sql = r#"
            SELECT u.id, o.total
            FROM "public"."users" u
            JOIN `shop`.`orders` AS o ON o.user_id = u.id
            WHERE EXISTS (
                SELECT 1 FROM audit.events e WHERE e.user_id = u.id
            )
        "#;
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should parse");

        let aliases = parser.extract_aliases(tree, sql);
        assert_eq!(aliases.get("u"), Some(&"public.users".to_string()));
        assert_eq!(aliases.get("o"), Some(&"shop.orders".to_string()));
        assert_eq!(aliases.get("e"), Some(&"audit.events".to_string()));

        let references = parser.extract_referenced_tables(tree, sql);
        assert!(references.contains(&"public.users".to_string()));
        assert!(references.contains(&"shop.orders".to_string()));
        assert!(references.contains(&"audit.events".to_string()));
    }

    #[test]
    fn extracts_references_from_dml_and_ddl_statements() {
        let sql = r#"
            UPDATE app.users u SET name = 'x' WHERE u.id = 1;
            INSERT INTO app.orders (user_id) VALUES (1);
            DELETE FROM app.sessions s WHERE s.user_id = 1;
            CREATE TABLE IF NOT EXISTS app.invoices (id int);
            DROP VIEW IF EXISTS app.active_users;
        "#;
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should parse");

        let aliases = parser.extract_aliases(tree, sql);
        assert_eq!(aliases.get("u"), Some(&"app.users".to_string()));
        assert_eq!(aliases.get("s"), Some(&"app.sessions".to_string()));

        let references = parser.extract_referenced_tables(tree, sql);
        assert!(references.contains(&"app.users".to_string()));
        assert!(references.contains(&"app.orders".to_string()));
        assert!(references.contains(&"app.sessions".to_string()));
        assert!(references.contains(&"app.invoices".to_string()));
        assert!(references.contains(&"app.active_users".to_string()));
    }

    #[test]
    fn respects_keyword_boundaries_when_extracting_references() {
        let sql = "SELECT * FROM information_schema.tables";
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should parse");

        let references = parser.extract_referenced_tables(tree, sql);
        assert_eq!(references, vec!["information_schema.tables".to_string()]);
    }

    #[test]
    fn ignores_keywords_inside_comments_literals_and_quoted_identifiers() {
        let sql = r#"
            -- FROM hidden.comment_table c
            SELECT 'JOIN hidden.literal_table l', "FROM", `JOIN`
            FROM public.users u
            /* UPDATE hidden.block_table b */
            WHERE u.note = 'FROM hidden.note_table n'
        "#;
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should parse");

        let aliases = parser.extract_aliases(tree, sql);
        assert_eq!(aliases.get("u"), Some(&"public.users".to_string()));
        assert!(!aliases.contains_key("c"));
        assert!(!aliases.contains_key("l"));
        assert!(!aliases.contains_key("b"));
        assert!(!aliases.contains_key("n"));

        let references = parser.extract_referenced_tables(tree, sql);
        assert_eq!(references, vec!["public.users".to_string()]);
    }

    #[test]
    fn ignores_references_inside_dollar_quoted_bodies() {
        let sql = r#"
            CREATE FUNCTION app.hidden_lookup()
            RETURNS integer
            LANGUAGE SQL
            AS $body$ SELECT id FROM hidden.internal WHERE active = true; $body$;
            SELECT * FROM public.users;
        "#;
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should parse");

        let references = parser.extract_referenced_tables(tree, sql);
        assert_eq!(references, vec!["public.users".to_string()]);
    }

    #[test]
    fn ignores_cte_names_when_extracting_references() {
        let sql = r#"
            WITH RECURSIVE recent_orders AS MATERIALIZED (
                SELECT * FROM app.orders WHERE created_at > now() - interval '7 days'
            ),
            user_rollup(user_id, total) AS NOT MATERIALIZED (
                SELECT user_id, count(*) FROM recent_orders GROUP BY user_id
            )
            SELECT u.id, r.total
            FROM user_rollup r
            JOIN app.users u ON u.id = r.user_id
            JOIN public.recent_orders archived ON archived.user_id = u.id;
        "#;
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should parse");

        let references = parser.extract_referenced_tables(tree, sql);
        assert!(references.contains(&"app.orders".to_string()));
        assert!(references.contains(&"app.users".to_string()));
        assert!(references.contains(&"public.recent_orders".to_string()));
        assert!(!references.contains(&"recent_orders".to_string()));
        assert!(!references.contains(&"user_rollup".to_string()));
    }

    #[test]
    fn extracts_on_relation_for_index_trigger_and_policy_ddl() {
        let sql = r#"
            CREATE INDEX users_email_idx ON app.users (email);
            DROP INDEX old_users_idx ON app.users;
            CREATE TRIGGER audit_users AFTER INSERT ON app.users
                FOR EACH ROW EXECUTE FUNCTION audit_user_change();
            CREATE POLICY users_tenant_policy ON app.users USING (tenant_id = current_setting('app.tenant_id')::int);
            SELECT * FROM app.users u JOIN app.orders o ON o.user_id = u.id;
        "#;
        let mut parser = SqlParser::new();
        let result = parser.parse(sql);
        let tree = result.tree.as_ref().expect("SQL should parse");

        let references = parser.extract_referenced_tables(tree, sql);
        assert!(references.contains(&"app.users".to_string()));
        assert!(references.contains(&"app.orders".to_string()));
        assert!(!references.contains(&"o.user_id".to_string()));
    }
}
