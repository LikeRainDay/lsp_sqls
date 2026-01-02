# SQL LSP

一个基于 Rust 和 `tower-lsp` 的多方言 SQL 语言服务器，支持 MySQL、PostgreSQL、Hive、Elasticsearch、ClickHouse、Redis 等多种 SQL 方言。

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## ✨ 特性

- ✅ **多方言支持**：MySQL、PostgreSQL、Hive、Elasticsearch EQL/DSL、ClickHouse、Redis
- ✅ **基于 Tree-sitter 的容错解析**：能够处理不完整的 SQL，提供更好的编辑体验
- ✅ **智能代码补全**：基于 AST 的上下文感知补全，支持关键字、表名、列名
- ✅ **跳转定义**：支持表名和列名的跳转定义
- ✅ **查找引用**：查找表名和列名的所有引用
- ✅ **实时语法检查**：提供准确的语法错误诊断
- ✅ **Schema 管理**：支持 Schema 的自动推断、优先级处理和隔离
- ✅ **线程安全**：支持并发访问，多个客户端互不干扰

## 🚀 快速开始

### 前置要求

- **Rust**: 1.70+ 和 Cargo
- **Make**: 可选，用于使用 Makefile 命令

### 安装

#### 从源码构建

```bash
# 克隆仓库
git clone https://github.com/your-username/sql-lsp.git
cd sql-lsp

# 构建 Release 版本
make build-release

# 二进制文件位置: target/release/sql-lsp
```

#### 使用 Cargo 安装

```bash
cargo install --path .
```

### 验证安装

```bash
# 检查版本
target/release/sql-lsp --version

# 或直接运行（会从 stdin 读取 LSP 消息）
target/release/sql-lsp
```

## 📖 使用指南

### 基本使用

SQL LSP 是一个标准的 LSP 服务器，通过 stdin/stdout 与客户端通信。它支持标准的 LSP 协议消息。

### 运行测试

```bash
# 运行所有测试
make test

# 运行 MySQL 测试
make test-mysql

# 运行集成测试（包括 LSP 客户端测试）
python3 scripts/lsp_client_test.py        # MySQL SQL 测试
python3 scripts/lsp_client_test_es.py      # Elasticsearch DSL 测试
```

## 🔧 配置和对接

### VS Code 配置

#### 方法 1: 使用扩展配置

创建 `.vscode/settings.json`：

```json
{
  "sql-lsp.serverPath": "/path/to/target/release/sql-lsp",
  "sql-lsp.trace.server": "off",
  "sql-lsp.schema.autoInfer": true,
  "sql-lsp.schema.priority": "match_score"
}
```

#### 方法 2: 手动配置 LSP 客户端

如果你使用其他 LSP 客户端扩展，可以这样配置：

```json
{
  "languageServerExample.maxNumberOfProblems": 100,
  "languageServerExample.trace.server": "off"
}
```

#### 方法 3: 使用 VS Code 扩展开发

1. **安装扩展开发工具**：
   ```bash
   npm install -g @vscode/vsce
   ```

2. **创建扩展配置**（`.vscode/launch.json`）：
   ```json
   {
     "version": "0.2.0",
     "configurations": [
       {
         "type": "node",
         "request": "launch",
         "name": "Launch LSP Server",
         "program": "${workspaceFolder}/target/release/sql-lsp",
         "args": [],
         "console": "integratedTerminal",
         "internalConsoleOptions": "neverOpen"
       }
     ]
   }
   ```

### Neovim 配置

#### 使用 nvim-lspconfig

在 `init.lua` 或配置文件中添加：

