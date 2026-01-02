# SQL LSP

A multi-dialect SQL Language Server Protocol (LSP) implementation in Rust, supporting MySQL, PostgreSQL, Hive, Elasticsearch, ClickHouse, Redis, and more.

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/your-username/sql-lsp/workflows/CI/badge.svg)](https://github.com/your-username/sql-lsp/actions)

## ✨ Features

- ✅ **Multi-dialect support**: MySQL, PostgreSQL, Hive, Elasticsearch EQL/DSL, ClickHouse, Redis
- ✅ **Fault-tolerant parsing**: Tree-sitter based parser handles incomplete SQL gracefully
- ✅ **Intelligent code completion**: AST-based context-aware completion for keywords, tables, and columns
- ✅ **Go to definition**: Navigate to table and column definitions
- ✅ **Find references**: Find all occurrences of tables and columns
- ✅ **Real-time diagnostics**: Accurate syntax error detection
- ✅ **Schema management**: Auto-inference, priority handling, and isolation
- ✅ **Thread-safe**: Concurrent access support

## 🚀 Quick Start

### Prerequisites

- **Rust**: 1.70+ and Cargo
- **Make**: Optional, for Makefile commands

### Installation

#### Build from Source

```bash
git clone https://github.com/your-username/sql-lsp.git
cd sql-lsp

# Build release binary
make build-release

# Binary location: target/release/sql-lsp
```

#### Install via Cargo

```bash
cargo install --path .
```

### Verify Installation

```bash
target/release/sql-lsp --version
```

## 📖 Usage

SQL LSP is a standard LSP server that communicates via stdin/stdout. It supports standard LSP protocol messages.

### Editor Integration

#### VS Code

Create `.vscode/settings.json`:

```json
{
  "sql-lsp.serverPath": "/path/to/target/release/sql-lsp"
}
```

#### Neovim

Using `nvim-lspconfig`:

```lua
require('lspconfig').sql_lsp.setup({
  cmd = {'/path/to/target/release/sql-lsp'},
  filetypes = {'sql', 'mysql', 'postgresql'}
})
```

See [docs/editor-integration.md](docs/editor-integration.md) for detailed setup instructions.

## ⚙️ Configuration

### Schema Configuration

SQL LSP supports schema configuration via LSP's `workspace/didChangeConfiguration` notification.

**Schema Format**:

```json
{
  "schemas": [{
    "id": "schema1",
    "tables": [{
      "name": "users",
      "columns": [
        {"name": "id", "type": "INT"},
        {"name": "name", "type": "VARCHAR(255)"}
      ]
    }]
  }]
}
```

See [docs/configuration.md](docs/configuration.md) for detailed configuration options.

### Dialect Detection

SQL LSP detects dialects from:

1. **File extension** (highest priority)
   - `.mysql.sql` → MySQL
   - `.postgres.sql` → PostgreSQL
   - `.hive.sql` → Hive
   - `.es.eql` → Elasticsearch EQL
   - `.es.dsl` → Elasticsearch DSL
   - `.clickhouse.sql` → ClickHouse
   - `.redis` → Redis

2. **`languageId` parameter** (fallback)
   - `mysql`, `postgresql`, `hive`, `elasticsearch-eql`, `elasticsearch-dsl`, `clickhouse`, `redis`

3. **Default**: MySQL (if unable to detect)

**Note**: SQL LSP supports both `file://` and `untitled://` URIs. See [docs/uri-support.md](docs/uri-support.md) for details.

## 🧪 Testing

```bash
# Run all tests
make test

# Run specific tests
make test-mysql
make test-schema

# Run LSP client integration tests
python3 scripts/lsp_client_test.py
python3 scripts/lsp_client_test_es.py
```

## 📁 Project Structure

```
sql-lsp/
├── src/
│   ├── main.rs              # Entry point
│   ├── lib.rs                # Library exports
│   ├── server.rs             # LSP server implementation
│   ├── dialect.rs            # Dialect trait
│   ├── schema.rs             # Schema management
│   ├── parser/               # Parsers (Tree-sitter based)
│   └── dialects/             # Dialect implementations
├── tests/                    # Test files
├── scripts/                  # Test scripts
├── docs/                     # Documentation
└── Cargo.toml
```

## 🔌 Supported Dialects

- **MySQL**: `.mysql.sql`
- **PostgreSQL**: `.postgres.sql`
- **Hive**: `.hive.sql`
- **Elasticsearch EQL**: `.es.eql`
- **Elasticsearch DSL**: `.es.dsl`
- **ClickHouse**: `.clickhouse.sql`
- **Redis**: `.redis`

See [docs/dialects.md](docs/dialects.md) for detailed dialect information and examples.

## 🛠️ Development

### Setup

```bash
# Install development tools
make dev-setup

# Format code
make fmt

# Run Clippy
make clippy

# Run tests
make test

# Pre-commit checks
make pre-commit
```

### Git Pre-commit Hook

Install the pre-commit hook to automatically run checks before commits:

```bash
make install-pre-commit
```

The hook runs:
- Code formatting check (`cargo fmt --check`)
- Clippy check (`cargo clippy -- -D warnings`)
- Tests (`cargo test --all-features`)

See [docs/development.md](docs/development.md) for detailed development guide.

## 📚 Documentation

- [Editor Integration](docs/editor-integration.md) - Detailed setup for VS Code, Neovim, Vim, Emacs
- [Configuration](docs/configuration.md) - Schema configuration and options
- [API Reference](docs/api.md) - LSP API documentation
- [Dialects](docs/dialects.md) - Supported SQL dialects and examples
- [Development](docs/development.md) - Development guide and contributing
- [Troubleshooting](docs/troubleshooting.md) - Common issues and solutions

## 📝 License

[Add your license here, e.g., MIT, Apache 2.0]

## 🤝 Contributing

Contributions are welcome! Please see [docs/contributing.md](docs/contributing.md) for guidelines.

## 🙏 Acknowledgments

- [tower-lsp](https://github.com/ebkalderon/tower-lsp) - LSP server framework
- [tree-sitter](https://tree-sitter.github.io/tree-sitter/) - Incremental parser generator
- [tree-sitter-sql](https://github.com/derekstride/tree-sitter-sql) - SQL Tree-sitter grammar
- [sqls-server/sqls](https://github.com/sqls-server/sqls) - Reference implementation

---

**Made with ❤️ using Rust**
