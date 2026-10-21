# README recording of alfredo-tui. Driven by record.py (real run, real model).
# Capture at 2x (2560x1440); record.py scales to 1280x720.
Output raw.mp4
Set Shell bash
Set FontSize 46
Set Width 2560
Set Height 1440
Set Padding 24
Set Framerate 25
Set TypingSpeed 18ms
Set CursorBlink false
Set WaitTimeout 240s

# setup, not recorded: neutral HOME, prompt and PATH, sample repo prepared by record.py
Hide
Type "export HOME=/tmp/demo-home PS1='$ ' PATH=/tmp/demo-home/bin:/usr/bin:/bin ALFREDO_STATE_DIR=/tmp/demo-home/state ALFREDO_NO_MOTION=1 && cd ~/sample && clear"
Enter
Sleep 1s
Show

Sleep 300ms
Type "alfredo-tui --model qwen2.5-coder:14b"
Sleep 400ms
Enter
Wait+Screen@60s /warm/
Sleep 800ms
# the goal is typed unrecorded and appears at once, like a paste
Hide
Type "/go Add initials(name) to textutil.py, e.g. initials('ada lovelace') == 'AL', with unittest tests in test_textutil.py."
Show
Sleep 1000ms
Enter
Sleep 1000ms

# cut 1: the architect model plans (labelled by record.py)
Hide
Wait+Screen@120s /dispatch on/
Show
Sleep 2500ms

# cut 2: workers generate files and run checks (labelled by record.py)
Hide
Wait+Screen@120s /Repair #/
Show
Sleep 1500ms
Type "/watch 3"
Sleep 400ms
Enter
Sleep 2500ms
Escape
Wait+Screen@120s /Autopilot done/
Sleep 2500ms

# leave the TUI: the user's branch is untouched, one review branch exists
Ctrl+Q
Sleep 1s
Type "git branch && git log --oneline"
Sleep 300ms
Enter
Sleep 2500ms
# end marker for record.py
Hide
