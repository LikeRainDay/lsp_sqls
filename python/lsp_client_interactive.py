#!/usr/bin/env python3
"""
Interactive LSP Client for sql-lsp server

This client demonstrates full LSP lifecycle management with:
- Complete LSP protocol implementation (initialize → didOpen → completion → shutdown)
- Interactive terminal interface with real-time completions
- Comprehensive logging of all LSP communication

Usage:
    python lsp_client_interactive.py [--debug] [--lsp-path PATH]

Commands:
    /quit      - Exit the client
    /clear     - Clear current SQL buffer
    /schema    - Inject sample schema
    /format    - Format current SQL
    /help      - Show help message
"""

import os
import sys
import json
import subprocess
import threading
import time
import logging
from typing import Optional, Dict, Any, List
from dataclasses import dataclass
import argparse

try:
    from colorama import init, Fore, Style, Back
    init(autoreset=True)
    HAS_COLOR = True
except ImportError:
    HAS_COLOR = False
    # Fallback if colorama is not installed
    class Fore:
        RED = GREEN = YELLOW = BLUE = MAGENTA = CYAN = WHITE = RESET = ""
    class Style:
        BRIGHT = DIM = RESET_ALL = ""
    class Back:
        RED = GREEN = YELLOW = BLUE = MAGENTA = CYAN = WHITE = RESET = ""


@dataclass
class Position:
    """LSP Position (line, character)"""
    line: int
    character: int

    def to_dict(self):
        return {"line": self.line, "character": self.character}


@dataclass
class CompletionItem:
    """LSP Completion Item"""
    label: str
    kind: int
    detail: Optional[str] = None
    documentation: Optional[str] = None
    insert_text: Optional[str] = None

    @classmethod
    def from_dict(cls, data: Dict[str, Any]):
        return cls(
            label=data["label"],
            kind=data.get("kind", 1),
            detail=data.get("detail"),
            documentation=data.get("documentation"),
            insert_text=data.get("insertText", data["label"])
        )

    def kind_name(self) -> str:
        """Convert LSP kind number to readable name"""
        kinds = {
            1: "Text", 2: "Method", 3: "Function", 4: "Constructor",
            5: "Field", 6: "Variable", 7: "Class", 8: "Interface",
            9: "Module", 10: "Property", 11: "Unit", 12: "Value",
            13: "Enum", 14: "Keyword", 15: "Snippet", 16: "Color",
            17: "File", 18: "Reference", 19: "Folder", 20: "EnumMember",
            21: "Constant", 22: "Struct", 23: "Event", 24: "Operator",
            25: "TypeParameter"
        }
        return kinds.get(self.kind, f"Kind{self.kind}")


