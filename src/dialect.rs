use crate::schema::Schema;
use async_trait::async_trait;
use tower_lsp::lsp_types::{
    CompletionContext as LspCompletionContext, CompletionItem, Diagnostic, Hover, Location,
    Position,
};

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

    /// 获取代码补全，并保留 LSP 触发上下文（手动触发、触发字符、incomplete retry）。
    /// 默认回退到不带上下文的补全，方言可以按需覆盖。
    async fn completion_with_context(
        &self,
        sql: &str,
        position: Position,
        schema: Option<&Schema>,
        _context: Option<&LspCompletionContext>,
    ) -> Vec<CompletionItem> {
        self.completion(sql, position, schema).await
    }

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{CompletionTriggerKind, DiagnosticSeverity, Range};

    struct TestDialect;

    #[async_trait]
    impl Dialect for TestDialect {
        fn name(&self) -> &str {
            "test"
        }

        async fn parse(&self, _sql: &str, _schema: Option<&Schema>) -> Vec<Diagnostic> {
            Vec::new()
        }

        async fn completion(
            &self,
            _sql: &str,
            _position: Position,
            _schema: Option<&Schema>,
        ) -> Vec<CompletionItem> {
            vec![CompletionItem {
                label: "fallback".to_string(),
                ..Default::default()
            }]
        }

        async fn hover(
            &self,
            _sql: &str,
            _position: Position,
            _schema: Option<&Schema>,
        ) -> Option<Hover> {
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
            sql.to_string()
        }

        async fn validate(&self, _sql: &str, _schema: Option<&Schema>) -> Vec<Diagnostic> {
            vec![Diagnostic {
                range: Range::default(),
                severity: Some(DiagnosticSeverity::HINT),
                message: "test".to_string(),
                ..Default::default()
            }]
        }
    }

    #[tokio::test]
    async fn default_completion_with_context_falls_back_to_completion() {
        let context = LspCompletionContext {
            trigger_kind: CompletionTriggerKind::TRIGGER_CHARACTER,
            trigger_character: Some(".".to_string()),
        };

        let items = TestDialect
            .completion_with_context(
                "SELECT u.",
                Position {
                    line: 0,
                    character: 9,
                },
                None,
                Some(&context),
            )
            .await;

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "fallback");
    }
}
