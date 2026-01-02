#!/usr/bin/env python3
"""
Elasticsearch DSL LSP 客户端测试脚本
用于测试 Elasticsearch DSL LSP 服务器的功能
"""

import json
import subprocess
import sys
import time
import os
import re
import threading
from datetime import datetime
from enum import Enum

# 尝试导入 select 和 fcntl（Unix/Linux/macOS）
try:
    import select
    HAS_SELECT = True
except ImportError:
    HAS_SELECT = False

try:
    import fcntl
    HAS_FCNTL = True
except ImportError:
    HAS_FCNTL = False

class LogLevel(Enum):
    DEBUG = 0
    INFO = 1
    WARNING = 2
    ERROR = 3

class TestResult(Enum):
    """测试结果状态"""
    SUCCESS = "✓"
    FAILURE = "✗"
    WARNING = "⚠"
    SKIP = "⊘"
    INFO = "ℹ"

class Logger:
    """增强的日志记录器，支持更好的格式化和问题分析"""
    def __init__(self, level=LogLevel.INFO):
        self.level = level
        self.colors = {
            'DEBUG': '\033[36m',    # Cyan
            'INFO': '\033[32m',     # Green
            'WARNING': '\033[33m',  # Yellow
            'ERROR': '\033[31m',    # Red
            'RESET': '\033[0m',     # Reset
            'BOLD': '\033[1m',
            'DIM': '\033[2m',
            'BLUE': '\033[34m',
            'MAGENTA': '\033[35m',
        }
        self.stats = {
            'requests': 0,
            'responses': 0,
            'notifications': 0,
            'errors': 0,
            'warnings': 0,
            'tests_passed': 0,
            'tests_failed': 0,
            'tests_warning': 0,
        }
        self.current_test = None
        self.test_results = []

    def log(self, level, message, details=None):
        """记录日志"""
        if level.value < self.level.value:
            return

        timestamp = datetime.now().strftime("%H:%M:%S.%f")[:-3]
        level_name = level.name
        color = self.colors.get(level_name, '')
        reset = self.colors['RESET']

        # 立即刷新输出，避免缓冲
        print(f"{color}[{timestamp}] [{level_name}]{reset} {message}", flush=True)

        if details:
            if isinstance(details, dict):
                print(f"  {json.dumps(details, indent=2, ensure_ascii=False)}", flush=True)
            else:
                print(f"  {details}", flush=True)

    def debug(self, message, details=None):
        self.log(LogLevel.DEBUG, message, details)

    def info(self, message, details=None):
        self.log(LogLevel.INFO, message, details)
        if '请求' in message or 'request' in message.lower():
            self.stats['requests'] += 1
        elif '响应' in message or 'response' in message.lower():
            self.stats['responses'] += 1

    def warning(self, message, details=None):
        self.log(LogLevel.WARNING, message, details)
        self.stats['warnings'] += 1

    def error(self, message, details=None):
        self.log(LogLevel.ERROR, message, details)
        self.stats['errors'] += 1

    def print_section(self, title, level=1):
        """打印分节标题"""
        colors = {
            1: self.colors['BOLD'] + self.colors['BLUE'],
            2: self.colors['BOLD'] + self.colors['MAGENTA'],
            3: self.colors['BOLD'],
        }
        reset = self.colors['RESET']
        color = colors.get(level, self.colors['BOLD'])

        if level == 1:
            print(f"\n{color}{'='*80}{reset}")
            print(f"{color}{title.center(80)}{reset}")
            print(f"{color}{'='*80}{reset}\n")
        elif level == 2:
            print(f"\n{color}{'─'*80}{reset}")
            print(f"{color}  {title}{reset}")
            print(f"{color}{'─'*80}{reset}\n")
        else:
            print(f"\n{color}  ▶ {title}{reset}\n")

    def print_test_header(self, test_name, description=None):
        """打印测试用例头部"""
        self.current_test = test_name
        print(f"\n{self.colors['BOLD']}{self.colors['BLUE']}{'═'*80}{self.colors['RESET']}")
        print(f"{self.colors['BOLD']}{self.colors['BLUE']}  🧪 测试用例: {test_name}{self.colors['RESET']}")
        if description:
            print(f"{self.colors['DIM']}     {description}{self.colors['RESET']}")
        print(f"{self.colors['BOLD']}{self.colors['BLUE']}{'═'*80}{self.colors['RESET']}\n")

    def print_test_result(self, result: TestResult, message, details=None, analysis=None):
        """打印测试结果"""
        result_symbols = {
            TestResult.SUCCESS: (self.colors['BOLD'] + self.colors['INFO'], "✓"),
            TestResult.FAILURE: (self.colors['BOLD'] + self.colors['ERROR'], "✗"),
            TestResult.WARNING: (self.colors['BOLD'] + self.colors['WARNING'], "⚠"),
            TestResult.SKIP: (self.colors['DIM'], "⊘"),
            TestResult.INFO: (self.colors['BOLD'] + self.colors['INFO'], "ℹ"),
        }

        color, symbol = result_symbols[result]
        reset = self.colors['RESET']

        print(f"{color}{symbol}{reset} {message}")

        if details:
            if isinstance(details, dict):
                print(f"  {self.colors['DIM']}{json.dumps(details, indent=2, ensure_ascii=False)}{self.colors['RESET']}")
            else:
                print(f"  {self.colors['DIM']}{details}{self.colors['RESET']}")

        if analysis:
            print(f"\n  {self.colors['BOLD']}📊 分析:{self.colors['RESET']}")
            for line in analysis.split('\n'):
                print(f"     {line}")

        # 记录测试结果
        self.test_results.append({
            'test': self.current_test,
            'result': result,
            'message': message,
            'details': details,
            'analysis': analysis
        })

        if result == TestResult.SUCCESS:
            self.stats['tests_passed'] += 1
        elif result == TestResult.FAILURE:
            self.stats['tests_failed'] += 1
        elif result == TestResult.WARNING:
            self.stats['tests_warning'] += 1

    def print_code_block(self, title, code, language="json"):
        """打印代码块"""
        print(f"\n{self.colors['BOLD']}📝 {title}:{self.colors['RESET']}")
        print(f"{self.colors['DIM']}{'─'*80}{self.colors['RESET']}")
        lines = code.split('\n')
        for i, line in enumerate(lines, 1):
            print(f"{self.colors['DIM']}{i:3d} │{self.colors['RESET']} {line}")
        print(f"{self.colors['DIM']}{'─'*80}{self.colors['RESET']}")
        print(f"{self.colors['DIM']}总长度: {len(code)} 字符, {len(code.split())} 个单词{self.colors['RESET']}\n")

    def print_diagnostics_analysis(self, diagnostics, expected_errors=False):
        """分析并打印诊断信息

        Args:
            diagnostics: 诊断信息列表
            expected_errors: 如果为 True，表示这个测试用例期望有错误（如语法错误测试），
                           此时检测到错误应该标记为成功
        """
        if not diagnostics:
            if expected_errors:
                self.print_test_result(TestResult.WARNING, "语法检查通过，但测试期望检测到错误")
            else:
                self.print_test_result(TestResult.SUCCESS, "语法检查通过，未发现错误")
            return

        error_count = sum(1 for d in diagnostics if d.get('severity') == 1)
        warning_count = sum(1 for d in diagnostics if d.get('severity') == 2)
        info_count = sum(1 for d in diagnostics if d.get('severity') == 3)
        hint_count = sum(1 for d in diagnostics if d.get('severity') == 4)

        print(f"\n{self.colors['BOLD']}📊 诊断分析:{self.colors['RESET']}")
        print(f"  错误: {self.colors['ERROR']}{error_count}{self.colors['RESET']} 个")
        print(f"  警告: {self.colors['WARNING']}{warning_count}{self.colors['RESET']} 个")
        print(f"  信息: {info_count} 个")
        print(f"  提示: {hint_count} 个")

        if error_count > 0:
            if expected_errors:
                # 如果期望有错误，检测到错误是成功的
                self.print_test_result(TestResult.SUCCESS, f"成功检测到 {error_count} 个语法错误（符合预期）")
            else:
                self.print_test_result(TestResult.FAILURE, f"发现 {error_count} 个语法错误")
        elif warning_count > 0:
            self.print_test_result(TestResult.WARNING, f"发现 {warning_count} 个警告")
        else:
            self.print_test_result(TestResult.SUCCESS, "语法检查通过")

        print(f"\n{self.colors['BOLD']}详细诊断信息:{self.colors['RESET']}")
        for i, diag in enumerate(diagnostics, 1):
            severity_map = {1: ("错误", self.colors['ERROR']), 2: ("警告", self.colors['WARNING']),
                          3: ("信息", self.colors['INFO']), 4: ("提示", self.colors['DIM'])}
            severity_name, severity_color = severity_map.get(diag.get("severity", 0), ("未知", ""))
            code = diag.get("code", "")
            message = diag.get("message", "")
            range_info = diag.get("range", {})
            start = range_info.get("start", {})
            end = range_info.get("end", {})

            print(f"\n  {i}. {severity_color}[{severity_name}]{self.colors['RESET']} {code}: {message}")
            print(f"     位置: 行 {start.get('line', 0)}:{start.get('character', 0)} - "
                  f"行 {end.get('line', 0)}:{end.get('character', 0)}")

    def print_completion_analysis(self, items, context=None):
        """分析并打印补全信息"""
        if not items:
            self.print_test_result(TestResult.WARNING, "未收到任何补全项")
            return

        print(f"\n{self.colors['BOLD']}📋 补全分析:{self.colors['RESET']}")
        print(f"  总计: {len(items)} 个补全项")

        # 按类型分组
        by_kind = {}
        kind_names = {
            1: "文本", 2: "方法", 3: "函数", 4: "构造函数",
            5: "字段", 6: "变量", 7: "类", 8: "接口",
            9: "模块", 10: "属性", 11: "枚举", 12: "关键字",
            13: "片段", 14: "颜色", 15: "文件", 16: "引用",
            17: "文件夹", 18: "枚举成员", 19: "常量", 20: "结构体",
            21: "事件", 22: "操作符", 23: "类型参数", 25: "单元"
        }

        for item in items:
            kind = item.get("kind", 0)
            kind_name = kind_names.get(kind, f"未知({kind})")
            if kind_name not in by_kind:
                by_kind[kind_name] = []
            by_kind[kind_name].append(item)

        # 分析上下文相关性（针对 DSL）
        context_relevant = 0
        if context:
            # DSL 特定的关键字和字段
            dsl_keywords = ["match", "term", "bool", "range", "exists", "query", "aggs", "sort"]
            context_lower = context.lower()
            for item in items:
                label = item.get("label", "").lower().strip('"').strip("'")
                detail = item.get("detail", "").lower()
                # 检查是否是 DSL 关键字或字段
                if any(kw in label for kw in dsl_keywords) or any(kw in detail for kw in dsl_keywords):
                    context_relevant += 1
                # 检查是否在上下文中提到
                elif any(kw in context_lower for kw in dsl_keywords if kw in label):
                    context_relevant += 1

        # 打印统计
        print(f"  类型分布:")
        for kind_name, kind_items in sorted(by_kind.items()):
            print(f"    • {kind_name}: {len(kind_items)} 项")

        if context:
            relevance = (context_relevant / len(items) * 100) if items else 0
            print(f"\n  上下文相关性: {context_relevant}/{len(items)} ({relevance:.1f}%)")
            # 对于 DSL，只要有相关的关键字或字段就认为相关性足够
            if context_relevant > 0:
                self.print_test_result(TestResult.SUCCESS, f"补全项包含 DSL 相关项 ({context_relevant} 项相关)")
            elif len(items) > 0:
                # 即使相关性计算为 0，如果提供了补全项，也认为是成功的（可能是默认补全）
                self.print_test_result(TestResult.WARNING, f"补全项与上下文相关性较低，但提供了 {len(items)} 个补全项")
            else:
                self.print_test_result(TestResult.FAILURE, f"未提供任何补全项")

        # 显示前10个补全项
        print(f"\n{self.colors['BOLD']}补全项示例 (前10个):{self.colors['RESET']}")
        for i, item in enumerate(items[:10], 1):
            label = item.get("label", "")
            detail = item.get("detail", "")
            kind_name = kind_names.get(item.get("kind", 0), "未知")
            print(f"  {i:2d}. [{kind_name}] {label}" + (f" - {detail}" if detail else ""))

    def print_smart_analysis(self, test_name, diagnostics=None, completion=None, hover=None):
        """智能分析测试结果"""
        print(f"\n{self.colors['BOLD']}{self.colors['MAGENTA']}🔍 智能分析: {test_name}{self.colors['RESET']}")
        print(f"{self.colors['DIM']}{'─'*80}{self.colors['RESET']}")

        analysis_parts = []

        # 诊断分析
        if diagnostics is not None:
            if not diagnostics:
                analysis_parts.append("✓ 语法检查: 通过，未发现错误")
            else:
                error_count = sum(1 for d in diagnostics if d.get('severity') == 1)
                warning_count = sum(1 for d in diagnostics if d.get('severity') == 2)
                if error_count > 0:
                    analysis_parts.append(f"✗ 语法检查: 发现 {error_count} 个错误，需要修复")
                elif warning_count > 0:
                    analysis_parts.append(f"⚠ 语法检查: 发现 {warning_count} 个警告，建议检查")
                else:
                    analysis_parts.append("✓ 语法检查: 通过（仅有信息或提示）")

        # 补全分析
        if completion:
            items = completion.get("result", [])
            if isinstance(items, dict) and "items" in items:
                items = items["items"]

            if items:
                # 检查是否有上下文相关的补全
                query_types = ["match", "term", "bool", "range", "exists"]
                agg_types = ["terms", "avg", "sum", "min", "max"]
                top_level = ["query", "aggs", "sort", "from", "size"]

                has_query_types = any(any(qt in item.get("label", "").lower() for qt in query_types) for item in items)
                has_agg_types = any(any(at in item.get("label", "").lower() for at in agg_types) for item in items)
                has_top_level = any(any(tl in item.get("label", "").lower() for tl in top_level) for item in items)

                if has_query_types or has_agg_types or has_top_level:
                    analysis_parts.append(f"✓ 代码补全: 提供了 {len(items)} 个相关补全项")
                else:
                    analysis_parts.append(f"⚠ 代码补全: 提供了 {len(items)} 个补全项，但可能不够相关")
            else:
                analysis_parts.append("✗ 代码补全: 未提供任何补全项")

        # 悬停分析
        if hover:
            result = hover.get("result")
            if result:
                analysis_parts.append("✓ 悬停信息: 已提供")
            else:
                analysis_parts.append("⚠ 悬停信息: 未提供（可能不在有效标识符上）")

        # 打印分析结果
        for part in analysis_parts:
            print(f"  {part}")

        print(f"{self.colors['DIM']}{'─'*80}{self.colors['RESET']}\n")

    def print_stats(self):
        """打印统计信息"""
        print(f"\n{self.colors['BOLD']}{self.colors['BLUE']}{'═'*80}{self.colors['RESET']}")
        print(f"{self.colors['BOLD']}{self.colors['BLUE']}  📊 测试统计{self.colors['RESET']}")
        print(f"{self.colors['BOLD']}{self.colors['BLUE']}{'═'*80}{self.colors['RESET']}\n")

        print(f"  {self.colors['BOLD']}LSP 通信:{self.colors['RESET']}")
        print(f"    请求数: {self.stats['requests']}")
        print(f"    响应数: {self.stats['responses']}")
        print(f"    通知数: {self.stats['notifications']}")

        print(f"\n  {self.colors['BOLD']}测试结果:{self.colors['RESET']}")
        print(f"    {self.colors['INFO']}✓ 通过: {self.stats['tests_passed']}{self.colors['RESET']}")
        print(f"    {self.colors['ERROR']}✗ 失败: {self.stats['tests_failed']}{self.colors['RESET']}")
        print(f"    {self.colors['WARNING']}⚠ 警告: {self.stats['tests_warning']}{self.colors['RESET']}")

        print(f"\n  {self.colors['BOLD']}问题统计:{self.colors['RESET']}")
        print(f"    错误数: {self.colors['ERROR']}{self.stats['errors']}{self.colors['RESET']}")
        print(f"    警告数: {self.colors['WARNING']}{self.stats['warnings']}{self.colors['RESET']}")

        # 计算成功率
        total_tests = self.stats['tests_passed'] + self.stats['tests_failed'] + self.stats['tests_warning']
        if total_tests > 0:
            success_rate = (self.stats['tests_passed'] / total_tests) * 100
            print(f"\n  {self.colors['BOLD']}成功率: {success_rate:.1f}%{self.colors['RESET']}")

        print(f"\n{self.colors['BOLD']}{self.colors['BLUE']}{'═'*80}{self.colors['RESET']}\n")

