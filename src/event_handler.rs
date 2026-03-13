use crossterm::event::{Event, KeyCode, KeyModifiers};

#[cfg(not(test))]
use std::process::{Command, Stdio};

use crate::state::{AppState, InputMode};

/// Deactivate IME (fcitx5) so that Normal-mode keybindings work immediately.
/// Silently ignored when fcitx5 is not installed.
#[cfg(not(test))]
fn ime_off() {
    Command::new("fcitx5-remote").arg("-c").status().ok();
}

#[cfg(test)]
fn ime_off() {}

/// Action to take after handling a keyboard event.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Continue the event loop.
    Continue,
    /// Quit the application.
    Quit,
}

/// Handle a keyboard event and return the appropriate action.
pub fn handle_key_event(event: &Event, state: &mut AppState) -> Action {
    match event {
        Event::Key(key) => {
            if state.show_help {
                return handle_help_mode(key.code, key.modifiers, state);
            }
            match state.input_mode {
                InputMode::Normal => handle_normal_mode(key.code, key.modifiers, state),
                InputMode::Input => handle_input_mode(key.code, key.modifiers, state),
                InputMode::Title => handle_title_mode(key.code, key.modifiers, state),
                InputMode::Broadcast => handle_broadcast_mode(key.code, key.modifiers, state),
                InputMode::Scroll => handle_scroll_mode(key.code, key.modifiers, state),
            }
        }
        Event::Paste(text) => handle_paste_event(text, state),
        _ => Action::Continue,
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Next,
    Prev,
}

/// Switch to the next/prev tab, maintaining selection by PID when possible.
fn switch_tab(state: &mut AppState, dir: Direction) {
    let selected_pid = state.selected_session().map(|s| s.pid);
    match dir {
        Direction::Next => state.tab_state.select_next_tab(),
        Direction::Prev => state.tab_state.select_prev_tab(),
    }
    state.preview_scroll = 0;
    let filtered = state.filtered_sessions();
    state.selected_index = selected_pid
        .and_then(|pid| filtered.iter().position(|s| s.pid == pid))
        .unwrap_or(0);
}

/// Handle Ctrl+ key combinations in normal mode.
fn handle_normal_ctrl(code: KeyCode, state: &mut AppState) -> Action {
    match code {
        KeyCode::Char('u') => {
            let half = state.preview_height / 2;
            let max = state.preview_height.saturating_mul(3);
            state.scroll_preview_up(half, max);
            if state.preview_scroll > 0 {
                state.input_mode = InputMode::Scroll;
            }
            Action::Continue
        }
        KeyCode::Char('d') => {
            let half = state.preview_height / 2;
            state.scroll_preview_down(half);
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle Esc in normal mode: forward Esc Esc to panes if coming from Input/Broadcast.
fn handle_normal_esc(state: &mut AppState) {
    match state.esc_source_mode.take() {
        Some(InputMode::Input) => {
            if let Some(pane_id) = state.selected_pane_id() {
                run_send_keys(pane_id, &["Escape"]);
                run_send_keys(pane_id, &["Escape"]);
            }
            state.input_mode = InputMode::Input;
        }
        Some(InputMode::Broadcast) => {
            for pane_id in state.marked_pane_ids() {
                run_send_keys(&pane_id, &["Escape"]);
                run_send_keys(&pane_id, &["Escape"]);
            }
            state.input_mode = InputMode::Broadcast;
        }
        _ => {}
    }
}

// Flat key→action match is more readable than splitting into sub-functions.
#[allow(clippy::too_many_lines)]
fn handle_normal_mode(code: KeyCode, modifiers: KeyModifiers, state: &mut AppState) -> Action {
    // Clear esc_source_mode on any key except Esc
    if code != KeyCode::Esc {
        state.esc_source_mode = None;
    }

    // Handle pending 'g' for gg (scroll to top)
    if state.pending_g {
        state.pending_g = false;
        return match code {
            KeyCode::Char('g') => {
                let max = state.preview_height.saturating_mul(3);
                state.preview_scroll = max;
                if state.preview_scroll > 0 {
                    state.input_mode = InputMode::Scroll;
                }
                Action::Continue
            }
            _ => handle_normal_mode(code, modifiers, state),
        };
    }

    if modifiers.contains(KeyModifiers::CONTROL) {
        return handle_normal_ctrl(code, state);
    }
    match code {
        KeyCode::Esc => {
            handle_normal_esc(state);
            Action::Continue
        }
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => {
            state.select_next();
            Action::Continue
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.select_prev();
            Action::Continue
        }
        KeyCode::Char(' ') => {
            state.toggle_mark();
            Action::Continue
        }
        KeyCode::Char('i') => {
            if state.selected_pane_id().is_some() {
                state.input_mode = InputMode::Input;
                state.reset_preview_scroll();
            }
            Action::Continue
        }
        KeyCode::Char('I') => {
            if !state.marked_pane_ids().is_empty() {
                state.input_mode = InputMode::Broadcast;
                state.reset_preview_scroll();
            }
            Action::Continue
        }
        KeyCode::Char('e') => {
            if let Some(session) = state.selected_session() {
                state.input_buffer = session.title.clone().unwrap_or_default();
                state.input_mode = InputMode::Title;
            }
            Action::Continue
        }
        KeyCode::Char('s') => {
            if let Some(pane_id) = state.selected_pane_id() {
                tmux_claude_state::tmux::switch_to_pane(pane_id);
            }
            Action::Continue
        }
        KeyCode::Char('G') => {
            state.reset_preview_scroll();
            Action::Continue
        }
        KeyCode::Char('g') => {
            state.pending_g = true;
            Action::Continue
        }
        KeyCode::Char('v') => {
            state.cycle_layout_mode();
            Action::Continue
        }
        KeyCode::Char('o') => {
            state.claudeye_visible = !state.claudeye_visible;
            Action::Continue
        }
        KeyCode::Char('h') | KeyCode::Left => {
            switch_tab(state, Direction::Prev);
            Action::Continue
        }
        KeyCode::Char('l') | KeyCode::Right => {
            switch_tab(state, Direction::Next);
            Action::Continue
        }
        KeyCode::Char('?') => {
            state.show_help = true;
            Action::Continue
        }
        KeyCode::Char(c @ '1'..='9') => {
            let idx = (c as usize) - ('1' as usize);
            if idx < crate::state::MAX_NUMBER_KEYS && idx < state.filtered_sessions().len() {
                state.selected_index = idx;
                state.preview_scroll = 0;
            }
            Action::Continue
        }
        _ => Action::Continue,
    }
}

fn handle_input_mode(code: KeyCode, modifiers: KeyModifiers, state: &mut AppState) -> Action {
    if code == KeyCode::Esc {
        state.esc_source_mode = Some(InputMode::Input);
        state.input_mode = InputMode::Normal;
        ime_off();
    } else {
        // All other keys are forwarded to the tmux pane immediately
        send_key_to_pane(code, modifiers, state);
    }
    Action::Continue
}

fn handle_broadcast_mode(code: KeyCode, modifiers: KeyModifiers, state: &mut AppState) -> Action {
    if code == KeyCode::Esc {
        state.esc_source_mode = Some(InputMode::Broadcast);
        state.input_mode = InputMode::Normal;
        ime_off();
    } else {
        send_key_to_marked_panes(code, modifiers, state);
    }
    Action::Continue
}

// HELP_TEXT is a short static string; line count never exceeds u16::MAX.
#[allow(clippy::cast_possible_truncation)]
fn help_line_count() -> u16 {
    crate::ui::HELP_TEXT.lines().count() as u16
}

fn handle_help_mode(code: KeyCode, modifiers: KeyModifiers, state: &mut AppState) -> Action {
    if modifiers.contains(KeyModifiers::CONTROL) {
        return match code {
            KeyCode::Char('u') => {
                state.help_scroll = state.help_scroll.saturating_sub(10);
                Action::Continue
            }
            KeyCode::Char('d') => {
                let max = help_line_count();
                state.help_scroll = state.help_scroll.saturating_add(10).min(max);
                Action::Continue
            }
            _ => Action::Continue,
        };
    }
    match code {
        KeyCode::Char('?') | KeyCode::Esc => {
            state.show_help = false;
            state.help_scroll = 0;
            Action::Continue
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max = help_line_count();
            state.help_scroll = state.help_scroll.saturating_add(1).min(max);
            Action::Continue
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.help_scroll = state.help_scroll.saturating_sub(1);
            Action::Continue
        }
        KeyCode::Char('g') => {
            state.help_scroll = 0;
            Action::Continue
        }
        KeyCode::Char('G') => {
            state.help_scroll = help_line_count();
            Action::Continue
        }
        _ => Action::Continue,
    }
}

fn handle_scroll_mode(code: KeyCode, modifiers: KeyModifiers, state: &mut AppState) -> Action {
    // Handle pending 'g' for gg (scroll to top)
    if state.pending_g {
        state.pending_g = false;
        return match code {
            KeyCode::Char('g') => {
                let max = state.preview_height.saturating_mul(3);
                state.preview_scroll = max;
                Action::Continue
            }
            _ => handle_scroll_mode(code, modifiers, state),
        };
    }

    // Handle Ctrl+ combinations
    if modifiers.contains(KeyModifiers::CONTROL) {
        return match code {
            KeyCode::Char('u') => {
                let half = state.preview_height / 2;
                let max = state.preview_height.saturating_mul(3);
                state.scroll_preview_up(half, max);
                Action::Continue
            }
            KeyCode::Char('d') => {
                let half = state.preview_height / 2;
                state.scroll_preview_down(half);
                if state.preview_scroll == 0 {
                    state.input_mode = InputMode::Normal;
                }
                Action::Continue
            }
            _ => Action::Continue,
        };
    }

    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.scroll_preview_down(1);
            if state.preview_scroll == 0 {
                state.input_mode = InputMode::Normal;
            }
            Action::Continue
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let max = state.preview_height.saturating_mul(3);
            state.scroll_preview_up(1, max);
            Action::Continue
        }
        // Intentionally separate from the Esc arm below for readability:
        // G resets scroll and exits, Esc does the same but semantically different.
        #[allow(clippy::match_same_arms)]
        KeyCode::Char('G') => {
            state.reset_preview_scroll();
            state.input_mode = InputMode::Normal;
            Action::Continue
        }
        KeyCode::Char('g') => {
            state.pending_g = true;
            Action::Continue
        }
        KeyCode::Esc => {
            state.reset_preview_scroll();
            state.input_mode = InputMode::Normal;
            Action::Continue
        }
        KeyCode::Char('i') => {
            if state.selected_pane_id().is_some() {
                state.reset_preview_scroll();
                state.input_mode = InputMode::Input;
            }
            Action::Continue
        }
        KeyCode::Char('I') => {
            if !state.marked_pane_ids().is_empty() {
                state.reset_preview_scroll();
                state.input_mode = InputMode::Broadcast;
            }
            Action::Continue
        }
        _ => Action::Continue,
    }
}

fn handle_title_mode(code: KeyCode, _modifiers: KeyModifiers, state: &mut AppState) -> Action {
    match code {
        KeyCode::Esc => {
            save_title(state);
            Action::Continue
        }
        KeyCode::Backspace => {
            state.input_buffer.pop();
            Action::Continue
        }
        KeyCode::Char(c) => {
            state.input_buffer.push(c);
            Action::Continue
        }
        _ => Action::Continue,
    }
}

fn save_title(state: &mut AppState) {
    let trimmed = state.input_buffer.trim().to_string();
    let title = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    if let Some(session) = state.selected_session_mut() {
        session.title = title;
    }
    state.input_buffer.clear();
    state.input_mode = InputMode::Normal;
    ime_off();
}

/// Handle a paste event by forwarding pasted text to the appropriate tmux pane(s).
fn handle_paste_event(text: &str, state: &AppState) -> Action {
    if state.show_help {
        return Action::Continue;
    }
    match state.input_mode {
        InputMode::Input => {
            if let Some(pane_id) = state.selected_pane_id() {
                send_paste_to_panes(&[pane_id], text);
            }
        }
        InputMode::Broadcast => {
            let pane_ids = state.marked_pane_ids();
            let refs: Vec<&str> = pane_ids.iter().map(String::as_str).collect();
            send_paste_to_panes(&refs, text);
        }
        InputMode::Normal | InputMode::Title | InputMode::Scroll => {}
    }
    Action::Continue
}

/// Send pasted text to tmux pane(s) using set-buffer + paste-buffer -p.
pub fn send_paste_to_panes(pane_ids: &[&str], text: &str) {
    if pane_ids.is_empty() {
        return;
    }
    run_tmux(&["set-buffer", "-b", "crmux-paste", "--", text]);
    for pane_id in pane_ids {
        run_tmux(&["paste-buffer", "-b", "crmux-paste", "-t", pane_id, "-p"]);
    }
    run_tmux(&["delete-buffer", "-b", "crmux-paste"]);
}

/// Encode a key event into tmux send-keys arguments and send to the given pane.
fn send_encoded_key(pane_id: &str, code: KeyCode, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = code {
            let key_name = format!("C-{c}");
            run_send_keys(pane_id, &[&key_name]);
        }
        return;
    }

    match code {
        KeyCode::Char(c) => {
            let s = c.to_string();
            run_send_keys(pane_id, &["-l", &s]);
        }
        _ => {
            if let Some(key_name) = keycode_to_tmux_name(code) {
                run_send_keys(pane_id, &[key_name]);
            }
        }
    }
}

/// Send a single key event to the selected tmux pane immediately.
fn send_key_to_pane(code: KeyCode, modifiers: KeyModifiers, state: &AppState) {
    let Some(pane_id) = state.selected_pane_id() else {
        return;
    };
    send_encoded_key(pane_id, code, modifiers);
}

/// Send a single key event to all marked tmux panes.
fn send_key_to_marked_panes(code: KeyCode, modifiers: KeyModifiers, state: &AppState) {
    for pane_id in state.marked_pane_ids() {
        send_encoded_key(&pane_id, code, modifiers);
    }
}

/// Map a `KeyCode` to its tmux key name for special keys.
const fn keycode_to_tmux_name(code: KeyCode) -> Option<&'static str> {
    match code {
        KeyCode::Enter => Some("Enter"),
        KeyCode::Backspace => Some("BSpace"),
        KeyCode::Tab => Some("Tab"),
        KeyCode::Left => Some("Left"),
        KeyCode::Right => Some("Right"),
        KeyCode::Up => Some("Up"),
        KeyCode::Down => Some("Down"),
        KeyCode::Home => Some("Home"),
        KeyCode::End => Some("End"),
        KeyCode::PageUp => Some("PageUp"),
        KeyCode::PageDown => Some("PageDown"),
        KeyCode::BackTab => Some("BTab"),
        KeyCode::Delete => Some("DC"),
        KeyCode::Esc => Some("Escape"),
        KeyCode::Insert => Some("IC"),
        _ => None,
    }
}

