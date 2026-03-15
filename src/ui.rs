use ansi_to_tui::IntoText as _;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::time::{Instant, SystemTime};
use tmux_claude_state::claude_state::{ClaudeState, PermissionMode};

use crate::state::{InputMode, LayoutMode, ManagedSession, PreviewEntry, Tab, TabState};

const STALE_MIN_SECS: u64 = 5;
const STALE_MAX_SECS: u64 = 15;

/// Minimum width (in columns) for a single preview pane.
pub const MIN_PANE_WIDTH: u16 = 80;

/// Compute grid dimensions (cols, rows) for `n` panes in the given width.
///
/// `cols = available_width / min_col_width` (at least 1),
/// `rows = ceil(n / cols)`.
pub fn compute_grid(n: usize, available_width: u16, min_col_width: u16) -> (usize, usize) {
    if n == 0 {
        return (1, 0);
    }
    let cols = (available_width / min_col_width).max(1) as usize;
    let cols = cols.min(n); // don't allocate more columns than panes
    let rows = n.div_ceil(cols);
    (cols, rows)
}

/// Return the number of items in each row of the grid.
///
/// All rows have `cols` items except the last, which gets the remainder.
fn grid_row_items(n: usize, cols: usize) -> Vec<usize> {
    if cols == 0 || n == 0 {
        return vec![];
    }
    let rows = n.div_ceil(cols);
    (0..rows)
        .map(|r| {
            if r < rows - 1 {
                cols
            } else {
                let rem = n % cols;
                if rem == 0 { cols } else { rem }
            }
        })
        .collect()
}

const SELECTED_ICON: &str = "> ";
const TITLE_COLOR: Color = Color::Rgb(200, 200, 200);
const TAB_INACTIVE_COLOR: Color = Color::Rgb(140, 140, 140);
const TAB_ARROW_COLOR: Color = Color::Rgb(140, 140, 140);
const EMPTY_MESSAGE_COLOR: Color = Color::Rgb(180, 180, 180);
const MODEL_COLOR: Color = Color::Rgb(140, 140, 140);
const PLACEHOLDER_COLOR: Color = Color::Rgb(100, 100, 100);

/// Determine if a session should pulse based on its state and elapsed time.
/// `has_worked` indicates whether the session has ever been in the Working state;
/// freshly launched sessions (never worked) do not pulse on Idle.
pub fn should_pulse(state: &ClaudeState, elapsed_secs: u64, has_worked: bool) -> bool {
    matches!(state, ClaudeState::WaitingForApproval)
        || (matches!(state, ClaudeState::Idle)
            && has_worked
            && (STALE_MIN_SECS..=STALE_MAX_SECS).contains(&elapsed_secs))
}

/// Convert a ratatui `Color` to an RGB tuple.
const fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Blue => (0, 0, 255),
        Color::LightRed => (255, 100, 100),
        Color::White => (255, 255, 255),
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (200, 200, 200),
    }
}

/// Return the current sine-wave factor (0.0–1.0) for pulse animations.
fn pulse_factor() -> f64 {
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    f64::midpoint((t * 16.0).sin(), 1.0) // 0.0 ~ 1.0
}

/// Calculate a pulsing background color (dimmed version of the base color).
fn pulse_bg_color(base: Color) -> Color {
    let intensity = pulse_factor() * 0.25; // 0.0 ~ 0.25
    let (r, g, b) = color_to_rgb(base);
    // intensity is 0.0..=0.25, so result fits in u8 and is non-negative.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Color::Rgb(
        (f64::from(r) * intensity) as u8,
        (f64::from(g) * intensity) as u8,
        (f64::from(b) * intensity) as u8,
    )
}

/// Format elapsed time since an `Instant` into a human-readable string.
pub fn format_elapsed(since: Instant) -> String {
    let secs = since.elapsed().as_secs();
    if secs < 60 {
        format!("{secs:2}s")
    } else if secs < 3600 {
        format!("{:2}m", secs / 60)
    } else {
        let hours = (secs / 3600).min(99);
        format!("{hours:2}h")
    }
}

/// Map a `ClaudeState` to a display color.
pub const fn state_color(state: &ClaudeState) -> Color {
    match state {
        ClaudeState::Working => Color::Green,
        ClaudeState::WaitingForApproval => Color::Rgb(255, 165, 0),
        ClaudeState::Idle => Color::White,
    }
}

/// Return the icon string for a `PermissionMode`, matching Claude Code's status bar.
pub const fn permission_mode_icon(mode: &PermissionMode) -> &'static str {
    match mode {
        PermissionMode::PlanMode => "⏸ ",
        PermissionMode::EditAutomatically => "⏵⏵",
        PermissionMode::AskBeforeEdits => "",
    }
}

/// Map a `ClaudeState` to a short label.
pub const fn state_label(state: &ClaudeState) -> &'static str {
    match state {
        ClaudeState::Working => "⚡",
        ClaudeState::WaitingForApproval => "⚠️",
        ClaudeState::Idle => "💤",
    }
}

/// Format a session/preview title: "{number.}{name} ({branch}/{worktree}) - {title}".
///
/// `number` is a 0-based index; displayed as 1-based. Indices >= `MAX_NUMBER_KEYS` are omitted.
fn format_title(name: &str, number: Option<usize>, title: Option<&String>, git_branch: Option<&String>, worktree_name: Option<&String>) -> String {
    let number_part = match number {
        Some(idx) if idx < crate::state::MAX_NUMBER_KEYS => format!("{}.", idx + 1),
        _ => String::new(),
    };
    let suffix_part = match title {
        Some(t) if !t.is_empty() => format!(" - {t}"),
        _ => String::new(),
    };
    let branch_part = match (git_branch, worktree_name) {
        (Some(b), Some(wt)) => format!(" ({b}/{wt})"),
        (Some(b), None) => format!(" ({b})"),
        (None, Some(wt)) => format!(" ({wt})"),
        (None, None) => String::new(),
    };
    format!("{number_part}{name}{branch_part}{suffix_part}")
}