class LSPClient:
    def __init__(self, server_path, logger=None):
        self.server_path = server_path
        self.process = None
        self.request_id = 1
        self.logger = logger or Logger()
        self.stderr_buffer = []
        self.stderr_thread = None

    def _read_stderr(self):
        """读取服务器 stderr 输出"""
        if not self.process or not self.process.stderr:
            return
        try:
            for line_bytes in iter(self.process.stderr.readline, b''):
                if not line_bytes:
                    break
                try:
                    line = line_bytes.decode('utf-8', errors='ignore').strip()
                    if line:
                        self.stderr_buffer.append(line)
                        self.logger.debug(f"服务器输出: {line}")
                except Exception as e:
                    self.logger.debug(f"解码 stderr 错误: {e}")
        except Exception as e:
            self.logger.debug(f"读取 stderr 错误: {e}")

    def start(self):
        """启动 LSP 服务器"""
        self.logger.info(f"启动 LSP 服务器: {self.server_path}")
        try:
            self.process = subprocess.Popen(
                [self.server_path],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=False,
                bufsize=0
            )
            self.logger.debug(f"服务器进程已启动，PID: {self.process.pid}")

            self.stderr_thread = threading.Thread(target=self._read_stderr, daemon=True)
            self.stderr_thread.start()

            time.sleep(0.5)
            self.logger.info("服务器启动完成")
        except Exception as e:
            self.logger.error(f"启动服务器失败: {e}")
            raise

    def stop(self):
        """停止 LSP 服务器"""
        if self.process:
            self.logger.info("正在停止 LSP 服务器...")
            try:
                self.process.stdin.close()
            except Exception:
                pass

            self.process.terminate()
            try:
                exit_code = self.process.wait(timeout=2)
                self.logger.debug(f"服务器已退出，退出码: {exit_code}")
            except subprocess.TimeoutExpired:
                self.logger.warning("服务器未在 2 秒内退出，强制终止")
                self.process.kill()
                self.process.wait()

            if self.stderr_buffer:
                self.logger.debug("服务器输出日志:")
                for line in self.stderr_buffer:
                    print(f"  {line}")

    def send_request(self, method, params=None, is_notification=False):
        """发送 LSP 请求"""
        request = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {}
        }

        if not is_notification:
            request["id"] = self.request_id
            request_id = self.request_id
            self.request_id += 1
        else:
            request_id = None
            self.logger.stats['notifications'] += 1

        content = json.dumps(request, ensure_ascii=False)
        content_bytes = content.encode('utf-8')
        header = f"Content-Length: {len(content_bytes)}\r\n\r\n".encode('utf-8')
        message = header + content_bytes

        if self.process and self.process.stdin:
            try:
                self.process.stdin.write(message)
                self.process.stdin.flush()
                return True
            except Exception as e:
                self.logger.error(f"发送请求时出错: {e}")
                return False
        return False

    def read_response(self, timeout=3):
        """读取响应"""
        if not self.process or not self.process.stdout:
            return None

        try:
            start_time = time.time()

            if HAS_SELECT:
                ready, _, _ = select.select([self.process.stdout], [], [], min(timeout, 2.0))
                if not ready:
                    return None

            # 读取头部
            headers = {}
            for _ in range(10):
                if time.time() - start_time > timeout:
                    return None

                if HAS_SELECT:
                    ready, _, _ = select.select([self.process.stdout], [], [], 0.1)
                    if not ready:
                        continue

                line_bytes = self.process.stdout.readline()
                if not line_bytes:
                    break

                line = line_bytes.decode('utf-8', errors='ignore').rstrip('\n\r')
                if not line:
                    break

                if ':' in line:
                    key, value = line.split(':', 1)
                    headers[key.strip().lower()] = value.strip()

            # 读取内容
            content_length = int(headers.get('content-length', 0))
            if content_length > 0:
                content_bytes = b''
                remaining = content_length
                read_start = time.time()

                while remaining > 0:
                    if time.time() - read_start > timeout:
                        break

                    if HAS_SELECT:
                        ready, _, _ = select.select([self.process.stdout], [], [], 0.1)
                        if not ready:
                            continue

                    chunk = self.process.stdout.read(min(remaining, 4096))
                    if not chunk:
                        break

                    content_bytes += chunk
                    remaining -= len(chunk)

                content = content_bytes.decode('utf-8').strip()
                while content.startswith('\n') or content.startswith('\r'):
                    content = content[1:]

                try:
                    response = json.loads(content)
                    if "id" in response:
                        self.logger.stats['responses'] += 1
                    else:
                        self.logger.stats['notifications'] += 1
                    return response
                except json.JSONDecodeError as e:
                    self.logger.error(f"JSON 解析错误: {e}")
                    return {"raw": content, "error": str(e)}
        except Exception as e:
            self.logger.error(f"读取响应时出错: {e}")
        return None

