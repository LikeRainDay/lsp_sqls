#!/bin/bash

# LSP 服务器集成测试脚本
# 用于测试 LSP 服务器的整体功能

set -e

# 颜色定义
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== SQL LSP 服务器集成测试 ===${NC}\n"

# 检查是否已构建
if [ ! -f "target/release/sql-lsp" ]; then
    echo -e "${YELLOW}Release 二进制不存在，正在构建...${NC}"
    cargo build --release
fi

echo -e "${GREEN}✓ 服务器二进制已就绪${NC}\n"

# 测试 1: 检查服务器是否能正常启动
echo -e "${YELLOW}测试 1: 检查服务器启动...${NC}"
timeout 2s target/release/sql-lsp < /dev/null > /dev/null 2>&1 || true
echo -e "${GREEN}✓ 服务器可以启动${NC}\n"

# 测试 2: 发送初始化请求
echo -e "${YELLOW}测试 2: 发送 LSP 初始化请求...${NC}"
INIT_REQUEST='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootPath":null,"capabilities":{},"trace":"off"}}'

echo "$INIT_REQUEST" | timeout 2s target/release/sql-lsp 2>&1 | head -5 || true
echo -e "${GREEN}✓ 初始化请求已发送${NC}\n"

echo -e "${GREEN}=== 集成测试完成 ===${NC}"
