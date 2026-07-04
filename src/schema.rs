use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

/// Schema ID，用于隔离不同的 schema 数据
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SchemaId(pub Uuid);

impl SchemaId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl FromStr for SchemaId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl Default for SchemaId {
    fn default() -> Self {
        Self::new()
    }
}

/// 数据库 Schema 信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    /// Schema ID
    pub id: SchemaId,
    /// 数据库名称
    pub database: String,
    /// 表列表
    pub tables: Vec<Table>,
    /// 函数列表
    pub functions: Vec<Function>,
    /// Schema 定义文件的 URI (可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
}

/// 表信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Table {
    /// 表名
    pub name: String,
    /// 对象类型，例如 BASE TABLE / VIEW / SYSTEM VIEW
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    /// 列列表
    pub columns: Vec<Column>,
    /// 索引列表
    #[serde(default)]
    pub indexes: Vec<Index>,
    /// 约束列表
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    /// 表注释
    pub comment: Option<String>,
    /// 表定义位置的 URI 和行号 (可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<(String, u32)>,
}

/// 列信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Column {
    /// 列名
    pub name: String,
    /// 数据类型
    pub data_type: String,
    /// 是否可空
    pub nullable: bool,
    /// 是否主键列
    #[serde(default)]
    pub primary_key: bool,
    /// 是否唯一列
    #[serde(default)]
    pub unique: bool,
    /// 是否索引列
    #[serde(default)]
    pub indexed: bool,
    /// 列注释
    pub comment: Option<String>,
    /// 列定义位置的 URI 和行号 (可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<(String, u32)>,
}

/// 索引信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    /// 索引名
    pub name: String,
    /// 索引列
    #[serde(default)]
    pub columns: Vec<String>,
    /// 是否唯一索引
    #[serde(default)]
    pub is_unique: bool,
    /// 是否主键索引
    #[serde(default)]
    pub is_primary: bool,
    /// 索引类型
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_type: Option<String>,
    /// 数据库原始定义
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
}

/// 约束信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Constraint {
    /// 约束名
    pub name: String,
    /// 约束类型，例如 PRIMARY KEY / FOREIGN KEY / UNIQUE / CHECK
    pub constraint_type: String,
    /// 本表列
    #[serde(default)]
    pub columns: Vec<String>,
    /// 引用 schema
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referenced_schema: Option<String>,
    /// 引用表
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referenced_table: Option<String>,
    /// 引用列
    #[serde(default)]
    pub referenced_columns: Vec<String>,
    /// 数据库原始定义
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
}

/// 函数信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    /// 函数名
    pub name: String,
    /// 函数/过程类型，例如 function / procedure
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine_type: Option<String>,
    /// 参数列表
    pub parameters: Vec<FunctionParameter>,
    /// 返回类型
    pub return_type: String,
    /// 函数描述
    pub description: Option<String>,
}

/// 函数参数信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionParameter {
    /// 参数名
    pub name: String,
    /// 参数类型
    pub data_type: String,
    /// 是否可选
    pub optional: bool,
}