class LspClient:
    """LSP Client for sql-lsp server"""

    def __init__(self, lsp_path: str, debug: bool = False):
        self.lsp_path = lsp_path
        self.debug = debug
        self.process: Optional[subprocess.Popen] = None
        self.request_id = 0
        self.response_queue: Dict[int, Any] = {}
        self.lock = threading.Lock()
        self.running = False

        # Setup logging
        log_level = logging.DEBUG if debug else logging.INFO
        logging.basicConfig(
            level=log_level,
            format='%(asctime)s [%(levelname)s] %(message)s',
            datefmt='%H:%M:%S'
        )
        self.logger = logging.getLogger(__name__)

    def start(self):
        """Start the LSP server process"""
        self.logger.info(f"{Fore.CYAN}Starting sql-lsp server: {self.lsp_path}")

        env = os.environ.copy()
        if self.debug:
            env['RUST_LOG'] = 'debug'

        try:
            self.process = subprocess.Popen(
                [self.lsp_path],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=env,
                bufsize=0
            )
        except FileNotFoundError:
            self.logger.error(f"{Fore.RED}Error: LSP binary not found at {self.lsp_path}")
            self.logger.error(f"{Fore.YELLOW}Please build it with: cargo build --release")
            sys.exit(1)

        self.running = True

        # Start threads to read responses and stderr
        self.response_thread = threading.Thread(target=self._read_responses, daemon=True)
        self.response_thread.start()

        self.stderr_thread = threading.Thread(target=self._read_stderr, daemon=True)
        self.stderr_thread.start()

        self.logger.info(f"{Fore.GREEN}✓ LSP server started (PID: {self.process.pid})")

    def _read_responses(self):
        """Read responses from LSP server stdout"""
        while self.running and self.process:
            try:
                # Read Content-Length header
                header = self.process.stdout.readline().decode('utf-8').strip()
                if not header:
                    continue

                if not header.startswith('Content-Length:'):
                    continue

                content_length = int(header.split(':')[1].strip())

                # Read empty line
                self.process.stdout.readline()

                # Read JSON content
                content = self.process.stdout.read(content_length).decode('utf-8')
                message = json.loads(content)

                self._log_message("RECV", message)

                # Handle response or notification
                if 'id' in message:
                    # Response to our request
                    with self.lock:
                        self.response_queue[message['id']] = message
                elif 'method' in message:
                    # Server notification
                    self._handle_notification(message)

            except json.JSONDecodeError as e:
                if self.running:
                    self.logger.error(f"{Fore.RED}JSON decode error: {e}")
                    self.logger.error(f"{Fore.RED}Content: {content[:200]}")
                continue
            except Exception as e:
                if self.running:
                    self.logger.error(f"{Fore.RED}Error reading response: {e}", exc_info=self.debug)
                continue

    def _read_stderr(self):
        """Read and log stderr from LSP server"""
        while self.running and self.process:
            try:
                line = self.process.stderr.readline()
                if not line:
                    break
                line = line.decode('utf-8').strip()
                if line:
                    self.logger.debug(f"{Fore.YELLOW}[SERVER] {line}")
            except Exception as e:
                if self.running:
                    self.logger.error(f"{Fore.RED}Error reading stderr: {e}")
                break

    def _handle_notification(self, message: Dict[str, Any]):
        """Handle server notifications"""
        method = message.get('method')

        if method == 'textDocument/publishDiagnostics':
            diagnostics = message.get('params', {}).get('diagnostics', [])
            if diagnostics:
                self.logger.info(f"{Fore.MAGENTA}📋 Diagnostics ({len(diagnostics)}):")
                for diag in diagnostics:
                    severity = {1: "ERROR", 2: "WARNING", 3: "INFO", 4: "HINT"}.get(
                        diag.get('severity', 1), "UNKNOWN"
                    )
                    msg = diag.get('message', '')
                    self.logger.info(f"{Fore.MAGENTA}  [{severity}] {msg}")
        else:
            self.logger.debug(f"{Fore.CYAN}Notification: {method}")

    def _log_message(self, direction: str, message: Dict[str, Any]):
        """Log LSP message with formatting"""
        if direction == "SEND":
            color = Fore.GREEN
            arrow = "→"
        else:
            color = Fore.BLUE
            arrow = "←"

        # Safely extract method name - result could be a list (e.g., completion response)
        method = message.get('method', 'response')
        msg_id = message.get('id', '')

        if self.debug:
            formatted = json.dumps(message, indent=2)
            self.logger.debug(f"{color}{arrow} {direction} [{msg_id}] {method}:")
            for line in formatted.split('\n'):
                self.logger.debug(f"{color}  {line}")
        else:
            truncated = str(message)[:100] + "..." if len(str(message)) > 100 else str(message)
            self.logger.info(f"{color}{arrow} {direction} [{msg_id}] {method}")

    def _send_request(self, method: str, params: Optional[Dict[str, Any]] = None) -> int:
        """Send LSP request and return request ID"""
        self.request_id += 1
        request = {
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
            "params": params or {}
        }

        self._send_message(request)
        return self.request_id

    def _send_notification(self, method: str, params: Optional[Dict[str, Any]] = None):
        """Send LSP notification (no response expected)"""
        notification = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params or {}
        }

        self._send_message(notification)

    def _send_message(self, message: Dict[str, Any]):
        """Send message to LSP server"""
        content = json.dumps(message)
        content_bytes = content.encode('utf-8')

        header = f"Content-Length: {len(content_bytes)}\r\n\r\n"

        self._log_message("SEND", message)

        try:
            self.process.stdin.write(header.encode('utf-8'))
            self.process.stdin.write(content_bytes)
            self.process.stdin.flush()
        except Exception as e:
            self.logger.error(f"{Fore.RED}Error sending message: {e}")

    def _wait_for_response(self, request_id: int, timeout: float = 5.0) -> Optional[Dict[str, Any]]:
        """Wait for response to request"""
        start_time = time.time()

        while time.time() - start_time < timeout:
            with self.lock:
                if request_id in self.response_queue:
                    return self.response_queue.pop(request_id)
            time.sleep(0.01)

        self.logger.error(f"{Fore.RED}Timeout waiting for response to request {request_id}")
        return None

    def initialize(self) -> bool:
        """Initialize LSP server"""
        self.logger.info(f"{Fore.CYAN}Initializing LSP server...")

        params = {
            "processId": os.getpid(),
            "rootUri": f"file://{os.getcwd()}",
            "capabilities": {
                "textDocument": {
                    "completion": {
                        "dynamicRegistration": True,
                        "completionItem": {
                            "snippetSupport": False,
                            "documentationFormat": ["markdown", "plaintext"]
                        }
                    },
                    "hover": {"dynamicRegistration": True},
                    "synchronization": {
                        "didSave": True,
                        "willSave": True
                    }
                },
                "workspace": {
                    "configuration": True,
                    "didChangeConfiguration": {"dynamicRegistration": True}
                }
            }
        }

        request_id = self._send_request("initialize", params)
        response = self._wait_for_response(request_id)

        if response and 'result' in response:
            caps = response['result'].get('capabilities', {})
            self.logger.info(f"{Fore.GREEN}✓ Server initialized")
            self.logger.info(f"{Fore.CYAN}  Capabilities: {list(caps.keys())}")

            # Send initialized notification
            self._send_notification("initialized", {})
            return True

        self.logger.error(f"{Fore.RED}✗ Initialization failed")
        return False

    def did_open(self, uri: str, text: str, language_id: str = "sql"):
        """Notify server that document is opened"""
        params = {
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": text
            }
        }
        self._send_notification("textDocument/didOpen", params)

    def did_change(self, uri: str, text: str, version: int):
        """Notify server of document changes"""
        params = {
            "textDocument": {
                "uri": uri,
                "version": version
            },
            "contentChanges": [
                {"text": text}
            ]
        }
        self._send_notification("textDocument/didChange", params)

    def completion(self, uri: str, position: Position) -> List[CompletionItem]:
        """Request completions at position"""
        params = {
            "textDocument": {"uri": uri},
            "position": position.to_dict()
        }

        request_id = self._send_request("textDocument/completion", params)
        response = self._wait_for_response(request_id)

        if response and 'result' in response:
            result = response['result']
            items = result if isinstance(result, list) else result.get('items', [])
            return [CompletionItem.from_dict(item) for item in items]

        return []

    def did_change_configuration(self, settings: Dict[str, Any]):
        """Send configuration change notification"""
        params = {"settings": settings}
        self._send_notification("workspace/didChangeConfiguration", params)

    def shutdown(self):
        """Shutdown LSP server gracefully"""
        self.logger.info(f"{Fore.CYAN}Shutting down LSP server...")

        request_id = self._send_request("shutdown", {})
        self._wait_for_response(request_id, timeout=2.0)

        self._send_notification("exit", {})

        self.running = False

        if self.process:
            self.process.wait(timeout=2.0)
            self.logger.info(f"{Fore.GREEN}✓ Server shutdown complete")


