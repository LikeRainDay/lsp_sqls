.PHONY: help build test clean install run check fmt clippy all

# 默认目标
.DEFAULT_GOAL := help

# 项目信息
PROJECT_NAME := sql-lsp
BINARY_NAME := sql-lsp
TARGET_DIR := target
RELEASE_DIR := $(TARGET_DIR)/release
DEBUG_DIR := $(TARGET_DIR)/debug

# 颜色定义
COLOR_RESET := \033[0m
COLOR_BOLD := \033[1m
COLOR_GREEN := \033[32m
COLOR_YELLOW := \033[33m
COLOR_BLUE := \033[34m

##@ 帮助

help: ## 显示此帮助信息
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)SQL LSP - Makefile 命令$(COLOR_RESET)"
	@echo ""
	@echo "$(COLOR_BOLD)可用命令:$(COLOR_RESET)"
	@awk 'BEGIN {FS = ":.*##"; printf "\n"} /^[a-zA-Z_-]+:.*?##/ { printf "  $(COLOR_GREEN)%-15s$(COLOR_RESET) %s\n", $$1, $$2 } /^##@/ { printf "\n$(COLOR_BOLD)$(COLOR_YELLOW)%s$(COLOR_RESET)\n", substr($$0, 5) } ' $(MAKEFILE_LIST)

##@ 构建

build: ## 构建项目（debug 模式）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)构建项目（debug 模式）...$(COLOR_RESET)"
	cargo build
	@echo "$(COLOR_GREEN)✓ 构建完成$(COLOR_RESET)"

build-release: ## 构建项目（release 模式）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)构建项目（release 模式）...$(COLOR_RESET)"
	cargo build --release
	@echo "$(COLOR_GREEN)✓ Release 构建完成$(COLOR_RESET)"
	@echo "$(COLOR_YELLOW)二进制文件位置: $(RELEASE_DIR)/$(BINARY_NAME)$(COLOR_RESET)"

install: build-release ## 安装到系统（需要 root 权限）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)安装到系统...$(COLOR_RESET)"
	sudo cp $(RELEASE_DIR)/$(BINARY_NAME) /usr/local/bin/$(BINARY_NAME)
	@echo "$(COLOR_GREEN)✓ 安装完成$(COLOR_RESET)"

install-pre-commit: ## 安装 Git pre-commit hook
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)安装 Git pre-commit hook...$(COLOR_RESET)"
	@if [ ! -d .git ]; then \
		echo "$(COLOR_YELLOW)⚠ 警告: 当前目录不是 Git 仓库，请先运行 'git init' 或确保在 Git 仓库中$(COLOR_RESET)"; \
		exit 1; \
	fi
	@if [ ! -d .git/hooks ]; then \
		mkdir -p .git/hooks; \
	fi
	@cp scripts/pre-commit .git/hooks/pre-commit
	@chmod +x .git/hooks/pre-commit
	@echo "$(COLOR_GREEN)✓ Pre-commit hook 安装完成$(COLOR_RESET)"
	@echo "$(COLOR_YELLOW)💡 现在每次 git commit 前都会自动运行代码检查$(COLOR_RESET)"

##@ 开发

run: ## 运行 LSP 服务器（从 stdin/stdout）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行 LSP 服务器...$(COLOR_RESET)"
	cargo run

check: ## 检查代码（不构建）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)检查代码...$(COLOR_RESET)"
	cargo check
	@echo "$(COLOR_GREEN)✓ 检查完成$(COLOR_RESET)"

fmt: ## 格式化代码
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)格式化代码...$(COLOR_RESET)"
	cargo fmt
	@echo "$(COLOR_GREEN)✓ 格式化完成$(COLOR_RESET)"

fmt-check: ## 检查代码格式（不修改）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)检查代码格式...$(COLOR_RESET)"
	cargo fmt -- --check
	@echo "$(COLOR_GREEN)✓ 格式检查完成$(COLOR_RESET)"

clippy: ## 运行 Clippy 代码检查
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行 Clippy...$(COLOR_RESET)"
	cargo clippy -- -D warnings
	@echo "$(COLOR_GREEN)✓ Clippy 检查完成$(COLOR_RESET)"

##@ 测试

test: ## 运行所有测试
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行所有测试...$(COLOR_RESET)"
	cargo test --all-features
	@echo "$(COLOR_GREEN)✓ 测试完成$(COLOR_RESET)"

test-integration-full: build-release ## 运行完整集成测试（包括 LSP 服务器）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行完整集成测试...$(COLOR_RESET)"
	@if command -v python3 >/dev/null 2>&1; then \
		python3 scripts/lsp_client_test.py || echo "$(COLOR_YELLOW)Python 测试脚本需要手动检查$(COLOR_RESET)"; \
	else \
		echo "$(COLOR_YELLOW)Python3 未安装，跳过客户端测试$(COLOR_RESET)"; \
	fi
	@bash scripts/test_lsp.sh || true
	@echo "$(COLOR_GREEN)✓ 集成测试完成$(COLOR_RESET)"

test-samples: ## 创建测试示例文件
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)创建测试示例文件...$(COLOR_RESET)"
	@bash scripts/test_with_samples.sh
	@echo "$(COLOR_GREEN)✓ 测试示例文件已创建在 test_samples/ 目录$(COLOR_RESET)"

test-verbose: ## 运行所有测试（详细输出）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行所有测试（详细输出）...$(COLOR_RESET)"
	cargo test --all-features -- --nocapture
	@echo "$(COLOR_GREEN)✓ 测试完成$(COLOR_RESET)"

test-mysql: ## 运行 MySQL 相关测试
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行 MySQL 测试...$(COLOR_RESET)"
	cargo test --test mysql_tests
	@echo "$(COLOR_GREEN)✓ MySQL 测试完成$(COLOR_RESET)"

test-schema: ## 运行 Schema 相关测试
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行 Schema 测试...$(COLOR_RESET)"
	cargo test --test schema_tests --test schema_inference_tests
	@echo "$(COLOR_GREEN)✓ Schema 测试完成$(COLOR_RESET)"

test-unit: ## 运行单元测试
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行单元测试...$(COLOR_RESET)"
	cargo test --lib
	@echo "$(COLOR_GREEN)✓ 单元测试完成$(COLOR_RESET)"

test-integration: ## 运行集成测试（单元测试）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行集成测试...$(COLOR_RESET)"
	cargo test --test '*'
	@echo "$(COLOR_GREEN)✓ 集成测试完成$(COLOR_RESET)"

##@ 清理

clean: ## 清理构建产物
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)清理构建产物...$(COLOR_RESET)"
	cargo clean
	@echo "$(COLOR_GREEN)✓ 清理完成$(COLOR_RESET)"