/// Draw the full TUI: session list (left) + preview pane (right).
// TODO: bundle draw parameters into a struct to reduce argument count.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    selected_index: usize,
    preview_contents: &[PreviewEntry],
    input_mode: InputMode,
    input_buffer: &str,
    show_help: bool,
    help_scroll: u16,
    preview_scroll: u16,
    tab_state: &TabState,
    layout_mode: LayoutMode,
    update_available: Option<&str>,
) {
    let size = f.area();

    // Top-level vertical split: main content | footer
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(size);

    // Main content: horizontal split left (sidebar) | right (preview)
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(0)])
        .split(v_chunks[0]);

    // Left panel: sessions list
    draw_left_panel(f, sessions, h_chunks[0], selected_index, input_mode, input_buffer, tab_state);

    // Right panel: preview
    let selected_pane_id = sessions
        .get(selected_index)
        .map(|s| s.pane_id.as_str());
    let preview_cursor = draw_preview_panes(f, preview_contents, h_chunks[1], selected_pane_id, preview_scroll, layout_mode);

    // Footer: app name + mode indicator + keybindings (full width)
    let footer_line = footer_spans(input_mode, layout_mode, update_available);
    let instructions = Paragraph::new(Line::from(footer_line))
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::Gray));
    f.render_widget(instructions, v_chunks[1]);

    // Show cursor inside preview pane in Insert/Broadcast mode (IME anchor)
    if matches!(input_mode, InputMode::Input | InputMode::Broadcast)
        && let Some((cx, cy)) = preview_cursor
    {
        f.set_cursor_position((cx, cy));
        // Use bar cursor because block cursor double-inverts the reverse-video cell and becomes invisible
        crossterm::execute!(std::io::stdout(), crossterm::cursor::SetCursorStyle::SteadyBar).ok();
    } else {
        // Reset cursor shape to default when returning to Normal mode
        crossterm::execute!(std::io::stdout(), crossterm::cursor::SetCursorStyle::DefaultUserShape).ok();
    }

    // Help popup overlay
    if show_help {
        draw_help_popup(f, size, help_scroll);
    }
}

/// Build the footer spans: app name, optional vim-style mode indicator, and keybindings.
fn footer_spans(input_mode: InputMode, layout_mode: LayoutMode, update_available: Option<&str>) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        concat!("crmux v", env!("CARGO_PKG_VERSION")),
        Style::default().fg(Color::White),
    )];
    if let Some(v) = update_available {
        spans.push(Span::styled(
            format!(" ({v} available! Run: crmux update)"),
            Style::default().fg(Color::Yellow),
        ));
    }
    let input_keys = "Input(i:Selected I:Marked)";
    match input_mode {
        InputMode::Normal => {
            let next_label = layout_mode.next().short_label();
            let v_label = format!("v:{next_label}");
            spans.push(Span::raw(format!(" | Nav:hjkl/1-9/t ScrollUp:C-u s:Switch Space:Mark {v_label} {input_keys} o:Claudeye ?:Help q:Quit")));
        }
        InputMode::Input => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "-- INSERT --",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" | Keys sent to selected pane via send-keys. Esc:Back"));
        }
        InputMode::Title => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "-- TITLE --",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" | Edit session title. Esc:Save&Exit"));
        }
        InputMode::Broadcast => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "-- BROADCAST --",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" | Keys sent to marked panes. Esc:Back"));
        }
        InputMode::Scroll => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "-- SCROLL --",
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(format!(" | Line:j/k HalfPage:C-u/C-d Top:gg Bottom:G {input_keys} Esc:Back")));
        }
    }
    spans
}


/// Compute cursor position for IME anchor within a preview pane.
///
/// If `cursor_pos` is `Some`, place the cursor at the detected reverse-video cell,
/// adjusted for scroll offset and inner area origin. Otherwise, fall back to bottom-left.
fn compute_cursor_pos(inner: Rect, cursor_pos: Option<(u16, u16)>, scroll_y: u16) -> Option<(u16, u16)> {
    let (crow, ccol) = cursor_pos?;
    let y = inner.y + crow.saturating_sub(scroll_y);
    let x = inner.x + ccol;
    Some((
        x.min(inner.x + inner.width.saturating_sub(1)),
        y.min(inner.y + inner.height.saturating_sub(1)),
    ))
}

/// Draw one or more preview panes, splitting the area vertically.
/// Returns the cursor position (x, y) for the selected preview pane's bottom-left (IME anchor).
// Single/multi-pane rendering is already split into branches; further extraction hurts readability.
#[allow(clippy::too_many_lines)]
/// Convert `GitDiffInfo` to a colored `Line` for `title_bottom` display.
/// GitHub-style colors: green for additions/staged, yellow for modified, red for deletions.
fn git_diff_line(info: &crate::state::GitDiffInfo) -> Line<'static> {
    let green = Style::default().fg(Color::Rgb(63, 185, 80));
    let yellow = Style::default().fg(Color::Rgb(210, 153, 34));
    let red = Style::default().fg(Color::Rgb(248, 81, 73));
    let gray = Style::default().fg(Color::Gray);

    if info.staged_files == 0 && info.modified_files == 0 {
        return Line::from(vec![Span::styled(" no changes ", gray)]).right_aligned();
    }

    let mut spans = vec![Span::styled(" ", gray)];
    if info.staged_files > 0 {
        spans.push(Span::styled(format!("+{}", info.staged_files), green));
        spans.push(Span::raw(" "));
    }
    if info.modified_files > 0 {
        spans.push(Span::styled(format!("~{}", info.modified_files), yellow));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("(", gray));
    spans.push(Span::styled(format!("+{}", info.insertions), green));
    spans.push(Span::styled(" ", gray));
    spans.push(Span::styled(format!("-{}", info.deletions), red));
    spans.push(Span::styled(") ", gray));

    Line::from(spans).right_aligned()
}