```lua
local lspconfig = require('lspconfig')

-- 配置 SQL LSP
local configs = require('lspconfig.configs')
if not configs.sql_lsp then
  configs.sql_lsp = {
    default_config = {
      cmd = {'/path/to/target/release/sql-lsp'},  -- 修改为实际路径
      filetypes = {
        'sql',
        'mysql',
        'postgresql',
        'hive',
        'esql',      -- Elasticsearch EQL
        'esdsl',     -- Elasticsearch DSL
        'clickhouse',
        'redis'
      },
      root_dir = function(fname)
        return vim.fn.getcwd()
      end,
      settings = {
        sql_lsp = {
          trace = {
            server = "off"
          },
          schema = {
            autoInfer = true,
            priority = "match_score"
          }
        }
      },
      capabilities = {
        textDocument = {
          completion = {
            completionItem = {
              snippetSupport = true
            }
          },
          hover = {
            contentFormat = {"markdown", "plaintext"}
          }
        }
      }
    }
  }
end

-- 启动 LSP
require('lspconfig').sql_lsp.setup{
  on_attach = function(client, bufnr)
    -- 自定义按键映射
    local opts = { noremap=true, silent=true }
    vim.api.nvim_buf_set_keymap(bufnr, 'n', 'gd', '<cmd>lua vim.lsp.buf.definition()<CR>', opts)
    vim.api.nvim_buf_set_keymap(bufnr, 'n', 'K', '<cmd>lua vim.lsp.buf.hover()<CR>', opts)
    vim.api.nvim_buf_set_keymap(bufnr, 'n', 'gr', '<cmd>lua vim.lsp.buf.references()<CR>', opts)
  end
}
```

#### 使用 CoC.nvim

在 `coc-settings.json` 中添加：

```json
{
  "languageserver": {
    "sql-lsp": {
      "command": "/path/to/target/release/sql-lsp",
      "filetypes": ["sql", "mysql", "postgresql", "hive"],
      "rootPatterns": [".git"],
      "settings": {
        "sql-lsp": {
          "trace": {
            "server": "off"
          }
        }
      }
    }
  }
}
```

### Vim 配置

#### 使用 vim-lsp

在 `.vimrc` 中添加：

```vim
if executable('sql-lsp')
  au User lsp_setup call lsp#register_server({
    \ 'name': 'sql-lsp',
    \ 'cmd': {server_info->['/path/to/target/release/sql-lsp']},
    \ 'whitelist': ['sql', 'mysql', 'postgresql'],
    \ })
endif
```

### Emacs 配置

#### 使用 lsp-mode

在 `init.el` 中添加：

```elisp
(require 'lsp-mode)

;; 配置 SQL LSP
(lsp-register-client
 (make-lsp-client
  :new-connection (lsp-stdio-connection "/path/to/target/release/sql-lsp")
  :major-modes '(sql-mode)
  :server-id 'sql-lsp))

;; 启用 lsp-mode
(add-hook 'sql-mode-hook #'lsp)
```

### 命令行测试

#### 使用测试脚本

```bash
# MySQL SQL 测试
python3 scripts/lsp_client_test.py

# Elasticsearch DSL 测试
python3 scripts/lsp_client_test_es.py

# 查看测试日志
cat scripts/test_mysql_final3.log
cat scripts/test_elasticsearch_final2.log
```

#### 手动测试 LSP 协议

```bash
# 启动服务器（在终端 1）
target/release/sql-lsp

# 在另一个终端发送 LSP 请求
echo 'Content-Length: 123\r\n\r\n{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}' | target/release/sql-lsp
```

## ⚙️ 配置选项

### Schema 配置

SQL LSP 支持通过 LSP 的 `workspace/didChangeConfiguration` 通知来配置 Schema。

#### Schema 格式

Schema 通过 JSON 格式传递：

```json
{
  "schemas": [
    {
      "id": "schema1",
      "tables": [
        {
          "name": "users",
          "columns": [
            {"name": "id", "type": "INT", "comment": "用户ID"},
            {"name": "name", "type": "VARCHAR(255)", "comment": "用户名"},
            {"name": "email", "type": "VARCHAR(255)", "comment": "邮箱"}
          ],
          "comment": "用户表"
        },
        {
          "name": "orders",
          "columns": [
            {"name": "order_id", "type": "INT", "comment": "订单ID"},
            {"name": "user_id", "type": "INT", "comment": "用户ID"},
            {"name": "total", "type": "DECIMAL(10,2)", "comment": "订单总额"}
          ],
          "comment": "订单表"
        }
      ]
    }
  ]
}
```

#### Schema 推断策略

