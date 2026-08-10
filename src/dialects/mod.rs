pub mod clickhouse;
pub(crate) mod common;
pub mod elasticsearch_dsl;
pub mod elasticsearch_eql;
pub mod hive;
pub mod mongodb;
pub mod mysql;
pub mod postgres;
pub mod redis;
pub mod sqlite;

pub use clickhouse::ClickHouseDialect;
pub use elasticsearch_dsl::ElasticsearchDslDialect;
pub use elasticsearch_eql::ElasticsearchEqlDialect;
pub use hive::HiveDialect;
pub use mongodb::MongoDbDialect;
pub use mysql::MysqlDialect;
pub use postgres::PostgresDialect;
pub use redis::RedisDialect;
pub use sqlite::SqliteDialect;

use crate::dialect::Dialect;
use std::collections::HashMap;
use std::sync::Arc;

pub const MYSQL_COMPATIBILITY_ALIASES: &[&str] = &[
    "databend",
    "doris",
    "gbase",
    "goldendb",
    "manticoresearch",
    "starrocks",
    "sundb",
    "tidb",
];

pub const POSTGRES_COMPATIBILITY_ALIASES: &[&str] = &[
    "access",
    "cockroachdb",
    "dameng",
    "db2",
    "exasol",
    "firebird",
    "gaussdb",
    "greenplum",
    "h2",
    "highgo",
    "informix",
    "iris",
    "kingbase",
    "kwdb",
    "oceanbase-oracle",
    "opengauss",
    "oracle",
    "questdb",
    "redshift",
    "sqlserver",
    "timescaledb",
    "vastbase",
    "vertica",
    "xugu",
    "yashandb",
    "yugabytedb",
];

pub const SQLITE_COMPATIBILITY_ALIASES: &[&str] = &["cloudflare-d1", "turso"];

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
        let mysql = Arc::new(MysqlDialect::new());
        registry.register(mysql.clone());
        registry.register_alias("mariadb", mysql.clone());
        for alias in MYSQL_COMPATIBILITY_ALIASES {
            registry.register_alias(alias, mysql.clone());
        }

        let postgres = Arc::new(PostgresDialect::new());
        registry.register(postgres.clone());
        registry.register_alias("postgresql", postgres.clone());
        registry.register_alias("pgsql", postgres.clone());
        registry.register_alias("psql", postgres.clone());
        for alias in POSTGRES_COMPATIBILITY_ALIASES {
            registry.register_alias(alias, postgres.clone());
        }

        let sqlite = Arc::new(SqliteDialect::new());
        registry.register(sqlite.clone());
        registry.register_alias("sqlite3", sqlite.clone());
        // DuckDB shares the SQLite-compatible completion baseline for now.
        // Keep its public language ID distinct so external driver packages can
        // describe their real engine without being reduced to generic SQL.
        registry.register_alias("duckdb", sqlite.clone());
        for alias in SQLITE_COMPATIBILITY_ALIASES {
            registry.register_alias(alias, sqlite.clone());
        }

        let hive = Arc::new(HiveDialect::new());
        registry.register(hive.clone());
        registry.register_alias("hql", hive);

        let elasticsearch_eql = Arc::new(ElasticsearchEqlDialect::new());
        registry.register(elasticsearch_eql.clone());
        registry.register_alias("eql", elasticsearch_eql.clone());
        registry.register_alias("es-eql", elasticsearch_eql);

        let elasticsearch_dsl = Arc::new(ElasticsearchDslDialect::new());
        registry.register(elasticsearch_dsl.clone());
        registry.register_alias("elasticsearch", elasticsearch_dsl.clone());
        registry.register_alias("elastic", elasticsearch_dsl.clone());
        registry.register_alias("es", elasticsearch_dsl.clone());
        registry.register_alias("es-dsl", elasticsearch_dsl);

        let clickhouse = Arc::new(ClickHouseDialect::new());
        registry.register(clickhouse.clone());
        registry.register_alias("ch", clickhouse);

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

#[cfg(test)]
mod tests {
    use super::DialectRegistry;

    #[test]
    fn resolves_duckdb_to_the_sqlite_compatible_baseline() {
        let registry = DialectRegistry::new();
        let dialect = registry
            .get_by_name("duckdb")
            .expect("DuckDB alias should be registered");

        assert_eq!(dialect.name(), "sqlite");
    }
}
