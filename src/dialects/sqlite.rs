//! SQLite shares the ANSI-style relation and column completion surface with
//! PostgreSQL, but must retain its own dialect id so file mappings and schema
//! metadata are never silently routed to a different configured database.

use crate::dialect::Dialect;
use crate::dialects::MysqlDialect;
use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionContext as LspCompletionContext, CompletionItem, Diagnostic, Hover, Location,
    Position,
};

/// Compatibility adapter for SQLite's common SELECT/DML completion grammar.
///
/// The underlying parser intentionally follows the unqualified relation
/// completion behaviour SQLite users expect (`FROM users`, not `FROM main.`).
/// SQLite-specific DDL validation remains the responsibility of the native
/// driver.
pub struct SqliteDialect {
    relational: MysqlDialect,
}

impl Default for SqliteDialect {
    fn default() -> Self {
        Self::new()
    }
}

impl SqliteDialect {
    pub fn new() -> Self {
        Self {
            relational: MysqlDialect::new(),
        }
    }
}

#[async_trait]
impl Dialect for SqliteDialect {
    fn name(&self) -> &str {
        "sqlite"
    }

    async fn parse(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.relational.parse(sql, schema).await
    }

    async fn completion(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Vec<CompletionItem> {
        self.relational.completion(sql, position, schema).await
    }

    async fn completion_with_context(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
        context: Option<&LspCompletionContext>,
    ) -> Vec<CompletionItem> {
        self.relational
            .completion_with_context(sql, position, schema, context)
            .await
    }

    async fn hover(&self, sql: &str, position: Position, schema: Option<&Schema>) -> Option<Hover> {
        self.relational.hover(sql, position, schema).await
    }

    async fn goto_definition(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Option<Location> {
        self.relational.goto_definition(sql, position, schema).await
    }

    async fn references(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Vec<Location> {
        self.relational.references(sql, position, schema).await
    }

    async fn format(&self, sql: &str) -> String {
        self.relational.format(sql).await
    }

    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
        self.relational.validate(sql, schema).await
    }
}