def test_initialize(client, logger):
    """测试初始化"""
    logger.print_section("测试 1: 初始化 LSP 服务器", level=2)

    params = {
        "processId": None,
        "rootPath": None,
        "capabilities": {
            "textDocument": {
                "completion": {"dynamicRegistration": True},
                "hover": {"dynamicRegistration": True}
            }
        },
        "trace": "off"
    }

    if not client.send_request("initialize", params):
        logger.print_test_result(TestResult.FAILURE, "发送初始化请求失败")
        return False

    time.sleep(0.2)
    response = client.read_response(timeout=3)

    if response:
        if "result" in response:
            result = response["result"]
            logger.print_test_result(TestResult.SUCCESS, "初始化成功", {
                "name": result.get("serverInfo", {}).get("name"),
                "version": result.get("serverInfo", {}).get("version"),
            })
            return True
        elif "error" in response:
            logger.print_test_result(TestResult.FAILURE, "初始化失败", response["error"])
            return False
    else:
        logger.print_test_result(TestResult.FAILURE, "未收到初始化响应")
        return False

def test_did_open(client, logger, uri, text, language_id="json"):
    """测试打开文档"""
    logger.print_section(f"打开文档: {uri}", level=2)

    logger.print_code_block("用户输入的 DSL", text, language="json")

    params = {
        "textDocument": {
            "uri": uri,
            "languageId": language_id,
            "version": 1,
            "text": text
        }
    }

    if client.send_request("textDocument/didOpen", params, is_notification=True):
        logger.print_test_result(TestResult.SUCCESS, "文档打开通知已发送")
        time.sleep(0.3)
        return True
    else:
        logger.print_test_result(TestResult.FAILURE, "发送文档打开通知失败")
        return False

