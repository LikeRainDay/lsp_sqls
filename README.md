# SQL LSP

<div align="center">

A **high-performance**, **multi-dialect** SQL Language Server Protocol (LSP) implementation in Rust.

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/your-org/lsp_sqls/workflows/CI/badge.svg)](https://github.com/your-org/lsp_sqls/actions)

[Features](#-features) • [Quick Start](#-quick-start) • [Documentation](#-documentation) • [Contributing](#-contributing)

</div>

---

## ✨ Features

### Core Capabilities

- 🎯 **Multi-Dialect Support** - MySQL, PostgreSQL, Hive, ClickHouse, Elasticsearch (EQL/DSL), Redis
- 🔍 **Intelligent Completion** - Context-aware suggestions for keywords, tables, columns, and functions
- 📍 **Code Navigation** - Go-to-definition with source location tracking
- 🔎 **Find References** - Locate all usages of tables and columns
- ⚡ **Real-Time Diagnostics** - Instant syntax error detection as you type
- 🎨 **SQL Formatting** - Professional formatting with customizable options
- 📊 **Rich Hover Information** - Detailed schema information in Markdown format

### Advanced Features

- � **Runtime Configuration** - Update schemas without restarting the server
- 📝 **Incremental Sync** - Efficient text synchronization for large files
- 🌳 **AST-Based Analysis** - Tree-sitter powered parsing for accuracy
- 🧵 **Thread-Safe** - Concurrent request handling
- 📦 **Schema Management** - Auto-inference with priority-based matching
- 📋 **Structured Logging** - Observable with `tracing` framework

## 🚀 Quick Start

### Installation

#### Option 1: Build from Source

```bash
git clone https://github.com/your-org/lsp_sqls.git
cd lsp_sqls
cargo build --release

# Binary location: target/release/sql-lsp
```

#### Option 2: Install via Cargo

```bash
cargo install --path .
```

### Basic Usage

```bash
# Start the LSP server
./target/release/sql-lsp

# With debug logging
RUST_LOG=debug ./target/release/sql-lsp
```

### Editor Integration

Choose your editor for detailed setup instructions:

- **[VS Code](docs/editor-integration.md#vs-code)** - Extension or manual setup
- **[Neovim](docs/editor-integration.md#neovim)** - nvim-lspconfig integration
- **[Vim](docs/editor-integration.md#vim-vim-lsp)** - vim-lsp configuration
- **[Emacs](docs/editor-integration.md#emacs-lsp-mode)** - lsp-mode setup
- **[Sublime Text](docs/editor-integration.md#sublime-text-lsp-package)** - LSP package

## 📖 Documentation

### User Guides

- **[Configuration Guide](docs/configuration.md)** - Schema and dialect configuration
- **[Editor Integration](docs/editor-integration.md)** - Step-by-step editor setup
- **[Troubleshooting](docs/configuration.md#troubleshooting)** - Common issues and solutions

### Developer Resources

- **[Architecture](ARCHITECTURE.md)** - System design and components
- **[Contributing Guide](CONTRIBUTING.md)** - How to contribute
- **[Error Handling](docs/error-handling.md)** - Error handling strategies

## 🎯 Example

### Configure a Database Schema

```json
{
  "schemas": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "database": "my_app",
      "source_uri": "file:///path/to/schema.sql",
      "tables": [
        {
          "name": "users",
          "source_location": ["file:///path/to/schema.sql", 42],
          "columns": [
            {
              "name": "id",
              "data_type": "INT",
              "nullable": false,
              "comment": "Primary key"
            },
            {
              "name": "email",
              "data_type": "VARCHAR(255)",
              "nullable": false,
              "comment": "User email"
            }
          ],
          "comment": "User accounts"
        }
      ]
    }
  ]
}
```

### Features in Action

**Hover Information:**

```markdown
**Table**: `users`

User accounts

**Columns** (10)

- `id`: INT NOT NULL
- `email`: VARCHAR(255) NOT NULL
- `name`: VARCHAR(255) NOT NULL
  ...
```

**Intelligent Completion:**

- Keywords based on context (SELECT, FROM, WHERE, etc.)
- Tables from your schema
- Columns context-aware (knows which table you're querying)
- Functions with parameter hints

## 🛠 Development

### Prerequisites

- Rust 1.70 or later
- Cargo

### Build

```bash
# Development build
cargo build

# Release build with optimizations
cargo build --release

# Run tests
cargo test

# Run linter
cargo clippy
```

### Project Structure

```
lsp_sqls/
├── src/
│   ├── main.rs           # Entry point
│   ├── server.rs         # LSP server
│   ├── dialect.rs        # Dialect trait
│   ├── dialects/         # SQL dialect implementations
│   ├── parser/           # SQL parsers (tree-sitter)
│   ├── schema.rs         # Schema management
│   └── token.rs          # Token definitions
├── docs/                 # Documentation
├── tests/                # Integration tests
└── scripts/              # Helper scripts
```

## 🤝 Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

### Quick Contribution Guide

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/amazing-feature`
3. Make your changes
4. Run tests: `cargo test`
5. Commit: `git commit -m 'feat: add amazing feature'`
6. Push: `git push origin feature/amazing-feature`
7. Open a Pull Request

## � Supported SQL Dialects

| Dialect           | Status           | Notes                 |
| ----------------- | ---------------- | --------------------- |
| MySQL             | ✅ Full support  | MySQL 5.7+ syntax     |
| PostgreSQL        | ✅ Full support  | PostgreSQL 12+ syntax |
| Hive              | ✅ Full support  | HiveQL syntax         |
| ClickHouse        | ✅ Full support  | ClickHouse SQL        |
| Elasticsearch EQL | ✅ Full support  | Event Query Language  |
| Elasticsearch DSL | ✅ Full support  | Query DSL (JSON)      |
| Redis             | ✅ Basic support | Redis commands        |

## 📝 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Built with [tower-lsp](https://github.com/ebkalderon/tower-lsp) - LSP framework
- Powered by [tree-sitter](https://tree-sitter.github.io/) - Parser generator
- Formatted with [sqlformat](https://github.com/shssoichiro/sqlformat-rs) - SQL formatter

## 📬 Contact

- **Issues**: [GitHub Issues](https://github.com/your-org/lsp_sqls/issues)
- **Discussions**: [GitHub Discussions](https://github.com/your-org/lsp_sqls/discussions)

---

<div align="center">

Made with ❤️ by the SQL LSP community

</div>
