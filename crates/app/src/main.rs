use herdr_client::{HcCommand, Transport};
use herdr_frontend::home::Home;
use herdr_frontend::link::Live;

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
    // `--snap`, `--tree` and `--script` draw scenes with no core, and never touch a real one.
    if rune::is_headless() {
        herdr_frontend::app().run();
        return;
    }
    // `hc connect`, as Herdr app connects: the core `HC_SESSION` names, or the default one.
    let link = Live::new(Transport::Hc(HcCommand::found()));
    herdr_frontend::live(link, Some(Home::from_env())).run();
}
