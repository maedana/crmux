#!/bin/bash
# Record crmux demo GIF using ffmpeg + xdotool
#
# Usage:
#   bash demos/record.sh

set -e

OUTPUT="demos/demo.gif"
TMP_VIDEO="/tmp/crmux-demo.mp4"

# --- Demo tmux session setup ---
if ! tmux has-session -t demo 2>/dev/null; then
  echo "Creating demo tmux session..."

  # Window 1 with left/right split
  tmux new-session -d -s demo -c ~/src/github.com/maedana/crmux
  tmux split-window -h -t demo

  # Window 2 with left/right split
  tmux new-window -t demo
  tmux split-window -h -t demo

  # Window 3 with left/right split
  tmux new-window -t demo
  tmux split-window -h -t demo

  # Window 4 for crmux launch
  tmux new-window -t demo

  echo "Demo tmux session created."
  tmux list-windows -t demo
  tmux list-panes -t demo -a | grep "^demo:"

  # Launch claude in all 6 panes of windows 1-3
  for pane in demo:1.0 demo:1.1 demo:2.0 demo:2.1 demo:3.0 demo:3.1; do
    tmux send-keys -t "$pane" "cd ~/src/github.com/maedana/crmux && claude --permission-mode plan" Enter
  done

  echo "Claudeの起動を待っています..."
  sleep 15

  # Send simple prompts to 5 claude sessions
  PROMPTS=(
    "Add a badge to the top of README.md"
    "Change the description in Cargo.toml to Japanese"
    "Create a LICENSE file with MIT license"
    "Add a copyright comment to the top of main.rs"
    "Create an .editorconfig file"
  )
  PANES=(demo:1.0 demo:1.1 demo:2.0 demo:2.1 demo:3.0)

  for i in "${!PANES[@]}"; do
    tmux send-keys -t "${PANES[$i]}" "${PROMPTS[$i]}" Enter
    sleep 1
  done

  echo "プロンプトを送信しました。各セッションがタスクを実行中です。"
else
  echo "Demo tmux session already exists."
fi

# --- Recording setup ---
echo "Click on the tmux window you want to record..."
WINDOW_ID=$(xdotool selectwindow)

eval "$(xdotool getwindowgeometry --shell "$WINDOW_ID")"
WIDTH=$((WIDTH / 2 * 2))
HEIGHT=$((HEIGHT / 2 * 2))
xdotool windowfocus --sync "$WINDOW_ID"

echo "Recording window $WINDOW_ID (${WIDTH}x${HEIGHT})..."

ffmpeg -y -video_size "${WIDTH}x${HEIGHT}" \
  -framerate 15 \
  -f x11grab -i "$DISPLAY+${X},${Y}" \
  -pix_fmt yuv420p \
  "$TMP_VIDEO" &>/dev/null &
FFMPEG_PID=$!
sleep 2

# --- Scene 1: Juggling between panes (~3s) ---
echo "Scene 1: Juggling between panes..."
for pane in demo:1.0 demo:1.1 demo:2.0 demo:2.1; do
  tmux select-window -t "${pane%.*}"
  tmux select-pane -t "$pane"
  sleep 0.7
done

# --- Scene 2: Launch crmux (~3s) ---
echo "Scene 2: Launch crmux..."
tmux select-window -t demo:4
sleep 0.5
tmux send-keys -t demo:4 "crmux -w demo"
sleep 1
tmux send-keys -t demo:4 Enter
sleep 2

# --- Scene 3: Preview sessions (~3s) ---
echo "Scene 3: Preview sessions..."
tmux send-keys -t demo:4 1
sleep 1
tmux send-keys -t demo:4 3
sleep 1
tmux send-keys -t demo:4 5
sleep 1

# --- Scene 4: Send prompt via input mode (~5s) ---
echo "Scene 4: Send instruction via input mode..."
tmux send-keys -t demo:4 i
sleep 0.5
# Select option 4 "Type here to tell Claude what to change"
tmux send-keys -t demo:4 "4"
sleep 0.5
tmux send-keys -t demo:4 "Add a comment to Cargo.toml"
sleep 0.5
tmux send-keys -t demo:4 Enter
sleep 2
tmux send-keys -t demo:4 Escape
sleep 3

# --- Cleanup & convert ---
kill -INT $FFMPEG_PID 2>/dev/null || true
wait $FFMPEG_PID 2>/dev/null || true

echo "Converting to GIF..."
ffmpeg -y -i "$TMP_VIDEO" \
  -vf "fps=10,scale=960:-1:flags=lanczos, \
    drawtext=text='Which one needs approval?':enable='between(t,2,5)': \
      fontcolor=white:fontsize=24:font=Sans: \
      x=(w-text_w)/2:y=h-th-20: \
      box=1:boxcolor=black@0.5:boxborderw=8, \
    drawtext=text='Now you know.':enable='between(t,6,9)': \
      fontcolor=white:fontsize=24:font=Sans: \
      x=(w-text_w)/2:y=h-th-20: \
      box=1:boxcolor=black@0.5:boxborderw=8, \
    drawtext=text='See everything at a glance.':enable='gte(t,13)': \
      fontcolor=white:fontsize=24:font=Sans: \
      x=(w-text_w)/2:y=h-th-20: \
      box=1:boxcolor=black@0.5:boxborderw=8, \
    split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse" \
  "$OUTPUT" &>/dev/null

rm -f "$TMP_VIDEO"
echo "Done: $OUTPUT"
