//! How a tab's program runs, as Herdr app's `tab.new {argv}` runs it (`app:src/window.rs`
//! `agent_argv`, `unix_agent_argv`; `app:src/remote_shell.rs` `launch`): inside the person's
//! login shell, so the profiles that put tools on `PATH` (Homebrew, `~/.local/bin`) are loaded,
//! and then that shell stays in the terminal. A program that should leave no shell behind ends
//! its terminal itself, as the review's does.

/// `cmd` in `shell` (this computer's `$SHELL`) as a login shell (macOS terminal sessions load
/// login profiles), then that shell, interactive.
pub fn login(shell: &str, cmd: Vec<String>) -> Vec<String> {
    let fish = std::path::Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        == Some("fish");
    let script = if fish {
        "command $argv[2..]; exec $argv[1] -li"
    } else {
        "\"$@\"; exec \"$0\" -li"
    };
    let mut run = vec![
        shell.to_owned(),
        "-lic".to_owned(),
        script.to_owned(),
        shell.to_owned(),
    ];
    run.extend(cmd);
    run
}

/// `cmd` on another machine: in the shell there (its `$SHELL`, not this computer's), as a login
/// shell, since its core may have started with only the SSH session's minimal `PATH`. Only fixed
/// scripts become shell source; `cmd` stays as arguments.
pub fn remote(cmd: Vec<String>) -> Vec<String> {
    let quote = |s: &str| s.replace('\'', "'\\''");
    let fish = "command $argv[2..]; exec $argv[1] -li";
    let posix = "\"$@\"; exec \"$0\" -li";
    let fallback = "\"$@\"; exec \"${SHELL:-sh}\" -i";
    let script = format!(
        r#"shell=${{SHELL:-/bin/sh}}
case "${{shell##*/}}" in
  fish) exec "$shell" -lic '{}' "$shell" "$@" ;;
  sh|bash|zsh|dash|ksh) exec "$shell" -lic '{}' "$shell" "$@" ;;
  *) exec sh -c '{}' sh "$@" ;;
esac"#,
        quote(fish),
        quote(posix),
        quote(fallback),
    );
    let mut argv = vec!["sh".to_owned(), "-c".to_owned(), script, "sh".to_owned()];
    argv.extend(cmd);
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd() -> Vec<String> {
        vec!["review".to_owned(), "--all".to_owned()]
    }

    #[test]
    fn a_local_program_runs_in_the_login_shell_which_then_stays() {
        assert_eq!(
            login("/bin/zsh", cmd()),
            [
                "/bin/zsh",
                "-lic",
                "\"$@\"; exec \"$0\" -li",
                "/bin/zsh",
                "review",
                "--all"
            ]
        );
        assert_eq!(
            login("/opt/homebrew/bin/fish", cmd()),
            [
                "/opt/homebrew/bin/fish",
                "-lic",
                "command $argv[2..]; exec $argv[1] -li",
                "/opt/homebrew/bin/fish",
                "review",
                "--all"
            ]
        );
    }

    #[test]
    fn a_remote_program_picks_the_shell_there_and_keeps_its_arguments_apart() {
        let argv = remote(cmd());
        assert_eq!(argv[..2], ["sh", "-c"]);
        assert!(argv[2].starts_with("shell=${SHELL:-/bin/sh}\n"));
        assert!(
            argv[2].contains("fish) exec \"$shell\" -lic 'command $argv[2..]; exec $argv[1] -li'")
        );
        assert_eq!(argv[3..], ["sh", "review", "--all"]);
    }
}