fn draw_preview_panes(
    f: &mut ratatui::Frame,
    preview_contents: &[PreviewEntry],
    area: ratatui::layout::Rect,
    selected_pane_id: Option<&str>,
    preview_scroll: u16,
    layout_mode: LayoutMode,
) -> Option<(u16, u16)> {
    if preview_contents.is_empty() {
        let preview = Paragraph::new("No session selected").block(
            Block::default()
                .title("Preview")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );
        f.render_widget(preview, area);
        return None;
    }

    if preview_contents.len() == 1 {
        let entry = &preview_contents[0];
        let preview_text = entry
            .content
            .as_str()
            .into_text()
            .unwrap_or_else(|_| Text::raw(entry.content.as_str()));
        // Terminal pane content never exceeds u16::MAX lines.
        #[allow(clippy::cast_possible_truncation)]
        let text_lines = preview_text.lines.len() as u16;
        let inner_height = area.height.saturating_sub(2);
        let max_scroll = text_lines.saturating_sub(inner_height);
        let effective_scroll = preview_scroll.min(max_scroll);
        let scroll_y = max_scroll.saturating_sub(effective_scroll);
        let mut title = format_title(&entry.name, Some(entry.index), entry.title.as_ref(), entry.git_branch.as_ref(), entry.worktree_name.as_ref());
        if preview_scroll > 0 {
            title.push_str(" [SCROLL]");
        }
        let color = state_color(&entry.state);
        let mut block = Block::default()
            .title(format!("{SELECTED_ICON}{title}"))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color));
        if let Some(info) = entry.git_diff.as_ref() {
            block = block.title_bottom(git_diff_line(info));
        }
        let preview = Paragraph::new(preview_text)
            .block(block)
            .scroll((scroll_y, 0));
        let inner = Block::default().borders(Borders::ALL).inner(area);
        let cursor_pos = compute_cursor_pos(inner, entry.cursor_pos, scroll_y);
        f.render_widget(preview, area);
        return cursor_pos;
    }

    // Multiple previews: layout depends on mode
    let main_content_width = if layout_mode == LayoutMode::MainVertical && !preview_contents.is_empty() {
        #[allow(clippy::cast_possible_truncation)]
        let max_w = preview_contents[0].content.lines()
            .map(|line| {
                let stripped = crate::app::strip_ansi_for_prompt(line);
                unicode_width::UnicodeWidthStr::width(stripped.as_str()) as u16
            })
            .max()
            .unwrap_or(0);
        Some(max_w)
    } else {
        None
    };
    let cell_areas = compute_cell_areas(preview_contents.len(), area, layout_mode, main_content_width);

    let mut cursor_pos = None;
    for (i, entry) in preview_contents.iter().enumerate() {
        let cell_area = cell_areas[i];
        let pos = render_preview_cell(f, entry, cell_area, selected_pane_id, preview_scroll);
        if pos.is_some() {
            cursor_pos = pos;
        }
    }
    cursor_pos
}

/// Compute cell areas for multiple preview panes based on layout mode.
#[allow(clippy::cast_possible_truncation)]
fn compute_cell_areas(n: usize, area: Rect, layout_mode: LayoutMode, main_content_width: Option<u16>) -> Vec<Rect> {
    match layout_mode {
        LayoutMode::EvenHorizontal | LayoutMode::EvenVertical => {
            let direction = match layout_mode {
                LayoutMode::EvenHorizontal => Direction::Horizontal,
                _ => Direction::Vertical,
            };
            let constraints: Vec<Constraint> = (0..n)
                .map(|_| Constraint::Ratio(1, n as u32))
                .collect();
            Layout::default()
                .direction(direction)
                .constraints(constraints)
                .split(area)
                .to_vec()
        }
        LayoutMode::MainVertical | LayoutMode::MainHorizontal => {
            if n <= 1 {
                return vec![area];
            }
            let (main_direction, sub_direction, main_pct, sub_pct) = match layout_mode {
                LayoutMode::MainVertical => {
                    let pct = match main_content_width {
                        Some(w) if area.width > 0 => {
                            let needed = (u32::from(w) + 2) * 100 / u32::from(area.width);
                            #[allow(clippy::cast_possible_truncation)]
                            let needed_u16 = needed as u16;
                            needed_u16.clamp(60, 80)
                        }
                        _ => 60,
                    };
                    (Direction::Horizontal, Direction::Vertical, pct, 100 - pct)
                }
                _ => (Direction::Vertical, Direction::Horizontal, 60, 40),
            };
            let main_split = Layout::default()
                .direction(main_direction)
                .constraints([
                    Constraint::Percentage(main_pct),
                    Constraint::Percentage(sub_pct),
                ])
                .split(area);
            let mut areas = vec![main_split[0]];
            let sub_count = n - 1;
            let sub_constraints: Vec<Constraint> = (0..sub_count)
                .map(|_| Constraint::Ratio(1, sub_count as u32))
                .collect();
            let sub_areas = Layout::default()
                .direction(sub_direction)
                .constraints(sub_constraints)
                .split(main_split[1]);
            areas.extend(sub_areas.iter());
            areas
        }
        LayoutMode::Single | LayoutMode::Grid => {
            let (cols, rows) = compute_grid(n, area.width, MIN_PANE_WIDTH);
            let row_items = grid_row_items(n, cols);
            let row_constraints: Vec<Constraint> = row_items
                .iter()
                .map(|_| Constraint::Ratio(1, rows as u32))
                .collect();
            let row_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(row_constraints)
                .split(area);
            let mut areas = Vec::with_capacity(n);
            for (row_idx, &items_in_row) in row_items.iter().enumerate() {
                let col_constraints: Vec<Constraint> = (0..items_in_row)
                    .map(|_| Constraint::Ratio(1, items_in_row as u32))
                    .collect();
                let col_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints(col_constraints)
                    .split(row_chunks[row_idx]);
                for col_idx in 0..items_in_row {
                    areas.push(col_chunks[col_idx]);
                }
            }
            areas
        }
    }
}

