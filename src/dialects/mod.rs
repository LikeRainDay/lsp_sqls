pub mod clickhouse;
pub(crate) mod common;
pub mod elasticsearch_dsl;
pub mod elasticsearch_eql;
pub mod hive;
pub mod mongodb;
pub mod mysql;
pub mod postgres;
pub mod redis;

pub use clickhouse::ClickHouseDialect;
pub use elasticsearch_dsl::ElasticsearchDslDialect;
pub use elasticsearch_eql::ElasticsearchEqlDialect;
pub use hive::HiveDialect;
pub use mongodb::MongoDbDialect;
pub use mysql::MysqlDialect;
pub use postgres::PostgresDialect;
pub use redis::RedisDialect;

use crate::dialect::Dialect;
use std::collections::HashMap;
use std::sync::Arc;

/// 方言注册表
pub struct DialectRegistry {
    dialects: Vec<Arc<dyn Dialect>>,
    aliases: HashMap<String, Arc<dyn Dialect>>,
}

impl DialectRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            dialects: Vec::new(),
            aliases: HashMap::new(),
        };

        // 注册所有方言
        registry.register(Arc::new(MysqlDialect::new()));
        registry.register(Arc::new(PostgresDialect::new()));
        registry.register(Arc::new(HiveDialect::new()));
        registry.register(Arc::new(ElasticsearchEqlDialect::new()));
        registry.register(Arc::new(ElasticsearchDslDialect::new()));
        registry.register(Arc::new(ClickHouseDialect::new()));
        registry.register(Arc::new(RedisDialect::new()));
        let mongodb = Arc::new(MongoDbDialect::new());
        registry.register(mongodb.clone());
        registry.register_alias("mongo", mongodb.clone());
        registry.register_alias("json", mongodb);

        registry
    }

    pub fn register(&mut self, dialect: Arc<dyn Dialect>) {
        self.dialects.push(dialect);
    }

    pub fn register_alias(&mut self, alias: &str, dialect: Arc<dyn Dialect>) {
        self.aliases.insert(alias.to_lowercase(), dialect);
    }

    pub fn get_by_name(&self, name: &str) -> Option<Arc<dyn Dialect>> {
        if let Some(dialect) = self.aliases.get(&name.to_lowercase()) {
            return Some(dialect.clone());
        }

        self.dialects
            .iter()
            .find(|d| d.name().eq_ignore_ascii_case(name))
            .cloned()
    }

    pub fn list_names(&self) -> Vec<String> {
        self.dialects.iter().map(|d| d.name().to_string()).collect()
    }
}

impl Default for DialectRegistry {
    fn default() -> Self {
        Self::new()
    }
}
