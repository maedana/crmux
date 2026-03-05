use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
        EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tmux_claude_state::monitor::MonitorState;

use std::process::{Command, Stdio};

use crate::event_handler::{self, Action};
use crate::state::{AppState, PreviewEntry};
use crate::ui;

/// Strip OSC 8 hyperlink sequences, preserving the visible link text.
///
/// OSC 8 format: `\x1b]8;;URL\x1b\\ link_text \x1b]8;;\x1b\\`
/// The terminator can be either ST (`\x1b\\`) or BEL (`\x07`).
fn strip_osc8_hyperlinks(input: &str) -> String {
    // OSC 8 marker: \x1b]8;  (ESC ] 8 ;)
    const MARKER: &str = "\x1b]8;";
    let mut result = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(pos) = rest.find(MARKER) {
        result.push_str(&rest[..pos]);
        // Skip past the marker
        rest = &rest[pos + MARKER.len()..];
        // Skip until ST (\x1b\\) or BEL (\x07)
        let end = rest.find("\x1b\\").map(|p| p + 2)
            .or_else(|| rest.find('\x07').map(|p| p + 1));
        match end {
            Some(e) => rest = &rest[e..],
            None => break,
        }
    }
    result.push_str(rest);
    result
}

/// Capture a tmux pane with scrollback history (ANSI escapes preserved).
fn capture_pane_with_scrollback(pane_id: &str, scrollback_lines: u16) -> String {
    let start_line = format!("-{scrollback_lines}");
    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-e", "-S", &start_line, "-t", pane_id])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// Parse a version string like "claudeye 0.3.0\n" into (major, minor, patch).
fn parse_claudeye_version(output: &str) -> Option<(u32, u32, u32)> {
    let version_str = output.trim().strip_prefix("claudeye ")?;
    let mut parts = version_str.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Check if version meets the minimum required version.
const fn version_meets_minimum(
    version: (u32, u32, u32),
    minimum: (u32, u32, u32),
) -> bool {
    if version.0 != minimum.0 {
        return version.0 > minimum.0;
    }
    if version.1 != minimum.1 {
        return version.1 > minimum.1;
    }
    version.2 >= minimum.2
}

/// Minimum claudeye version required for --crmux support.
const MIN_CLAUDEYE_VERSION: (u32, u32, u32) = (0, 7, 0);

/// Try to launch claudeye with --crmux flag if a compatible version is installed.
fn launch_claudeye() -> Option<std::process::Child> {
    // Check if claudeye is available
    let version_output = Command::new("claudeye")
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !version_output.status.success() {
        return None;
    }

    let version_str = String::from_utf8_lossy(&version_output.stdout);
    let version = parse_claudeye_version(&version_str)?;
    if !version_meets_minimum(version, MIN_CLAUDEYE_VERSION) {
        return None;
    }

    Command::new("claudeye")
        .arg("--crmux")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Run the TUI application.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let own_pid = std::process::id();

    // Start monitor polling
    let monitor_state = Arc::new(Mutex::new(MonitorState::default()));
    tmux_claude_state::monitor::start_polling(Arc::clone(&monitor_state));

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    // Enable Kitty keyboard protocol for Ctrl+Enter detection
    let keyboard_enhancement = supports_keyboard_enhancement().unwrap_or(false);
    if keyboard_enhancement {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }

    // Enable bracketed paste so pasted text arrives as Event::Paste
    execute!(stdout, EnableBracketedPaste)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app_state = Arc::new(Mutex::new(AppState::new(Some(own_pid))));

    let handler_state = Arc::clone(&app_state);
    let handler: crate::rpc::RequestHandler = Arc::new(move |method, _params| {
        if method == "get_sessions"
            && let Ok(state) = handler_state.lock()
        {
            return state.serialize_sessions();
        }
        serde_json::Value::Null
    });
    let rpc_server = crate::rpc::RpcServer::start(Some(handler)).ok();

    let mut claudeye_child: Option<std::process::Child> = None;

    let result = run_event_loop(
        &mut terminal,
        &monitor_state,
        &app_state,
        rpc_server.as_ref(),
        &mut claudeye_child,
    );

    // Shut down claudeye child process
    if let Some(ref mut child) = claudeye_child {
        let _ = child.kill();
        let _ = child.wait();
    }

    // Terminal cleanup
    execute!(terminal.backend_mut(), DisableBracketedPaste)?;
    if keyboard_enhancement {
        execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags)?;
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result?;
    Ok(())
}

fn run_event_loop<B: ratatui::backend::Backend<Error = io::Error>>(
    terminal: &mut Terminal<B>,
    monitor_state: &Arc<Mutex<MonitorState>>,
    app_state: &Arc<Mutex<AppState>>,
    rpc_server: Option<&crate::rpc::RpcServer>,
    claudeye_child: &mut Option<std::process::Child>,
) -> io::Result<()> {
    let mut last_branch_refresh = std::time::Instant::now()
        .checked_sub(Duration::from_secs(10))
        .unwrap_or_else(std::time::Instant::now);

    loop {
        {
            let mut state = app_state.lock().map_err(|e| {
                io::Error::other(e.to_string())
            })?;

            // Sync with monitor state
            if let Ok(monitor) = monitor_state.lock() {
                state.sync_with_monitor(&monitor);
            }

            // Refresh git branches and auto titles periodically (every 5 seconds)
            if last_branch_refresh.elapsed() >= Duration::from_secs(5) {
                state.refresh_git_branches();
                state.refresh_auto_titles();
                last_branch_refresh = std::time::Instant::now();
            }

            // Process RPC messages
            if let Some(server) = rpc_server {
                let mut received_rpc = false;
                while let Some(msg) = server.try_recv() {
                    state.handle_rpc_message(&msg);
                    received_rpc = true;
                }
                if received_rpc {
                    state.refresh_auto_titles();
                }
            }

            // Update preview contents
            let marked = state.marked_sessions();
            if marked.is_empty() {
                // No marked sessions: show the selected session
                if let Some(session) = state.selected_session() {
                    let content = if state.preview_scroll > 0 {
                        let scrollback_lines = state.preview_height.saturating_mul(3);
                        strip_osc8_hyperlinks(&capture_pane_with_scrollback(&session.pane_id, scrollback_lines))
                    } else {
                        strip_osc8_hyperlinks(&tmux_claude_state::tmux::capture_pane_with_ansi(&session.pane_id))
                    };
                    state.preview_contents = vec![PreviewEntry {
                        name: session.project_name.clone(),
                        pane_id: session.pane_id.clone(),
                        title: session.display_title().map(String::from),
                        content,
                    }];
                } else {
                    state.preview_contents.clear();
                }
            } else {
                // Show all marked sessions (scrollback only for focused pane)
                let selected_pane = state.selected_pane_id().map(String::from);
                let entries: Vec<PreviewEntry> = marked
                    .iter()
                    .map(|s| {
                        let is_focused =
                            selected_pane.as_deref() == Some(s.pane_id.as_str());
                        let content = if is_focused && state.preview_scroll > 0 {
                            let scrollback_lines =
                                state.preview_height.saturating_mul(3);
                            strip_osc8_hyperlinks(&capture_pane_with_scrollback(&s.pane_id, scrollback_lines))
                        } else {
                            strip_osc8_hyperlinks(&tmux_claude_state::tmux::capture_pane_with_ansi(&s.pane_id))
                        };
                        PreviewEntry {
                            name: s.project_name.clone(),
                            pane_id: s.pane_id.clone(),
                            title: s.display_title().map(String::from),
                            content,
                        }
                    })
                    .collect();
                state.preview_contents = entries;
            }

            // Draw TUI
            let frame = terminal.draw(|f| {
                ui::draw(
                    f,
                    &state.sessions,
                    state.selected_index,
                    &state.preview_contents,
                    state.input_mode,
                    &state.input_buffer,
                    state.show_help,
                    state.help_scroll,
                    state.preview_scroll,
                );
            })?;

            // Update preview_height from terminal size
            let total_preview_height = frame.area.height.saturating_sub(5);
            let preview_count = state.preview_contents.len();
            if preview_count > 1 {
                let available_width = frame.area.width.saturating_sub(30);
                let (_cols, rows) =
                    ui::compute_grid(preview_count, available_width, ui::MIN_PANE_WIDTH);
                #[allow(clippy::cast_possible_truncation)]
                {
                    state.preview_height =
                        total_preview_height / (rows.max(1) as u16);
                }
            } else {
                state.preview_height = total_preview_height;
            }
        } // lock released here before polling for events

        // Launch claudeye on first toggle to visible
        if claudeye_child.is_none() {
            if let Ok(s) = app_state.lock() {
                if s.claudeye_visible {
                    *claudeye_child = launch_claudeye();
                }
            }
        }

        // Wait for at least one event or timeout for periodic refresh
        if event::poll(Duration::from_millis(50))? {
            let mut state = app_state.lock().map_err(|e| {
                io::Error::other(e.to_string())
            })?;
            let ev = event::read()?;
            match event_handler::handle_key_event(&ev, &mut state) {
                Action::Quit => return Ok(()),
                Action::Continue => {}
            }
            // Drain all remaining pending events before next capture/draw cycle
            while event::poll(Duration::ZERO)? {
                let ev = event::read()?;
                match event_handler::handle_key_event(&ev, &mut state) {
                    Action::Quit => return Ok(()),
                    Action::Continue => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_claudeye_version() {
        assert_eq!(parse_claudeye_version("claudeye 0.3.0\n"), Some((0, 3, 0)));
        assert_eq!(parse_claudeye_version("claudeye 1.0.0\n"), Some((1, 0, 0)));
        assert_eq!(parse_claudeye_version("claudeye 0.12.3\n"), Some((0, 12, 3)));
        assert_eq!(parse_claudeye_version("invalid"), None);
        assert_eq!(parse_claudeye_version(""), None);
        assert_eq!(parse_claudeye_version("claudeye abc\n"), None);
    }

    #[test]
    fn test_strip_osc8_hyperlinks_basic() {
        let input = "\x1b]8;;file:///path/to/file.rb\x1b\\file.rb\x1b]8;;\x1b\\";
        assert_eq!(strip_osc8_hyperlinks(input), "file.rb");
    }

    #[test]
    fn test_strip_osc8_hyperlinks_in_context() {
        let input = "Update(\x1b]8;;file:///spec/test_spec.rb\x1b\\spec/test_spec.rb\x1b]8;;\x1b\\)";
        assert_eq!(
            strip_osc8_hyperlinks(input),
            "Update(spec/test_spec.rb)"
        );
    }

    #[test]
    fn test_strip_osc8_hyperlinks_no_links() {
        let input = "normal text with \x1b[31mcolor\x1b[0m";
        assert_eq!(strip_osc8_hyperlinks(input), input);
    }

    #[test]
    fn test_strip_osc8_hyperlinks_multiple() {
        let input = "\x1b]8;;url1\x1b\\a.rb\x1b]8;;\x1b\\ and \x1b]8;;url2\x1b\\b.rb\x1b]8;;\x1b\\";
        assert_eq!(strip_osc8_hyperlinks(input), "a.rb and b.rb");
    }

    #[test]
    fn test_strip_osc8_hyperlinks_multibyte() {
        let input = "日本語テキスト \x1b]8;;file:///path\x1b\\リンク\x1b]8;;\x1b\\ の表示";
        assert_eq!(strip_osc8_hyperlinks(input), "日本語テキスト リンク の表示");
    }

    #[test]
    fn test_strip_osc8_hyperlinks_bel_terminator() {
        // Some terminals use BEL (\x07) instead of ST (\x1b\\)
        let input = "\x1b]8;;file:///path\x07link text\x1b]8;;\x07";
        assert_eq!(strip_osc8_hyperlinks(input), "link text");
    }

    #[test]
    fn test_version_meets_minimum() {
        assert!(version_meets_minimum((0, 3, 0), (0, 3, 0)));
        assert!(version_meets_minimum((0, 4, 0), (0, 3, 0)));
        assert!(version_meets_minimum((1, 0, 0), (0, 3, 0)));
        assert!(!version_meets_minimum((0, 2, 0), (0, 3, 0)));
        assert!(!version_meets_minimum((0, 2, 9), (0, 3, 0)));
        assert!(version_meets_minimum((0, 3, 1), (0, 3, 0)));
    }
}
