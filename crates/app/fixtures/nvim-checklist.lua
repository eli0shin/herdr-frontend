-- The Neovim checklist screen (ticket 018): splits, a float with a rounded border, diagnostic
-- undercurls in their colors, a statusline with Nerd Font icons and Powerline separators, CJK and
-- emoji, and the cmdline with the cursor in it. `fixtures/README.md` has the command line that
-- records it, and `scripts/ghostty-diff nvim-checklist` runs it in Ghostty.
vim.o.termguicolors = true
vim.o.number = true
vim.o.cursorline = true
vim.o.laststatus = 2
vim.o.showmode = false
vim.o.swapfile = false
vim.o.shadafile = 'NONE'

local buf = vim.api.nvim_get_current_buf()
vim.api.nvim_buf_set_lines(buf, 0, -1, false, {
  'local function greet(name)',
  '  return "hello, " .. nam',
  'end',
  '',
  '-- 日本語のコメント and an emoji 🚀',
  'local unused = greet("world")',
  'print(greet("Ω"))',
})
vim.bo.filetype = 'lua'
vim.bo.modified = false

-- Undercurls as a colorscheme sets them (the default one underlines diagnostics plainly), in
-- the default diagnostic colors.
vim.api.nvim_set_hl(0, 'DiagnosticUnderlineError', { undercurl = true, sp = '#ffc0b9' })
vim.api.nvim_set_hl(0, 'DiagnosticUnderlineWarn', { undercurl = true, sp = '#fce094' })
vim.api.nvim_set_hl(0, 'DiagnosticUnderlineHint', { underdotted = true, sp = '#a6dbff' })

local ns = vim.api.nvim_create_namespace('checklist')
vim.diagnostic.config({ virtual_text = true, underline = true, signs = true })
vim.diagnostic.set(ns, buf, {
  { lnum = 1, col = 22, end_col = 25, severity = vim.diagnostic.severity.ERROR, message = 'undefined global nam' },
  { lnum = 5, col = 6, end_col = 12, severity = vim.diagnostic.severity.WARN, message = 'unused local' },
  { lnum = 6, col = 0, end_col = 5, severity = vim.diagnostic.severity.HINT, message = 'hint' },
})

-- Powerline separators and Nerd Font icons: a branch, a file, Lua, a clock.
vim.o.statusline = table.concat({
  '%#PmenuSel# \u{e0a0} main %#StatusLine#\u{e0b0}',
  ' \u{f15c} %f %m%=',
  '\u{e0b2}%#PmenuSel# \u{e620} lua \u{e0b3} %l:%c ',
})

vim.cmd('vsplit')

local float = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_lines(float, 0, -1, false, {
  ' \u{f188} undefined global nam',
  ' \u{f0eb} did you mean name?',
  ' ✓ 日本 ▏▎▍▌ ⣿⣀ ━━',
})
vim.api.nvim_open_win(float, false, {
  relative = 'editor',
  row = 9,
  col = 12,
  width = 28,
  height = 3,
  border = 'rounded',
  title = ' Diagnostics ',
  style = 'minimal',
})
