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

/// Capture a tmux pane, strip OSC8 hyperlinks, and trim trailing blank lines.
fn capture_pane_content(pane_id: &str, scrollback_lines: Option<u16>) -> String {
    let raw = scrollback_lines.map_or_else(
        || tmux_claude_state::tmux::capture_pane_with_ansi(pane_id),
        |lines| capture_pane_with_scrollback(pane_id, lines),
    );
    strip_osc8_hyperlinks(&raw).trim_end().to_string()
}

/// Process SGR parameter string and determine whether reverse-video is toggled.
///
/// Parses semicolon-separated SGR params left-to-right:
/// - `0` or empty → reset (reverse OFF)
/// - `7` → reverse ON
/// - `27` → reverse OFF
/// - `38`/`48` → extended color; skip sub-parameters (`38;5;idx` or `38;2;r;g;b`)
///
/// Returns the final reverse state after processing all params.
fn sgr_updates_reverse(params: &str, mut in_reverse: bool) -> bool {
    if params.is_empty() {
        return false; // bare ESC[m is a reset
    }
    let parts: Vec<&str> = params.split(';').collect();
    let mut i = 0;
    while i < parts.len() {
        let code: u32 = parts[i].parse().unwrap_or(0);
        match code {
            7 => in_reverse = true,
            0 | 27 => in_reverse = false,
            38 | 48 => {
                // Extended color: skip sub-parameters
                if i + 1 < parts.len() {
                    let next: u32 = parts[i + 1].parse().unwrap_or(0);
                    match next {
                        2 => i += 4, // 38;2;r;g;b
                        5 => i += 2, // 38;5;idx
                        _ => i += 1,
                    }
                }
            }
            _ => {} // other SGR params don't affect reverse
        }
        i += 1;
    }
    in_reverse
}

/// Maximum number of lines from the bottom to scan for cursor detection.
const CURSOR_SCAN_LINES: usize = 10;

/// Detect cursor position from ANSI content.
///
/// Scans the bottom `max_scan_lines` lines. First tries reverse-video detection
/// (Claude Code renders cursor as `\x1b[7m \x1b[0m`), then falls back to
/// finding a `❯ ` prompt pattern and returning the text end position.
fn detect_cursor_position(content: &str, max_scan_lines: usize) -> Option<(u16, u16)> {
    let lines: Vec<&str> = content.split('\n').collect();
    let start = lines.len().saturating_sub(max_scan_lines);
    detect_cursor_by_reverse_video(&lines, start)
        .or_else(|| detect_cursor_by_prompt(&lines, start))
}

/// Detect cursor position by finding a reverse-video cell (bottom-up scan).
fn detect_cursor_by_reverse_video(lines: &[&str], start: usize) -> Option<(u16, u16)> {
    for (i, line) in lines[start..].iter().enumerate().rev() {
        let row = start + i;
        let mut col: u16 = 0;
        let mut chars = line.chars().peekable();
        let mut in_reverse = false;

        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                // Parse ESC sequence
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    let mut params = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_digit() || c == ';' {
                            params.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Consume intermediate bytes (0x20-0x2F, e.g. '?' in \x1b[?25h)
                    while let Some(&c) = chars.peek() {
                        if (0x20..=0x2F).contains(&(c as u32)) {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if let Some(&final_byte) = chars.peek() {
                        chars.next(); // consume final byte
                        if final_byte == 'm' {
                            in_reverse = sgr_updates_reverse(&params, in_reverse);
                        }
                    }
                } else if chars.peek() == Some(&']') {
                    // OSC sequence - skip until ST or BEL
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                continue;
            }

            if in_reverse {
                // Row count is bounded by terminal rows.
                #[allow(clippy::cast_possible_truncation)]
                return Some((row as u16, col));
            }

            let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            // Column position in a terminal row fits in u16.
            #[allow(clippy::cast_possible_truncation)]
            { col += width as u16; }
        }
    }
    None
}

/// Detect cursor position by finding a `❯ ` prompt pattern (bottom-up scan).
// Character width is at most 2, so usize→u16 truncation never occurs.
#[allow(clippy::cast_possible_truncation)]
fn detect_cursor_by_prompt(lines: &[&str], start: usize) -> Option<(u16, u16)> {
    for (i, line) in lines[start..].iter().enumerate().rev() {
        let row = start + i;
        let stripped = strip_ansi_for_prompt(line);
        if let Some(pos) = stripped.find("❯ ") {
            let after_prompt = &stripped[pos + "❯ ".len()..];
            let prompt_col: u16 = stripped[..pos]
                .chars()
                .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16)
                .sum();
            let text_width: u16 = after_prompt
                .chars()
                .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16)
                .sum();
            #[allow(clippy::cast_possible_truncation)]
            return Some((row as u16, prompt_col + 2 + text_width));
        }
    }
    None
}

