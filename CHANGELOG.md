# Changelog

## Unreleased

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