impl Function {
    pub fn routine_kind(&self) -> &'static str {
        match self.routine_type.as_deref().map(str::to_ascii_lowercase) {
            Some(value) if value == "procedure" => "Procedure",
            _ => "Function",
        }
    }

    pub fn signature(&self) -> String {
        let args = self
            .parameters
            .iter()
            .map(|parameter| {
                let data_type = parameter.data_type.trim();
                let name = parameter.name.trim();
                if name.is_empty() {
                    data_type.to_string()
                } else if data_type.is_empty() {
                    name.to_string()
                } else {
                    format!("{name} {data_type}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        format!("{}({})", self.name, args)
    }

    pub fn documentation(&self) -> String {
        let mut lines = Vec::new();

        if let Some(description) = self
            .description
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push(description.to_string());
            lines.push(String::new());
        }

        if self.routine_kind() == "Procedure" {
            lines.push("Routine type: procedure".to_string());
        } else {
            lines.push(format!("Returns: {}", self.return_type));
        }

        if !self.parameters.is_empty() {
            lines.push(String::new());
            lines.push("Parameters:".to_string());
            for parameter in &self.parameters {
                let optional = if parameter.optional {
                    " (optional)"
                } else {
                    ""
                };
                let name = parameter.name.trim();
                if name.is_empty() {
                    lines.push(format!("- {}{}", parameter.data_type, optional));
                } else {
                    lines.push(format!(
                        "- {}: {}{}",
                        parameter.name, parameter.data_type, optional
                    ));
                }
            }
        }

        lines.join("\n")
    }

    pub fn markdown_documentation(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "**{}**: `{}`",
            self.routine_kind(),
            self.signature()
        ));

        if let Some(description) = self
            .description
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push(String::new());
            lines.push(description.to_string());
        }

        lines.push(String::new());
        if self.routine_kind() == "Procedure" {
            lines.push("**Routine type**: `procedure`".to_string());
        } else {
            lines.push(format!("**Returns**: `{}`", self.return_type));
        }

        if !self.parameters.is_empty() {
            lines.push(String::new());
            lines.push("**Parameters**:".to_string());
            for parameter in &self.parameters {
                let optional = if parameter.optional {
                    " (optional)"
                } else {
                    ""
                };
                let name = parameter.name.trim();
                if name.is_empty() {
                    lines.push(format!("- `{}`{}", parameter.data_type, optional));
                } else {
                    lines.push(format!(
                        "- `{}`: `{}`{}",
                        parameter.name, parameter.data_type, optional
                    ));
                }
            }
        }

        lines.join("\n")
    }
}

impl Table {
    pub fn object_kind(&self) -> &'static str {
        let Some(object_type) = self.object_type.as_deref() else {
            return "Table";
        };
        let normalized = object_type.to_ascii_uppercase();
        if normalized.contains("MATERIALIZED") && normalized.contains("VIEW") {
            "Materialized View"
        } else if normalized.contains("VIEW") {
            "View"
        } else {
            "Table"
        }
    }

    pub fn documentation(&self) -> Option<String> {
        let mut lines = Vec::new();

        if self
            .object_type
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            lines.push(format!("Object: {}", self.object_kind()));
        }

        if let Some(comment) = self.comment.as_deref().filter(|value| !value.is_empty()) {
            lines.push(comment.to_string());
        }

        if !self.columns.is_empty() {
            let preview = self
                .columns
                .iter()
                .take(8)
                .map(|column| format!("{} {}", column.name, column.data_type))
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if self.columns.len() > 8 { ", ..." } else { "" };
            lines.push(format!("Columns: {}{}", preview, suffix));
        }

        let primary_columns = self
            .indexes
            .iter()
            .find(|index| index.is_primary)
            .map(|index| index.columns.clone())
            .or_else(|| {
                self.constraints
                    .iter()
                    .find(|constraint| {
                        constraint
                            .constraint_type
                            .eq_ignore_ascii_case("PRIMARY KEY")
                    })
                    .map(|constraint| constraint.columns.clone())
            })
            .unwrap_or_else(|| {
                self.columns
                    .iter()
                    .filter(|column| column.primary_key)
                    .map(|column| column.name.clone())
                    .collect()
            });
        if !primary_columns.is_empty() {
            lines.push(format!("Primary key: {}", primary_columns.join(", ")));
        }

        let unique_count = self
            .indexes
            .iter()
            .filter(|index| index.is_unique && !index.is_primary)
            .count();
        let foreign_key_count = self
            .constraints
            .iter()
            .filter(|constraint| {
                constraint
                    .constraint_type
                    .eq_ignore_ascii_case("FOREIGN KEY")
            })
            .count();
        if unique_count > 0 || foreign_key_count > 0 || !self.indexes.is_empty() {
            lines.push(format!(
                "Indexes: {}, unique: {}, foreign keys: {}",
                self.indexes.len(),
                unique_count,
                foreign_key_count
            ));
        }

        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }
}

