use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tmux_claude_state::monitor::MonitorState;

use crate::event_handler::{self, Action};
use crate::layout::{self, SessionInfo};
use crate::state::AppState;
use crate::tmux_ops;
use crate::ui;

/// Run the sidebar TUI application.
pub fn run_sidebar() -> Result<(), Box<dyn std::error::Error>> {
    // Get our own pane PID for exclusion
    let own_pid = std::process::id();

    // Start monitor polling
    let monitor_state = Arc::new(Mutex::new(MonitorState::default()));
    tmux_claude_state::monitor::start_polling(Arc::clone(&monitor_state));

    // Get our own pane ID
    let own_pane_id = tmux_ops::get_own_pane_id().unwrap_or_default();

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(
        &mut terminal,
        &monitor_state,
        own_pid,
        &own_pane_id,
    );

    // Terminal cleanup
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

fn run_event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    monitor_state: &Arc<Mutex<MonitorState>>,
    own_pid: u32,
    own_pane_id: &str,
) -> io::Result<()> {
    let mut app_state = AppState::new(Some(own_pid));

    loop {
        // Sync with monitor state
        if let Ok(monitor) = monitor_state.lock() {
            let diff = app_state.sync_with_monitor(&monitor);

            // Aggregate new sessions into the claude window
            for pid in &diff.added {
                if let Some(session) = monitor.sessions.iter().find(|s| s.pane.pid == *pid) {
                    let _ = tmux_ops::join_pane_to_claude_window(&session.pane.id);
                }
            }

            // Rebuild layout if sessions changed
            let needs_layout = !diff.added.is_empty()
                || !diff.removed.is_empty()
                || !diff.state_changed.is_empty();

            if needs_layout {
                apply_layout(&app_state, own_pane_id);
            }
        }

        // Draw TUI
        terminal.draw(|f| {
            ui::draw_sidebar(f, &app_state.sessions, app_state.selected_index);
        })?;

        // Handle keyboard events (non-blocking with 200ms poll)
        if event::poll(Duration::from_millis(200))? {
            let ev = event::read()?;
            let prev_selected = app_state.selected_index;
            match event_handler::handle_key_event(&ev, &mut app_state) {
                Action::Quit => return Ok(()),
                Action::SelectionChanged => {
                    if prev_selected != app_state.selected_index {
                        apply_layout(&app_state, own_pane_id);
                    }
                }
                Action::FocusSelected | Action::Continue => {}
            }
        }
    }
}

/// Compute and apply the layout for the current session state.
fn apply_layout(app_state: &AppState, sidebar_pane_id: &str) {
    let sessions: Vec<SessionInfo> = app_state
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| SessionInfo {
            pane_id: s.pane_id.clone(),
            state: s.state.clone(),
            is_selected: i == app_state.selected_index,
        })
        .collect();

    let Ok((win_w, win_h)) = tmux_ops::get_window_size() else {
        return;
    };

    let plan = layout::compute_layout(sidebar_pane_id, &sessions, win_w, win_h);

    // Apply sidebar width
    let _ = tmux_ops::resize_pane(sidebar_pane_id, Some(plan.sidebar.width), None);

    // Apply main pane size
    if let Some(ref main_pane) = plan.main_pane {
        let _ = tmux_ops::resize_pane(&main_pane.pane_id, Some(main_pane.width), Some(main_pane.height));
    }

    // Apply running pane sizes
    for pane in &plan.running_panes {
        let _ = tmux_ops::resize_pane(&pane.pane_id, Some(pane.width), Some(pane.height));
    }

    // Apply other pane sizes
    for pane in &plan.other_panes {
        let _ = tmux_ops::resize_pane(&pane.pane_id, Some(pane.width), Some(pane.height));
    }
}
