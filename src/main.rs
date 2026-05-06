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
  get-pane-id   Get the pane ID where crmux is running (request)
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

    /// Switch to the tmux pane where crmux is running
    Focus,

    /// Launch a Claude Code session in a new tmux window
    #[command(
        trailing_var_arg = true,
        long_about = "\
Launch a Claude Code session in a new tmux window with a specified width.
All arguments except -x and -e are passed through to the claude command.

Examples:
  crmux claude
  crmux claude --resume
  crmux claude -x 120 -p \"fix the bug\"
  crmux claude -e CLAUDE_MODEL=sonnet -e ANTHROPIC_API_KEY=sk-xxx
  echo 'hello' | crmux claude"
    )]
    Claude {
        /// Window width in columns (default: 100)
        #[arg(short = 'x', default_value = "100")]
        width: u16,

        /// Set environment variable for the tmux window (KEY=VALUE, can be specified multiple times)
        #[arg(short = 'e', value_parser = validate_env_var)]
        envs: Vec<String>,

        /// Arguments to pass to the claude command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
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
        Some(Commands::Focus) => {
            if let Err(e) = handle_focus() {
                eprintln!("crmux focus error: {e}");
                std::process::exit(1);
            }
        }
        Some(Commands::Claude { width, envs, args }) => {
            if let Err(e) = handle_claude(width, &envs, &args) {
                eprintln!("crmux claude error: {e}");
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

fn handle_focus() -> Result<(), Box<dyn std::error::Error>> {
    let result = rpc::send_request("get_pane_id", &serde_json::json!({}))?;
    let pane_id = result
        .as_str()
        .ok_or("crmux is not running or pane ID is unavailable")?;
    let status = std::process::Command::new("tmux")
        .args(["switch-client", "-t", pane_id])
        .status()?;
    if !status.success() {
        return Err(format!("tmux switch-client failed (exit {status})").into());
    }
    Ok(())
}

fn read_stdin_if_piped() -> Result<Option<String>, Box<dyn std::error::Error>> {
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Ok(None);
    }
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
    Ok(Some(buf))
}

#[allow(clippy::literal_string_with_formatting_args)]
fn build_tmux_args(cwd: &str, envs: &[String], claude_args: &[String]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "new-window".into(),
        "-d".into(),
        "-c".into(),
        cwd.into(),
    ];
    for env in envs {
        args.push("-e".into());
        args.push(env.clone());
    }
    args.extend([
        "-P".into(),
        "-F".into(),
        "#{window_id}".into(),
        "--".into(),
        "claude".into(),
    ]);
    args.extend(claude_args.iter().cloned());
    args
}

fn handle_claude(width: u16, envs: &[String], args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let stdin_content = read_stdin_if_piped()?;

    let claude_args = build_claude_args(args, stdin_content.as_deref());
    let cwd = env::current_dir()?.to_string_lossy().to_string();

    let tmux_args = build_tmux_args(&cwd, envs, &claude_args);

    let output = std::process::Command::new("tmux")
        .args(&tmux_args)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }

    let window_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let output = std::process::Command::new("tmux")
        .args([
            "resize-window",
            "-t",
            &window_id,
            "-x",
            &width.to_string(),
        ])
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }

    Ok(())
}

fn validate_env_var(s: &str) -> Result<String, String> {
    match s.split_once('=') {
        Some((key, _)) if !key.is_empty() => Ok(s.to_string()),
        _ => Err(format!("invalid environment variable format: '{s}' (expected KEY=VALUE)")),
    }
}

/// Build the args to pass to the claude command.
///
/// Combines explicit args with stdin content (appended as a positional argument).
fn build_claude_args(args: &[String], stdin: Option<&str>) -> Vec<String> {
    let mut result = args.to_vec();
    if let Some(input) = stdin {
        result.push(input.to_string());
    }
    result
}

