# SQL LSP Server

<div align="center">

A **high-performance**, **multi-dialect** SQL Language Server Protocol (LSP) implementation in Rust.

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[Features](#-features) • [Usage](#-usage) • [API Reference](#-api-reference) • [Development](#-development)

</div>

---

## ✨ Features

- 🎯 **Multi-Dialect Support** - MySQL, PostgreSQL, Hive, ClickHouse, Elasticsearch (EQL/DSL), Redis
- 🔍 **Intelligent Completion** - Context-aware suggestions with AST-based analysis
- 📍 **Code Navigation** - Go-to-definition and find references
- ⚡ **Real-Time Diagnostics** - Tree-sitter powered syntax error detection
- 🎨 **SQL Formatting** - Professional code formatting with sqlformat
- 📊 **Rich Hover Information** - Detailed schema information in Markdown
- 🧵 **Thread-Safe** - Concurrent request handling with async/await
- 📦 **Schema Management** - Dynamic schema updates and auto-inference

## 🚀 Usage

### Installation

```bash
# Build from source
git clone https://github.com/your-org/lsp_sqls.git
cd lsp_sqls
cargo build --release

# Or install via cargo
cargo install --path .
```

### Starting the Server

The LSP server communicates via stdin/stdout using JSON-RPC 2.0 protocol:

```bash
# Start server
./target/release/sql-lsp

# With debug logging
RUST_LOG=debug ./target/release/sql-lsp
```

### LSP Communication Protocol

All requests and responses follow the [LSP specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/).

#### 1. Initialize

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "processId": 12345,
    "rootUri": "file:///path/to/workspace",
    "capabilities": {
      "textDocument": {
        "completion": { "dynamicRegistration": true },
        "hover": { "dynamicRegistration": true }
      }
    }
  }
}
```

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "capabilities": {
      "textDocumentSync": 2,
      "completionProvider": { "triggerCharacters": [".", " "] },
      "hoverProvider": true,
      "definitionProvider": true,
      "referencesProvider": true,
      "documentFormattingProvider": true
    },
    "serverInfo": {
      "name": "sql-lsp",
      "version": "0.1.0"
    }
  }
}
```

#### 2. Document Sync

**Open Document:**

```json
{
  "jsonrpc": "2.0",
  "method": "textDocument/didOpen",
  "params": {
    "textDocument": {
      "uri": "file:///path/to/query.sql",
      "languageId": "sql",
      "version": 1,
      "text": "SELECT * FROM users WHERE "
    }
  }
}
```

> **Note on URIs**: The `uri` field can be either:
>
> - **File URI**: `file:///path/to/query.sql` (saved file)
> - **Virtual URI**: `untitled:Untitled-1` (in-memory, unsaved document)
> - **Custom scheme**: `inmemory://model/1` or any custom identifier
>
> The server identifies documents by their URI, so as long as the URI is unique and consistent across requests, it will work correctly.

**Update Document:**

```json
{
  "jsonrpc": "2.0",
  "method": "textDocument/didChange",
  "params": {
    "textDocument": {
      "uri": "file:///path/to/query.sql",
      "version": 2
    },
    "contentChanges": [
      {
        "text": "SELECT * FROM users WHERE id = "
      }
    ]
  }
}
```

#### 3. Completion

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "textDocument/completion",
  "params": {
    "textDocument": { "uri": "file:///path/to/query.sql" },
    "position": { "line": 0, "character": 30 }
  }
}
```

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "isIncomplete": false,
    "items": [
      {
        "label": "id",
        "kind": 5,
        "detail": "Column: id (INT)",
        "documentation": "User ID",
        "sortText": "0id",
        "insertText": "id"
      },
      {
        "label": "email",
        "kind": 5,
        "detail": "Column: email (VARCHAR)",
        "sortText": "0email",
        "insertText": "email"
      },
      {
        "label": "LIKE",
        "kind": 24,
        "detail": "Operator: LIKE",
        "sortText": "1LIKE",
        "insertText": "LIKE"
      }
    ]
  }
}
```

**Completion Item Kinds:**

- `5` = Field (column)
- `7` = Class (table)
- `3` = Function
- `14` = Keyword
- `24` = Operator

