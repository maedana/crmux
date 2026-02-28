use std::env;

mod app;
mod event_handler;
mod state;
mod ui;

fn main() {
    if env::var("TMUX").is_err() {
        eprintln!("crmux must be run inside tmux");
        std::process::exit(1);
    }

    if let Err(e) = app::run() {
        eprintln!("crmux error: {e}");
        std::process::exit(1);
    }
}
