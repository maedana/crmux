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
  Space          Mark/unmark session for multi-preview
  Enter          Switch tmux focus to the selected session's pane
  i              Enter input mode (type a prompt to send to the session)
  q / Esc        Quit crmux

Keybindings (Input mode):
  Ctrl+Enter     Send the typed text to the session and return to normal mode
  Ctrl+d         Same as Ctrl+Enter (universal fallback)
  Enter          Insert a newline in the input buffer
  Esc            Cancel input and return to normal mode
  Backspace      Delete the last character")]
struct Cli {}

fn main() {
    Cli::parse();

    if env::var("TMUX").is_err() {
        eprintln!("crmux must be run inside tmux");
        std::process::exit(1);
    }

    if let Err(e) = app::run() {
        eprintln!("crmux error: {e}");
        std::process::exit(1);
    }
}
