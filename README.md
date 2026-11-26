# herdr-frontend

A native desktop client for [herdr-core](https://github.com/herdrdev/herdr-core), written in Rust on
[Rune](https://github.com/anomalyco/rune), with terminals built on Ghostty's `libghostty-vt`.

It is a second front end on the same herdr-core as Herdr app: the same machines, agents, terminals,
spaces and tabs, so you can switch between the two at any time.

Status: under construction. See `docs/` for research and the plan.

## Develop

macOS only for now. Needs [rustup](https://rustup.rs) (the toolchain comes from
`rust-toolchain.toml`) and [just](https://just.systems).

```sh
just run          # open the window
just test         # pin check, fmt, clippy and tests, as CI runs them
just snap [scene] # render a scene headless to target/snap/<scene>.png
just check-pins   # check the Rune pin against Rune's patch list at that rev
```

Rune is pinned to one `dev` rev in `Cargo.toml`; bump it as `docs/research/rune.md` §1 describes.