def read_diagnostics(client, logger, timeout=1.0, expected_errors=False):
    """读取诊断通知

    Args:
        client: LSP 客户端
        logger: 日志记录器
        timeout: 超时时间
        expected_errors: 是否期望有错误（用于语法错误测试用例）
    """
    diagnostics_received = []
    start_time = time.time()

    while time.time() - start_time < timeout:
        response = client.read_response(timeout=0.5)
        if response:
            if response.get("method") == "textDocument/publishDiagnostics":
                params = response.get("params", {})
                uri = params.get("uri", "")
                diags = params.get("diagnostics", [])
                diagnostics_received.append({
                    "uri": uri,
                    "diagnostics": diags
                })
        else:
            time.sleep(0.1)

    if diagnostics_received:
        for diag_info in diagnostics_received:
            logger.print_diagnostics_analysis(diag_info["diagnostics"], expected_errors)

    return diagnostics_received

def test_completion(client, logger, uri, position, context_text=None):
    """测试代码补全"""
    logger.print_section("代码补全测试", level=2)

    if context_text:
        lines = context_text.split('\n')
        line_num = position.get("line", 0)
        char_pos = position.get("character", 0)
        logger.print_code_block(f"补全位置 (行 {line_num}, 列 {char_pos})",
                               '\n'.join(lines), language="json")

    params = {
        "textDocument": {"uri": uri},
        "position": position
    }

    if not client.send_request("textDocument/completion", params):
        logger.print_test_result(TestResult.FAILURE, "发送补全请求失败")
        return None

    time.sleep(0.2)
    response = client.read_response(timeout=3)

    if response:
        if "result" in response:
            result = response["result"]
            items = []
            if isinstance(result, list):
                items = result
            elif isinstance(result, dict) and "items" in result:
                items = result["items"]

            logger.print_completion_analysis(items, context_text)
            return response
        elif "error" in response:
            logger.print_test_result(TestResult.FAILURE, "补全请求失败", response["error"])
        else:
            logger.print_test_result(TestResult.WARNING, "收到意外的响应格式", response)
    else:
        logger.print_test_result(TestResult.WARNING, "未收到补全响应")

    return response

