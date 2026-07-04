pub mod clickhouse;
pub(crate) mod common;
pub mod elasticsearch_dsl;
pub mod elasticsearch_eql;
pub mod hive;
pub mod mysql;
pub mod postgres;
pub mod redis;

pub use clickhouse::ClickHouseDialect;
pub use elasticsearch_dsl::ElasticsearchDslDialect;
pub use elasticsearch_eql::ElasticsearchEqlDialect;
pub use hive::HiveDialect;
pub use mysql::MysqlDialect;
pub use postgres::PostgresDialect;
pub use redis::RedisDialect;

use crate::dialect::Dialect;
use std::sync::Arc;

/// 方言注册表
pub struct DialectRegistry {
    dialects: Vec<Arc<dyn Dialect>>,
}

impl DialectRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            dialects: Vec::new(),
        };

        // 注册所有方言
        registry.register(Arc::new(MysqlDialect::new()));
        registry.register(Arc::new(PostgresDialect::new()));
        registry.register(Arc::new(HiveDialect::new()));
        registry.register(Arc::new(ElasticsearchEqlDialect::new()));
        registry.register(Arc::new(ElasticsearchDslDialect::new()));
        registry.register(Arc::new(ClickHouseDialect::new()));
        registry.register(Arc::new(RedisDialect::new()));

        registry
    }

    pub fn register(&mut self, dialect: Arc<dyn Dialect>) {
        self.dialects.push(dialect);
    }

    pub fn get_by_name(&self, name: &str) -> Option<Arc<dyn Dialect>> {
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
