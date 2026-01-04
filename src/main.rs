use sql_lsp::SqlLspServer;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    // 初始化日志系统
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr) // LSP 使用 stdout，日志输出到 stderr
        .init();

    tracing::info!("SQL LSP server starting");

    // 检查版本参数
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-v") {
        println!("sql-lsp {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(SqlLspServer::new);
    tracing::info!("SQL LSP server initialized, starting service");
    Server::new(stdin, stdout, socket).serve(service).await;
    tracing::info!("SQL LSP server shutting down");
}