def test_hover(client, logger, uri, position, context_text=None):
    """测试悬停"""
    logger.print_section("悬停信息测试", level=2)

    if context_text:
        lines = context_text.split('\n')
        line_num = position.get("line", 0)
        char_pos = position.get("character", 0)
        logger.print_code_block(f"悬停位置 (行 {line_num}, 列 {char_pos})",
                               '\n'.join(lines), language="json")

    params = {
        "textDocument": {"uri": uri},
        "position": position
    }

    if not client.send_request("textDocument/hover", params):
        logger.print_test_result(TestResult.FAILURE, "发送悬停请求失败")
        return None

    time.sleep(0.2)
    response = client.read_response(timeout=3)

    if response:
        if "result" in response:
            result = response["result"]
            if result:
                logger.print_test_result(TestResult.SUCCESS, "收到悬停信息")
                contents = result.get("contents", {})
                if isinstance(contents, dict):
                    value = contents.get("value", contents.get("language", ""))
                    logger.info(f"  内容: {value}")
                elif isinstance(contents, str):
                    logger.info(f"  内容: {contents}")
            else:
                logger.print_test_result(TestResult.INFO, "无悬停信息（位置可能不在有效标识符上）")
        elif "error" in response:
            logger.print_test_result(TestResult.FAILURE, "悬停请求失败", response["error"])
    else:
        logger.print_test_result(TestResult.WARNING, "未收到悬停响应")

    return response

