use crate::dialect::Dialect;
use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, Hover, Location,
    MarkedString, NumberOrString, Position, Range,
};

pub struct RedisDialect;

impl Default for RedisDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl RedisDialect {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Dialect for RedisDialect {
    fn name(&self) -> &str {
        "redis"
    }

    async fn parse(&self, sql: &str, _schema: Option<&Schema>) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        if sql.trim().is_empty() {
            return diagnostics;
        }

        // Redis 查询语言（RediSearch/RedisGraph）特定的语法检查
        // Redis 支持多种查询语言，这里以 RediSearch 为例
        if sql.to_uppercase().contains("FT.SEARCH") && !sql.contains("@") {
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
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String("REDIS001".to_string())),
                code_description: None,
                source: Some("redis".to_string()),
                message: "FT.SEARCH query might benefit from field filters using @field"
                    .to_string(),
                related_information: None,
                tags: None,
                data: None,
            });
        }

        diagnostics
    }

    async fn completion(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();
        let prefix = crate::position::cursor_token_prefix(sql, position, is_token_char);

        // Redis 基础命令
        let basic_commands = vec![
            "SET",
            "GET",
            "DEL",
            "EXISTS",
            "EXPIRE",
            "TTL",
            "PERSIST",
            "HSET",
            "HGET",
            "HGETALL",
            "HDEL",
            "HKEYS",
            "HVALS",
            "HINCRBY",
            "LPUSH",
            "RPUSH",
            "LPOP",
            "RPOP",
            "LLEN",
            "LRANGE",
            "LINDEX",
            "SADD",
            "SMEMBERS",
            "SREM",
            "SCARD",
            "SISMEMBER",
            "SINTER",
            "ZADD",
            "ZRANGE",
            "ZREM",
            "ZCARD",
            "ZSCORE",
            "ZRANK",
        ];

        // RediSearch 命令
        let search_commands = [
            "FT.SEARCH",
            "FT.AGGREGATE",
            "FT.CREATE",
            "FT.DROPINDEX",
            "FT.INFO",
            "FT.ALIASADD",
            "FT.ALIASDEL",
            "FT.ALIASUPDATE",
            "FT.SUGADD",
            "FT.SUGGET",
            "FT.SUGDEL",
            "FT.SUGLEN",
        ];

        // RedisGraph 命令
        let graph_commands = [
            "GRAPH.QUERY",
            "GRAPH.DELETE",
            "GRAPH.EXPLAIN",
            "GRAPH.PROFILE",
        ];

        // RedisJSON 命令
        let json_commands = [
            "JSON.GET",
            "JSON.SET",
            "JSON.DEL",
            "JSON.MGET",
            "JSON.KEYS",
            "JSON.ARRAPPEND",
            "JSON.ARRINDEX",
            "JSON.ARRINSERT",
            "JSON.ARRLEN",
            "JSON.ARRPOP",
            "JSON.OBJKEYS",
            "JSON.OBJLEN",
        ];

        let commands: Vec<&str> = basic_commands
            .iter()
            .chain(search_commands.iter())
            .chain(graph_commands.iter())
            .chain(json_commands.iter())
            .copied()
            .collect();

        for cmd in commands {
            if !prefix.is_empty() && !cmd.to_ascii_lowercase().starts_with(&prefix) {
                continue;
            }

            let (kind, detail_prefix) = if cmd.starts_with("FT.") {
                (CompletionItemKind::FUNCTION, "RediSearch command")
            } else if cmd.starts_with("GRAPH.") {
                (CompletionItemKind::FUNCTION, "RedisGraph command")
            } else if cmd.starts_with("JSON.") {
                (CompletionItemKind::FUNCTION, "RedisJSON command")
            } else {
                (CompletionItemKind::FUNCTION, "Redis command")
            };

            items.push(CompletionItem {
                label: cmd.to_string(),
                kind: Some(kind),
                detail: Some(format!("{}: {}", detail_prefix, cmd)),
                documentation: None,
                deprecated: None,
                preselect: None,
                sort_text: Some(format!("0{}", cmd)),
                filter_text: Some(cmd.to_string()),
                insert_text: Some(cmd.to_string()),
                insert_text_format: None,
                text_edit: None,
                additional_text_edits: None,
                commit_characters: None,
                command: None,
                data: None,
                tags: None,
                insert_text_mode: None,
                label_details: None,
            });
        }

        // RediSearch 查询语法关键字和操作符
        // 注意：Redis 是命令式语言，这些是 RediSearch 查询字符串中的操作符
        let keywords = vec![
            "@",   // 字段前缀，如 @field:value
            "AND", // 逻辑与
            "OR",  // 逻辑或
            "NOT", // 逻辑非
            "-",   // 排除操作符
            "+",   // 必须包含
            "~",   // 模糊匹配
            "|",   // 或操作符（在聚合中使用）
            "(",   // 分组开始
            ")",   // 分组结束
            "{",   // 范围查询开始
            "}",   // 范围查询结束
            "[",   // 范围查询开始（包含）
            "]",   // 范围查询结束（包含）
        ];

        for keyword in keywords {
            if !prefix.is_empty() && !keyword.to_ascii_lowercase().starts_with(&prefix) {
                continue;
            }

            let detail = match keyword {
                "@" => "Field prefix for RediSearch queries (e.g., @field:value)",
                "AND" => "Logical AND operator",
                "OR" => "Logical OR operator",
                "NOT" => "Logical NOT operator",
                "-" => "Exclude operator (must not contain)",
                "+" => "Must contain operator",
                "~" => "Fuzzy match operator",
                "|" => "OR operator (used in aggregations)",
                "(" | ")" => "Grouping parentheses",
                "{" | "}" => "Range query braces (exclusive)",
                "[" | "]" => "Range query brackets (inclusive)",
                _ => "RediSearch query operator",
            };

            items.push(CompletionItem {
                label: keyword.to_string(),
                kind: Some(CompletionItemKind::OPERATOR),
                detail: Some(detail.to_string()),
                documentation: None,
                deprecated: None,
                preselect: None,
                sort_text: Some(format!("1{}", keyword)),
                filter_text: Some(keyword.to_string()),
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
            });
        }

        if let Some(schema) = schema {
            for table in &schema.tables {
                if !prefix.is_empty() && !table.name.to_ascii_lowercase().starts_with(&prefix) {
                    continue;
                }

                items.push(CompletionItem {
                    label: table.name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(format!("Redis Index/Key: {}", table.name)),
                    documentation: table
                        .comment
                        .clone()
                        .map(tower_lsp::lsp_types::Documentation::String),
                    deprecated: None,
                    preselect: None,
                    sort_text: Some(format!("2{}", table.name)),
                    filter_text: Some(table.name.clone()),
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
                                "Redis Index/Key: {}\n{}",
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

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '@')
}
