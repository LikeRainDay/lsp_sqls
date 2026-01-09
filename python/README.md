# Python LSP Interactive Client

这个目录包含一个完整的 Python LSP 客户端示例，用于演示如何与 `sql-lsp` 服务器进行交互。

## 快速开始

### 1. 安装依赖

```bash
pip3 install -r requirements.txt
```

### 2. 构建 sql-lsp 二进制（如果还没有）

```bash
cd ..
cargo build --release
cd python
```

### 3. 运行交互式客户端

```bash
python3 lsp_client_interactive.py
```

## 文件说明

- **lsp_client_interactive.py** - 交互式 LSP 客户端主程序

  - 完整的 LSP 生命周期管理
  - 交互式终端界面
  - 详细的日志记录

- **requirements.txt** - Python 依赖列表
  - colorama - 彩色终端输出

## 使用示例

```bash
# 基本使用
python3 lsp_client_interactive.py

# 使用自定义 LSP 路径
python3 lsp_client_interactive.py --lsp-path /path/to/sql-lsp

# 启用调试日志
python3 lsp_client_interactive.py --debug
```

## 交互命令

启动后，在终端中输入：

- `/quit` - 退出
- `/clear` - 清空 SQL 缓冲区
- `/schema` - 注入示例数据库模式
- `/help` - 显示帮助

然后直接输入 SQL 即可看到实时补全建议！

## 详细文档

查看 [PYTHON_CLIENT_USAGE.md](../PYTHON_CLIENT_USAGE.md) 获取完整的使用指南。