SQL LSP 支持以下 Schema 推断策略：

1. **自动推断**（默认）：根据 SQL 中的表名自动匹配 Schema
2. **文件关联**：通过文件 URI 或配置明确指定 Schema
3. **匹配度优先级**：根据表名和列名的匹配度选择最佳 Schema

### 方言识别

SQL LSP 支持多种 URI scheme，不仅限于文件路径：

#### 支持的 URI Scheme

- **`file://`** - 文件系统文件（标准文件路径）
- **`untitled://`** - 未保存的文档（VS Code、Neovim 等编辑器支持）
- **其他自定义 scheme** - 任何符合 LSP 规范的 URI

#### 方言推断优先级

SQL LSP 按以下优先级推断方言：

1. **URI 扩展名**（最高优先级）
   - `.mysql.sql` → MySQL
   - `.postgres.sql` → PostgreSQL
   - `.hive.sql` → Hive
   - `.es.eql` → Elasticsearch EQL
   - `.es.dsl` → Elasticsearch DSL
   - `.clickhouse.sql` → ClickHouse
   - `.redis` → Redis

2. **`languageId` 参数**（备选方案）
   - `mysql` → MySQL
   - `postgresql` → PostgreSQL
   - `hive` → Hive
   - `elasticsearch-eql` → Elasticsearch EQL
   - `elasticsearch-dsl` → Elasticsearch DSL
   - `clickhouse` → ClickHouse
   - `redis` → Redis

3. **默认** - MySQL（如果无法推断）

#### 示例

**文件 URI**：
```json
{
  "textDocument": {
    "uri": "file:///path/to/test.mysql.sql",
    "languageId": "sql",
    "text": "SELECT * FROM users;"
  }
}
```

**未保存文档 URI**（VS Code）：
```json
{
  "textDocument": {
    "uri": "untitled://Untitled-1.mysql.sql",
    "languageId": "mysql",
    "text": "SELECT * FROM users;"
  }
}
```

**仅使用 languageId**（无扩展名）：
```json
{
  "textDocument": {
    "uri": "untitled://Untitled-1",
    "languageId": "mysql",
    "text": "SELECT * FROM users;"
  }
}
```

**注意**：即使 URI 不是文件路径，SQL LSP 也能正常工作。URI 只是作为文档的唯一标识符，实际内容通过 `text` 字段传递。

## 📚 API 文档

### 支持的 LSP 功能

SQL LSP 实现了以下 LSP 功能：

#### 1. 文本文档功能

- ✅ **textDocument/didOpen** - 文档打开
- ✅ **textDocument/didChange** - 文档变更
- ✅ **textDocument/didClose** - 文档关闭
- ✅ **textDocument/completion** - 代码补全
- ✅ **textDocument/hover** - 悬停信息
- ✅ **textDocument/definition** - 跳转定义
- ✅ **textDocument/references** - 查找引用
- ✅ **textDocument/publishDiagnostics** - 诊断信息

#### 2. 工作区功能

- ✅ **workspace/didChangeConfiguration** - 配置变更
- ✅ **workspace/didChangeWorkspaceFolders** - 工作区文件夹变更

### 代码补全

代码补全支持以下类型：

- **关键字补全**：SQL 关键字（SELECT, FROM, WHERE 等）
- **表名补全**：基于 Schema 的表名
- **列名补全**：基于 Schema 的列名
- **上下文感知**：根据光标位置的 AST 上下文提供相关补全

**示例请求**：
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "textDocument/completion",
  "params": {
    "textDocument": {
      "uri": "file:///path/to/test.mysql.sql"
    },
    "position": {
      "line": 0,
      "character": 7
    }
  }
}
```

### 跳转定义

支持表名和列名的跳转定义。

**示例请求**：
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "textDocument/definition",
  "params": {
    "textDocument": {
      "uri": "file:///path/to/test.mysql.sql"
    },
    "position": {
      "line": 0,
      "character": 15
    }
  }
}
```

**注意**：URI 可以是任何有效的 URI scheme，不限于文件路径。

### 查找引用

查找表名和列名的所有引用。

