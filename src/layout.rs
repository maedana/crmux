use tmux_claude_state::claude_state::ClaudeState;

/// A pane placement in the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanePlacement {
    pub pane_id: String,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// The computed layout plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutPlan {
    /// Sidebar pane placement (always leftmost).
    pub sidebar: PanePlacement,
    /// Main (selected) pane placement (top right, largest).
    pub main_pane: Option<PanePlacement>,
    /// Running session pane placements (middle row).
    pub running_panes: Vec<PanePlacement>,
    /// Idle/Approval session pane placements (bottom row).
    pub other_panes: Vec<PanePlacement>,
}

/// Session info needed for layout computation.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub pane_id: String,
    pub state: ClaudeState,
    pub is_selected: bool,
}

const SIDEBAR_WIDTH: u16 = 30;
const MIN_PANE_WIDTH: u16 = 20;
const MIN_PANE_HEIGHT: u16 = 5;

/// Compute layout for all session panes plus the sidebar.
///
/// The layout follows:
/// - Sidebar: fixed width on the left
/// - Main (selected): ~60% of remaining height, full remaining width
/// - Running: ~30% of remaining height, split horizontally
/// - Idle/Approval: ~10% of remaining height, split horizontally
pub fn compute_layout(
    sidebar_pane_id: &str,
    sessions: &[SessionInfo],
    window_width: u16,
    window_height: u16,
) -> LayoutPlan {
    let sidebar = PanePlacement {
        pane_id: sidebar_pane_id.to_string(),
        x: 0,
        y: 0,
        width: SIDEBAR_WIDTH,
        height: window_height,
    };

    if sessions.is_empty() {
        return LayoutPlan {
            sidebar,
            main_pane: None,
            running_panes: Vec::new(),
            other_panes: Vec::new(),
        };
    }

    let content_width = window_width.saturating_sub(SIDEBAR_WIDTH + 1); // -1 for separator
    let content_x = SIDEBAR_WIDTH + 1;

    // Categorize sessions
    let selected: Option<&SessionInfo> = sessions.iter().find(|s| s.is_selected);
    let running: Vec<&SessionInfo> = sessions
        .iter()
        .filter(|s| !s.is_selected && s.state == ClaudeState::Working)
        .collect();
    let others: Vec<&SessionInfo> = sessions
        .iter()
        .filter(|s| {
            !s.is_selected
                && (s.state == ClaudeState::Idle || s.state == ClaudeState::WaitingForApproval)
        })
        .collect();

    // Calculate row heights
    let (main_h, running_h, others_h) =
        compute_row_heights(window_height, selected.is_some(), running.len(), others.len());

    let mut y_offset = 0;

    // Main pane
    let main_pane = selected.map(|s| {
        let p = PanePlacement {
            pane_id: s.pane_id.clone(),
            x: content_x,
            y: y_offset,
            width: content_width,
            height: main_h,
        };
        y_offset += main_h + 1; // +1 for separator
        p
    });

    // Running panes row
    let running_panes = distribute_horizontal(&running, content_x, y_offset, content_width, running_h);
    if !running.is_empty() {
        y_offset += running_h + 1;
    }

    // Other panes row
    let other_panes = distribute_horizontal(&others, content_x, y_offset, content_width, others_h);

    LayoutPlan {
        sidebar,
        main_pane,
        running_panes,
        other_panes,
    }
}

/// Compute heights for each row based on how many sessions are in each category.
fn compute_row_heights(
    total_height: u16,
    has_selected: bool,
    running_count: usize,
    others_count: usize,
) -> (u16, u16, u16) {
    let active_rows = usize::from(has_selected) + usize::from(running_count > 0) + usize::from(others_count > 0);
    if active_rows == 0 {
        return (0, 0, 0);
    }

    // Account for separators between rows
    #[allow(clippy::cast_possible_truncation)] // active_rows is at most 3
    let separators = active_rows.saturating_sub(1) as u16;
    let available = total_height.saturating_sub(separators);

    match (has_selected, running_count > 0, others_count > 0) {
        // Only selected
        (true, false, false) => (available, 0, 0),
        // Only running
        (false, true, false) => (0, available, 0),
        // Only others
        (false, false, true) => (0, 0, available),
        // Selected + running
        (true, true, false) => {
            let main_h = (available * 65) / 100;
            let running_h = available - main_h;
            (main_h.max(MIN_PANE_HEIGHT), running_h.max(MIN_PANE_HEIGHT), 0)
        }
        // Selected + others
        (true, false, true) => {
            let main_h = (available * 80) / 100;
            let others_h = available - main_h;
            (main_h.max(MIN_PANE_HEIGHT), 0, others_h.max(MIN_PANE_HEIGHT))
        }
        // Running + others
        (false, true, true) => {
            let running_h = (available * 70) / 100;
            let others_h = available - running_h;
            (0, running_h.max(MIN_PANE_HEIGHT), others_h.max(MIN_PANE_HEIGHT))
        }
        // All three
        (true, true, true) => {
            let main_h = (available * 60) / 100;
            let running_h = (available * 30) / 100;
            let others_h = available - main_h - running_h;
            (
                main_h.max(MIN_PANE_HEIGHT),
                running_h.max(MIN_PANE_HEIGHT),
                others_h.max(MIN_PANE_HEIGHT),
            )
        }
        (false, false, false) => (0, 0, 0),
    }
}