class InteractiveClient:
    """Interactive terminal client for SQL LSP"""

    def __init__(self, lsp_client: LspClient):
        self.lsp_client = lsp_client
        self.document_uri = "inmemory://interactive"
        self.sql_buffer = ""
        self.version = 1
        self.logger = logging.getLogger(__name__)

    def inject_sample_schema(self):
        """Inject sample database schema"""
        self.logger.info(f"{Fore.CYAN}Injecting sample schema...")

        schema = {
            "schemas": [
                    {
                        "id": "550e8400-e29b-41d4-a716-446655440000",
                        "database": "demo_db",
                        "source_uri": None,
                        "tables": [
                            {
                                "name": "users",
                                "comment": "User accounts",
                                "source_location": None,
                                "columns": [
                                    {
                                        "name": "id",
                                        "data_type": "INT",
                                        "nullable": False,
                                        "comment": "Primary key",
                                        "source_location": None
                                    },
                                    {
                                        "name": "email",
                                        "data_type": "VARCHAR(255)",
                                        "nullable": False,
                                        "comment": "User email address",
                                        "source_location": None
                                    },
                                    {
                                        "name": "name",
                                        "data_type": "VARCHAR(255)",
                                        "nullable": True,
                                        "comment": "User full name",
                                        "source_location": None
                                    },
                                    {
                                        "name": "created_at",
                                        "data_type": "TIMESTAMP",
                                        "nullable": False,
                                        "comment": "Creation timestamp",
                                        "source_location": None
                                    }
                                ]
                            },
                            {
                                "name": "orders",
                                "comment": "Customer orders",
                                "source_location": None,
                                "columns": [
                                    {
                                        "name": "id",
                                        "data_type": "INT",
                                        "nullable": False,
                                        "comment": "Primary key",
                                        "source_location": None
                                    },
                                    {
                                        "name": "user_id",
                                        "data_type": "INT",
                                        "nullable": False,
                                        "comment": "Foreign key to users",
                                        "source_location": None
                                    },
                                    {
                                        "name": "total",
                                        "data_type": "DECIMAL(10,2)",
                                        "nullable": False,
                                        "comment": "Order total amount",
                                        "source_location": None
                                    },
                                    {
                                        "name": "status",
                                        "data_type": "VARCHAR(50)",
                                        "nullable": False,
                                        "comment": "Order status",
                                        "source_location": None
                                    }
                                ]
                            }
                        ],
                        "functions": []
                    }
                ],
                # CRITICAL: Map this document URI to the schema ID
                # Without this mapping, the server has the schema but doesn't know which file uses it!
                "fileSchemas": {
                    self.document_uri: "550e8400-e29b-41d4-a716-446655440000"
                }
        }

        self.lsp_client.did_change_configuration(schema)
        self.logger.info(f"{Fore.GREEN}✓ Schema injected (tables: users, orders)")

    def display_completions(self, items: List[CompletionItem]):
        """Display completion items"""
        if not items:
            print(f"{Fore.YELLOW}  (no completions)")
            return

        print(f"{Fore.CYAN}  Completions ({len(items)}):")
        for i, item in enumerate(items[:20], 1):  # Limit to 20 items
            kind_color = {
                5: Fore.GREEN,    # Field
                7: Fore.BLUE,     # Class/Table
                3: Fore.MAGENTA,  # Function
                14: Fore.YELLOW,  # Keyword
                24: Fore.CYAN,    # Operator
            }.get(item.kind, Fore.WHITE)

            label = item.label
            kind = item.kind_name()
            detail = f" - {item.detail}" if item.detail else ""

            print(f"{kind_color}  {i:2}. [{kind:8}] {label}{Style.DIM}{detail}{Style.RESET_ALL}")

        if len(items) > 20:
            print(f"{Fore.YELLOW}  ... and {len(items) - 20} more")

    def get_cursor_position(self, text: str) -> Position:
        """Calculate cursor position from text"""
        lines = text.split('\n')
        line = len(lines) - 1
        character = len(lines[-1])
        return Position(line, character)

    def show_help(self):
        """Show help message"""
        print(f"\n{Fore.CYAN}{'='*60}")
        print(f"{Fore.CYAN}{Style.BRIGHT}SQL LSP Interactive Client - Commands")
        print(f"{Fore.CYAN}{'='*60}{Style.RESET_ALL}")
        print(f"{Fore.GREEN}/quit{Style.RESET_ALL}      - Exit the client")
        print(f"{Fore.GREEN}/clear{Style.RESET_ALL}     - Clear current SQL buffer")
        print(f"{Fore.GREEN}/schema{Style.RESET_ALL}    - Inject sample schema (users, orders tables)")
        print(f"{Fore.GREEN}/help{Style.RESET_ALL}      - Show this help message")
        print(f"\n{Fore.YELLOW}Type SQL and press Enter to see completions at cursor position")
        print(f"{Fore.YELLOW}Each input is independent - no buffer accumulation{Style.RESET_ALL}\n")

    def run(self):
        """Run interactive loop"""
        print(f"\n{Fore.CYAN}{'='*60}")
        print(f"{Fore.CYAN}{Style.BRIGHT}SQL LSP Interactive Client")
        print(f"{Fore.CYAN}{'='*60}{Style.RESET_ALL}")
        print(f"{Fore.YELLOW}Type SQL to see completions, or /help for commands{Style.RESET_ALL}\n")

        # Open document
        self.lsp_client.did_open(self.document_uri, self.sql_buffer)

        # Inject sample schema
        self.inject_sample_schema()

        print(f"{Fore.GREEN}Ready! Start typing SQL...{Style.RESET_ALL}\n")

        while True:
            try:
                # Show prompt
                prompt = f"{Fore.MAGENTA}SQL> {Style.RESET_ALL}"
                user_input = input(prompt)

                # Handle commands
                if user_input.strip().lower() == '/quit':
                    print(f"{Fore.YELLOW}Goodbye!{Style.RESET_ALL}")
                    break
                elif user_input.strip().lower() == '/clear':
                    self.sql_buffer = ""
                    self.version += 1
                    self.lsp_client.did_change(self.document_uri, self.sql_buffer, self.version)
                    print(f"{Fore.GREEN}✓ Buffer cleared{Style.RESET_ALL}")
                    continue
                elif user_input.strip().lower() == '/schema':
                    self.inject_sample_schema()
                    continue
                elif user_input.strip().lower() == '/help':
                    self.show_help()
                    continue

                # Use current input as standalone SQL (no buffer accumulation)
                # This makes testing completions easier - each input is independent
                current_sql = user_input

                # Update document with current input
                self.version += 1
                self.lsp_client.did_change(self.document_uri, current_sql, self.version)

                # Get completions at cursor (end of current input)
                position = self.get_cursor_position(current_sql)
                items = self.lsp_client.completion(self.document_uri, position)

                # Display
                self.display_completions(items)
                print()

            except KeyboardInterrupt:
                print(f"\n{Fore.YELLOW}Use /quit to exit{Style.RESET_ALL}")
            except EOFError:
                break
            except Exception as e:
                self.logger.error(f"{Fore.RED}Error: {e}", exc_info=True)


def main():
    parser = argparse.ArgumentParser(description="Interactive LSP client for sql-lsp")
    parser.add_argument(
        '--lsp-path',
        default='../target/release/sql-lsp',
        help='Path to sql-lsp binary (default: ../target/release/sql-lsp)'
    )
    parser.add_argument(
        '--debug',
        action='store_true',
        help='Enable debug logging'
    )

    args = parser.parse_args()

    # Create and start LSP client
    lsp_client = LspClient(args.lsp_path, debug=args.debug)
    lsp_client.start()

    # Initialize
    if not lsp_client.initialize():
        sys.exit(1)

    # Run interactive client
    interactive = InteractiveClient(lsp_client)

    try:
        interactive.run()
    finally:
        lsp_client.shutdown()


if __name__ == '__main__':
    main()
