use crossterm::event::{Event, KeyCode, KeyModifiers};

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
        }
    } else {
        Action::Continue
    }
}

fn handle_normal_mode(code: KeyCode, state: &mut AppState) -> Action {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => {
            state.select_next();
            Action::Continue
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.select_prev();
            Action::Continue
        }
        KeyCode::Char('i') => {
            if state.selected_pane_id().is_some() {
                state.input_mode = InputMode::Input;
                state.input_buffer.clear();
            }
            Action::Continue
        }
        KeyCode::Enter => {
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
        KeyCode::Esc => {
            state.input_buffer.clear();
            state.input_mode = InputMode::Normal;
            Action::Continue
        }
        // Ctrl+Enter (requires Kitty keyboard protocol) or Ctrl+D (universal fallback)
        KeyCode::Enter | KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            send_keys_to_selected_pane(state);
            state.input_buffer.clear();
            state.input_mode = InputMode::Normal;
            Action::Continue
        }
        KeyCode::Enter => {
            state.input_buffer.push('\n');
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

fn send_keys_to_selected_pane(state: &AppState) {
    if let Some(pane_id) = state.selected_pane_id() {
        let text = &state.input_buffer;
        if text.is_empty() {
            return;
        }
        // Send the text literally
        let _ = std::process::Command::new("tmux")
            .args(["send-keys", "-t", pane_id, "-l", text])
            .output();
        // Send Enter to execute
        let _ = std::process::Command::new("tmux")
            .args(["send-keys", "-t", pane_id, "Enter"])
            .output();
    }
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
    fn test_quit_on_esc() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(action, Action::Quit);
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
    fn test_enter_continues() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Enter), &mut state);
        assert_eq!(action, Action::Continue);
    }

    #[test]
    fn test_other_key_continues() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('x')), &mut state);
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

    #[test]
    fn test_i_clears_buffer_on_enter() {
        let mut state = make_state_with_session();
        state.input_buffer = "leftover".to_string();
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert!(state.input_buffer.is_empty());
    }

    // --- Input mode tests ---

    #[test]
    fn test_input_mode_char_appended_to_buffer() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        handle_key_event(&make_key_event(KeyCode::Char('h')), &mut state);
        handle_key_event(&make_key_event(KeyCode::Char('i')), &mut state);
        assert_eq!(state.input_buffer, "hi");
    }

    #[test]
    fn test_input_mode_enter_adds_newline() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        handle_key_event(&make_key_event(KeyCode::Char('a')), &mut state);
        handle_key_event(&make_key_event(KeyCode::Enter), &mut state);
        handle_key_event(&make_key_event(KeyCode::Char('b')), &mut state);
        assert_eq!(state.input_buffer, "a\nb");
    }

    #[test]
    fn test_input_mode_backspace_removes_char() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        state.input_buffer = "abc".to_string();
        handle_key_event(&make_key_event(KeyCode::Backspace), &mut state);
        assert_eq!(state.input_buffer, "ab");
    }

    #[test]
    fn test_input_mode_backspace_on_empty() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        handle_key_event(&make_key_event(KeyCode::Backspace), &mut state);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_input_mode_esc_cancels() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        state.input_buffer = "some text".to_string();
        handle_key_event(&make_key_event(KeyCode::Esc), &mut state);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_input_mode_ctrl_enter_sends_and_returns_to_normal() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        state.input_buffer = "hello".to_string();
        let action = handle_key_event(
            &make_key_event_with_modifiers(KeyCode::Enter, KeyModifiers::CONTROL),
            &mut state,
        );
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_input_mode_ctrl_d_sends_and_returns_to_normal() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        state.input_buffer = "hello".to_string();
        let action = handle_key_event(
            &make_key_event_with_modifiers(KeyCode::Char('d'), KeyModifiers::CONTROL),
            &mut state,
        );
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.input_buffer.is_empty());
    }

    #[test]
    fn test_input_mode_q_does_not_quit() {
        let mut state = make_state_with_session();
        state.input_mode = InputMode::Input;
        let action = handle_key_event(&make_key_event(KeyCode::Char('q')), &mut state);
        assert_eq!(action, Action::Continue);
        assert_eq!(state.input_buffer, "q");
    }
}
