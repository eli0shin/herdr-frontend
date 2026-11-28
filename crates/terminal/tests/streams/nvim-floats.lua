-- Neovim for the repaint replay test (../repaint.rs): line numbers, a cursorline, a sign column
-- and a status line in every window, and `Float()`, which opens a bordered float over the
-- middle of the screen as plugins' pickers and file trees do.
vim.o.termguicolors = true
vim.o.number = true
vim.o.cursorline = true
vim.o.signcolumn = "yes"
vim.o.laststatus = 2
vim.o.swapfile = false

function Float()
  local buf = vim.api.nvim_create_buf(false, true)
  local lines = {}
  for i = 1, 12 do
    lines[i] = ("  %02d  entry %d of the float, over whatever was here"):format(i, i)
  end
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  vim.api.nvim_open_win(buf, true, {
    relative = "editor", row = 6, col = 18, width = 60, height = 12,
    border = "rounded", title = " Float ", title_pos = "center",
  })
end
