use clap::Parser;
use std::env;

mod app;
mod event_handler;
mod state;
mod ui;

#[derive(Parser)]
#[command(version, about, after_help = "\
Keybindings (Normal mode):
  j / ↓          Move cursor down in session list
  k / ↑          Move cursor up in session list
  Space          Mark for preview multiple tmux panes
  s              Switch to tmux pane
  i              Enter input mode (type a prompt to send to the session)
  e              Enter title mode (set a title for the session)
  q              Quit crmux

Keybindings (Input mode):
  Ctrl+o         Return to normal mode
  Any other key  Forwarded to the tmux pane via send-keys

Keybindings (Title mode):
  Ctrl+o         Save and return to normal mode
  Backspace      Delete the last character")]
struct Cli {}

fn main() {
    let _cli = Cli::parse();

    if env::var("TMUX").is_err() {
        eprintln!("crmux must be run inside tmux");
        std::process::exit(1);
    }

    if let Err(e) = app::run() {
        eprintln!("crmux error: {e}");
        std::process::exit(1);
    }
}
