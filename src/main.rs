use sql_lsp::SqlLspServer;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    // 检查版本参数
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-v") {
        println!("sql-lsp {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(SqlLspServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
