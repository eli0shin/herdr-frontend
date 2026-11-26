fn main() {
    // The terminals need a libghostty-vt the bindings work with; stop before opening a window.
    if let Err(mismatch) = ghostty_vt_sys::check_version() {
        eprintln!("herdr-frontend: {mismatch}");
        std::process::exit(1);
    }
    herdr_frontend::app().run();
}