/// Strip ANSI escape sequences for prompt pattern detection.
fn strip_ansi_for_prompt(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // Skip CSI parameters and final byte
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() || c == ';' {
                        chars.next();
                    } else {
                        break;
                    }
                }
                // Skip intermediate bytes
                while let Some(&c) = chars.peek() {
                    if (0x20..=0x2F).contains(&(c as u32)) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                // Skip final byte
                if chars.peek().is_some() {
                    chars.next();
                }
            } else if chars.peek() == Some(&']') {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }
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

    let app_state = {
        let mut state = AppState::new(Some(own_pid));
        if let Ok(cwd) = std::env::current_dir() {
            let cwd_str = cwd.to_string_lossy();
            let project_name = cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            state.load_historical_plans(&cwd_str, &project_name);
        }
        Arc::new(Mutex::new(state))
    };

    let handler_state = Arc::clone(&app_state);
    let handler: crate::rpc::RequestHandler = Arc::new(move |method, params| {
        let Ok(state) = handler_state.lock() else {
            return serde_json::Value::Null;
        };
        match method {
            "get_sessions" => state.serialize_sessions(params),
            "get_plans" => state.serialize_plans(params),
            _ => serde_json::Value::Null,
        }
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

// Main event loop: sync → draw → handle events. Splitting would scatter the sequential logic.
#[allow(clippy::too_many_lines)]
fn run_event_loop<B: ratatui::backend::Backend<Error = io::Error>>(
    terminal: &mut Terminal<B>,
    monitor_state: &Arc<Mutex<MonitorState>>,
    app_state: &Arc<Mutex<AppState>>,
    rpc_server: Option<&crate::rpc::RpcServer>,
    claudeye_child: &mut Option<std::process::Child>,
) -> io::Result<()> {
    let mut last_git_refresh = std::time::Instant::now()
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
            if last_git_refresh.elapsed() >= Duration::from_secs(5) {
                state.refresh_git_info();
                state.refresh_auto_titles();
                last_git_refresh = std::time::Instant::now();
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

            // Collect filtered sessions once for preview + draw
            let filtered: Vec<_> = state.filtered_sessions().into_iter().cloned().collect();

            // Update preview contents based on layout mode
            match state.layout_mode {
                crate::state::LayoutMode::Single => {
                    // Show the selected session only
                    if let Some(session) = state.selected_session() {
                        let content = if state.preview_scroll > 0 {
                            let scrollback_lines = state.preview_height.saturating_mul(3);
                            capture_pane_content(&session.pane_id, Some(scrollback_lines))
                        } else {
                            capture_pane_content(&session.pane_id, None)
                        };
                        let cursor_pos = detect_cursor_position(&content, CURSOR_SCAN_LINES);
                        state.preview_contents = vec![PreviewEntry {
                            name: session.project_name.clone(),
                            pane_id: session.pane_id.clone(),
                            index: state.selected_index,
                            title: session.display_title().map(String::from),
                            git_branch: session.git_branch.clone(),
                            worktree_name: session.worktree_name.clone(),
                            content,
                            cursor_pos,
                            git_diff: session.git_diff.clone(),
                        }];
                    } else {
                        state.preview_contents.clear();
                    }
                }
                crate::state::LayoutMode::Grid | crate::state::LayoutMode::EvenHorizontal | crate::state::LayoutMode::EvenVertical | crate::state::LayoutMode::MainVertical | crate::state::LayoutMode::MainHorizontal => {
                    // Show all filtered sessions in a grid (scrollback only for focused pane)
                    let selected_pane = state.selected_pane_id().map(String::from);
                    let entries: Vec<PreviewEntry> = filtered
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            let is_focused =
                                selected_pane.as_deref() == Some(s.pane_id.as_str());
                            let content = if is_focused && state.preview_scroll > 0 {
                                let scrollback_lines =
                                    state.preview_height.saturating_mul(3);
                                capture_pane_content(&s.pane_id, Some(scrollback_lines))
                            } else {
                                capture_pane_content(&s.pane_id, None)
                            };
                            let cursor_pos = detect_cursor_position(&content, CURSOR_SCAN_LINES);
                            PreviewEntry {
                                name: s.project_name.clone(),
                                pane_id: s.pane_id.clone(),
                                index: i,
                                title: s.display_title().map(String::from),
                                git_branch: s.git_branch.clone(),
                                worktree_name: s.worktree_name.clone(),
                                content,
                                cursor_pos,
                                git_diff: s.git_diff.clone(),
                            }
                        })
                        .collect();
                    // For MainVertical/MainHorizontal, put selected session first
                    if matches!(state.layout_mode, crate::state::LayoutMode::MainVertical | crate::state::LayoutMode::MainHorizontal) {
                        if let Some(sel_pane) = selected_pane.as_deref() {
                            let mut sorted = Vec::with_capacity(entries.len());
                            let mut rest = Vec::new();
                            for e in entries {
                                if e.pane_id == sel_pane && sorted.is_empty() {
                                    sorted.push(e);
                                } else {
                                    rest.push(e);
                                }
                            }
                            sorted.extend(rest);
                            state.preview_contents = sorted;
                        } else {
                            state.preview_contents = entries;
                        }
                    } else {
                        state.preview_contents = entries;
                    }
                }
            }

            // Draw TUI
            let frame = terminal.draw(|f| {
                ui::draw(
                    f,
                    &filtered,
                    state.selected_index,
                    &state.preview_contents,
                    state.input_mode,
                    &state.input_buffer,
                    state.show_help,
                    state.help_scroll,
                    state.preview_scroll,
                    &state.tab_state,
                    state.layout_mode,
                );
            })?;

            // Update preview_height from terminal size
            let total_preview_height = frame.area.height.saturating_sub(5);
            let preview_count = state.preview_contents.len();
            if preview_count > 1 {
                #[allow(clippy::cast_possible_truncation)]
                match state.layout_mode {
                    crate::state::LayoutMode::EvenHorizontal => {
                        // Horizontal split: all panes side by side, full height each
                        state.preview_height = total_preview_height;
                    }
                    crate::state::LayoutMode::EvenVertical => {
                        // Vertical split: stacked, height divided by n
                        state.preview_height =
                            total_preview_height / (preview_count.max(1) as u16);
                    }
                    crate::state::LayoutMode::MainVertical => {
                        // Main pane gets full height
                        state.preview_height = total_preview_height;
                    }
                    crate::state::LayoutMode::MainHorizontal => {
                        // Main pane gets 60% height
                        state.preview_height = total_preview_height * 60 / 100;
                    }
                    crate::state::LayoutMode::Single | crate::state::LayoutMode::Grid => {
                        // Grid layout
                        let available_width = frame.area.width.saturating_sub(30);
                        let (_cols, rows) =
                            ui::compute_grid(preview_count, available_width, ui::MIN_PANE_WIDTH);
                        state.preview_height =
                            total_preview_height / (rows.max(1) as u16);
                    }
                }
            } else {
                state.preview_height = total_preview_height;
            }
        } // lock released here before polling for events

        // Launch claudeye on first toggle to visible
        if claudeye_child.is_none()
            && let Ok(s) = app_state.lock()
            && s.claudeye_visible
        {
            *claudeye_child = launch_claudeye();
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
    fn test_detect_cursor_basic() {
        // reverse-video space at col 2 on row 0
        let input = "ab\x1b[7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 2)));
    }

    #[test]
    fn test_detect_cursor_multiline() {
        let input = "line1\n\x1b[39m❯  \x1b[7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((1, 3)));
    }

    #[test]
    fn test_detect_cursor_none() {
        let input = "no cursor here\njust text";
        assert_eq!(detect_cursor_position(input, usize::MAX), None);
    }

    #[test]
    fn test_detect_cursor_with_wide_chars() {
        // "日本" takes 4 columns, then reverse space at col 4
        let input = "日本\x1b[7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 4)));
    }

    #[test]
    fn test_detect_cursor_with_ansi_before() {
        // color codes don't add to column count
        let input = "\x1b[31mab\x1b[0m\x1b[7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 2)));
    }

    #[test]
    fn test_detect_cursor_returns_last_match() {
        // Two reverse-video cells: row 0 col 2 and row 2 col 3
        // Should return the last one (row 2, col 3)
        let input = "ab\x1b[7m \x1b[0mmore\nplain line\n❯  \x1b[7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((2, 3)));
    }

    #[test]
    fn test_detect_cursor_compound_sgr_bold_reverse() {
        // \x1b[1;7m = bold + reverse
        let input = "ab\x1b[1;7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 2)));
    }

    #[test]
    fn test_detect_cursor_compound_sgr_reverse_with_color() {
        // \x1b[7;38;5;245m = reverse + 256-color fg
        let input = "ab\x1b[7;38;5;245m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 2)));
    }

    #[test]
    fn test_detect_cursor_compound_sgr_truecolor() {
        // \x1b[7;38;2;100;200;50m = reverse + truecolor fg
        let input = "ab\x1b[7;38;2;100;200;50m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 2)));
    }

    #[test]
    fn test_detect_cursor_compound_reset() {
        // \x1b[0;39m = reset + default fg color → reverse OFF
        let input = "ab\x1b[7m \x1b[0;39m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 2)));
    }

    #[test]
    fn test_detect_cursor_reset_then_reverse() {
        // \x1b[0;7m = reset then reverse → reverse ON
        let input = "ab\x1b[0;7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 2)));
    }

    #[test]
    fn test_detect_cursor_reverse_then_reset() {
        // \x1b[7;0m = reverse then reset → reverse OFF, so no cursor
        let input = "\x1b[7;0mab";
        assert_eq!(detect_cursor_position(input, usize::MAX), None);
    }

    // --- max_scan_lines tests ---

    #[test]
    fn test_detect_cursor_scan_limit_within_range() {
        // 5 lines, reverse-video on last line (row 4), scan limit 3 → within range
        let input = "line0\nline1\nline2\nline3\nab\x1b[7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, 3), Some((4, 2)));
    }

    #[test]
    fn test_detect_cursor_scan_limit_outside_range() {
        // 5 lines, reverse-video on row 0 only, scan limit 3 → outside range, not found
        let input = "ab\x1b[7m \x1b[0m\nline1\nline2\nline3\nline4";
        assert_eq!(detect_cursor_position(input, 3), None);
    }

    #[test]
    fn test_detect_cursor_scan_limit_none_scans_all() {
        // reverse-video on row 0, no limit → found
        let input = "ab\x1b[7m \x1b[0m\nline1\nline2\nline3\nline4";
        assert_eq!(detect_cursor_position(input, usize::MAX), Some((0, 2)));
    }

    // --- prompt pattern fallback tests ---

    #[test]
    fn test_detect_cursor_prompt_fallback_with_text() {
        // No reverse-video, but has prompt pattern with text
        let input = "some output\n\x1b[39m❯ hello";
        assert_eq!(detect_cursor_position(input, 10), Some((1, 7)));
    }

    #[test]
    fn test_detect_cursor_prompt_fallback_empty() {
        // No reverse-video, empty prompt → cursor at col 2 (after "❯ ")
        let input = "some output\n❯ ";
        assert_eq!(detect_cursor_position(input, 10), Some((1, 2)));
    }

    #[test]
    fn test_detect_cursor_prompt_fallback_no_prompt() {
        // No reverse-video, no prompt pattern → None
        let input = "some output\njust text";
        assert_eq!(detect_cursor_position(input, 10), None);
    }

    #[test]
    fn test_detect_cursor_reverse_video_preferred_over_prompt() {
        // Both reverse-video and prompt pattern present → reverse-video wins
        let input = "❯ hello\nab\x1b[7m \x1b[0m";
        assert_eq!(detect_cursor_position(input, 10), Some((1, 2)));
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