/// Run a tmux command with the given arguments, suppressing all I/O.
#[cfg(not(test))]
pub fn run_tmux(args: &[&str]) {
    let _ = Command::new("tmux")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
}

/// No-op stub so that tests never send real tmux commands.
#[cfg(test)]
pub fn run_tmux(_args: &[&str]) {}

/// Run `tmux send-keys -t <pane_id> <extra_args>` and wait for completion.
fn run_send_keys(pane_id: &str, extra_args: &[&str]) {
    let mut args = vec!["send-keys", "-t", pane_id];
    args.extend(extra_args);
    run_tmux(&args);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::time::Instant;
    use tmux_claude_state::claude_state::ClaudeState;

    fn make_key_event(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn make_state_with_session() -> AppState {
        use crate::state::ManagedSession;
        let mut state = AppState::new(None);
        state.sessions.push(ManagedSession {
            pid: 100,
            pane_id: "%1".to_string(),
            project_name: "test-project".to_string(),
            state: ClaudeState::Idle,
            state_changed_at: Instant::now(),
            marked: false,
            title: None,
            session_id: None,
            model: None,
            context_percent: None,
            cwd: "/home/user/test-project".to_string(),
            git_branch: None,
            auto_title: None,
            permission_mode: tmux_claude_state::claude_state::PermissionMode::AskBeforeEdits,
            jsonl_mtime: None,
            has_worked: false,
            worktree_name: None,
            git_diff: None,
            tmux_session: String::new(),
        });
        state
    }

    fn make_state_with_sessions(n: u32) -> AppState {
        use crate::state::ManagedSession;
        let mut state = AppState::new(None);
        for i in 0..n {
            state.sessions.push(ManagedSession {
                pid: 100 + i,
                pane_id: format!("%{}", i + 1),
                project_name: format!("project-{}", i + 1),
                state: ClaudeState::Idle,
                state_changed_at: Instant::now(),
                marked: false,
                title: None,
                session_id: None,
                model: None,
                context_percent: None,
                cwd: format!("/home/user/project-{}", i + 1),
                git_branch: None,
                auto_title: None,
                permission_mode: tmux_claude_state::claude_state::PermissionMode::AskBeforeEdits,
                jsonl_mtime: None,
                has_worked: false,
                worktree_name: None,
                git_diff: None,
                tmux_session: String::new(),
            });
        }
        state
    }

    // --- Normal mode tests ---

    #[test]
    fn test_quit_on_q() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('q')), &mut state);
        assert_eq!(action, Action::Quit);
    }

    #[test]
    fn test_esc_continues_in_normal_mode() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(action, Action::Continue);
    }

    #[test]
    fn test_navigate_j() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('j')), &mut state);
        assert_eq!(action, Action::Continue);
    }

    #[test]
    fn test_navigate_k() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('k')), &mut state);
        assert_eq!(action, Action::Continue);
    }

    #[test]
    fn test_navigate_down_arrow() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Down), &mut state);
        assert_eq!(action, Action::Continue);
    }

    #[test]
    fn test_navigate_up_arrow() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Up), &mut state);
        assert_eq!(action, Action::Continue);
    }

    #[test]
    fn test_s_continues() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('s')), &mut state);
        assert_eq!(action, Action::Continue);
    }

    #[test]
    fn test_other_key_continues() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('x')), &mut state);
        assert_eq!(action, Action::Continue);
    }

    // --- Space key toggles mark ---

    #[test]
    fn test_space_toggles_mark() {
        let mut state = make_state_with_session();
        assert!(!state.sessions[0].marked);
        let action = handle_key_event(&make_key_event(KeyCode::Char(' ')), &mut state);
        assert_eq!(action, Action::Continue);
        assert!(state.sessions[0].marked);
    }

    #[test]
    fn test_space_unmarks_marked_session() {
        let mut state = make_state_with_session();
        state.sessions[0].marked = true;
        handle_key_event(&make_key_event(KeyCode::Char(' ')), &mut state);
        assert!(!state.sessions[0].marked);
    }

    #[test]
    fn test_space_on_empty_sessions() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char(' ')), &mut state);
        assert_eq!(action, Action::Continue);
    }

    // --- i key enters input mode ---

    #[test]
    fn test_i_enters_input_mode_with_session() {
        let mut state = make_state_with_session();
        let action = handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Input);
    }

    #[test]
    fn test_i_resets_scroll_on_enter_input_mode() {
        let mut state = make_state_with_session();
        state.preview_scroll = 42;
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert_eq!(state.preview_scroll, 0);
    }

    #[test]
    fn test_i_does_nothing_without_session() {
        let mut state = AppState::new(None);
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    // --- Input mode tests (passthrough) ---

    #[test]
    fn test_input_mode_esc_returns_to_normal() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        let action = handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_input_mode_q_does_not_quit() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        let action = handle_key_event(&make_key_event(KeyCode::Char('q')), &mut state);
        assert_eq!(action, Action::Continue);
        // Should stay in Input mode, not quit
        assert_eq!(state.input_mode, InputMode::Input);
    }

    #[test]
    fn test_input_mode_keys_do_not_modify_buffer() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        handle_key_event(&make_key_event(KeyCode::Char('h')), &mut state);
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        // Passthrough mode: keys are sent to tmux, not buffered
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_input_mode_enter_does_not_modify_buffer() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        let action = handle_key_event(&make_key_event(KeyCode::Enter), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Input);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_input_mode_special_keys_stay_in_input() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        for key in [KeyCode::Backspace, KeyCode::Tab, KeyCode::BackTab, KeyCode::Left, KeyCode::Right] {
            let action = handle_key_event(&make_key_event(key), &mut state);
            assert_eq!(action, Action::Continue);
            assert_eq!(state.input_mode, InputMode::Input);
        }
    }

    // --- e key enters title mode ---

    #[test]
    fn test_e_enters_title_mode() {
        let mut state = make_state_with_session();
        let action = handle_key_event(&make_key_event(KeyCode::Char('e')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Title);
    }

    #[test]
    fn test_e_does_nothing_without_session() {
        let mut state = AppState::new(None);
        handle_key_event(&make_key_event(KeyCode::Char('e')), &mut state);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_e_prefills_buffer() {
        let mut state = make_state_with_session();
        state.sessions[0].title = Some("existing title".to_string());
        handle_key_event(&make_key_event(KeyCode::Char('e')), &mut state);
        assert_eq!(state.input_buffer, "existing title");
    }

    #[test]
    fn test_e_clears_buffer_when_no_title() {
        let mut state = make_state_with_session();
        state.input_buffer = "leftover".to_string();
        handle_key_event(&make_key_event(KeyCode::Char('e')), &mut state);
        assert!(state.input_buffer.is_empty());
    }

    // --- Title mode tests ---

    #[test]
    fn test_title_esc_saves_and_exits() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Title;
        state.input_buffer = "new title".to_string();
        let action = handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.input_buffer.is_empty());
        assert_eq!(state.sessions[0].title, Some("new title".to_string()));
    }

    #[test]
    fn test_title_esc_empty_stores_none() {
        let mut state = make_state_with_session();
        state.sessions[0].title = Some("old".to_string());
        state.input_mode = InputMode::Title;
        state.input_buffer.clear();
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(state.sessions[0].title, None);
    }

    #[test]
    fn test_title_esc_whitespace_only_stores_none() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Title;
        state.input_buffer = "  \t  ".to_string();
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(state.sessions[0].title, None);
    }

    #[test]
    fn test_title_enter_stays_in_title_mode() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Title;
        state.input_buffer = "abc".to_string();
        let action = handle_key_event(&make_key_event(KeyCode::Enter), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Title);
    }

    #[test]
    fn test_title_char_appended() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Title;
        handle_key_event(&make_key_event(KeyCode::Char('a')), &mut state);
        handle_key_event(&make_key_event(KeyCode::Char('b')), &mut state);
        assert_eq!(state.input_buffer, "ab");
    }

    #[test]
    fn test_title_backspace() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Title;
        state.input_buffer = "abc".to_string();
        handle_key_event(&make_key_event(KeyCode::Backspace), &mut state);
        assert_eq!(state.input_buffer, "ab");
    }

    // --- Help popup tests ---

    #[test]
    fn test_question_mark_opens_help() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('?')), &mut state);
        assert_eq!(action, Action::Continue);
        assert!(state.show_help);
    }

    #[test]
    fn test_question_mark_closes_help() {
        let mut state = AppState::new(None);
        state.show_help = true;
        let action = handle_key_event(&make_key_event(KeyCode::Char('?')), &mut state);
        assert_eq!(action, Action::Continue);
        assert!(!state.show_help);
    }

    #[test]
    fn test_esc_closes_help() {
        let mut state = AppState::new(None);
        state.show_help = true;
        let action = handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(action, Action::Continue);
        assert!(!state.show_help);
    }

    #[test]
    fn test_q_does_not_quit_during_help() {
        let mut state = AppState::new(None);
        state.show_help = true;
        let action = handle_key_event(&make_key_event(KeyCode::Char('q')), &mut state);
        assert_eq!(action, Action::Continue);
        assert!(state.show_help);
    }

    #[test]
    fn test_help_j_scrolls_down() {
        let mut state = AppState::new(None);
        state.show_help = true;
        handle_key_event(&make_key_event(KeyCode::Char('j')), &mut state);
        assert_eq!(state.help_scroll, 1);
        assert!(state.show_help);
    }

    #[test]
    fn test_help_k_scrolls_up() {
        let mut state = AppState::new(None);
        state.show_help = true;
        state.help_scroll = 5;
        handle_key_event(&make_key_event(KeyCode::Char('k')), &mut state);
        assert_eq!(state.help_scroll, 4);
    }

    #[test]
    fn test_help_k_clamps_to_zero() {
        let mut state = AppState::new(None);
        state.show_help = true;
        state.help_scroll = 0;
        handle_key_event(&make_key_event(KeyCode::Char('k')), &mut state);
        assert_eq!(state.help_scroll, 0);
    }

    #[test]
    fn test_help_esc_resets_scroll() {
        let mut state = AppState::new(None);
        state.show_help = true;
        state.help_scroll = 10;
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert!(!state.show_help);
        assert_eq!(state.help_scroll, 0);
    }

    #[test]
    fn test_help_shift_g_scrolls_to_bottom() {
        let mut state = AppState::new(None);
        state.show_help = true;
        handle_key_event(&make_key_event(KeyCode::Char('G')), &mut state);
        assert!(state.help_scroll > 0);
        assert!(state.show_help);
    }

    #[test]
    fn test_help_g_scrolls_to_top() {
        let mut state = AppState::new(None);
        state.show_help = true;
        state.help_scroll = 20;
        handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        assert_eq!(state.help_scroll, 0);
    }

    #[test]
    fn test_help_ctrl_d_scrolls_down() {
        let mut state = AppState::new(None);
        state.show_help = true;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('d')), &mut state);
        assert_eq!(state.help_scroll, 10);
    }

    #[test]
    fn test_help_ctrl_u_scrolls_up() {
        let mut state = AppState::new(None);
        state.show_help = true;
        state.help_scroll = 15;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('u')), &mut state);
        assert_eq!(state.help_scroll, 5);
    }

    // --- Broadcast mode tests ---

    fn make_state_with_marked_sessions() -> AppState {
        use crate::state::ManagedSession;
        let mut state = AppState::new(None);
        state.sessions.push(ManagedSession {
            pid: 100,
            pane_id: "%1".to_string(),
            project_name: "project-a".to_string(),
            state: ClaudeState::Idle,
            state_changed_at: Instant::now(),
            marked: true,
            title: None,
            session_id: None,
            model: None,
            context_percent: None,
            cwd: "/home/user/project-a".to_string(),
            git_branch: None,
            auto_title: None,
            permission_mode: tmux_claude_state::claude_state::PermissionMode::AskBeforeEdits,
            jsonl_mtime: None,
            has_worked: false,
            worktree_name: None,
            git_diff: None,
            tmux_session: String::new(),
        });
        state.sessions.push(ManagedSession {
            pid: 200,
            pane_id: "%2".to_string(),
            project_name: "project-b".to_string(),
            state: ClaudeState::Idle,
            state_changed_at: Instant::now(),
            marked: false,
            title: None,
            session_id: None,
            model: None,
            context_percent: None,
            cwd: "/home/user/project-b".to_string(),
            git_branch: None,
            auto_title: None,
            permission_mode: tmux_claude_state::claude_state::PermissionMode::AskBeforeEdits,
            jsonl_mtime: None,
            has_worked: false,
            worktree_name: None,
            git_diff: None,
            tmux_session: String::new(),
        });
        state.sessions.push(ManagedSession {
            pid: 300,
            pane_id: "%3".to_string(),
            project_name: "project-c".to_string(),
            state: ClaudeState::Idle,
            state_changed_at: Instant::now(),
            marked: true,
            title: None,
            session_id: None,
            model: None,
            context_percent: None,
            cwd: "/home/user/project-c".to_string(),
            git_branch: None,
            auto_title: None,
            permission_mode: tmux_claude_state::claude_state::PermissionMode::AskBeforeEdits,
            jsonl_mtime: None,
            has_worked: false,
            worktree_name: None,
            git_diff: None,
            tmux_session: String::new(),
        });
        state
    }

    #[test]
    fn test_shift_i_enters_broadcast_mode_with_marked_sessions() {
        let mut state = make_state_with_marked_sessions();
        let action = handle_key_event(&make_key_event(KeyCode::Char('I')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Broadcast);
    }

    #[test]
    fn test_shift_i_resets_scroll_on_enter_broadcast_mode() {
        let mut state = make_state_with_marked_sessions();
        state.preview_scroll = 42;
        handle_key_event(&make_key_event(KeyCode::Char('I')), &mut state);
        assert_eq!(state.preview_scroll, 0);
    }

    #[test]
    fn test_shift_i_does_nothing_without_marked_sessions() {
        let mut state = make_state_with_session();
        // session exists but none marked
        let action = handle_key_event(&make_key_event(KeyCode::Char('I')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_shift_i_does_nothing_without_sessions() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('I')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_broadcast_mode_esc_returns_to_normal() {
        let mut state = make_state_with_marked_sessions();
        state.input_mode = InputMode::Broadcast;
        let action = handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_broadcast_mode_q_does_not_quit() {
        let mut state = make_state_with_marked_sessions();
        state.input_mode = InputMode::Broadcast;
        let action = handle_key_event(&make_key_event(KeyCode::Char('q')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Broadcast);
    }

    #[test]
    fn test_broadcast_mode_keys_do_not_modify_buffer() {
        let mut state = make_state_with_marked_sessions();
        state.input_mode = InputMode::Broadcast;
        handle_key_event(&make_key_event(KeyCode::Char('h')), &mut state);
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert!(state.input_buffer.is_empty());
    }

    // --- Scroll key tests ---

    fn make_ctrl_key_event(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    #[test]
    fn test_ctrl_u_scrolls_up() {
        let mut state = make_state_with_session();
        state.preview_height = 30;
        let action = handle_key_event(&make_ctrl_key_event(KeyCode::Char('u')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.preview_scroll, 15); // half of preview_height
    }

    #[test]
    fn test_ctrl_d_scrolls_down() {
        let mut state = make_state_with_session();
        state.preview_height = 30;
        state.preview_scroll = 20;
        let action = handle_key_event(&make_ctrl_key_event(KeyCode::Char('d')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.preview_scroll, 5); // 20 - 15
    }

    #[test]
    fn test_shift_g_resets_scroll() {
        let mut state = make_state_with_session();
        state.preview_scroll = 42;
        let action = handle_key_event(&make_key_event(KeyCode::Char('G')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.preview_scroll, 0);
    }

    #[test]
    fn test_ctrl_u_clamps_to_max() {
        let mut state = make_state_with_session();
        state.preview_height = 30;
        // Scroll up repeatedly; should clamp to preview_height * 3
        for _ in 0..10 {
            handle_key_event(&make_ctrl_key_event(KeyCode::Char('u')), &mut state);
        }
        assert_eq!(state.preview_scroll, 90); // preview_height * 3
    }

    #[test]
    fn test_ctrl_d_clamps_to_zero() {
        let mut state = make_state_with_session();
        state.preview_height = 30;
        state.preview_scroll = 5;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('d')), &mut state);
        assert_eq!(state.preview_scroll, 0); // 5 - 15 = clamped to 0
    }

    // --- gg (scroll to top) tests ---

    #[test]
    fn test_g_sets_pending_g() {
        let mut state = make_state_with_session();
        let action = handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        assert_eq!(action, Action::Continue);
        assert!(state.pending_g);
    }

    #[test]
    fn test_gg_scrolls_to_top() {
        let mut state = make_state_with_session();
        state.preview_height = 30;
        // First g
        handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        // Second g → scroll to top
        let action = handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.preview_scroll, 90); // preview_height * 3
        assert!(!state.pending_g);
    }

    #[test]
    fn test_g_then_other_key_cancels_pending() {
        let mut state = make_state_with_session();
        // First g
        handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        assert!(state.pending_g);
        // Different key → cancel pending_g and handle normally
        let action = handle_key_event(&make_key_event(KeyCode::Char('j')), &mut state);
        assert_eq!(action, Action::Continue);
        assert!(!state.pending_g);
    }

    // --- Paste event tests ---

    fn make_paste_event(text: &str) -> Event {
        Event::Paste(text.to_string())
    }

    #[test]
    fn test_paste_ignored_in_normal_mode() {
        let mut state = make_state_with_session();
        let action = handle_key_event(&make_paste_event("hello\nworld"), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_paste_ignored_in_title_mode() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Title;
        state.input_buffer = "existing".to_string();
        let action = handle_key_event(&make_paste_event("pasted text"), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Title);
        assert_eq!(state.input_buffer, "existing");
    }

    #[test]
    fn test_paste_ignored_during_help() {
        let mut state = make_state_with_session();
        state.show_help = true;
        let action = handle_key_event(&make_paste_event("pasted text"), &mut state);
        assert_eq!(action, Action::Continue);
        assert!(state.show_help);
    }

    #[test]
    fn test_paste_in_input_mode_does_not_change_state() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        let action = handle_key_event(&make_paste_event("hello\nworld"), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Input);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_paste_in_broadcast_mode_does_not_change_state() {
        let mut state = make_state_with_marked_sessions();
        state.input_mode = InputMode::Broadcast;
        let action = handle_key_event(&make_paste_event("hello\nworld"), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Broadcast);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_paste_ignored_in_scroll_mode() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        let action = handle_key_event(&make_paste_event("hello"), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    // --- Scroll mode entry tests ---

    #[test]
    fn test_ctrl_u_enters_scroll_mode() {
        let mut state = make_state_with_session();
        state.preview_height = 30;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('u')), &mut state);
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    #[test]
    fn test_gg_enters_scroll_mode() {
        let mut state = make_state_with_session();
        state.preview_height = 30;
        handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    #[test]
    fn test_ctrl_d_does_not_enter_scroll_mode_at_bottom() {
        let mut state = make_state_with_session();
        state.preview_height = 30;
        state.preview_scroll = 0;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('d')), &mut state);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    // --- Scroll mode tests ---

    #[test]
    fn test_scroll_mode_j_scrolls_down() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        handle_key_event(&make_key_event(KeyCode::Char('j')), &mut state);
        assert_eq!(state.preview_scroll, 9);
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    #[test]
    fn test_scroll_mode_k_scrolls_up() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        state.preview_height = 30;
        handle_key_event(&make_key_event(KeyCode::Char('k')), &mut state);
        assert_eq!(state.preview_scroll, 11);
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    #[test]
    fn test_scroll_mode_down_arrow_scrolls_down() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 5;
        handle_key_event(&make_key_event(KeyCode::Down), &mut state);
        assert_eq!(state.preview_scroll, 4);
    }

    #[test]
    fn test_scroll_mode_up_arrow_scrolls_up() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 5;
        state.preview_height = 30;
        handle_key_event(&make_key_event(KeyCode::Up), &mut state);
        assert_eq!(state.preview_scroll, 6);
    }

    #[test]
    fn test_scroll_mode_j_to_zero_exits_scroll() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 1;
        handle_key_event(&make_key_event(KeyCode::Char('j')), &mut state);
        assert_eq!(state.preview_scroll, 0);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_scroll_mode_ctrl_u_scrolls_up() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        state.preview_height = 30;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('u')), &mut state);
        assert_eq!(state.preview_scroll, 25); // 10 + 15
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    #[test]
    fn test_scroll_mode_ctrl_d_scrolls_down() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 20;
        state.preview_height = 30;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('d')), &mut state);
        assert_eq!(state.preview_scroll, 5); // 20 - 15
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    #[test]
    fn test_scroll_mode_ctrl_d_to_zero_exits_scroll() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 5;
        state.preview_height = 30;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('d')), &mut state);
        assert_eq!(state.preview_scroll, 0);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_scroll_mode_shift_g_exits_scroll() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 42;
        handle_key_event(&make_key_event(KeyCode::Char('G')), &mut state);
        assert_eq!(state.preview_scroll, 0);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_scroll_mode_esc_resets_and_exits() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 42;
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(state.preview_scroll, 0);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_scroll_mode_i_enters_input_mode() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert_eq!(state.input_mode, InputMode::Input);
        assert_eq!(state.preview_scroll, 0);
    }

    #[test]
    fn test_scroll_mode_shift_i_enters_broadcast_mode() {
        let mut state = make_state_with_marked_sessions();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        handle_key_event(&make_key_event(KeyCode::Char('I')), &mut state);
        assert_eq!(state.input_mode, InputMode::Broadcast);
        assert_eq!(state.preview_scroll, 0);
    }

    #[test]
    fn test_scroll_mode_shift_i_does_nothing_without_marks() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        handle_key_event(&make_key_event(KeyCode::Char('I')), &mut state);
        assert_eq!(state.input_mode, InputMode::Scroll);
        assert_eq!(state.preview_scroll, 10);
    }

    #[test]
    fn test_scroll_mode_gg_scrolls_to_top() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_height = 30;
        state.preview_scroll = 10;
        handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        assert_eq!(state.preview_scroll, 90); // preview_height * 3
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    #[test]
    fn test_scroll_mode_q_does_not_quit() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        let action = handle_key_event(&make_key_event(KeyCode::Char('q')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    #[test]
    fn test_scroll_mode_g_then_other_key_cancels_pending() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        handle_key_event(&make_key_event(KeyCode::Char('g')), &mut state);
        assert!(state.pending_g);
        handle_key_event(&make_key_event(KeyCode::Char('j')), &mut state);
        assert!(!state.pending_g);
        assert_eq!(state.preview_scroll, 9);
    }

    #[test]
    fn test_scroll_mode_i_does_nothing_without_session() {
        let mut state = AppState::new(None);
        state.input_mode = InputMode::Scroll;
        state.preview_scroll = 10;
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert_eq!(state.input_mode, InputMode::Scroll);
    }

    // --- Esc Esc cancel forwarding tests ---

    #[test]
    fn test_input_mode_esc_sets_esc_source_mode() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.esc_source_mode, Some(InputMode::Input));
    }

    #[test]
    fn test_broadcast_mode_esc_sets_esc_source_mode() {
        let mut state = make_state_with_marked_sessions();
        state.input_mode = InputMode::Broadcast;
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.esc_source_mode, Some(InputMode::Broadcast));
    }

    #[test]
    fn test_normal_esc_with_input_source_returns_to_input() {
        let mut state = make_state_with_session();
        state.esc_source_mode = Some(InputMode::Input);
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(state.input_mode, InputMode::Input);
        assert_eq!(state.esc_source_mode, None);
    }

    #[test]
    fn test_normal_esc_with_broadcast_source_returns_to_broadcast() {
        let mut state = make_state_with_marked_sessions();
        state.esc_source_mode = Some(InputMode::Broadcast);
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(state.input_mode, InputMode::Broadcast);
        assert_eq!(state.esc_source_mode, None);
    }

    #[test]
    fn test_normal_esc_without_source_does_nothing() {
        let mut state = make_state_with_session();
        state.esc_source_mode = None;
        let action = handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert_eq!(state.esc_source_mode, None);
    }

    #[test]
    fn test_normal_other_key_clears_esc_source_mode() {
        let mut state = make_state_with_session();
        state.esc_source_mode = Some(InputMode::Input);
        handle_key_event(&make_key_event(KeyCode::Char('j')), &mut state);
        assert_eq!(state.esc_source_mode, None);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_esc_source_mode_cleared_on_ctrl_key() {
        let mut state = make_state_with_session();
        state.esc_source_mode = Some(InputMode::Input);
        state.preview_height = 30;
        handle_key_event(&make_ctrl_key_event(KeyCode::Char('u')), &mut state);
        assert_eq!(state.esc_source_mode, None);
    }

    // --- v key toggles claudeye_visible ---

    #[test]
    // --- o key toggles claudeye visible ---
    fn test_o_toggles_claudeye_visible() {
        let mut state = AppState::new(None);
        assert!(!state.claudeye_visible);
        handle_key_event(&make_key_event(KeyCode::Char('o')), &mut state);
        assert!(state.claudeye_visible);
        handle_key_event(&make_key_event(KeyCode::Char('o')), &mut state);
        assert!(!state.claudeye_visible);
    }

    // --- Tab switching tests ---

    fn make_state_with_tabs() -> AppState {
        use crate::state::ManagedSession;
        let mut state = AppState::new(None);
        for (pid, pane, project) in [(100, "%1", "aegis"), (200, "%2", "crmux"), (300, "%3", "crmux")] {
            state.sessions.push(ManagedSession {
                pid,
                pane_id: pane.to_string(),
                project_name: project.to_string(),
                state: ClaudeState::Idle,
                state_changed_at: Instant::now(),
                marked: false,
                title: None,
                session_id: None,
                model: None,
                context_percent: None,
                cwd: format!("/home/user/{project}"),
                git_branch: None,
                auto_title: None,
                permission_mode: tmux_claude_state::claude_state::PermissionMode::AskBeforeEdits,
                jsonl_mtime: None,
                has_worked: false,
            worktree_name: None,
            git_diff: None,
            tmux_session: String::new(),
            });
        }
        state.tab_state.rebuild_tabs(&state.sessions, None);
        state
    }

    #[test]
    fn test_l_moves_to_next_tab() {
        let mut state = make_state_with_tabs();
        assert_eq!(*state.tab_state.current_tab(), crate::state::Tab::All);
        handle_key_event(&make_key_event(KeyCode::Char('l')), &mut state);
        assert_eq!(*state.tab_state.current_tab(), crate::state::Tab::Project("aegis".to_string()));
    }

    #[test]
    fn test_h_moves_to_prev_tab() {
        let mut state = make_state_with_tabs();
        // Move to last tab first
        state.tab_state.selected_tab = 2; // crmux
        handle_key_event(&make_key_event(KeyCode::Char('h')), &mut state);
        assert_eq!(*state.tab_state.current_tab(), crate::state::Tab::Project("aegis".to_string()));
    }

    #[test]
    fn test_tab_switch_wraps_around() {
        let mut state = make_state_with_tabs();
        // tabs: All, aegis, crmux (3 tabs)
        state.tab_state.selected_tab = 2; // crmux (last)
        handle_key_event(&make_key_event(KeyCode::Char('l')), &mut state);
        assert_eq!(*state.tab_state.current_tab(), crate::state::Tab::All); // wraps to first
    }

    #[test]
    fn test_tab_switch_resets_preview_scroll() {
        let mut state = make_state_with_tabs();
        state.preview_scroll = 42;
        handle_key_event(&make_key_event(KeyCode::Char('l')), &mut state);
        assert_eq!(state.preview_scroll, 0);
    }

    #[test]
    fn test_input_mode_h_does_not_switch_tab() {
        let mut state = make_state_with_tabs();
        state.input_mode = InputMode::Input;
        let tab_before = state.tab_state.selected_tab;
        handle_key_event(&make_key_event(KeyCode::Char('h')), &mut state);
        assert_eq!(state.tab_state.selected_tab, tab_before);
    }

    #[test]
    fn test_broadcast_mode_l_does_not_switch_tab() {
        let mut state = make_state_with_tabs();
        state.input_mode = InputMode::Broadcast;
        let tab_before = state.tab_state.selected_tab;
        handle_key_event(&make_key_event(KeyCode::Char('l')), &mut state);
        assert_eq!(state.tab_state.selected_tab, tab_before);
    }

    #[test]
    fn test_title_mode_h_does_not_switch_tab() {
        let mut state = make_state_with_tabs();
        state.input_mode = InputMode::Title;
        let tab_before = state.tab_state.selected_tab;
        handle_key_event(&make_key_event(KeyCode::Char('h')), &mut state);
        assert_eq!(state.tab_state.selected_tab, tab_before);
    }

    #[test]
    fn test_right_arrow_moves_tab_in_normal_mode() {
        let mut state = make_state_with_tabs();
        handle_key_event(&make_key_event(KeyCode::Right), &mut state);
        assert_eq!(*state.tab_state.current_tab(), crate::state::Tab::Project("aegis".to_string()));
    }

    #[test]
    fn test_left_arrow_moves_tab_in_normal_mode() {
        let mut state = make_state_with_tabs();
        handle_key_event(&make_key_event(KeyCode::Left), &mut state);
        // wraps to last
        assert_eq!(*state.tab_state.current_tab(), crate::state::Tab::Project("crmux".to_string()));
    }

    // --- v key cycles layout mode ---

    #[test]
    fn test_v_cycles_layout_mode() {
        use crate::state::LayoutMode;
        let mut state = make_state_with_session();
        assert_eq!(state.layout_mode, LayoutMode::MainVertical);
        let action = handle_key_event(&make_key_event(KeyCode::Char('v')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.layout_mode, LayoutMode::Single);
        let action = handle_key_event(&make_key_event(KeyCode::Char('v')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.layout_mode, LayoutMode::Grid);
        let action = handle_key_event(&make_key_event(KeyCode::Char('v')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.layout_mode, LayoutMode::EvenHorizontal);
        let action = handle_key_event(&make_key_event(KeyCode::Char('v')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.layout_mode, LayoutMode::EvenVertical);
        let action = handle_key_event(&make_key_event(KeyCode::Char('v')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.layout_mode, LayoutMode::MainHorizontal);
        let action = handle_key_event(&make_key_event(KeyCode::Char('v')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.layout_mode, LayoutMode::MainVertical);
    }

    // --- number keys select session ---

    #[test]
    fn test_number_1_selects_first_session() {
        let mut state = make_state_with_sessions(3);
        state.selected_index = 2;
        let action = handle_key_event(&make_key_event(KeyCode::Char('1')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_number_3_selects_third_session() {
        let mut state = make_state_with_sessions(5);
        state.selected_index = 0;
        handle_key_event(&make_key_event(KeyCode::Char('3')), &mut state);
        assert_eq!(state.selected_index, 2);
    }

    #[test]
    fn test_number_beyond_session_count_is_noop() {
        let mut state = make_state_with_sessions(2);
        state.selected_index = 0;
        handle_key_event(&make_key_event(KeyCode::Char('5')), &mut state);
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_number_on_empty_sessions_is_noop() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('1')), &mut state);
        assert_eq!(action, Action::Continue);
    }

}