def main():
    debug_mode = "--debug" in sys.argv or "-d" in sys.argv
    log_level = LogLevel.DEBUG if debug_mode else LogLevel.INFO

    logger = Logger(level=log_level)

    logger.print_section("Elasticsearch DSL LSP 服务器集成测试", level=1)

    # 查找服务器二进制
    server_path = "target/release/sql-lsp"
    if not os.path.exists(server_path):
        server_path = "target/debug/sql-lsp"
        if not os.path.exists(server_path):
            logger.error("找不到 LSP 服务器二进制文件")
            logger.info("请先运行: make build-release 或 make build")
            sys.exit(1)
        else:
            logger.info(f"使用 Debug 二进制: {server_path}")
    else:
        logger.info(f"使用 Release 二进制: {server_path}")

    client = LSPClient(server_path, logger)

    try:
        # 启动服务器
        client.start()

        # 测试初始化
        if not test_initialize(client, logger):
            logger.error("初始化失败，终止测试")
            return 1

        # 发送 initialized 通知
        client.send_request("initialized", {}, is_notification=True)
        time.sleep(0.2)

        # 测试用例 1: 基本查询
        logger.print_test_header("基本查询", "测试简单的 match 查询")

        dsl1 = """{
  "query": {
    "match": {
      "title": "elasticsearch"
    }
  }
}"""

        test_did_open(client, logger, "file:///test.es.dsl", dsl1, language_id="json")
        diagnostics1 = read_diagnostics(client, logger)

        # 在 query 对象内测试补全
        test_completion(
            client,
            logger,
            "file:///test.es.dsl",
            {"line": 2, "character": 10},  # query 对象内
            context_text=dsl1
        )

        # 智能分析
        logger.print_smart_analysis("基本查询测试", diagnostics1[0]["diagnostics"] if diagnostics1 else [])

        # 测试用例 2: 聚合查询
        logger.print_test_header("聚合查询", "测试 aggs 对象内的补全")

        dsl2 = """{
  "query": {
    "match_all": {}
  },
  "aggs": {
    "avg_price": {
      "avg": {
        "field": "price"
      }
    }
  }
}"""

        test_did_open(client, logger, "file:///test.es.dsl", dsl2, language_id="json")
        diagnostics2 = read_diagnostics(client, logger)

        # 在 aggs 对象内测试补全
        test_completion(
            client,
            logger,
            "file:///test.es.dsl",
            {"line": 4, "character": 10},  # aggs 对象内
            context_text=dsl2
        )

        logger.print_smart_analysis("聚合查询测试", diagnostics2[0]["diagnostics"] if diagnostics2 else [])

        # 测试用例 3: Bool 查询
        logger.print_test_header("Bool 查询", "测试 bool 查询内的补全")

        dsl3 = """{
  "query": {
    "bool": {
      "must": [
        {
          "match": {
            "title": "search"
          }
        }
      ],
      "filter": [
        {
          "term": {
            "status": "published"
          }
        }
      ]
    }
  }
}"""

        test_did_open(client, logger, "file:///test.es.dsl", dsl3, language_id="json")
        diagnostics3 = read_diagnostics(client, logger)

        # 在 bool 查询内测试补全
        test_completion(
            client,
            logger,
            "file:///test.es.dsl",
            {"line": 3, "character": 10},  # bool 对象内
            context_text=dsl3
        )

        logger.print_smart_analysis("Bool 查询测试", diagnostics3[0]["diagnostics"] if diagnostics3 else [])

        # 测试用例 4: 不完整的 DSL
        logger.print_test_header("不完整的 DSL", "测试 Tree-sitter 容错性")

        dsl4 = """{
  "query": {
    "match": {
      "title": """

        test_did_open(client, logger, "file:///test.es.dsl", dsl4, language_id="json")
        diagnostics4 = read_diagnostics(client, logger, expected_errors=True)

        logger.print_smart_analysis("不完整 DSL 测试", diagnostics4[0]["diagnostics"] if diagnostics4 else [])

        # 测试用例 5: 语法错误
        logger.print_test_header("语法错误", "测试错误检测")

        dsl5 = """{
  "query": {
    "match": {
      "title": "test"
    }
  }
  "aggs": {
    "avg_price": {
      "avg": {
        "field": "price"
      }
    }
  }
}"""

        test_did_open(client, logger, "file:///test.es.dsl", dsl5, language_id="json")
        diagnostics5 = read_diagnostics(client, logger, expected_errors=True)

        logger.print_smart_analysis("语法错误测试", diagnostics5[0]["diagnostics"] if diagnostics5 else [])

        logger.print_section("所有测试完成", level=1)
        logger.print_stats()

        return 0

    except KeyboardInterrupt:
        logger.warning("\n测试被用户中断")
        return 130
    except Exception as e:
        logger.error(f"测试过程中发生错误: {e}")
        import traceback
        logger.debug(traceback.format_exc())
        return 1
    finally:
        client.stop()
        logger.info("测试结束")

if __name__ == "__main__":
    sys.exit(main())
