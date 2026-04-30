//! Token 类型定义
//! 参考 sqls-server/sqls 的 token 实现
//! https://github.com/sqls-server/sqls/tree/master/token

use tower_lsp::lsp_types::Position;

/// Token 类型
/// 参考 sqls 的 token 类型定义
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenType {
    // 关键字
    Keyword,
    // 标识符（表名、列名等）
    Identifier,
    // 字符串字面量
    String,
    // 数字字面量
    Number,
    // 操作符
    Operator,
    // 分隔符（逗号、分号等）
    Delimiter,
    // 注释
    Comment,
    // 空白
    Whitespace,
    // 未知/错误
    Unknown,
}

/// Token 信息
#[derive(Debug, Clone)]
pub struct Token {
    /// Token 类型
    pub token_type: TokenType,
    /// Token 文本内容
    pub text: String,
    /// Token 在源代码中的位置
    pub position: Position,
    /// Token 长度
    pub length: u32,
}

impl Token {
    pub fn new(token_type: TokenType, text: String, position: Position) -> Self {
        let length = text.len() as u32;
        Self {
            token_type,
            text,
            position,
            length,
        }
    }
}

/// SQL 关键字列表（参考 sqls）
pub struct Keywords;

impl Keywords {
    /// 获取所有 SQL 关键字
    pub fn all() -> Vec<&'static str> {
        vec![
            // DML
            "SELECT",
            "INSERT",
            "UPDATE",
            "DELETE",
            "REPLACE",
            // DDL
            "CREATE",
            "DROP",
            "ALTER",
            "TRUNCATE",
            "RENAME",
            // 表相关
            "TABLE",
            "INDEX",
            "VIEW",
            "DATABASE",
            "SCHEMA",
            // JOIN
            "JOIN",
            "INNER",
            "LEFT",
            "RIGHT",
            "FULL",
            "OUTER",
            "CROSS",
            "ON",
            // WHERE/HAVING
            "WHERE",
            "HAVING",
            "AND",
            "OR",
            "NOT",
            "IN",
            "LIKE",
            "BETWEEN",
            "IS",
            "NULL",
            // GROUP/ORDER
            "GROUP",
            "BY",
            "ORDER",
            "ASC",
            "DESC",
            // LIMIT/OFFSET
            "LIMIT",
            "OFFSET",
            "FETCH",
            // 聚合函数
            "COUNT",
            "SUM",
            "AVG",
            "MAX",
            "MIN",
            "DISTINCT",
            // 其他
            "AS",
            "FROM",
            "INTO",
            "SET",
            "VALUES",
            "UNION",
            "ALL",
            "EXISTS",
            "WITH",
            "EXCEPT",
            "CASE",
            "WHEN",
            "THEN",
            "ELSE",
            "END",
            "IF",
            "ELSEIF",
            "ELSE",
            "ENDIF",
            "FOR",
            "WHILE",
            "LOOP",
            "END",
            "DECLARE",
            "BEGIN",
            "COMMIT",
            "ROLLBACK",
            "TRANSACTION",
            "GRANT",
            "REVOKE",
            "PRIVILEGES",
        ]
    }

    /// 检查是否是关键字
    pub fn is_keyword(text: &str) -> bool {
        Self::all().contains(&text.to_uppercase().as_str())
    }

    /// 获取 MySQL 特定关键字
    pub fn mysql() -> Vec<&'static str> {
        vec![
            "AUTO_INCREMENT",
            "ENGINE",
            "CHARSET",
            "COLLATE",
            "SHOW",
            "DESCRIBE",
            "EXPLAIN",
            "USE",
            "LOCK",
            "UNLOCK",
            "TABLES",
        ]
    }

    /// 获取 PostgreSQL 特定关键字
    pub fn postgres() -> Vec<&'static str> {
        vec![
            "ILIKE",
            "SIMILAR",
            "TO",
            "ARRAY",
            "JSONB",
            "RETURNING",
            "WITH",
            "RECURSIVE",
        ]
    }

    /// 获取 Hive 特定关键字
    pub fn hive() -> Vec<&'static str> {
        vec![
            "PARTITION",
            "PARTITIONED",
            "CLUSTERED",
            "SORTED",
            "STORED",
            "AS",
            "TEXTFILE",
            "ORCFILE",
            "PARQUET",
            "LATERAL",
            "VIEW",
            "EXPLODE",
        ]
    }

    /// 获取 BigQuery 特定关键字
    pub fn bigquery() -> Vec<&'static str> {
        vec![
            "MERGE",
            "WINDOW",
            "QUALIFY",
            "UNNEST",
            "STRUCT",
            "ARRAY",
            "PARTITION",
            "CLUSTER",
            "OPTIONS",
            "SYSTEM_TIME",
            "OF",
        ]
    }
}

/// 操作符列表
pub struct Operators;

impl Operators {
    /// 获取所有操作符
    pub fn all() -> Vec<&'static str> {
        vec![
            // 算术操作符
            "+", "-", "*", "/", "%", // 比较操作符
            "=", "!=", "<>", "<", ">", "<=", ">=", "<=>", // 逻辑操作符
            "AND", "OR", "NOT", // 其他
            "||", "&&", "::", "->", "->>", "#>", "#>>",
        ]
    }

    /// 检查是否是操作符
    pub fn is_operator(text: &str) -> bool {
        Self::all().contains(&text)
    }
}

/// 分隔符列表
pub struct Delimiters;

impl Delimiters {
    /// 获取所有分隔符
    pub fn all() -> Vec<&'static str> {
        vec![",", ";", ".", "(", ")", "[", "]", "{", "}", ":"]
    }

    /// 检查是否是分隔符
    pub fn is_delimiter(text: &str) -> bool {
        Self::all().contains(&text)
    }
}
