fn main() {
    // `ls`, `do`, `status` and `commands` talk to the running window (`docs/cli.md`).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if herdr_frontend::control::is_subcommand(&args) {
        std::process::exit(herdr_frontend::control::client(&args));
    }
    // The terminals need a libghostty-vt the bindings work with; stop before opening a window.
    if let Err(mismatch) = ghostty_vt_sys::check_version() {
        eprintln!("herdr-frontend: {mismatch}");
        std::process::exit(1);
    }
    herdr_frontend::app().run();
}
