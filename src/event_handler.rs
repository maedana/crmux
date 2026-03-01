use crossterm::event::{Event, KeyCode, KeyModifiers};

use std::process::{Command, Stdio};

use crate::state::{AppState, InputMode};

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
    if let Event::Key(key) = *event {
        match state.input_mode {
            InputMode::Normal => handle_normal_mode(key.code, state),
            InputMode::Input => handle_input_mode(key.code, key.modifiers, state),
            InputMode::Title => handle_title_mode(key.code, key.modifiers, state),
        }
    } else {
        Action::Continue
    }
}

fn handle_normal_mode(code: KeyCode, state: &mut AppState) -> Action {
    match code {
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
        _ => Action::Continue,
    }
}

fn handle_input_mode(code: KeyCode, modifiers: KeyModifiers, state: &mut AppState) -> Action {
    match code {
        KeyCode::Char('o') if modifiers.contains(KeyModifiers::CONTROL) => {
            state.input_mode = InputMode::Normal;
            Action::Continue
        }
        // All other keys are forwarded to the tmux pane immediately
        _ => {
            send_key_to_pane(code, modifiers, state);
            Action::Continue
        }
    }
}

fn handle_title_mode(code: KeyCode, modifiers: KeyModifiers, state: &mut AppState) -> Action {
    match code {
        KeyCode::Char('o') if modifiers.contains(KeyModifiers::CONTROL) => {
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
}

/// Send a single key event to the selected tmux pane immediately.
fn send_key_to_pane(code: KeyCode, modifiers: KeyModifiers, state: &AppState) {
    let Some(pane_id) = state.selected_pane_id() else {
        return;
    };

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

/// Map a `KeyCode` to its tmux key name for special keys.
fn keycode_to_tmux_name(code: KeyCode) -> Option<&'static str> {
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

/// Run `tmux send-keys -t <pane_id> <extra_args>` and wait for completion.
fn run_send_keys(pane_id: &str, extra_args: &[&str]) {
    let _ = Command::new("tmux")
        .arg("send-keys")
        .arg("-t")
        .arg(pane_id)
        .args(extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
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

    fn make_key_event_with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
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
        });
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
    fn test_i_does_nothing_without_session() {
        let mut state = AppState::new(None);
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    // --- Input mode tests (passthrough) ---

    #[test]
    fn test_input_mode_ctrl_o_returns_to_normal() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        let action = handle_key_event(
            &make_key_event_with_modifiers(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &mut state,
        );
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_input_mode_esc_stays_in_input() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        let action = handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Input);
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
    fn test_title_ctrl_o_saves_and_exits() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Title;
        state.input_buffer = "new title".to_string();
        let action = handle_key_event(
            &make_key_event_with_modifiers(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &mut state,
        );
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.input_buffer.is_empty());
        assert_eq!(state.sessions[0].title, Some("new title".to_string()));
    }

    #[test]
    fn test_title_ctrl_o_empty_stores_none() {
        let mut state = make_state_with_session();
        state.sessions[0].title = Some("old".to_string());
        state.input_mode = InputMode::Title;
        state.input_buffer.clear();
        handle_key_event(
            &make_key_event_with_modifiers(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &mut state,
        );
        assert_eq!(state.sessions[0].title, None);
    }

    #[test]
    fn test_title_ctrl_o_whitespace_only_stores_none() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Title;
        state.input_buffer = "  \t  ".to_string();
        handle_key_event(
            &make_key_event_with_modifiers(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &mut state,
        );
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
}
