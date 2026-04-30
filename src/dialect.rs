use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{CompletionItem, Diagnostic, Hover, Location, Position};

/// SQL 方言抽象 trait
/// 所有 SQL 方言都需要实现这个 trait
#[async_trait]
pub trait Dialect: Send + Sync {
    /// 方言名称
    fn name(&self) -> &str;

    /// 解析 SQL 并返回诊断信息
    async fn parse(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic>;

    /// 获取代码补全
    async fn completion(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Vec<CompletionItem>;

    /// 获取悬停信息
    async fn hover(&self, sql: &str, position: Position, schema: Option<&Schema>) -> Option<Hover>;

    /// 跳转到定义
    async fn goto_definition(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Option<Location>;

    /// 查找引用
    async fn references(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
    ) -> Vec<Location>;

    /// 格式化 SQL
    async fn format(&self, sql: &str) -> String;

    /// 验证 SQL 语法
    async fn validate(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic>;

    /// 触发后台异步获取数据（如 Schema）
    async fn trigger_background_fetch(&self, _sql: &str) {}
}
