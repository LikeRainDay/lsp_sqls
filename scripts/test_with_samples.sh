#!/bin/bash

# 使用示例 SQL 文件测试 LSP 服务器

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${GREEN}=== 使用示例 SQL 文件测试 LSP 服务器 ===${NC}\n"

# 创建测试目录和示例文件
TEST_DIR="test_samples"
mkdir -p "$TEST_DIR"

# 创建示例 SQL 文件
cat > "$TEST_DIR/test.mysql.sql" << 'EOF'
SELECT * FROM users WHERE id > 10;
SELECT id, name, email FROM users;
INSERT INTO users (id, name) VALUES (1, 'test');
EOF

cat > "$TEST_DIR/test.postgres.sql" << 'EOF'
SELECT * FROM users WHERE id > 10;
SELECT id, name FROM users WHERE name ILIKE '%test%';
EOF

cat > "$TEST_DIR/test.hive.sql" << 'EOF'
SELECT * FROM users PARTITION (dt='2024-01-01');
SELECT id, name FROM users WHERE dt = '2024-01-01';
EOF

cat > "$TEST_DIR/test.es.eql" << 'EOF'
sequence
  [process where process.name == "cmd.exe"]
  [file where file.name == "notepad.exe"]
EOF

cat > "$TEST_DIR/test.es.dsl" << 'EOF'
{
  "query": {
    "match": {
      "title": "test"
    }
  }
}
EOF

echo -e "${BLUE}已创建测试 SQL 文件：${NC}"
ls -lh "$TEST_DIR"/*.sql "$TEST_DIR"/*.eql "$TEST_DIR"/*.dsl 2>/dev/null || true

echo -e "\n${GREEN}测试文件已准备就绪${NC}"
echo -e "${YELLOW}提示：可以使用 LSP 客户端（如 VS Code、Neovim）打开这些文件进行测试${NC}\n"
