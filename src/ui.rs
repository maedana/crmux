use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::time::Instant;
use tmux_claude_state::claude_state::ClaudeState;

use crate::state::ManagedSession;

/// Format elapsed time since an `Instant` into a human-readable string.
pub fn format_elapsed(since: Instant) -> String {
    let secs = since.elapsed().as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

/// Map a `ClaudeState` to a display color.
pub const fn state_color(state: &ClaudeState) -> Color {
    match state {
        ClaudeState::Working => Color::Blue,
        ClaudeState::WaitingForApproval => Color::LightRed,
        ClaudeState::Idle => Color::White,
    }
}

/// Map a `ClaudeState` to a short label.
pub const fn state_label(state: &ClaudeState) -> &'static str {
    match state {
        ClaudeState::Working => "Running",
        ClaudeState::WaitingForApproval => "Approval",
        ClaudeState::Idle => "Idle",
    }
}

/// Draw the sidebar TUI.
pub fn draw_sidebar(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    selected_index: usize,
) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(size);

    // Title
    let title = Paragraph::new("crmux")
        .block(Block::default().title("Claude Sessions").borders(Borders::ALL))
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(title, chunks[0]);

    // Sessions list
    draw_sessions_list(f, sessions, chunks[1], selected_index);

    // Instructions
    let instructions = Paragraph::new("j/k: Navigate | Enter: Focus | q: Quit")
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(instructions, chunks[2]);
}

/// Draw the list of Claude sessions.
fn draw_sessions_list(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    area: ratatui::layout::Rect,
    selected_index: usize,
) {
    let block = Block::default()
        .title(format!("Sessions ({})", sessions.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    if sessions.is_empty() {
        let empty_msg = Paragraph::new("No Claude sessions detected")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_msg, inner_area);
        return;
    }

    let constraints: Vec<Constraint> = sessions.iter().map(|_| Constraint::Length(3)).collect();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner_area);

    for (idx, session) in sessions.iter().enumerate() {
        if idx >= layout.len() {
            break;
        }
        let is_selected = idx == selected_index;
        let color = state_color(&session.state);
        let elapsed = format_elapsed(session.state_changed_at);
        let label = state_label(&session.state);

        let text_color = if is_selected { Color::Yellow } else { color };
        let spans = vec![
            Span::styled(
                &session.project_name,
                Style::default()
                    .fg(text_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(label, Style::default().fg(text_color)),
            Span::raw(" "),
            Span::styled(elapsed, Style::default().fg(text_color)),
        ];

        let border_style = if is_selected {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(color)
        };

        let bg_style = if is_selected {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        let paragraph = Paragraph::new(Line::from(spans))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style),
            )
            .style(bg_style);

        f.render_widget(paragraph, layout[idx]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_elapsed_seconds() {
        let now = Instant::now();
        let result = format_elapsed(now);
        assert_eq!(result, "0s");
    }

    #[test]
    fn test_format_elapsed_minutes() {
        let since = Instant::now() - std::time::Duration::from_secs(120);
        assert_eq!(format_elapsed(since), "2m");
    }

    #[test]
    fn test_format_elapsed_hours() {
        let since = Instant::now() - std::time::Duration::from_secs(7200);
        assert_eq!(format_elapsed(since), "2h");
    }

    #[test]
    fn test_format_elapsed_boundary_59s() {
        let since = Instant::now() - std::time::Duration::from_secs(59);
        assert_eq!(format_elapsed(since), "59s");
    }

    #[test]
    fn test_format_elapsed_boundary_60s() {
        let since = Instant::now() - std::time::Duration::from_secs(60);
        assert_eq!(format_elapsed(since), "1m");
    }

    #[test]
    fn test_format_elapsed_boundary_3599s() {
        let since = Instant::now() - std::time::Duration::from_secs(3599);
        assert_eq!(format_elapsed(since), "59m");
    }

    #[test]
    fn test_format_elapsed_boundary_3600s() {
        let since = Instant::now() - std::time::Duration::from_secs(3600);
        assert_eq!(format_elapsed(since), "1h");
    }

    #[test]
    fn test_state_color() {
        assert_eq!(state_color(&ClaudeState::Working), Color::Blue);
        assert_eq!(
            state_color(&ClaudeState::WaitingForApproval),
            Color::LightRed
        );
        assert_eq!(state_color(&ClaudeState::Idle), Color::White);
    }

    #[test]
    fn test_state_label() {
        assert_eq!(state_label(&ClaudeState::Working), "Running");
        assert_eq!(state_label(&ClaudeState::WaitingForApproval), "Approval");
        assert_eq!(state_label(&ClaudeState::Idle), "Idle");
    }
}
