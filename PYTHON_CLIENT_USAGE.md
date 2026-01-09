# Python LSP Interactive Client 使用指南

这是一个完整的 Python LSP 客户端示例，展示如何与 `sql-lsp` 服务器进行交互。

## 特性

✅ **完整的 LSP 生命周期管理**

- Initialize → didOpen → completion → shutdown 完整流程
- 自动维护文档状态和版本号
- 优雅的服务器启动和关闭

✅ **交互式终端界面**

- 实时输入 SQL 并获取自动补全建议
- 支持多行 SQL 语句
- 带颜色的输出（使用 colorama）

✅ **详细的日志记录**

- 所有 JSON-RPC 请求/响应日志
- 服务器 stderr 输出捕获
- Debug 模式显示完整协议详情

## 安装

### 1. 构建 sql-lsp 二进制文件

```bash
cargo build --release
```

### 2. 安装 Python 依赖

```bash
pip3 install -r requirements.txt
```

## 使用方法

### 基本使用

```bash
# 启动交互式客户端
cd python
python3 lsp_client_interactive.py

# 使用自定义 LSP 二进制路径
python3 lsp_client_interactive.py --lsp-path ../target/release/sql-lsp

# 启用 Debug 日志（显示完整 JSON-RPC 通信）
python3 lsp_client_interactive.py --debug
```

### 交互命令

在交互式终端中，您可以使用以下命令：

| 命令      | 说明                                  |
| --------- | ------------------------------------- |
| `/quit`   | 退出客户端                            |
| `/clear`  | 清空当前 SQL 缓冲区                   |
| `/schema` | 注入示例数据库模式 (users, orders 表) |
| `/help`   | 显示帮助信息                          |

### 示例会话

```
SQL LSP Interactive Client
============================================================
Type SQL to see completions, or /help for commands

13:45:30 [INFO] Starting sql-lsp server: ./target/release/sql-lsp
13:45:30 [INFO] ✓ LSP server started (PID: 12345)
13:45:30 [INFO] Initializing LSP server...
13:45:30 [INFO] → SEND [1] initialize
13:45:30 [INFO] ← RECV [1] response
13:45:30 [INFO] ✓ Server initialized
13:45:30 [INFO]   Capabilities: ['textDocumentSync', 'completionProvider', 'hoverProvider', ...]
13:45:30 [INFO] Injecting sample schema...
13:45:30 [INFO] ✓ Schema injected (tables: users, orders)

Ready! Start typing SQL...

SQL> SELECT * FROM users WHERE
13:45:35 [INFO] → SEND [3] textDocument/completion
13:45:35 [INFO] ← RECV [3] response
  Completions (8):
   1. [Field   ] id - Column: id (INT)
   2. [Field   ] email - Column: email (VARCHAR(255))
   3. [Field   ] name - Column: name (VARCHAR(255))
   4. [Field   ] created_at - Column: created_at (TIMESTAMP)
   5. [Operator] LIKE - Operator: LIKE
   6. [Operator] IN - Operator: IN
   7. [Operator] BETWEEN - Operator: BETWEEN
   8. [Operator] IS NULL - Operator: IS NULL

SQL> SELECT u.
  Completions (4):
   1. [Field   ] id
   2. [Field   ] email
   3. [Field   ] name
   4. [Field   ] created_at

SQL> /quit
Goodbye!
13:45:50 [INFO] Shutting down LSP server...
13:45:50 [INFO] ✓ Server shutdown complete
```

## 工作原理

### 架构

```
┌─────────────────────────┐
│ lsp_client_interactive  │
│   (Python Client)       │
└───────────┬─────────────┘
            │ JSON-RPC over stdin/stdout
            │
┌───────────▼─────────────┐
│      sql-lsp            │
│   (Rust LSP Server)     │
└─────────────────────────┘
```

### LSP 通信流程

1. **初始化 (Initialize)**

   ```python
   client.start()           # 启动子进程
   client.initialize()      # 发送 initialize 请求
   # → 服务器返回 capabilities
   ```

2. **打开文档 (didOpen)**

   ```python
   client.did_open(uri, text)  # 通知服务器文档已打开
   ```

3. **文档更新 (didChange)**

   ```python
   client.did_change(uri, new_text, version)  # 更新文档内容
   ```