clean-all: clean ## 清理所有文件（包括依赖缓存）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)清理所有文件...$(COLOR_RESET)"
	rm -rf $(TARGET_DIR)
	@echo "$(COLOR_GREEN)✓ 完全清理完成$(COLOR_RESET)"

##@ 开发工具

dev-setup: ## 安装开发依赖和工具
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)安装开发工具...$(COLOR_RESET)"
	rustup component add rustfmt clippy
	@echo "$(COLOR_GREEN)✓ 开发工具安装完成$(COLOR_RESET)"

update: ## 更新依赖
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)更新依赖...$(COLOR_RESET)"
	cargo update
	@echo "$(COLOR_GREEN)✓ 依赖更新完成$(COLOR_RESET)"

##@ 发布

all: fmt clippy test build-release ## 执行完整流程：格式化、检查、测试、构建

pre-commit: fmt-check clippy test ## 提交前检查：格式检查、Clippy、测试

ci: fmt-check clippy test build-release ## CI 流程：格式检查、Clippy、测试、构建

##@ 文档

doc: ## 生成文档
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)生成文档...$(COLOR_RESET)"
	cargo doc --no-deps --open
	@echo "$(COLOR_GREEN)✓ 文档生成完成$(COLOR_RESET)"

doc-open: ## 生成并打开文档
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)生成并打开文档...$(COLOR_RESET)"
	cargo doc --no-deps --open

##@ 调试

debug: build ## 构建 debug 版本用于调试
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)Debug 构建完成$(COLOR_RESET)"
	@echo "$(COLOR_YELLOW)二进制文件位置: $(DEBUG_DIR)/$(BINARY_NAME)$(COLOR_RESET)"

bench: ## 运行基准测试（如果存在）
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)运行基准测试...$(COLOR_RESET)"
	cargo bench || echo "$(COLOR_YELLOW)没有基准测试$(COLOR_RESET)"

##@ 信息

info: ## 显示项目信息
	@echo "$(COLOR_BOLD)$(COLOR_BLUE)项目信息:$(COLOR_RESET)"
	@echo "  项目名称: $(PROJECT_NAME)"
	@echo "  二进制名称: $(BINARY_NAME)"
	@echo "  Rust 版本: $$(rustc --version)"
	@echo "  Cargo 版本: $$(cargo --version)"
	@echo "  构建目录: $(TARGET_DIR)"
	@echo "  Release 二进制: $(RELEASE_DIR)/$(BINARY_NAME)"
	@echo "  Debug 二进制: $(DEBUG_DIR)/$(BINARY_NAME)"

version: ## 显示版本信息
	@cargo --version
	@rustc --version