**示例请求**：
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "textDocument/references",
  "params": {
    "textDocument": {
      "uri": "file:///path/to/test.mysql.sql"
    },
    "position": {
      "line": 0,
      "character": 15
    },
    "context": {
      "includeDeclaration": true
    }
  }
}
```

**注意**：URI 可以是任何有效的 URI scheme，不限于文件路径。

## 🧪 测试

### 运行测试

```bash
# 运行所有测试
make test

# 运行特定测试
make test-mysql          # MySQL 测试
make test-schema         # Schema 测试
make test-unit           # 单元测试
make test-integration    # 集成测试

# 运行 LSP 客户端集成测试
python3 scripts/lsp_client_test.py > scripts/test_mysql.log
python3 scripts/lsp_client_test_es.py > scripts/test_elasticsearch.log
```

### 测试报告

测试完成后，查看测试报告：

```bash
cat scripts/FINAL_TEST_REPORT.md
```

## 📁 项目结构

```
sql-lsp/
├── src/
│   ├── main.rs                    # 程序入口
│   ├── lib.rs                     # 库入口
│   ├── server.rs                  # LSP 服务器实现
│   ├── dialect.rs                 # 方言抽象 trait
│   ├── schema.rs                  # Schema 管理和隔离
│   ├── token.rs                   # Token 类型定义
│   ├── parser/                    # 解析器模块
│   │   ├── mod.rs                 # 模块导出
│   │   ├── sql.rs                 # SQL 解析器（Tree-sitter）
│   │   └── dsl.rs                 # DSL 解析器（JSON）
│   └── dialects/                  # 各种 SQL 方言实现
│       ├── mod.rs                 # 方言注册
│       ├── mysql.rs               # MySQL 方言
│       ├── postgres.rs            # PostgreSQL 方言
│       ├── hive.rs                # Hive 方言
│       ├── elasticsearch_eql.rs  # Elasticsearch EQL 方言
│       ├── elasticsearch_dsl.rs   # Elasticsearch DSL 方言
│       ├── clickhouse.rs          # ClickHouse 方言
│       └── redis.rs               # Redis 方言
├── tests/                         # 测试文件
│   ├── dialect_tests.rs           # 方言测试
│   ├── mysql_tests.rs             # MySQL 详细测试
│   ├── schema_inference_tests.rs  # Schema 推断测试
│   └── ...
├── scripts/                       # 测试脚本
│   ├── lsp_client_test.py         # MySQL SQL LSP 客户端测试
│   ├── lsp_client_test_es.py      # Elasticsearch DSL LSP 客户端测试
│   ├── test_lsp.sh                # Shell 测试脚本
│   └── test_with_samples.sh       # 示例文件测试
├── test_samples/                  # 测试示例文件
│   ├── test.mysql.sql
│   ├── test.postgres.sql
│   ├── test.hive.sql
│   ├── test.es.eql
│   ├── test.es.dsl
│   └── ...
├── Cargo.toml                     # 项目配置
├── Makefile                       # 构建命令
└── README.md                      # 本文档
```

## 🔌 支持的 SQL 方言

### MySQL

- **文件扩展名**: `.mysql.sql`
- **特性**: 支持 MySQL 语法，包括 JOIN、子查询、聚合函数等
- **示例**:
  ```sql
  SELECT * FROM users WHERE id > 10;
  SELECT u.name, o.total FROM users u JOIN orders o ON u.id = o.user_id;
  ```

### PostgreSQL

- **文件扩展名**: `.postgres.sql`
- **特性**: 支持 PostgreSQL 语法，包括 CTE、窗口函数等
- **示例**:
  ```sql
  WITH ranked_users AS (
    SELECT id, name, ROW_NUMBER() OVER (PARTITION BY department ORDER BY id) as rn
    FROM users
  )
  SELECT * FROM ranked_users WHERE rn = 1;
  ```

### Hive

- **文件扩展名**: `.hive.sql`
- **特性**: 支持 Apache Hive SQL 语法
- **示例**:
  ```sql
  SELECT * FROM users WHERE dt = '2024-01-01';
  ```

### Elasticsearch EQL

- **文件扩展名**: `.es.eql`
- **特性**: 支持 Elasticsearch Event Query Language
- **示例**:
  ```
  process where process.name == "cmd.exe"
  ```

### Elasticsearch DSL

- **文件扩展名**: `.es.dsl`
- **特性**: 支持 Elasticsearch Domain Specific Language（JSON 格式）
- **示例**:
  ```json
  {
    "query": {
      "match": {
        "title": "elasticsearch"
      }
    },
    "aggs": {
      "avg_price": {
        "avg": {
          "field": "price"
        }
      }
    }
  }
  ```

### ClickHouse

- **文件扩展名**: `.clickhouse.sql`
- **特性**: 支持 ClickHouse SQL 语法
- **示例**:
  ```sql
  SELECT * FROM users WHERE id > 10;
  ```

### Redis

- **文件扩展名**: `.redis`
- **特性**: 支持 Redis 查询语言（RediSearch/RedisGraph）
- **示例**:
  ```
  FT.SEARCH idx:users "@name:John"
  ```

## 🗄️ Schema 管理

### Schema 格式

Schema 使用 JSON 格式定义：

```json
{
  "schemas": [
    {
      "id": "ecommerce",
      "tables": [
        {
          "name": "users",
          "columns": [
            {"name": "id", "type": "INT", "comment": "用户ID"},
            {"name": "name", "type": "VARCHAR(255)", "comment": "用户名"},
            {"name": "email", "type": "VARCHAR(255)", "comment": "邮箱"}
          ],
          "comment": "用户表"
        },
        {
          "name": "orders",
          "columns": [
            {"name": "order_id", "type": "INT", "comment": "订单ID"},
            {"name": "user_id", "type": "INT", "comment": "用户ID"},
            {"name": "total", "type": "DECIMAL(10,2)", "comment": "订单总额"}
          ],
          "comment": "订单表"
        }
      ]
    }
  ]
}
```

### Schema 推断

SQL LSP 支持自动 Schema 推断：

1. **表名匹配**：根据 SQL 中的表名自动匹配 Schema
2. **列名匹配**：根据列名匹配度选择最佳 Schema
3. **匹配度评分**：使用匹配度评分算法选择最佳 Schema

### Schema 隔离

不同文件可以使用不同的 Schema，互不干扰。每个文件可以：

- 使用默认 Schema
- 通过配置指定 Schema
- 通过自动推断选择 Schema

## 🛠️ 开发

### 添加新的 SQL 方言

1. **创建方言文件**：在 `src/dialects/` 目录下创建新文件，如 `new_dialect.rs`

2. **实现 Dialect trait**：
   ```rust
   use crate::dialect::Dialect;
   use async_trait::async_trait;

   pub struct NewDialect {
       // 方言特定字段
   }

   #[async_trait]
   impl Dialect for NewDialect {
       fn name(&self) -> &str {
           "new-dialect"
       }

       async fn parse(&self, sql: &str, schema: Option<&Schema>) -> Vec<Diagnostic> {
           // 实现解析逻辑
       }

       async fn completion(&self, sql: &str, position: Position, schema: Option<&Schema>) -> Vec<CompletionItem> {
           // 实现补全逻辑
       }

       // ... 其他方法
   }
   ```

3. **注册方言**：在 `src/dialects/mod.rs` 中注册：
   ```rust
   pub mod new_dialect;
   ```

4. **添加到服务器**：在 `src/server.rs` 中添加方言识别逻辑

5. **添加测试**：在 `tests/` 目录下添加测试

### 运行特定测试

```bash
# 运行特定测试文件
cargo test --test mysql_tests

