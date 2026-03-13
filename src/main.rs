use clap::{CommandFactory, FromArgMatches, Parser};
use std::env;

mod app;
mod auto_title;
mod event_handler;
mod rpc;
mod state;
mod ui;
mod update;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Initial workspace (tmux session name) to filter by
    #[arg(short, long)]
    workspace: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Call an RPC method on the running crmux instance
    #[command(
        verbatim_doc_comment,
        long_about = "\
Call an RPC method on the running crmux instance

For notification methods, reads JSON params from stdin.
For request methods (get-*), sends a request and prints the JSON response.

Methods:
  send-text     Send text to a session pane (notification)
                Params: {\"text\": \"...\", \"project\": \"...\", \"no_execute\": true, \"mode\": \"plan-mode|accept-edits\"}
  get-sessions  Get all sessions as JSON (request)
                Params: {\"project\": \"...\"}
  get-plans     Get all accumulated plans as JSON (request)
                Params: {\"project\": \"...\"}

Examples:
  echo '{\"text\": \"hello\"}' | crmux rpc send-text
  crmux rpc get-sessions
  crmux rpc get-plans
  echo '{\"project\": \"myapp\"}' | crmux rpc get-plans"
    )]
    Rpc {
        /// Method name (e.g., send-text, get-sessions, get-plans)
        method: String,
    },

    /// [deprecated] Use 'rpc' instead. Will be removed in a future version.
    #[command(hide = true)]
    Notify {
        /// Event type
        event: String,
    },

    /// Update crmux to the latest version
    Update {
        /// Skip version check and force re-download
        #[arg(long)]
        force: bool,
        /// Check for updates without installing
        #[arg(long)]
        check: bool,
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
        Some(Commands::Rpc { method }) => {
            if let Err(e) = handle_rpc(&method) {
                eprintln!("crmux rpc error: {e}");
                std::process::exit(1);
            }
        }
        Some(Commands::Notify { event }) => {
            eprintln!("warning: 'crmux notify' is deprecated, use 'crmux rpc' instead");
            if let Err(e) = handle_rpc(&event) {
                eprintln!("crmux notify error: {e}");
                std::process::exit(1);
            }
        }
        Some(Commands::Update { force, check }) => {
            handle_update(force, check);
        }
        None => {
            if env::var("TMUX").is_err() {
                eprintln!("crmux must be run inside tmux");
                std::process::exit(1);
            }

            if let Err(e) = app::run(cli.workspace) {
                eprintln!("crmux error: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn handle_update(force: bool, check: bool) {
    let current = env!("CARGO_PKG_VERSION");
    println!("crmux v{current} - checking for updates...");

    if check {
        match update::fetch_latest_version() {
            Ok(latest) => match update::check_update_needed(current, &latest) {
                update::UpdateStatus::AlreadyLatest(v) => {
                    println!("Already up to date (latest: {v})");
                }
                update::UpdateStatus::UpdateAvailable(v) => {
                    println!("Update available: {v}");
                    println!("Run `crmux update` to install");
                }
            },
            Err(e) => {
                eprintln!("Failed to check for updates: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    if force {
        // Force: set current_version to 0.0.0 so self_update always downloads
        match update::perform_update_force() {
            Ok(status) => println!("{status}"),
            Err(e) => {
                eprintln!("Update failed: {e}");
                std::process::exit(1);
            }
        }
    } else {
        match update::perform_update() {
            Ok(status) => println!("{status}"),
            Err(e) => {
                eprintln!("Update failed: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn handle_rpc(event: &str) -> Result<(), Box<dyn std::error::Error>> {
    let method = event.replace('-', "_");

    // Request-type methods: send RPC request and print response
    if method.starts_with("get_") {
        let params = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            serde_json::json!({})
        } else {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
            serde_json::from_str(input.trim()).unwrap_or_else(|_| serde_json::json!({}))
        };
        let result = rpc::send_request(&method, &params)?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    // Notification-type methods: read params from stdin
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

    rpc::send_notification(&method, &params)?;
    Ok(())
}