fn handle_rpc(event: &str) -> Result<(), Box<dyn std::error::Error>> {
    let method = event.replace('-', "_");

    // Request-type methods: send RPC request and print response
    if method.starts_with("get_") {
        let params = read_stdin_if_piped()?.map_or_else(
            || serde_json::json!({}),
            |input| serde_json::from_str(input.trim()).unwrap_or_else(|_| serde_json::json!({})),
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_claude_args_no_args_no_stdin() {
        let result = build_claude_args(&[], None);
        assert!(result.is_empty());
    }

    #[test]
    fn build_claude_args_with_args() {
        let args = vec!["--resume".to_string()];
        assert_eq!(build_claude_args(&args, None), vec!["--resume"]);
    }

    #[test]
    fn build_claude_args_stdin_no_args() {
        let result = build_claude_args(&[], Some("hello"));
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn build_claude_args_stdin_appended_when_args_present() {
        let args = vec!["--resume".to_string()];
        let result = build_claude_args(&args, Some("hello"));
        assert_eq!(result, vec!["--resume", "hello"]);
    }

    #[test]
    fn validate_env_var_valid() {
        assert!(validate_env_var("FOO=bar").is_ok());
    }

    #[test]
    fn validate_env_var_empty_value() {
        assert!(validate_env_var("FOO=").is_ok());
    }

    #[test]
    fn validate_env_var_no_equals() {
        assert!(validate_env_var("FOO").is_err());
    }

    #[test]
    fn validate_env_var_empty_key() {
        assert!(validate_env_var("=bar").is_err());
    }

    #[test]
    fn build_tmux_args_no_envs() {
        let result = build_tmux_args("/home/user/project", &[], &["--resume".to_string()]);
        assert_eq!(
            result,
            vec![
                "new-window", "-d", "-c", "/home/user/project",
                "-P", "-F", "#{window_id}",
                "--", "claude", "--resume"
            ]
        );
    }

    #[test]
    fn build_tmux_args_with_envs() {
        let envs = vec!["FOO=bar".to_string(), "BAZ=qux".to_string()];
        let result = build_tmux_args("/home/user/project", &envs, &[]);
        assert_eq!(
            result,
            vec![
                "new-window", "-d", "-c", "/home/user/project",
                "-e", "FOO=bar", "-e", "BAZ=qux",
                "-P", "-F", "#{window_id}",
                "--", "claude"
            ]
        );
    }

    #[test]
    fn cli_claude_parses_env_option() {
        let cli = Cli::try_parse_from(["crmux", "claude", "-e", "FOO=bar"]).unwrap();
        match cli.command {
            Some(Commands::Claude { envs, args, .. }) => {
                assert_eq!(envs, vec!["FOO=bar"]);
                assert!(args.is_empty());
            }
            _ => panic!("expected Claude command"),
        }
    }

    #[test]
    fn cli_claude_parses_multiple_env_options() {
        let cli =
            Cli::try_parse_from(["crmux", "claude", "-e", "A=1", "-e", "B=2", "--resume"])
                .unwrap();
        match cli.command {
            Some(Commands::Claude { envs, args, .. }) => {
                assert_eq!(envs, vec!["A=1", "B=2"]);
                assert_eq!(args, vec!["--resume"]);
            }
            _ => panic!("expected Claude command"),
        }
    }

    #[test]
    fn cli_claude_rejects_invalid_env() {
        let result = Cli::try_parse_from(["crmux", "claude", "-e", "NOEQUALS"]);
        assert!(result.is_err());
    }

    #[test]
    fn build_tmux_args_with_envs_and_claude_args() {
        let envs = vec!["X=1".to_string()];
        let result = build_tmux_args("/tmp", &envs, &["-p".to_string(), "hello".to_string()]);
        assert_eq!(
            result,
            vec![
                "new-window", "-d", "-c", "/tmp",
                "-e", "X=1",
                "-P", "-F", "#{window_id}",
                "--", "claude", "-p", "hello"
            ]
        );
    }
}