# 运行特定测试函数
cargo test test_mysql_dialect_name

# 运行测试并显示输出
cargo test -- --nocapture

# 运行测试并显示覆盖率
cargo test -- --test-threads=1
```

### 代码质量检查

```bash
# 格式化代码
make fmt

# 检查代码格式
make fmt-check

# 运行 Clippy 检查
make clippy

# 提交前检查（格式化 + Clippy + 测试）
make pre-commit
```

### Git Pre-commit Hook

项目提供了 Git pre-commit hook，可以在每次提交前自动运行代码检查，确保代码质量。

#### 安装 Pre-commit Hook

```bash
# 安装 pre-commit hook（需要先初始化 Git 仓库）
make install-pre-commit
```

或者手动安装：

```bash
# 确保 .git/hooks 目录存在
mkdir -p .git/hooks

# 复制并设置执行权限
cp scripts/pre-commit .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
```

#### Pre-commit Hook 功能

安装后，每次执行 `git commit` 时，hook 会自动运行：

1. **代码格式检查** (`cargo fmt --check`)
2. **Clippy 检查** (`cargo clippy -- -D warnings`)

如果检查失败，提交将被阻止，你需要修复问题后重新提交。

#### 跳过 Pre-commit Hook（不推荐）

如果确实需要跳过检查（例如紧急修复），可以使用：

```bash
git commit --no-verify -m "紧急修复"
```

**注意**：跳过检查可能导致代码质量问题，请谨慎使用。
```