#### 4. Hover

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "textDocument/hover",
  "params": {
    "textDocument": { "uri": "file:///path/to/query.sql" },
    "position": { "line": 0, "character": 14 }
  }
}
```

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "contents": {
      "kind": "markdown",
      "value": "**Table**: `users`\n\nUser accounts\n\n**Columns** (3)\n- `id`: INT NOT NULL\n- `email`: VARCHAR(255) NOT NULL\n- `name`: VARCHAR(255) NULL"
    },
    "range": {
      "start": { "line": 0, "character": 14 },
      "end": { "line": 0, "character": 19 }
    }
  }
}
```

#### 5. Go to Definition

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "textDocument/definition",
  "params": {
    "textDocument": { "uri": "file:///path/to/query.sql" },
    "position": { "line": 0, "character": 14 }
  }
}
```

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "uri": "file:///path/to/schema.sql",
    "range": {
      "start": { "line": 42, "character": 0 },
      "end": { "line": 42, "character": 100 }
    }
  }
}
```

#### 6. Diagnostics

**Notification (Server → Client):**

```json
{
  "jsonrpc": "2.0",
  "method": "textDocument/publishDiagnostics",
  "params": {
    "uri": "file:///path/to/query.sql",
    "diagnostics": [
      {
        "range": {
          "start": { "line": 0, "character": 14 },
          "end": { "line": 0, "character": 18 }
        },
        "severity": 1,
        "code": "SYNTAX_ERROR",
        "source": "tree-sitter-sql",
        "message": "Syntax error: unexpected token"
      }
    ]
  }
}
```

**Severity Levels:**

- `1` = Error
- `2` = Warning
- `3` = Information
- `4` = Hint

#### 7. Document Formatting

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "textDocument/formatting",
  "params": {
    "textDocument": { "uri": "file:///path/to/query.sql" },
    "options": {
      "tabSize": 2,
      "insertSpaces": true
    }
  }
}
```

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "result": [
    {
      "range": {
        "start": { "line": 0, "character": 0 },
        "end": { "line": 0, "character": 50 }
      },
      "newText": "SELECT\n  *\nFROM\n  users\nWHERE\n  id = 1"
    }
  ]
}
```

### Schema Configuration

Configure schemas via `workspace/didChangeConfiguration`:

**Request:**

```json
{
  "jsonrpc": "2.0",
  "method": "workspace/didChangeConfiguration",
  "params": {
    "settings": {
      "sql": {
        "schemas": [
          {
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "database": "my_app",
            "source_uri": "file:///path/to/schema.sql",
            "tables": [
              {
                "name": "users",
                "source_location": ["file:///path/to/schema.sql", 42],
                "comment": "User accounts",
                "columns": [
                  {
                    "name": "id",
                    "data_type": "INT",
                    "nullable": false,
                    "comment": "Primary key",
                    "source_location": null
                  },
                  {
                    "name": "email",
                    "data_type": "VARCHAR(255)",
                    "nullable": false,
                    "comment": "User email address",
                    "source_location": null
                  }
                ]
              }
            ],
            "functions": []
          }
        ]
      }
    }
  }
}
```

**Schema Structure:**

```typescript
interface Schema {
  id: string; // UUID
  catalog?: string; // Optional catalog/database above the SQL schema
  database: string; // Database name
  source_uri?: string; // Optional schema file URI
  tables: Table[];
  functions: Function[];
}

interface Table {
  name: string;
  comment?: string;
  source_location?: [string, number]; // [URI, line number]
  columns: Column[];
}

interface Column {
  name: string;
  data_type: string; // e.g., "INT", "VARCHAR(255)"
  nullable: boolean;
  comment?: string;
  source_location?: [string, number];
}

interface Function {
  name: string;
  return_type: string;
  parameters: Parameter[];
  description?: string;
}

interface Parameter {
  name: string;
  data_type: string;
  optional: boolean;
}
```

## 📖 API Reference

### Supported LSP Methods