impl Column {
    pub fn documentation(&self) -> Option<String> {
        let mut lines = vec![format!("Type: {}", self.data_type)];
        lines.push(if self.nullable {
            "Nullable: yes".to_string()
        } else {
            "Nullable: no".to_string()
        });

        let mut tags = Vec::new();
        if self.primary_key {
            tags.push("primary key");
        }
        if self.unique {
            tags.push("unique");
        }
        if self.indexed {
            tags.push("indexed");
        }
        if !tags.is_empty() {
            lines.push(format!("Attributes: {}", tags.join(", ")));
        }

        if let Some(comment) = self.comment.as_deref().filter(|value| !value.is_empty()) {
            lines.push(comment.to_string());
        }

        Some(lines.join("\n"))
    }
}

/// Schema 管理器，用于管理和隔离不同的 schema
#[derive(Debug, Clone)]
pub struct SchemaManager {
    /// Schema 存储，使用 DashMap 实现线程安全的并发访问
    schemas: Arc<DashMap<SchemaId, Schema>>,
}

impl SchemaManager {
    pub fn new() -> Self {
        Self {
            schemas: Arc::new(DashMap::new()),
        }
    }

    /// 注册一个新的 schema
    pub fn register(&self, schema: Schema) -> SchemaId {
        let id = schema.id;
        self.schemas.insert(id, schema);
        id
    }

    /// 获取指定的 schema
    pub fn get(&self, id: SchemaId) -> Option<Schema> {
        self.schemas.get(&id).map(|s| s.clone())
    }

    /// 更新 schema
    pub fn update(&self, id: SchemaId, schema: Schema) -> bool {
        if self.schemas.contains_key(&id) {
            self.schemas.insert(id, schema);
            true
        } else {
            false
        }
    }

    /// 删除 schema
    pub fn remove(&self, id: SchemaId) -> bool {
        self.schemas.remove(&id).is_some()
    }

    /// 列出所有 schema ID
    pub fn list_ids(&self) -> Vec<SchemaId> {
        self.schemas.iter().map(|entry| *entry.key()).collect()
    }

    /// 清空所有 schema
    pub fn clear(&self) {
        self.schemas.clear();
    }
}

impl Default for SchemaManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_id() {
        let id1 = SchemaId::new();
        let id2 = SchemaId::new();
        assert_ne!(id1, id2);

        let id_str = id1.0.to_string();
        let id3 = SchemaId::from_str(&id_str).unwrap();
        assert_eq!(id1, id3);
    }

    #[test]
    fn test_schema_manager() {
        let manager = SchemaManager::new();

        let schema = Schema {
            id: SchemaId::new(),
            database: "test_db".to_string(),
            tables: vec![],
            functions: vec![],
            source_uri: None,
        };

        let id = manager.register(schema.clone());
        assert_eq!(id, schema.id);

        let retrieved = manager.get(id).unwrap();
        assert_eq!(retrieved.database, "test_db");

        manager.remove(id);
        assert!(manager.get(id).is_none());
    }

    #[tokio::test]
    async fn test_schema_manager_concurrent() {
        let manager = SchemaManager::new();
        let manager_clone = manager.clone();

        let schema1 = Schema {
            id: SchemaId::new(),
            database: "db1".to_string(),
            tables: vec![],
            functions: vec![],
            source_uri: None,
        };

        let schema2 = Schema {
            id: SchemaId::new(),
            database: "db2".to_string(),
            tables: vec![],
            functions: vec![],
            source_uri: None,
        };

        let id1 = manager.register(schema1);
        let id2 = manager_clone.register(schema2);

        assert_eq!(manager.get(id1).unwrap().database, "db1");
        assert_eq!(manager_clone.get(id2).unwrap().database, "db2");

        assert_ne!(id1, id2);
    }
}