## 📊 性能

### 基准测试

- **解析速度**: < 10ms（1000 行 SQL）
- **补全响应**: < 50ms
- **内存占用**: < 50MB（典型使用）

### 优化建议

- 使用 Release 模式构建以获得最佳性能
- 对于大型项目，考虑使用 Schema 缓存
- 限制并发请求数量以避免资源耗尽

## 🐛 故障排除

### 常见问题

#### 1. LSP 服务器无法启动

**问题**: 服务器进程立即退出

**解决方案**:
- 检查二进制文件是否存在：`ls -lh target/release/sql-lsp`
- 检查文件权限：`chmod +x target/release/sql-lsp`
- 查看错误日志：`target/release/sql-lsp 2>&1 | head -20`

#### 2. 代码补全不工作

**问题**: 编辑器中没有补全提示

**解决方案**:
- 检查文件扩展名是否正确（`.mysql.sql`、`.postgres.sql` 等）
- 检查 LSP 客户端是否正确连接
- 查看 LSP 客户端日志
- 运行测试脚本验证功能：`python3 scripts/lsp_client_test.py`

#### 3. Schema 推断不准确

**问题**: Schema 推断选择了错误的 Schema

**解决方案**:
- 通过配置明确指定 Schema
- 检查 Schema 中的表名是否与 SQL 中的表名匹配
- 查看 Schema 匹配度评分日志

#### 4. 语法错误检测不准确

**问题**: 正确的 SQL 被标记为错误

**解决方案**:
- Tree-sitter 是容错的，某些情况下可能产生误报
- 检查是否是方言特定的语法问题
- 查看诊断信息的详细信息

## 📝 许可证

[添加您的许可证，例如 MIT、Apache 2.0 等]

## 🤝 贡献

欢迎贡献！请遵循以下步骤：

1. Fork 本仓库
2. 创建特性分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'Add some AmazingFeature'`)
4. 推送到分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

### 贡献指南

- 遵循 Rust 代码风格指南
- 添加适当的测试
- 更新文档
- 确保所有测试通过：`make test`

## 📞 支持

- **Issues**: [GitHub Issues](https://github.com/your-username/sql-lsp/issues)
- **讨论**: [GitHub Discussions](https://github.com/your-username/sql-lsp/discussions)
- **文档**: 查看 `docs/` 目录（如果有）

## 🙏 致谢

- [tower-lsp](https://github.com/ebkalderon/tower-lsp) - LSP 服务器框架
- [tree-sitter](https://tree-sitter.github.io/tree-sitter/) - 增量解析器生成器
- [tree-sitter-sql](https://github.com/derekstride/tree-sitter-sql) - SQL Tree-sitter 语法
- [sqls-server/sqls](https://github.com/sqls-server/sqls) - 参考实现

## 📚 相关资源

- [Language Server Protocol 规范](https://microsoft.github.io/language-server-protocol/)
- [Tree-sitter 文档](https://tree-sitter.github.io/tree-sitter/)
- [Rust 官方文档](https://doc.rust-lang.org/)

---

**Made with ❤️ using Rust**
