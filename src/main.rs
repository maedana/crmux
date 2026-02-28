use clap::Parser;
use std::env;

mod app;
mod event_handler;
mod layout;
mod state;
mod tmux_ops;
mod ui;

#[derive(Parser)]
#[command(name = "crmux")]
#[command(about = "Aggregate and manage Claude Code sessions in tmux")]
#[command(version)]
struct Args {
    /// Run as the sidebar TUI (internal use)
    #[arg(long)]
    internal_sidebar: bool,
}

fn main() {
    let args = Args::parse();

    if args.internal_sidebar {
        if let Err(e) = app::run_sidebar() {
            eprintln!("crmux sidebar error: {e}");
            std::process::exit(1);
        }
    } else {
        // Launcher mode: create/select the "claude" tmux window with sidebar
        if env::var("TMUX").is_err() {
            eprintln!("crmux must be run inside tmux");
            std::process::exit(1);
        }
        if let Err(e) = tmux_ops::launch_sidebar_window() {
            eprintln!("Failed to launch sidebar window: {e}");
            std::process::exit(1);
        }
    }
}
