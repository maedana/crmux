use crossterm::event::{Event, KeyCode};

use crate::state::AppState;
use crate::tmux_ops;

/// Action to take after handling a keyboard event.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Continue the event loop.
    Continue,
    /// Quit the application.
    Quit,
    /// Selection changed; may need layout swap.
    SelectionChanged,
    /// Focus the selected pane via tmux.
    FocusSelected,
}

/// Handle a keyboard event and return the appropriate action.
pub fn handle_key_event(event: &Event, state: &mut AppState) -> Action {
    if let Event::Key(key) = *event {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            KeyCode::Char('j') | KeyCode::Down => {
                state.select_next();
                Action::SelectionChanged
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.select_prev();
                Action::SelectionChanged
            }
            KeyCode::Enter => {
                if let Some(pane_id) = state.selected_pane_id() {
                    let _ = tmux_ops::select_pane(pane_id);
                }
                Action::FocusSelected
            }
            _ => Action::Continue,
        }
    } else {
        Action::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn make_key_event(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

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
        assert_eq!(action, Action::SelectionChanged);
    }

    #[test]
    fn test_navigate_k() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('k')), &mut state);
        assert_eq!(action, Action::SelectionChanged);
    }

    #[test]
    fn test_navigate_down_arrow() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Down), &mut state);
        assert_eq!(action, Action::SelectionChanged);
    }

    #[test]
    fn test_navigate_up_arrow() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Up), &mut state);
        assert_eq!(action, Action::SelectionChanged);
    }

    #[test]
    fn test_enter_focus_selected() {
        let mut state = AppState::new(None);
        // No sessions, so enter won't actually call tmux
        let action = handle_key_event(&make_key_event(KeyCode::Enter), &mut state);
        assert_eq!(action, Action::FocusSelected);
    }

    #[test]
    fn test_other_key_continues() {
        let mut state = AppState::new(None);
        let action = handle_key_event(&make_key_event(KeyCode::Char('x')), &mut state);
        assert_eq!(action, Action::Continue);
    }
}