| Method                             | Description                    | Status |
| ---------------------------------- | ------------------------------ | ------ |
| `initialize`                       | Initialize server capabilities | ✅     |
| `textDocument/didOpen`             | Open document notification     | ✅     |
| `textDocument/didChange`           | Document change notification   | ✅     |
| `textDocument/didClose`            | Close document notification    | ✅     |
| `textDocument/completion`          | Code completion                | ✅     |
| `textDocument/hover`               | Hover information              | ✅     |
| `textDocument/definition`          | Go to definition               | ✅     |
| `textDocument/references`          | Find references                | ✅     |
| `textDocument/formatting`          | Document formatting            | ✅     |
| `workspace/didChangeConfiguration` | Configuration updates          | ✅     |

### Completion Context Detection

The server uses AST-based context analysis to provide accurate completions:

| Context         | Suggestions                     | Example                                 |
| --------------- | ------------------------------- | --------------------------------------- |
| `FromClause`    | Tables only                     | `SELECT * FROM ‸`                       |
| `SelectClause`  | Columns + keywords              | `SELECT ‸ FROM users`                   |
| `WhereClause`   | Columns + operators             | `SELECT * FROM users WHERE ‸`           |
| `OrderByClause` | Columns + ASC/DESC              | `SELECT * FROM users ORDER BY ‸`        |
| `GroupByClause` | Columns only                    | `SELECT COUNT(*) FROM users GROUP BY ‸` |
| `HavingClause`  | Columns + functions + operators | `... HAVING ‸`                          |
| `JoinClause`    | Tables only                     | `SELECT * FROM users JOIN ‸`            |
| `TableColumn`   | Specific table columns          | `SELECT u.‸ FROM users u`               |

**Operator Filtering:**

- Only keyword operators are suggested: `LIKE`, `IN`, `BETWEEN`, `IS NULL`, `IS NOT NULL`
- Symbol operators (`=`, `>`, `<`, etc.) are excluded to reduce noise

## 🗄️ Supported SQL Dialects

| Dialect               | Status   | Features                                    |
| --------------------- | -------- | ------------------------------------------- |
| **MySQL**             | ✅ Full  | MySQL 5.7+ syntax, context-aware completion |
| **PostgreSQL**        | ✅ Full  | PostgreSQL 12+ syntax, ILIKE support        |
| **Hive**              | ✅ Full  | HiveQL syntax, PARTITION keyword            |
| **ClickHouse**        | ✅ Full  | ClickHouse SQL, MergeTree support           |
| **Elasticsearch EQL** | ✅ Full  | Event Query Language                        |
| **Elasticsearch DSL** | ✅ Full  | Query DSL (JSON)                            |
| **Redis**             | ✅ Basic | Redis commands (FT.SEARCH, etc.)            |

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
cargo test --all-features

# Run linter
cargo clippy -- -D warnings

# Format code
cargo fmt
```

### Project Structure

```
lsp_sqls/
├── src/
│   ├── main.rs           # Entry point
│   ├── server.rs         # LSP server implementation
│   ├── dialect.rs        # Dialect trait definition
│   ├── dialects/         # SQL dialect implementations
│   │   ├── mysql.rs      # MySQL dialect
│   │   ├── postgres.rs   # PostgreSQL dialect
│   │   └── ...
│   ├── parser/           # SQL parsers
│   │   └── sql.rs        # Tree-sitter SQL parser
│   ├── schema.rs         # Schema management
│   └── token.rs          # Token definitions
├── tests/                # Integration tests
├── docs/                 # Documentation
└── scripts/              # Helper scripts
    └── pre-commit        # Git pre-commit hook
```

### Running Tests

```bash
# Run all tests
cargo test --all-features -- --nocapture

# Run specific test
cargo test test_comprehensive_completion_scenarios -- --nocapture

# Run with coverage
cargo tarpaulin --all-features
```

## 🤝 Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/amazing-feature`
3. Make your changes and add tests
4. Run pre-commit checks: `make install-pre-commit`
5. Commit: `git commit -m 'feat: add amazing feature'`
6. Push: `git push origin feature/amazing-feature`
7. Open a Pull Request

## 📝 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Built with [tower-lsp](https://github.com/ebkalderon/tower-lsp) - LSP framework for Rust
- Powered by [tree-sitter](https://tree-sitter.github.io/) - Parser generator
- Formatted with [sqlformat](https://github.com/shssoichiro/sqlformat-rs) - SQL formatter

---

<div align="center">

Made with ❤️ using Rust

</div>
