use clap::{CommandFactory, FromArgMatches, Parser};
use std::env;

mod app;
mod auto_title;
mod event_handler;
mod rpc;
mod state;
mod ui;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Send a notification to running crmux instance
    #[command(
        verbatim_doc_comment,
        long_about = "\
Send a notification to running crmux instance

Reads JSON params from stdin and sends them as an RPC message.

Events:
  send-text  Send text to a session pane
             Params: {\"text\": \"...\", \"project\": \"...\", \"no_execute\": true}

Example: echo '{\"text\": \"hello\"}' | crmux notify send-text"
    )]
    Notify {
        /// Event type (e.g., send-text)
        event: String,
    },
}

fn main() {
    let cli = Cli::from_arg_matches(
        &Cli::command()
            .after_help(ui::HELP_TEXT)
            .get_matches(),
    )
    .expect("failed to parse CLI arguments");

    match cli.command {
        Some(Commands::Notify { event }) => {
            if let Err(e) = handle_notify(&event) {
                eprintln!("crmux notify error: {e}");
                std::process::exit(1);
            }
        }
        None => {
            if env::var("TMUX").is_err() {
                eprintln!("crmux must be run inside tmux");
                std::process::exit(1);
            }

            if let Err(e) = app::run() {
                eprintln!("crmux error: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn handle_notify(event: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;

    let mut params: serde_json::Value =
        serde_json::from_str(input.trim()).unwrap_or_else(|_| serde_json::json!({}));

    // Add pane_id from $TMUX_PANE if available.
    // $TMUX_PANE is in %XX format, but tmux-claude-state uses session:window.pane format.
    // Convert via `tmux display-message` so RPC messages match managed sessions.
    if let Ok(pane_id) = env::var("TMUX_PANE") {
        let resolved = std::process::Command::new("tmux")
            .args([
                "display-message",
                "-p",
                "-t",
                &pane_id,
                "#{session_name}:#{window_index}.#{pane_index}",
            ])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or(pane_id);
        if let Some(obj) = params.as_object_mut() {
            obj.insert("pane_id".to_string(), serde_json::Value::String(resolved));
        }
    }

    let method = event.replace('-', "_");
    rpc::send_notification(&method, &params)?;
    Ok(())
}