/// Distribute panes horizontally within a row.
fn distribute_horizontal(
    sessions: &[&SessionInfo],
    x_start: u16,
    y: u16,
    total_width: u16,
    height: u16,
) -> Vec<PanePlacement> {
    if sessions.is_empty() || height == 0 {
        return Vec::new();
    }

    #[allow(clippy::cast_possible_truncation)] // bounded by tmux window size
    let count = sessions.len() as u16;
    let pane_width = (total_width / count).max(MIN_PANE_WIDTH);

    sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            #[allow(clippy::cast_possible_truncation)]
            let i_u16 = i as u16;
            let x = x_start + i_u16 * (pane_width + 1); // +1 for separator
            let w = if i_u16 == count - 1 {
                // Last pane takes remaining width
                total_width.saturating_sub(i_u16 * (pane_width + 1))
            } else {
                pane_width
            };
            PanePlacement {
                pane_id: s.pane_id.clone(),
                x,
                y,
                width: w,
                height,
            }
        })
        .collect()
}

/// Build a tmux custom layout string from a `LayoutPlan`.
/// Format: `checksum,WxH,0,0{sidebar_layout,content_layout}`
#[allow(dead_code)]
pub fn build_layout_string(plan: &LayoutPlan, window_width: u16, window_height: u16) -> String {
    // Content panes (everything else)
    let mut content_parts = Vec::new();
    if let Some(ref main_pane) = plan.main_pane {
        content_parts.push(format!(
            "{}x{},{},{}",
            main_pane.width, main_pane.height, main_pane.x, main_pane.y
        ));
    }

    // Running row
    if !plan.running_panes.is_empty() {
        let row_parts: Vec<String> = plan
            .running_panes
            .iter()
            .map(|p| format!("{}x{},{},{}", p.width, p.height, p.x, p.y))
            .collect();
        if row_parts.len() == 1 {
            content_parts.push(row_parts[0].clone());
        } else {
            content_parts.push(format!("{{{}}}", row_parts.join(",")));
        }
    }

    // Others row
    if !plan.other_panes.is_empty() {
        let row_parts: Vec<String> = plan
            .other_panes
            .iter()
            .map(|p| format!("{}x{},{},{}", p.width, p.height, p.x, p.y))
            .collect();
        if row_parts.len() == 1 {
            content_parts.push(row_parts[0].clone());
        } else {
            content_parts.push(format!("{{{}}}", row_parts.join(",")));
        }
    }

    let content = if content_parts.len() <= 1 {
        content_parts.join(",")
    } else {
        format!("[{}]", content_parts.join(","))
    };

    // Tmux layout format: checksum,WxH,0,0{sidebar,content}
    let layout_body = format!(
        "{}x{},0,0{{{}}}",
        window_width,
        window_height,
        if content.is_empty() {
            format!(
                "{}x{},{},{}",
                plan.sidebar.width, plan.sidebar.height, plan.sidebar.x, plan.sidebar.y
            )
        } else {
            format!(
                "{}x{},{},{},{}",
                plan.sidebar.width, plan.sidebar.height, plan.sidebar.x, plan.sidebar.y, content
            )
        }
    );

    // tmux layout checksum (simplified: use a fixed value, tmux recalculates)
    format!("0000,{layout_body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session_info(pane_id: &str, state: ClaudeState, is_selected: bool) -> SessionInfo {
        SessionInfo {
            pane_id: pane_id.to_string(),
            state,
            is_selected,
        }
    }

    // --- Empty sessions ---

    #[test]
    fn test_layout_empty_sessions() {
        let plan = compute_layout("%0", &[], 200, 50);
        assert_eq!(plan.sidebar.width, SIDEBAR_WIDTH);
        assert_eq!(plan.sidebar.height, 50);
        assert!(plan.main_pane.is_none());
        assert!(plan.running_panes.is_empty());
        assert!(plan.other_panes.is_empty());
    }

    // --- Single session (selected) ---

    #[test]
    fn test_layout_single_selected() {
        let sessions = vec![make_session_info("%1", ClaudeState::Working, true)];
        let plan = compute_layout("%0", &sessions, 200, 50);

        assert!(plan.main_pane.is_some());
        let main = plan.main_pane.unwrap();
        assert_eq!(main.pane_id, "%1");
        assert_eq!(main.x, SIDEBAR_WIDTH + 1);
        assert_eq!(main.width, 200 - SIDEBAR_WIDTH - 1);
        assert_eq!(main.height, 50); // Takes full height when alone
        assert!(plan.running_panes.is_empty());
        assert!(plan.other_panes.is_empty());
    }

    // --- Mixed sessions ---

    #[test]
    fn test_layout_mixed_sessions() {
        let sessions = vec![
            make_session_info("%1", ClaudeState::Working, true),   // selected
            make_session_info("%2", ClaudeState::Working, false),  // running
            make_session_info("%3", ClaudeState::Idle, false),     // idle
            make_session_info("%4", ClaudeState::WaitingForApproval, false), // approval
        ];
        let plan = compute_layout("%0", &sessions, 200, 50);

        assert!(plan.main_pane.is_some());
        assert_eq!(plan.main_pane.as_ref().unwrap().pane_id, "%1");
        assert_eq!(plan.running_panes.len(), 1);
        assert_eq!(plan.running_panes[0].pane_id, "%2");
        assert_eq!(plan.other_panes.len(), 2);
    }

    // --- Row heights ---

    #[test]
    fn test_row_heights_all_three() {
        let (main_h, running_h, others_h) = compute_row_heights(100, true, 1, 1);
        // With 2 separators, available = 98
        // main: ~60%, running: ~30%, others: ~10%
        assert!(main_h > running_h);
        assert!(running_h > others_h);
        assert!(main_h + running_h + others_h <= 100);
    }

    #[test]
    fn test_row_heights_selected_only() {
        let (main_h, running_h, others_h) = compute_row_heights(50, true, 0, 0);
        assert_eq!(main_h, 50);
        assert_eq!(running_h, 0);
        assert_eq!(others_h, 0);
    }

    #[test]
    fn test_row_heights_no_sessions() {
        let (main_h, running_h, others_h) = compute_row_heights(50, false, 0, 0);
        assert_eq!(main_h, 0);
        assert_eq!(running_h, 0);
        assert_eq!(others_h, 0);
    }

    // --- Horizontal distribution ---

    #[test]
    fn test_distribute_horizontal_single() {
        let s = make_session_info("%1", ClaudeState::Idle, false);
        let sessions = vec![&s];
        let placements = distribute_horizontal(&sessions, 31, 0, 169, 10);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].pane_id, "%1");
        assert_eq!(placements[0].x, 31);
        assert_eq!(placements[0].width, 169);
    }

    #[test]
    fn test_distribute_horizontal_multiple() {
        let s1 = make_session_info("%1", ClaudeState::Idle, false);
        let s2 = make_session_info("%2", ClaudeState::Idle, false);
        let s3 = make_session_info("%3", ClaudeState::Idle, false);
        let sessions = vec![&s1, &s2, &s3];
        let placements = distribute_horizontal(&sessions, 31, 0, 169, 10);
        assert_eq!(placements.len(), 3);
        // Each should have reasonable width
        for p in &placements {
            assert!(p.width >= MIN_PANE_WIDTH);
            assert_eq!(p.height, 10);
        }
    }

    #[test]
    fn test_distribute_horizontal_empty() {
        let sessions: Vec<&SessionInfo> = vec![];
        let placements = distribute_horizontal(&sessions, 31, 0, 169, 10);
        assert!(placements.is_empty());
    }

    // --- Selection change (swap detection) ---

    #[test]
    fn test_layout_selection_change_gives_different_main() {
        let sessions1 = vec![
            make_session_info("%1", ClaudeState::Working, true),
            make_session_info("%2", ClaudeState::Working, false),
        ];
        let plan1 = compute_layout("%0", &sessions1, 200, 50);

        let sessions2 = vec![
            make_session_info("%1", ClaudeState::Working, false),
            make_session_info("%2", ClaudeState::Working, true),
        ];
        let plan2 = compute_layout("%0", &sessions2, 200, 50);

        assert_eq!(plan1.main_pane.as_ref().unwrap().pane_id, "%1");
        assert_eq!(plan2.main_pane.as_ref().unwrap().pane_id, "%2");
    }

    // --- Sidebar is always present ---

    #[test]
    fn test_sidebar_always_present() {
        let plan = compute_layout("%0", &[], 200, 50);
        assert_eq!(plan.sidebar.pane_id, "%0");
        assert_eq!(plan.sidebar.width, SIDEBAR_WIDTH);
    }
}
