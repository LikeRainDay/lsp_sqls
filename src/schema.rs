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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    /// 表名
    pub name: String,
    /// 列列表
    pub columns: Vec<Column>,
    /// 表注释
    pub comment: Option<String>,
    /// 表定义位置的 URI 和行号 (可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<(String, u32)>,
}

/// 列信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    /// 列名
    pub name: String,
    /// 数据类型
    pub data_type: String,
    /// 是否可空
    pub nullable: bool,
    /// 列注释
    pub comment: Option<String>,
    /// 列定义位置的 URI 和行号 (可选)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<(String, u32)>,
}

/// 函数信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    /// 函数名
    pub name: String,
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
        };

        let schema2 = Schema {
            id: SchemaId::new(),
            database: "db2".to_string(),
            tables: vec![],
            functions: vec![],
        };

        let id1 = manager.register(schema1);
        let id2 = manager_clone.register(schema2);

        assert_eq!(manager.get(id1).unwrap().database, "db1");
        assert_eq!(manager_clone.get(id2).unwrap().database, "db2");

        assert_ne!(id1, id2);
    }
}
