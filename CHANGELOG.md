# Changelog

## Unreleased

### Added
- `send_text` RPC: `project` parameter for targeting idle sessions by project name
- `send_text` RPC: `no_execute` parameter to paste text without pressing Enter
- Display permission mode icon (Plan: ⏸, Auto Edit: ⏵⏵) in session list

### Changed
- Switch tmux-claude-state dependency to crates.io 0.3.0

## 0.9.0

### Added
- Tab-based session filtering by project name (`h`/`l` to switch tabs, default: all)
- Apply tab filter to claudeye overlay via `serialize_sessions`
- `send_text` RPC method: external tools can send text to the selected pane via `echo '{"text": "..."}' | crmux notify send-text`

### Fixed
- Fix preview area top clipping by trimming trailing blank lines

### Changed
- Improve Normal mode footer readability with grouped keybindings (`hjkl:Nav`, `Preview(...)`, `Input(...)`)

## 0.8.0

### Fixed
- Strip OSC 8 hyperlinks from captured pane content to fix rendering artifacts in preview

### Added
- GitHub Actions release workflow for automated binary builds on tag push
- `install.sh` for curl-based one-liner installation
- Release profile optimizations (LTO, strip, opt-level=s)
- Quick install section in README

## 0.7.0

### Added
- Esc Esc cancel forwarding: pressing `Esc` again right after returning to Normal mode from Input/Broadcast mode forwards Esc Esc to the tmux pane to cancel Claude Code
- claudeye socket integration with RPC request-response (`o` key to toggle)
- Show cursor at preview pane bottom in Insert/Broadcast mode for IME pre-edit text

## 0.6.0

### Added
- Grid layout for multi-mark preview: automatically arranges panes side-by-side on wide terminals (cols = width / 80), falling back to vertical stack on narrow screens

## 0.5.0

### Added
- Model display name via statusLine hook (e.g. "Opus" instead of "claude-opus-4-6")
- Context window usage percentage display in sidebar (e.g. "Opus (45%)")
- Scroll mode: `Ctrl+U`/`gg` triggers automatic entry, `j`/`k` for line-by-line scrolling, footer shows `-- SCROLL --` indicator
- Scrollable help popup with j/k/Ctrl+U/Ctrl+D/g/G navigation
- DRY help text shared between `crmux -h` and `?` popup

### Changed
- RPC params changed from flat `HashMap<String, String>` to `serde_json::Value` to support nested JSON

## 0.4.0

### Added
- Git branch display next to project name in sidebar (auto-refreshed every 5 seconds)
- RPC support for receiving session metadata (model, session_id) from Claude Code hooks
- Preview scroll with `Ctrl+u`/`Ctrl+d`, `gg` to jump to top, `G` to jump to bottom

## 0.3.2

### Added
- Paste forwarding support via bracketed paste (text paste works in Input and Broadcast modes)

## 0.3.0

### Added
- Broadcast input mode (`I` key) to send keys to all marked sessions simultaneously

## 0.2.1

### Changed
- Update README: add pulse animation feature, fix installation instructions, add roadmap

## 0.2.0

### Added
- Floating help popup on `?` key with keybinding reference
- Vim-style mode indicator in footer with persistent app name
- BackTab (Shift+Tab) mapping for tmux send-keys passthrough
- Auto-scroll preview panes to show latest output
- Session title (renamed from purpose) with `e` key to edit
- Focus icon for preview and session card titles

### Changed
- Input mode changed to real-time send-keys passthrough
- Input mode exit key changed from Esc to `Ctrl+O` to allow Esc passthrough
- Title mode exit unified to `Ctrl+O` with auto-save
- Switch keybinding rebound to `s` key
- Session card: merged indicator and title into single line
- Session card: moved project name to top border, status to bottom-right
- Session card: `>` indicator for selected session instead of yellow border
- Preview pane shows session title via PreviewEntry struct
- Footer shows detailed descriptions per mode
- Removed Esc key quit mapping from Normal mode to prevent accidental exits
- Pulse animation limited to background only

### Fixed
- Extracted `run_send_keys` helper with synchronous output to fix input ordering
- Drain pending events before next draw cycle to reduce input lag

### Removed
- MCP server and plan notification code (added then removed)

### Infrastructure
- Pulse animation for WaitingForApproval (continuous) and stale Idle (5-15s window) sessions
- Fixed poll interval to 50ms for smoother rendering
- Add repository field to Cargo.toml for crates.io

## 0.1.0

- Initial release
- Auto-detect Claude Code sessions running in tmux
- TUI sidebar with session list showing project name, state, and elapsed time
- Preview pane with ANSI color support
- Multi-session preview via Space key mark/unmark
- Input mode to send text to a selected session (`i` to enter, `Ctrl+Enter` or `Ctrl+d` to send)
- Sessions sorted by project name for stable ordering
- Keybinding reference available via `crmux -h`