/// Render a single preview cell within a grid/even layout.
fn render_preview_cell(
    f: &mut ratatui::Frame,
    entry: &PreviewEntry,
    cell_area: Rect,
    selected_pane_id: Option<&str>,
    preview_scroll: u16,
) -> Option<(u16, u16)> {
    let preview_text = entry
        .content
        .as_str()
        .into_text()
        .unwrap_or_else(|_| Text::raw(entry.content.as_str()));
    #[allow(clippy::cast_possible_truncation)]
    let text_lines = preview_text.lines.len() as u16;
    let inner_height = cell_area.height.saturating_sub(2);
    let is_focused = selected_pane_id == Some(entry.pane_id.as_str());
    let scroll_y = if is_focused {
        let max_scroll = text_lines.saturating_sub(inner_height);
        let effective_scroll = preview_scroll.min(max_scroll);
        max_scroll.saturating_sub(effective_scroll)
    } else {
        text_lines.saturating_sub(inner_height)
    };
    let title_prefix = if is_focused { SELECTED_ICON } else { "" };
    let title = format_title(&entry.name, Some(entry.index), entry.title.as_ref(), entry.git_branch.as_ref(), entry.worktree_name.as_ref());
    let color = state_color(&entry.state);
    let elapsed_secs = entry.state_changed_at.elapsed().as_secs();
    let is_pulsing = should_pulse(&entry.state, elapsed_secs, entry.has_worked);
    let mut block = Block::default()
        .title(format!("{title_prefix}{title}"))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));
    if is_pulsing && !is_focused {
        block = block.style(Style::default().bg(pulse_bg_color(color)));
    }
    if let Some(info) = entry.git_diff.as_ref() {
        block = block.title_bottom(git_diff_line(info));
    }
    let preview = Paragraph::new(preview_text)
        .block(block)
        .scroll((scroll_y, 0));
    let cursor = if is_focused {
        let inner = Block::default().borders(Borders::ALL).inner(cell_area);
        compute_cursor_pos(inner, entry.cursor_pos, scroll_y)
    } else {
        None
    };
    f.render_widget(preview, cell_area);
    cursor
}

/// Draw the tab bar for project filtering.
fn draw_tab_bar(f: &mut ratatui::Frame, tab_state: &TabState, area: Rect) {
    let width = area.width as usize;
    if width == 0 || tab_state.tabs.is_empty() {
        return;
    }

    // Build tab labels with their widths
    let labels: Vec<String> = tab_state
        .tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let name = match tab {
                Tab::All => "All".to_string(),
                Tab::Workspace(w) => format!("@{w}"),
                Tab::Marked => "Marked".to_string(),
                Tab::Project(p) => format!("+{p}"),
            };
            if i == tab_state.selected_tab {
                format!("[{name}]")
            } else {
                name
            }
        })
        .collect();

    // Calculate total width with separators
    let total_width: usize = labels.iter().map(|l| l.chars().count()).sum::<usize>()
        + labels.len().saturating_sub(1); // spaces between tabs

    // Determine visible range with horizontal scroll
    let mut spans = Vec::new();
    if total_width <= width {
        // Everything fits
        for (i, label) in labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            let style = if i == tab_state.selected_tab {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(TAB_INACTIVE_COLOR)
            };
            spans.push(Span::styled(label.clone(), style));
        }
    } else {
        // Need horizontal scroll - center on selected tab
        let mut tab_positions: Vec<(usize, usize)> = Vec::new(); // (start, end) for each tab
        let mut pos = 0;
        for (i, label) in labels.iter().enumerate() {
            if i > 0 {
                pos += 1; // space separator
            }
            let len = label.chars().count();
            tab_positions.push((pos, pos + len));
            pos += len;
        }

        let sel = tab_state.selected_tab;
        let (sel_start, sel_end) = tab_positions[sel];
        let sel_mid = usize::midpoint(sel_start, sel_end);
        let view_start = sel_mid.saturating_sub(width / 2);
        let view_end = view_start + width;

        let has_left = view_start > 0;
        let has_right = view_end < total_width;

        let effective_start = if has_left { view_start + 2 } else { view_start };
        let effective_end = if has_right { view_end - 2 } else { view_end };

        if has_left {
            spans.push(Span::styled("< ", Style::default().fg(TAB_ARROW_COLOR)));
        }

        for (i, label) in labels.iter().enumerate() {
            let (start, end) = tab_positions[i];
            // Include separator before this tab
            if i > 0 {
                let sep_pos = start - 1;
                if sep_pos >= effective_start && sep_pos < effective_end {
                    spans.push(Span::raw(" "));
                }
            }
            if end > effective_start && start < effective_end {
                let style = if i == tab_state.selected_tab {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default().fg(TAB_INACTIVE_COLOR)
                };
                // Truncate label if partially visible
                let label_start = effective_start.saturating_sub(start);
                let label_end = if end > effective_end { effective_end - start } else { label.chars().count() };
                if label_start < label_end {
                    let visible: String = label.chars().skip(label_start).take(label_end - label_start).collect();
                    spans.push(Span::styled(visible, style));
                }
            }
        }

        if has_right {
            spans.push(Span::styled(" >", Style::default().fg(TAB_ARROW_COLOR)));
        }
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, area);
}

pub const HELP_TEXT: &str = "\
Keybindings (Normal mode):
  h / ← / l / →  Switch project tab
  j / ↓          Move cursor down in session list
  k / ↑          Move cursor up in session list
  Ctrl+u         Scroll preview up (half page)
  Ctrl+d         Scroll preview down (half page)
  gg             Scroll preview to top
  G              Scroll preview to bottom
  1-9            Select session by number
  t              Switch to previously selected session
  s              Switch to tmux pane
  Space          Mark session (for filtering and broadcast)
  v              Cycle layout (MainV/Single/Grid/EvenH/EvenV/MainH)  // Update when LayoutMode::next() changes
  i              Enter input mode (send keys to the selected session)
  I              Enter input mode (send keys to all marked sessions)
  e              Enter title mode (set a title for the session)
  o              Toggle claudeye overlay (requires claudeye >= 0.7.0)
  ?              Show this help
  q              Quit crmux

Keybindings (Scroll mode):
  j / ↓          Scroll preview down (1 line)
  k / ↑          Scroll preview up (1 line)
  Ctrl+u         Scroll preview up (half page)
  Ctrl+d         Scroll preview down (half page)
  gg             Scroll preview to top
  G              Scroll preview to bottom (exit scroll mode)
  i              Enter input mode (reset scroll)
  I              Enter broadcast mode (reset scroll)
  Esc            Reset scroll and return to normal mode

Keybindings (Input mode):
  Esc            Return to normal mode
  Any other key  Forwarded to the tmux pane via send-keys

Keybindings (Broadcast mode):
  Esc            Return to normal mode
  Any other key  Forwarded to all marked panes via send-keys

Keybindings (Title mode):
  Esc            Save and return to normal mode
  Backspace      Delete the last character";

