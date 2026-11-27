# Program table and icons

- `herdr.json`: Herdr app's built-in program table, `defaults/programs.json` from
  `~/code/herdr-app` at `origin/master` `1180a5c`. Herdr app is Apache-2.0
  ([LICENSE-herdr-app.txt](LICENSE-herdr-app.txt)).
- `../icons/`: the icons that table names, Herdr app's `defaults/icons/` at the same commit
  (identical to what Herdr app 0.1.18 installs in `~/.config/herdr-app/icons/`). Their licenses
  are in [../icons/LICENSES.md](../icons/LICENSES.md): Simple Icons (CC0), Lucide (ISC), LobeHub
  (MIT) and oh-my-pi (MIT).
- `eli.json`: Eli's `~/.dotfiles/home/.config/herdr-app/programs.json`, applied over
  `herdr.json`: Pi in purple, Neovim with its full-color logo.
- `../program-icons/neovim.svg`: Eli's Neovim logo, from devicons/devicon (MIT; the file's first
  line says where it comes from).
- `../themes/arctic.json`: Eli's `~/.dotfiles/home/.config/herdr-app/themes/arctic.json`.

`crates/app/build.rs` embeds the icons; `crates/app/src/config/programs.rs` reads the tables.
