#!/bin/bash
# Record crmux demo GIF using ffmpeg + xdotool
#
# Usage:
#   1. Start 2-3 Claude Code sessions in tmux panes
#   2. Run: bash demos/record.sh
#   3. Click on the tmux window to select it for recording

set -e

OUTPUT="demos/demo.gif"
TMP_VIDEO="/tmp/crmux-demo.mp4"
DURATION=45

# Start screenkey overlay early so it's ready before recording
screenkey --font-color green --font-size medium --timeout 1.5 --no-systray --opacity 0.7 --compr-cnt 3 &
SCREENKEY_PID=$!
sleep 2

echo "Click on the tmux window you want to record..."
WINDOW_ID=$(xdotool selectwindow)

# Get window geometry
eval "$(xdotool getwindowgeometry --shell "$WINDOW_ID")"

# Focus the target window
xdotool windowfocus --sync "$WINDOW_ID"

echo "Recording window $WINDOW_ID (${WIDTH}x${HEIGHT}) for ${DURATION}s..."

# Start ffmpeg recording in background
ffmpeg -y -video_size "${WIDTH}x${HEIGHT}" \
  -framerate 15 \
  -f x11grab -i "$DISPLAY+${X},${Y}" \
  -t "$DURATION" \
  -pix_fmt yuv420p \
  "$TMP_VIDEO" &>/dev/null &
FFMPEG_PID=$!
sleep 3

# Launch crmux
xdotool type --delay 100 "crmux"
sleep 1.5
xdotool key Return
sleep 3

# Navigate sessions with j/k
xdotool key j
sleep 1.5
xdotool key j
sleep 1.5
xdotool key k
sleep 1.5

# Mark sessions with Space for multi-pane preview
xdotool key space
sleep 1.5
xdotool key j
sleep 0.5
xdotool key space
sleep 2

# Unmark sessions
xdotool key space
sleep 0.5
xdotool key k
sleep 0.5
xdotool key space
sleep 1.5

# Open claudeye integration with o
xdotool key o
sleep 3

# Close claudeye overlay
xdotool key o
sleep 1

# Send a prompt via input mode to trigger status/title update
xdotool key i
sleep 1
xdotool type --delay 50 "hello from crmux"
sleep 1
xdotool key Return
sleep 5
xdotool key Escape
sleep 0.5

# Show help popup with ?
xdotool key shift+slash
sleep 3
xdotool key shift+slash
sleep 1

# Quit
xdotool key q
sleep 1

# Wait for ffmpeg to finish
wait $FFMPEG_PID 2>/dev/null || true

kill $SCREENKEY_PID 2>/dev/null || true

# Convert to GIF
echo "Converting to GIF..."
ffmpeg -y -i "$TMP_VIDEO" \
  -vf "fps=10,scale=960:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse" \
  "$OUTPUT" &>/dev/null

rm -f "$TMP_VIDEO"
echo "Done: $OUTPUT"