/// Draw a centered help popup overlay.
fn draw_help_popup(f: &mut ratatui::Frame, area: Rect, help_scroll: u16) {
    let popup_width = area.width.min(65);
    let popup_height = area.height.saturating_sub(4).min(40);
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Help (? to close, j/k to scroll) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let paragraph = Paragraph::new(HELP_TEXT)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((help_scroll, 0));
    f.render_widget(paragraph, popup_area);
}

/// Draw the left panel (session list).
fn draw_left_panel(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    area: ratatui::layout::Rect,
    selected_index: usize,
    input_mode: InputMode,
    input_buffer: &str,
    tab_state: &TabState,
) {
    draw_sessions_list(f, sessions, area, selected_index, input_mode, input_buffer, tab_state);
}

/// Draw the list of Claude sessions.
// Per-session card rendering with inline title editing makes this long but cohesive.
#[allow(clippy::too_many_lines)]
fn draw_sessions_list(
    f: &mut ratatui::Frame,
    sessions: &[ManagedSession],
    area: ratatui::layout::Rect,
    selected_index: usize,
    input_mode: InputMode,
    input_buffer: &str,
    tab_state: &TabState,
) {
    let block_title = match tab_state.current_tab() {
        Tab::All => format!("All Sessions ({})", sessions.len()),
        Tab::Marked => format!("Marked ({})", sessions.len()),
        Tab::Workspace(name) => format!("Workspace @{name} ({})", sessions.len()),
        Tab::Project(name) => format!("Project {name} ({})", sessions.len()),
    };
    let block = Block::default()
        .title(block_title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White));

    let full_inner = block.inner(area);
    f.render_widget(block, area);

    // Tab bar (1 line) + session list
    let has_tabs = tab_state.tabs.len() > 1;
    let (tab_area, inner_area) = if has_tabs {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(full_inner);
        // Draw tab bar
        draw_tab_bar(f, tab_state, chunks[0]);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, full_inner)
    };
    let _ = tab_area;

    if sessions.is_empty() {
        let empty_msg = Paragraph::new("No Claude sessions detected")
            .style(Style::default().fg(EMPTY_MESSAGE_COLOR));
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
        let elapsed_secs = session.state_changed_at.elapsed().as_secs();
        let is_pulsing = should_pulse(&session.state, elapsed_secs, session.has_worked);
        let color = state_color(&session.state);
        let elapsed = format_elapsed(session.state_changed_at);
        let label = state_label(&session.state);

        let text_color = color;
        let mark_indicator = if session.marked { "* " } else { "  " };

        let border_style = Style::default().fg(color);

        let bg_style = if is_pulsing {
            Style::default().bg(pulse_bg_color(state_color(&session.state)))
        } else {
            Style::default()
        };

        let title_prefix = if is_selected { SELECTED_ICON } else { "" };
        let card_title = format_title(&session.project_name, Some(idx), None, session.git_branch.as_ref(), session.worktree_name.as_ref());
        let project_title = Line::from(Span::styled(
            format!("{title_prefix}{card_title}"),
            Style::default().fg(text_color).add_modifier(Modifier::BOLD),
        ));

        let mark_span = Span::styled(mark_indicator, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));

        let model_line = session.model.as_ref().map_or_else(
            || Line::from(""),
            |model| {
                let compact_model = model.replace(' ', "");
                let model_text = session.context_percent.map_or_else(
                    || compact_model.clone(),
                    |pct| format!("{compact_model}({pct}%)"),
                );
                Line::from(Span::styled(model_text, Style::default().fg(MODEL_COLOR)))
            },
        );

        let mut status_spans = Vec::new();
        let mode_icon = permission_mode_icon(&session.permission_mode);
        if !mode_icon.is_empty() {
            status_spans.push(Span::styled(
                mode_icon,
                Style::default().fg(Color::Cyan),
            ));
            status_spans.push(Span::raw(" "));
        }
        status_spans.push(Span::raw(label));
        status_spans.push(Span::raw(" "));
        status_spans.push(Span::raw(elapsed));
        let status_line = Line::from(status_spans);
        let is_editing_title = is_selected && input_mode == InputMode::Title;
        let combined_line = if is_editing_title {
            let (display_text, text_color) = if input_buffer.is_empty() {
                ("Type a title".to_string(), PLACEHOLDER_COLOR)
            } else {
                (input_buffer.to_string(), Color::Yellow)
            };
            Line::from(vec![
                mark_span,
                Span::styled(display_text, Style::default().fg(text_color)),
            ])
        } else if let Some(display) = session.display_title() {
            Line::from(vec![
                mark_span,
                Span::styled(display, Style::default().fg(TITLE_COLOR)),
            ])
        } else {
            Line::from(vec![
                mark_span,
                Span::styled("Press e to edit title", Style::default().fg(PLACEHOLDER_COLOR)),
            ])
        };
        let paragraph = Paragraph::new(vec![combined_line]);

        let card_border_style = if is_editing_title {
            Style::default().fg(Color::Yellow)
        } else {
            border_style
        };

        let block = Block::default()
            .title(project_title)
            .title_bottom(model_line.left_aligned())
            .title_bottom(status_line.right_aligned())
            .borders(Borders::ALL)
            .border_style(card_border_style);

        let paragraph = paragraph.block(block).style(bg_style);

        f.render_widget(paragraph, layout[idx]);

        // Set cursor position for inline title editing
        if is_editing_title {
            let inner = Block::default().borders(Borders::ALL).inner(layout[idx]);
            // Cursor position is bounded by terminal width, well within u16.
            #[allow(clippy::cast_possible_truncation)]
            let cursor_x = inner.x + 2 + input_buffer.chars().count().min((inner.width.saturating_sub(2)) as usize) as u16;
            let cursor_y = inner.y;
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_elapsed_seconds() {
        let now = Instant::now();
        let result = format_elapsed(now);
        assert_eq!(result, " 0s");
    }

    #[test]
    fn test_format_elapsed_minutes() {
        let since = Instant::now() - std::time::Duration::from_secs(120);
        assert_eq!(format_elapsed(since), " 2m");
    }

    #[test]
    fn test_format_elapsed_hours() {
        let since = Instant::now() - std::time::Duration::from_secs(7200);
        assert_eq!(format_elapsed(since), " 2h");
    }

    #[test]
    fn test_format_elapsed_boundary_59s() {
        let since = Instant::now() - std::time::Duration::from_secs(59);
        assert_eq!(format_elapsed(since), "59s");
    }

    #[test]
    fn test_format_elapsed_boundary_60s() {
        let since = Instant::now() - std::time::Duration::from_secs(60);
        assert_eq!(format_elapsed(since), " 1m");
    }

    #[test]
    fn test_format_elapsed_boundary_3599s() {
        let since = Instant::now() - std::time::Duration::from_secs(3599);
        assert_eq!(format_elapsed(since), "59m");
    }

    #[test]
    fn test_format_elapsed_boundary_3600s() {
        let since = Instant::now() - std::time::Duration::from_secs(3600);
        assert_eq!(format_elapsed(since), " 1h");
    }

    #[test]
    fn test_format_elapsed_capped_at_99h() {
        let since = Instant::now() - std::time::Duration::from_secs(100 * 3600);
        assert_eq!(format_elapsed(since), "99h");
    }

    #[test]
    fn test_state_color() {
        assert_eq!(state_color(&ClaudeState::Working), Color::Green);
        assert_eq!(
            state_color(&ClaudeState::WaitingForApproval),
            Color::Rgb(255, 165, 0)
        );
        assert_eq!(state_color(&ClaudeState::Idle), Color::White);
    }

    #[test]
    fn test_state_label() {
        assert_eq!(state_label(&ClaudeState::Working), "⚡");
        assert_eq!(state_label(&ClaudeState::WaitingForApproval), "⚠️");
        assert_eq!(state_label(&ClaudeState::Idle), "💤");
    }

    #[test]
    fn test_ansi_to_text_plain() {
        let text = "hello world".into_text().unwrap();
        assert_eq!(text.lines.len(), 1);
    }

    #[test]
    fn test_ansi_to_text_with_colors() {
        let ansi = "\x1b[31mred\x1b[0m normal";
        let text = ansi.into_text().unwrap();
        assert!(!text.lines.is_empty());
    }

    // --- should_pulse tests ---

    #[test]
    fn test_should_pulse_approval() {
        // WaitingForApproval should always pulse regardless of elapsed time or has_worked
        assert!(should_pulse(&ClaudeState::WaitingForApproval, 0, false));
        assert!(should_pulse(&ClaudeState::WaitingForApproval, 100, true));
    }

    #[test]
    fn test_should_pulse_idle_stale_after_work() {
        // Idle within STALE_MIN_SECS..=STALE_MAX_SECS should pulse only if has_worked
        assert!(should_pulse(&ClaudeState::Idle, 5, true));
        assert!(should_pulse(&ClaudeState::Idle, 10, true));
        assert!(should_pulse(&ClaudeState::Idle, 15, true));
    }

    #[test]
    fn test_should_pulse_idle_stale_never_worked() {
        // Idle within stale range but never worked — should NOT pulse
        assert!(!should_pulse(&ClaudeState::Idle, 5, false));
        assert!(!should_pulse(&ClaudeState::Idle, 10, false));
        assert!(!should_pulse(&ClaudeState::Idle, 15, false));
    }

    #[test]
    fn test_should_pulse_idle_not_stale() {
        // Idle outside the stale range should NOT pulse even if has_worked
        assert!(!should_pulse(&ClaudeState::Idle, 4, true));
        assert!(!should_pulse(&ClaudeState::Idle, 16, true));
    }

    #[test]
    fn test_should_pulse_working() {
        // Working should never pulse
        assert!(!should_pulse(&ClaudeState::Working, 0, false));
        assert!(!should_pulse(&ClaudeState::Working, 10, true));
    }

    // --- color_to_rgb tests ---

    #[test]
    fn test_color_to_rgb() {
        assert_eq!(color_to_rgb(Color::Blue), (0, 0, 255));
        assert_eq!(color_to_rgb(Color::LightRed), (255, 100, 100));
        assert_eq!(color_to_rgb(Color::White), (255, 255, 255));
    }

    // --- compute_cursor_pos tests ---

    #[test]
    fn test_compute_cursor_pos_with_position() {
        let inner = Rect::new(10, 20, 80, 40);
        // cursor_pos is Some → returns adjusted position
        assert_eq!(compute_cursor_pos(inner, Some((5, 3)), 0), Some((13, 25)));
    }

    #[test]
    fn test_compute_cursor_pos_none_returns_none() {
        let inner = Rect::new(10, 20, 80, 40);
        // cursor_pos is None → returns None (no fallback)
        assert_eq!(compute_cursor_pos(inner, None, 0), None);
    }

    #[test]
    fn test_compute_cursor_pos_with_scroll() {
        let inner = Rect::new(10, 20, 80, 40);
        // scroll_y=3, crow=5 → y = 20 + (5-3) = 22
        assert_eq!(compute_cursor_pos(inner, Some((5, 3)), 3), Some((13, 22)));
    }

    // --- pulse_bg_color tests ---

    #[test]
    fn test_pulse_bg_color_returns_rgb() {
        let result = pulse_bg_color(Color::LightRed);
        assert!(matches!(result, Color::Rgb(_, _, _)));
    }

    #[test]
    fn test_selected_icon_is_not_empty() {
        assert!(!SELECTED_ICON.is_empty());
    }

    // --- format_title tests ---

    #[test]
    fn test_format_title_with_title() {
        let title = "development".to_string();
        assert_eq!(format_title("crmux", None, Some(&title), None, None), "crmux - development");
    }

    #[test]
    fn test_format_title_without_title() {
        assert_eq!(format_title("crmux", None, None, None, None), "crmux");
    }

    #[test]
    fn test_format_title_with_empty_title() {
        let title = String::new();
        assert_eq!(format_title("crmux", None, Some(&title), None, None), "crmux");
    }

    #[test]
    fn test_format_title_with_branch() {
        let title = "dev".to_string();
        let branch = "main".to_string();
        assert_eq!(
            format_title("crmux", None, Some(&title), Some(&branch), None),
            "crmux (main) - dev"
        );
    }

    #[test]
    fn test_format_title_with_branch_and_worktree() {
        let title = "dev".to_string();
        let branch = "feature".to_string();
        let worktree = "wt-1".to_string();
        assert_eq!(
            format_title("crmux", None, Some(&title), Some(&branch), Some(&worktree)),
            "crmux (feature/wt-1) - dev"
        );
    }

    #[test]
    fn test_format_title_with_worktree_only() {
        let worktree = "wt-1".to_string();
        assert_eq!(
            format_title("crmux", None, None, None, Some(&worktree)),
            "crmux (wt-1)"
        );
    }

    #[test]
    fn test_format_title_with_number() {
        assert_eq!(format_title("crmux", Some(0), None, None, None), "1.crmux");
    }

    #[test]
    fn test_format_title_with_number_and_branch() {
        let branch = "main".to_string();
        assert_eq!(
            format_title("crmux", Some(2), None, Some(&branch), None),
            "3.crmux (main)"
        );
    }

    #[test]
    fn test_format_title_with_number_beyond_max() {
        assert_eq!(format_title("crmux", Some(crate::state::MAX_NUMBER_KEYS), None, None, None), "crmux");
    }

    // --- footer_spans tests ---

    #[test]
    fn test_footer_normal_mode_starts_with_app_name() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::Single, None);
        assert!(spans[0].content.starts_with("crmux v"));
    }

    #[test]
    fn test_footer_normal_mode_has_no_mode_indicator() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(!text.contains("--"));
    }

    #[test]
    fn test_footer_input_mode_starts_with_app_name() {
        let spans = footer_spans(InputMode::Input, LayoutMode::Single, None);
        assert!(spans[0].content.starts_with("crmux v"));
    }

    #[test]
    fn test_footer_input_mode_has_insert_indicator() {
        let spans = footer_spans(InputMode::Input, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("-- INSERT --"));
    }

    #[test]
    fn test_footer_title_mode_starts_with_app_name() {
        let spans = footer_spans(InputMode::Title, LayoutMode::Single, None);
        assert!(spans[0].content.starts_with("crmux v"));
    }

    #[test]
    fn test_footer_title_mode_has_title_edit_indicator() {
        let spans = footer_spans(InputMode::Title, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("-- TITLE --"));
    }

