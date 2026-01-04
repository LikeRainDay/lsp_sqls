# Architecture Analysis

This document provides a detailed overview of the architecture and logic of the `sql-lsp` (Rust-based SQL LSP) project.

## 1. High-Level Architecture

The project is a **Language Server Protocol (LSP)** implementation written in Rust, designed to support multiple SQL dialects. It uses a component-based architecture where core LSP logic is separated from dialect-specific analysis.

### Core Technologies

- **Server Framework**: `tower-lsp` handles the JSON-RPC communication and LSP lifecycle.
- **Parsing**: `tree-sitter` is used for fault-tolerant, incremental parsing.
- **Concurrency**: `DashMap` and `Arc` are used extensively for thread-safe state management across async tasks.
- **Async Runtime**: `tokio` drives the asynchronous I/O and task execution.

## 2. Component Logic

### 2.1 LSP Server (`src/server.rs`)

The `SqlLspServer` struct is the central coordinator. It implements the `LanguageServer` trait from `tower-lsp`.

- **State Management**:
  - `document_manager`: Stores the current content of open files.
  - `file_dialects`: Caches the detected dialect for each file URI.
  - `file_schemas`: Caches the associated Schema ID for each file.
  - `schema_manager`: Holds the actual schema definitions (tables, columns, functions).
- **Workflow**:
  1. **Initialization**: Configures capabilities (incremental sync, completion, hover, etc.).
  2. **File Open/Change**: Updates `DocumentManager`, infers dialect, and publishes initial diagnostics.
  3. **Requests (Completion/Hover/Definition)**:
     - Retrieves the document content.
     - Resolves the dialect and schema for the file.
     - Delegates the actual logic to the specific `Dialect` implementation.

### 2.2 Dialect System (`src/dialect.rs`, `src/dialects/`)

The project uses a plugin-like architecture for SQL variations via the `Dialect` trait.

- **`Dialect` Trait**: Defines the interface for all SQL flavors. Key methods include:
  - `parse()`: Returns syntax diagnostics.
  - `completion()`: Returns auto-complete items based on cursor position.
  - `hover()`, `goto_definition()`, `references()`: Semantic features.
- **Implementations**:
  - Located in `src/dialects/`.
  - Examples: `MysqlDialect`, `PostgresDialect`, `HiveDialect`.
  - Each implementation uses a `SqlParser` configured for its specific grammar.

### 2.3 Schema Management (`src/schema.rs`)

Handling database metadata (tables, columns) is decoupled from parsing.

- **Data Structures**: `Schema`, `Table`, `Column`, `Function`.
- **Inference Logic**:
  - `get_schema_for_file` attempts to link a SQL file to a known schema.
  - Strategy:
    1. Check for explicit association.
    2. **Auto-Inference**: Parse the SQL to find table names, then score against registered schemas to find the best match (fuzzy matching + hit count).

### 2.4 Parsing & AST Analysis (`src/parser/`)

The project relies on `tree-sitter` for the heavy lifting of parsing, which allows it to handle incomplete or invalid code gracefully (critical for an editor).

- **`SqlParser` Wrapper**:
  - Wraps the raw `tree-sitter` parser.
  - Provides high-level helpers like `get_node_at_position`, `analyze_completion_context`.
- **Context Analysis**:
  - The parser identifies the specialized context of the cursor (e.g., `FromClause`, `SelectClause`, `WhereClause`).
  - This allows the dialect to offer context-aware suggestions (e.g., offering table names in `FROM` but column names in `SELECT`).

## 3. Data Flow Example: Completion Request

1. **User Action**: User types `SELECT * FROM us` in VS Code.
2. **LSP Request**: Client sends `textDocument/completion` to `SqlLspServer`.
3. **Dispatch**:
   - Server retrieves full text of the file from `DocumentManager`.
   - Server looks up the file's dialect (e.g., `MysqlDialect`).
   - Server retrieves the associated `Schema`.
4. **Analysis**:
   - `MysqlDialect::completion` calls `SqlParser::parse`.
   - Parser identifies cursor is in a `FromClause` context.
5. **Result Generation**:
   - `MysqlDialect` generates completion items for known table names in the `Schema` that match "us".
   - Returns a list of `CompletionItem`s (e.g., "users", "user_profiles").

## 4. Directory Structure

- `src/main.rs`: Binary entry point.
- `src/lib.rs`: Library root.
- `src/server.rs`: Core LSP event loop and dispatch logic.
- `src/schema.rs`: Schema definitions and storage.
- `src/dialect.rs`: `Dialect` trait definition.
- `src/dialects/`: Specific implementations (MySQL, PG, etc.).
- `src/parser/`: Tree-sitter integration and AST helpers.