4. **请求补全 (completion)**

   ```python
   items = client.completion(uri, position)   # 获取补全建议
   ```

5. **关闭 (shutdown + exit)**
   ```python
   client.shutdown()        # 优雅关闭
   ```

### 日志级别

**INFO 模式（默认）**：

- 显示请求/响应摘要
- 显示补全结果
- 不显示完整 JSON

**DEBUG 模式（`--debug`）**：

- 显示完整 JSON-RPC 消息
- 显示服务器 stderr 输出
- 显示所有通知消息

## 代码结构

### LspClient 类

核心 LSP 客户端实现：

```python
class LspClient:
    def start()               # 启动 LSP 服务器子进程
    def initialize()          # LSP 初始化握手
    def did_open(uri, text)   # 打开文档通知
    def did_change(uri, text) # 文档变更通知
    def completion(uri, pos)  # 请求补全
    def shutdown()            # 关闭服务器
```

**线程管理**：

- `response_thread`: 读取服务器响应
- `stderr_thread`: 捕获服务器日志
- 使用 `threading.Lock` 保证线程安全

### InteractiveClient 类

交互式终端界面：

```python
class InteractiveClient:
    def run()                    # 主交互循环
    def inject_sample_schema()   # 注入示例模式
    def display_completions()    # 显示补全建议
    def get_cursor_position()    # 计算光标位置
```

## 示例 Schema

客户端会自动注入以下示例 schema：

**users 表**：

- `id`: INT (主键)
- `email`: VARCHAR(255)
- `name`: VARCHAR(255)
- `created_at`: TIMESTAMP

**orders 表**：

- `id`: INT (主键)
- `user_id`: INT (外键)
- `total`: DECIMAL(10,2)
- `status`: VARCHAR(50)

## 测试场景

参考 `examples/sql_demo.sql` 中的示例：

1. **基本 SELECT**：`SELECT * FROM users WHERE `
2. **表特定列**：`SELECT u.` （显示 users 表的列）
3. **JOIN 查询**：多表连接补全
4. **聚合函数**：`GROUP BY`, `HAVING` 子句
5. **子查询**：嵌套查询补全

## 故障排除

### 问题：找不到 sql-lsp 二进制文件

```bash
# 检查文件是否存在
ls -la ../target/release/sql-lsp

# 如果不存在，重新构建（在项目根目录）
cd .. && cargo build --release

# 使用绝对路径
python3 lsp_client_interactive.py --lsp-path /absolute/path/to/sql-lsp
```

### 问题：没有补全建议

1. 确保已注入 schema：输入 `/schema`
2. 检查 SQL 语法是否正确
3. 使用 `--debug` 模式查看详细日志
4. 确认光标位置在正确的位置

### 问题：colorama 未安装

```bash
# 如果没有安装 colorama，会回退到无颜色模式
# 安装 colorama 获得更好的体验
pip3 install colorama
```

## 扩展

### 添加自定义 Schema

编辑 `InteractiveClient.inject_sample_schema()` 方法：

```python
schema = {
    "sql": {
        "schemas": [{
            "id": "your-uuid",
            "database": "your_db",
            "tables": [
                {
                    "name": "your_table",
                    "columns": [
                        {
                            "name": "column_name",
                            "data_type": "VARCHAR(100)",
                            "nullable": False
                        }
                    ]
                }
            ]
        }]
    }
}
```

### 添加其他 LSP 功能

```python
# Hover 信息
def hover(self, uri: str, position: Position):
    params = {
        "textDocument": {"uri": uri},
        "position": position.to_dict()
    }
    request_id = self._send_request("textDocument/hover", params)
    return self._wait_for_response(request_id)

# 格式化
def format_document(self, uri: str):
    params = {
        "textDocument": {"uri": uri},
        "options": {"tabSize": 2, "insertSpaces": True}
    }
    request_id = self._send_request("textDocument/formatting", params)
    return self._wait_for_response(request_id)
```

## 参考资料

- [LSP 规范](https://microsoft.github.io/language-server-protocol/)
- [sql-lsp README](../README.md)
- [JSON-RPC 2.0](https://www.jsonrpc.org/specification)

## License

MIT License - 与 sql-lsp 项目相同