#[test]
    fn test_footer_normal_mode_contains_help_key() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("?:Help"), "Normal mode footer should contain '?:Help', got: {text}");
    }

    #[test]
    fn test_footer_broadcast_mode_has_broadcast_indicator() {
        let spans = footer_spans(InputMode::Broadcast, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("-- BROADCAST --"), "Broadcast mode footer should contain '-- BROADCAST --', got: {text}");
    }

    #[test]
    fn test_footer_normal_mode_contains_broadcast_key() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("I:Marked"), "Normal mode footer should contain 'I:Marked', got: {text}");
    }

    #[test]
    fn test_footer_normal_mode_contains_scroll_keys() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("ScrollUp:C-u"), "Normal mode footer should contain 'ScrollUp:C-u', got: {text}");
    }

    // --- compute_grid tests ---

    #[test]
    fn test_compute_grid_single_pane() {
        // 1 pane is always (1, 1)
        assert_eq!(compute_grid(1, 200, MIN_PANE_WIDTH), (1, 1));
    }

    #[test]
    fn test_compute_grid_horizontal_fit() {
        // width 160, 2 panes → side by side (2cols, 1row)
        assert_eq!(compute_grid(2, 160, MIN_PANE_WIDTH), (2, 1));
    }

    #[test]
    fn test_compute_grid_grid_layout() {
        // width 160, 4 panes → 2x2 grid
        assert_eq!(compute_grid(4, 160, MIN_PANE_WIDTH), (2, 2));
    }

    #[test]
    fn test_compute_grid_wide_screen() {
        // width 320, 4 panes → all side by side (4cols, 1row)
        assert_eq!(compute_grid(4, 320, MIN_PANE_WIDTH), (4, 1));
    }

    #[test]
    fn test_compute_grid_narrow_screen() {
        // width 79, 3 panes → all stacked (1col, 3rows)
        assert_eq!(compute_grid(3, 79, MIN_PANE_WIDTH), (1, 3));
    }

    #[test]
    fn test_compute_grid_boundary_exact() {
        // width 80, 1 pane → (1, 1)
        assert_eq!(compute_grid(1, 80, MIN_PANE_WIDTH), (1, 1));
    }

    #[test]
    fn test_compute_grid_boundary_two_panes() {
        // width 160, exactly 2 columns
        assert_eq!(compute_grid(3, 160, MIN_PANE_WIDTH), (2, 2));
    }

    #[test]
    fn test_compute_grid_zero_panes() {
        assert_eq!(compute_grid(0, 200, MIN_PANE_WIDTH), (1, 0));
    }

    // --- grid_row_items tests ---

    #[test]
    fn test_grid_row_items_even_split() {
        // 4 panes, 2 cols → [2, 2]
        assert_eq!(grid_row_items(4, 2), vec![2, 2]);
    }

    #[test]
    fn test_grid_row_items_remainder() {
        // 3 panes, 2 cols → [2, 1]
        assert_eq!(grid_row_items(3, 2), vec![2, 1]);
    }

    #[test]
    fn test_grid_row_items_single_column() {
        // 3 panes, 1 col → [1, 1, 1]
        assert_eq!(grid_row_items(3, 1), vec![1, 1, 1]);
    }

    #[test]
    fn test_grid_row_items_all_in_one_row() {
        // 3 panes, 3 cols → [3]
        assert_eq!(grid_row_items(3, 3), vec![3]);
    }

    #[test]
    fn test_grid_row_items_5_panes_3_cols() {
        // 5 panes, 3 cols → [3, 2]
        assert_eq!(grid_row_items(5, 3), vec![3, 2]);
    }

    // --- compute_cell_areas tests ---

    #[test]
    fn test_compute_cell_areas_main_vertical_3_panes() {
        let area = Rect::new(0, 0, 100, 40);
        let areas = compute_cell_areas(3, area, LayoutMode::MainVertical, None);
        assert_eq!(areas.len(), 3);
        // Main pane (left) should be ~60% width
        assert!(areas[0].width >= 58 && areas[0].width <= 62, "main width: {}", areas[0].width);
        assert_eq!(areas[0].height, 40);
        // Sub panes should be on the right, stacked vertically
        assert!(areas[1].width >= 38 && areas[1].width <= 42, "sub width: {}", areas[1].width);
        assert_eq!(areas[1].height, 20);
        assert_eq!(areas[2].height, 20);
        // Sub panes should be at same x position
        assert_eq!(areas[1].x, areas[2].x);
    }

    #[test]
    fn test_compute_cell_areas_main_horizontal_3_panes() {
        let area = Rect::new(0, 0, 100, 40);
        let areas = compute_cell_areas(3, area, LayoutMode::MainHorizontal, None);
        assert_eq!(areas.len(), 3);
        // Main pane (top) should be ~60% height
        assert!(areas[0].height >= 23 && areas[0].height <= 25, "main height: {}", areas[0].height);
        assert_eq!(areas[0].width, 100);
        // Sub panes should be on the bottom, side by side
        assert_eq!(areas[1].width, 50);
        assert_eq!(areas[2].width, 50);
        assert_eq!(areas[1].y, areas[2].y);
    }

    #[test]
    fn test_compute_cell_areas_main_vertical_1_pane() {
        let area = Rect::new(0, 0, 100, 40);
        let areas = compute_cell_areas(1, area, LayoutMode::MainVertical, None);
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0], area);
    }

    #[test]
    fn test_compute_cell_areas_main_horizontal_1_pane() {
        let area = Rect::new(0, 0, 100, 40);
        let areas = compute_cell_areas(1, area, LayoutMode::MainHorizontal, None);
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0], area);
    }

    #[test]
    fn test_compute_cell_areas_main_vertical_auto_width_mid() {
        let area = Rect::new(0, 0, 100, 40);
        // content width 75 → needed = (75+2)*100/100 = 77 → clamp(60,80) = 77
        let areas = compute_cell_areas(3, area, LayoutMode::MainVertical, Some(75));
        assert_eq!(areas.len(), 3);
        assert!(areas[0].width >= 75 && areas[0].width <= 79, "main width: {}", areas[0].width);
    }

    #[test]
    fn test_compute_cell_areas_main_vertical_auto_width_clamp_upper() {
        let area = Rect::new(0, 0, 100, 40);
        // content width 90 → needed = (90+2)*100/100 = 92 → clamp(60,80) = 80
        let areas = compute_cell_areas(3, area, LayoutMode::MainVertical, Some(90));
        assert_eq!(areas.len(), 3);
        assert!(areas[0].width >= 78 && areas[0].width <= 82, "main width: {}", areas[0].width);
    }

    #[test]
    fn test_compute_cell_areas_main_vertical_auto_width_clamp_lower() {
        let area = Rect::new(0, 0, 100, 40);
        // content width 40 → needed = (40+2)*100/100 = 42 → clamp(60,80) = 60
        let areas = compute_cell_areas(3, area, LayoutMode::MainVertical, Some(40));
        assert_eq!(areas.len(), 3);
        assert!(areas[0].width >= 58 && areas[0].width <= 62, "main width: {}", areas[0].width);
    }

    #[test]
    fn test_footer_main_vertical_label_and_next() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::MainVertical, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("v:Single"), "should contain v:Single, got: {text}");
    }

    #[test]
    fn test_footer_main_horizontal_label_and_next() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::MainHorizontal, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("v:MainV"), "should contain v:MainV, got: {text}");
    }

    #[test]
    fn test_footer_even_vertical_next_is_main_h() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::EvenVertical, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("v:MainH"), "should contain v:MainH, got: {text}");
    }

    #[test]
    fn test_footer_normal_mode_contains_claudeye_key() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("o:Claudeye"), "Normal mode footer should contain 'o:Claudeye', got: {text}");
    }

    #[test]
    fn test_footer_spans_input_mode_not_empty() {
        let spans = footer_spans(InputMode::Input, LayoutMode::Single, None);
        let text_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        assert!(text_len > 0, "Input mode footer should produce non-empty text");
    }

    #[test]
    fn test_footer_spans_broadcast_mode_not_empty() {
        let spans = footer_spans(InputMode::Broadcast, LayoutMode::Single, None);
        let text_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        assert!(text_len > 0, "Broadcast mode footer should produce non-empty text");
    }

    #[test]
    fn test_pulse_bg_color_within_intensity_range() {
        // bg intensity ranges from 0.0 to 0.25
        let base = Color::White; // (255, 255, 255)
        let result = pulse_bg_color(base);
        if let Color::Rgb(r, g, b) = result {
            assert!(r <= 63, "r={r} exceeds max intensity");
            assert!(g <= 63, "g={g} exceeds max intensity");
            assert!(b <= 63, "b={b} exceeds max intensity");
        } else {
            panic!("Expected Color::Rgb");
        }
    }

    // --- Scroll mode footer tests ---

    #[test]
    fn test_footer_scroll_mode_has_scroll_indicator() {
        let spans = footer_spans(InputMode::Scroll, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("-- SCROLL --"), "Scroll mode footer should contain '-- SCROLL --', got: {text}");
    }

    #[test]
    fn test_footer_scroll_mode_starts_with_app_name() {
        let spans = footer_spans(InputMode::Scroll, LayoutMode::Single, None);
        assert!(spans[0].content.starts_with("crmux v"));
    }

    #[test]
    fn test_footer_scroll_mode_contains_keybindings() {
        let spans = footer_spans(InputMode::Scroll, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Line:j/k"), "Scroll mode footer should contain 'Line:j/k', got: {text}");
        assert!(text.contains("Esc:Back"), "Scroll mode footer should contain 'Esc:Back', got: {text}");
    }

    // --- update notification tests ---

    #[test]
    fn test_footer_no_update_available() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::Single, None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(!text.contains("available!"));
    }

    #[test]
    fn test_footer_update_available() {
        let spans = footer_spans(InputMode::Normal, LayoutMode::Single, Some("v0.14.0"));
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("v0.14.0 available! Run: crmux update"), "got: {text}");
    }

    // --- permission_mode_icon tests ---

    #[test]
    fn test_permission_mode_icon_plan() {
        assert_eq!(permission_mode_icon(&PermissionMode::PlanMode), "⏸ ");
    }

    #[test]
    fn test_permission_mode_icon_auto_edit() {
        assert_eq!(permission_mode_icon(&PermissionMode::EditAutomatically), "⏵⏵");
    }

    #[test]
    fn test_permission_mode_icon_ask() {
        assert_eq!(permission_mode_icon(&PermissionMode::AskBeforeEdits), "");
    }
}
