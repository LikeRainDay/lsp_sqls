# Editor Integration

This guide covers how to integrate SQL LSP with popular code editors.

## VS Code

### Installation

1. Build the LSP server:

   ```bash
   cd lsp_sqls
   make build-release
   ```

2. Note the binary location: `target/release/sql-lsp` (or `sql-lsp.exe` on Windows)

### Configuration

Create or edit `.vscode/settings.json` in your project:

```json
{
  "sql-lsp.trace.server": "verbose",
  "[sql]": {
    "editor.defaultFormatter": "your-extension-id"
  }
}
```

### VS Code Extension (Recommended)

For the best experience, create a VS Code extension wrapper. See [docs/developing-vscode-extension.md](./developing-vscode-extension.md) (coming soon).

### Manual Setup (Advanced)

You can use the generic LSP client extension like `vscode-languageclient`:

```json
{
  "languageserver": {
    "sql": {
      "command": "/path/to/sql-lsp",
      "filetypes": ["sql", "mysql", "pgsql"]
    }
  }
}
```

## Neovim

### Using nvim-lspconfig

1. Install `nvim-lspconfig`:

   ```vim
   " Using vim-plug
   Plug 'neovim/nvim-lspconfig'
   ```

2. Add to your `init.lua`:

   ```lua
   local lspconfig = require('lspconfig')
   local configs = require('lspconfig.configs')

   -- Register SQL LSP if not already registered
   if not configs.sql_lsp then
     configs.sql_lsp = {
       default_config = {
         cmd = {'/path/to/sql-lsp'},
         filetypes = {'sql', 'mysql', 'pgsql', 'clickhouse'},
         root_dir = function(fname)
           return lspconfig.util.find_git_ancestor(fname) or vim.loop.cwd()
         end,
         settings = {},
       },
     }
   end

   lspconfig.sql_lsp.setup({
     on_attach = function(client, bufnr)
       -- Key mappings
       local bufopts = { noremap=true, silent=true, buffer=bufnr }
       vim.keymap.set('n', 'gd', vim.lsp.buf.definition, bufopts)
       vim.keymap.set('n', 'K', vim.lsp.buf.hover, bufopts)
       vim.keymap.set('n', 'gr', vim.lsp.buf.references, bufopts)
       vim.keymap.set('n', '<space>f', vim.lsp.buf.format, bufopts)
     end,
     capabilities = require('cmp_nvim_lsp').default_capabilities()
   })
   ```

3. Configure dialect by file pattern:

   ```lua
   vim.api.nvim_create_autocmd({"BufRead", "BufNewFile"}, {
     pattern = {"*.mysql.sql", "*.my.sql"},
     callback = function()
       vim.bo.filetype = "mysql"
     end,
   })

   vim.api.nvim_create_autocmd({"BufRead", "BufNewFile"}, {
     pattern = {"*.postgres.sql", "*.pg.sql"},
     callback = function()
       vim.bo.filetype = "pgsql"
     end,
   })
   ```

### Using coc.nvim

Add to `:CocConfig`:

```json
{
  "languageserver": {
    "sql": {
      "command": "/path/to/sql-lsp",
      "filetypes": ["sql", "mysql", "pgsql"],
      "rootPatterns": [".git", ".root"],
      "settings": {}
    }
  }
}
```

## Vim (vim-lsp)

1. Install `vim-lsp`:

   ```vim
   Plug 'prabirshrestha/vim-lsp'
   ```

2. Configure in `.vimrc`:

   ```vim
   if executable('sql-lsp')
     au User lsp_setup call lsp#register_server({
       \ 'name': 'sql-lsp',
       \ 'cmd': {server_info->['/path/to/sql-lsp']},
       \ 'allowlist': ['sql', 'mysql', 'pgsql'],
       \ })
   endif

   autocmd FileType sql,mysql,pgsql setlocal omnifunc=lsp#complete
   ```

## Emacs (lsp-mode)

Add to your Emacs configuration:

```elisp
(require 'lsp-mode)

(add-to-list 'lsp-language-id-configuration '(sql-mode . "sql"))

(lsp-register-client
 (make-lsp-client :new-connection (lsp-stdio-connection "/path/to/sql-lsp")
                  :major-modes '(sql-mode)
                  :server-id 'sql-lsp))

(add-hook 'sql-mode-hook #'lsp)
```

## Sublime Text (LSP package)

1. Install LSP package via Package Control

2. Add to LSP settings (`Preferences > Package Settings > LSP > Settings`):
   ```json
   {
     "clients": {
       "sql-lsp": {
         "enabled": true,
         "command": ["/path/to/sql-lsp"],
         "selector": "source.sql"
       }
     }
   }
   ```

## Troubleshooting

### Server Not Starting

1. Check binary permissions:

   ```bash
   chmod +x /path/to/sql-lsp
   ```

2. Test server manually:

   ```bash
   /path/to/sql-lsp
   ```

   The server should wait for JSON-RPC input on stdin.

3. Check editor logs:
   - **VS Code**: `Output` panel → select "Language Client"
   - **Neovim**: `:LspLog` or `~/.cache/nvim/lsp.log`
   - **Vim**: `:lua vim.lsp.set_log_level("debug")` then check messages

### No Completions

1. Verify file dialect is detected:

   - Check file extension (`.mysql.sql`, `.postgres.sql`, etc.)
   - Or set languageId explicitly

2. Configure a schema (see [configuration.md](./configuration.md))

3. Check LSP server logs for schema registration messages

### Formatting Not Working

The current formatter is basic. For advanced formatting, use external tools:

- MySQL: `sql-formatter`
- PostgreSQL: `pg_format`

Configure your editor to use these tools instead of LSP formatting.

## Performance Tips

1. **Large files**: LSP is optimized for files up to ~10K lines. For larger files, consider splitting.

2. **Large schemas**: Include only relevant tables in schema configuration.

3. **Multiple projects**: Use separate schema configurations per project workspace.

## See Also

- [Configuration Guide](./configuration.md)
- [Supported Dialects](./dialects.md)
- [Troubleshooting](./troubleshooting.md)
