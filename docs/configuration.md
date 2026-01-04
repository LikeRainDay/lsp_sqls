# Configuration Guide

This document explains how to configure SQL LSP for your project.

## Schema Configuration

SQL LSP uses workspace configuration to understand your database schema, enabling intelligent code completion, validation, and navigation.

### Configuration Methods

#### 1. Using LSP `workspace/didChangeConfiguration`

Send a `workspace/didChangeConfiguration` notification with the following structure:

```json
{
  "schemas": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "database": "my_database",
      "tables": [
        {
          "name": "users",
          "comment": "User accounts table",
          "columns": [
            {
              "name": "id",
              "data_type": "INT",
              "nullable": false,
              "comment": "Primary key"
            },
            {
              "name": "name",
              "data_type": "VARCHAR(255)",
              "nullable": false,
              "comment": "Full name"
            },
            {
              "name": "email",
              "data_type": "VARCHAR(255)",
              "nullable": false,
              "comment": "Email address"
            },
            {
              "name": "created_at",
              "data_type": "TIMESTAMP",
              "nullable": false,
              "comment": null
            }
          ]
        },
        {
          "name": "posts",
          "comment": "Blog posts table",
          "columns": [
            {
              "name": "id",
              "data_type": "INT",
              "nullable": false,
              "comment": "Primary key"
            },
            {
              "name": "user_id",
              "data_type": "INT",
              "nullable": false,
              "comment": "Foreign key to users table"
            },
            {
              "name": "title",
              "data_type": "VARCHAR(500)",
              "nullable": false,
              "comment": "Post title"
            },
            {
              "name": "content",
              "data_type": "TEXT",
              "nullable": true,
              "comment": "Post content"
            }
          ]
        }
      ],
      "functions": [
        {
          "name": "get_user_count",
          "parameters": [],
          "return_type": "INT",
          "description": "Returns the total number of users"
        },
        {
          "name": "get_user_by_id",
          "parameters": [
            {
              "name": "user_id",
              "data_type": "INT",
              "optional": false
            }
          ],
          "return_type": "TABLE",
          "description": "Returns user details by ID"
        }
      ]
    }
  ],
  "fileSchemas": {
    "file:///path/to/queries.sql": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

#### 2. Schema Auto-Inference

If no explicit schema mapping is configured, SQL LSP will automatically infer the best matching schema based on the tables referenced in your SQL files.

**How it works**:

1. Parser extracts table names from your SQL
2. Each registered schema is scored based on:
   - Exact name matches (+10 points per table)
   - Fuzzy name matches (+5 points per table)
   - Multiple table bonus (encourages multi-table matches)
3. The highest-scoring schema is selected

This allows you to work without configuration in simple scenarios.

## Dialect Configuration

SQL LSP automatically detects SQL dialects using:

### 1. File Extensions (Highest Priority)

- `*.mysql.sql` → MySQL
- `*.postgres.sql` or `*.pgsql` → PostgreSQL
- `*.hive.sql` or `*.hql` → Hive
- `*.es.eql` or `*.eql` → Elasticsearch EQL
- `*.es.dsl` or `*.es.json` → Elasticsearch DSL
- `*.clickhouse.sql` or `*.ch.sql` → ClickHouse
- `*.redis` → Redis

### 2. Language ID (Fallback)

If the file extension is generic (e.g., `.sql`), the LSP uses the `languageId` parameter from `textDocument/didOpen`:

- `mysql`, `mysql-sql` → MySQL
- `postgresql`, `postgres`, `pgsql` → PostgreSQL
- `hive`, `hql` → Hive
- `elasticsearch-eql`, `eql` → Elasticsearch EQL
- `elasticsearch-dsl`, `es-dsl`, `json` (with "elasticsearch" in URI) → Elasticsearch DSL
- `clickhouse`, `ch` → ClickHouse
- `redis` → Redis

### 3. Default

If detection fails, MySQL is used as the default dialect.

## Editor Integration Examples

### VS Code

Create `.vscode/settings.json`:

```json
{
  "sql-lsp.serverPath": "/path/to/sql-lsp",
  "sql-lsp.schemas": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "database": "my_app",
      "tables": [
        // ... your schema
      ]
    }
  ]
}
```

### Neovim (nvim-lspconfig)

```lua
require('lspconfig').sql_lsp.setup({
  cmd = {'/path/to/sql-lsp'},
  filetypes = {'sql', 'mysql', 'pgsql'},
  settings = {
    schemas = {
      {
        id = "550e8400-e29b-41d4-a716-446655440000",
        database = "my_app",
        tables = {
          -- ... your schema
        }
      }
    }
  }
})
```

## Configuration Tips

1. **Schema IDs**: Use UUIDs for schema IDs. Generate with `uuidgen` on Unix or online generators.

2. **File Mappings**: Explicit file-to-schema mappings override auto-inference for better control.

3. **Dynamic Updates**: Schemas can be updated at runtime via `workspace/didChangeConfiguration` without restarting the LSP server.

4. **Performance**: For large databases, include only frequently-used tables in the schema configuration to improve performance.

## Troubleshooting

- **No completions**: Check that a schema is registered and matches tables in your SQL file.
- **Wrong schema detected**: Use explicit `fileSchemas` mapping.
- **Performance issues**: Reduce the number of tables/columns in schema configuration.

For more information, see:

- [Editor Integration](./editor-integration.md)
- [Troubleshooting](./troubleshooting.md)
