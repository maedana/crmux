#!/bin/bash
# Record crmux demo GIF using ffmpeg + xdotool
#
# Usage:
#   1. Run: bash demos/record.sh
#   2. The script sets up a demo tmux session with Claude Code sessions
#   3. Press Enter when Claude sessions are ready
#   4. Click on the tmux window to select it for recording

set -e

OUTPUT="demos/demo.gif"
TMP_VIDEO="/tmp/crmux-demo.mp4"
DURATION=20

# --- Demo tmux session setup ---
if ! tmux has-session -t demo 2>/dev/null; then
  echo "Creating demo tmux session..."

  # Window 1 with left/right split
  tmux new-session -d -s demo -c ~/src/github.com/maedana/crmux
  tmux split-window -h -t demo:1

  # Window 2 with left/right split
  tmux new-window -t demo
  tmux split-window -h -t demo:2

  # Window 3 with left/right split
  tmux new-window -t demo
  tmux split-window -h -t demo:3

  # Window 4 for crmux launch
  tmux new-window -t demo

  # Launch claude in all 6 panes of windows 1-3
  for pane in demo:1.1 demo:1.2 demo:2.1 demo:2.2 demo:3.1 demo:3.2; do
    tmux send-keys -t "$pane" "cd ~/src/github.com/maedana/crmux && claude" Enter
  done

  echo "Demo tmux session created with 6 claude panes + 1 crmux pane."
  echo "Claudeの起動を待っています..."
  sleep 15

  # Send plan-mode prompts to 5 claude sessions
  PROMPTS=(
    "Add vim-style / search filtering to the sidebar"
    "Add built-in Pomodoro timer integration"
    "Implement mouse click support for session selection"
    "Add color theme customization via config file"
    "Add session grouping by git branch"
  )
  PANES=(demo:1.1 demo:1.2 demo:2.1 demo:2.2 demo:3.1)

  for i in "${!PANES[@]}"; do
    tmux send-keys -t "${PANES[$i]}" "/plan ${PROMPTS[$i]}" Enter
    sleep 1
  done

  echo "プランモードのプロンプトを送信しました。"
  echo "pane 1-4 のプランをキャンセル(Escape)し、pane 5 (demo:3.1) は承認待ちのまま残してください。"
else
  echo "Demo tmux session already exists."
fi

read -p "準備ができたらEnterを押してください... "

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
sleep 2

# --- Scene 1: Juggling between panes (~3s) ---
for pane in demo:1.1 demo:1.2 demo:2.1 demo:2.2; do
  tmux switch-client -t "$pane"
  sleep 0.7
done

# --- Scene 2: Launch crmux (~3s) ---
tmux switch-client -t demo:4
sleep 0.5
xdotool type --delay 100 "crmux -w demo"
sleep 0.5
xdotool key Return
sleep 2

# --- Scene 3: Preview sessions (~3s) ---
xdotool key 1
sleep 1
xdotool key 3
sleep 1
xdotool key 5
sleep 1

# --- Scene 4: Send prompt via input mode (~5s) ---
xdotool key i
sleep 0.5
xdotool key Escape Escape
sleep 0.5
xdotool type --delay 50 "hello from crmux"
sleep 0.5
xdotool key Return
sleep 2
xdotool key Escape
sleep 1

# Wait for ffmpeg to finish
wait $FFMPEG_PID 2>/dev/null || true

kill $SCREENKEY_PID 2>/dev/null || true

# Convert to GIF with captions
echo "Converting to GIF..."
ffmpeg -y -i "$TMP_VIDEO" \
  -vf "fps=10,scale=960:-1:flags=lanczos, \
    drawtext=text='Which one needs approval?':enable='between(t,0,3)': \
      fontcolor=white:fontsize=24:font=Sans: \
      x=(w-text_w)/2:y=h-th-20: \
      box=1:boxcolor=black@0.5:boxborderw=8, \
    drawtext=text='Now you know.':enable='between(t,3,6)': \
      fontcolor=white:fontsize=24:font=Sans: \
      x=(w-text_w)/2:y=h-th-20: \
      box=1:boxcolor=black@0.5:boxborderw=8, \
    split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse" \
  "$OUTPUT" &>/dev/null

rm -f "$TMP_VIDEO"
echo "Done: $OUTPUT"
