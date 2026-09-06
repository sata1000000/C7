#!/bin/bash

# This bash file tests the application runtime in the CI pipeline.
# If it fails, that doesn't particularly mean that there's a macOS bug, but it likely means there's a runtime bug.
#
# The `cargo build` command in the yml files usually catches compile-time errors. This script is for catching runtime errors.

# Make bash talk more.
set -euo pipefail

APP="/Applications/C7.app"
BEFORE="/tmp/C7_before.png"
AFTER="/tmp/C7_after.png"
SYS_LOG="/tmp/C7_launch.log"
CRASH_DIR="$HOME/Library/Logs/DiagnosticReports"

# Check out what hardware the macOS VM is using.
fastfetch --logo none

# Copy the built app to the Applications folder.
cp -r dist/C7.app "$APP"

# Check if the app was copied successfully.
if [ ! -d "$APP" ]; then
    echo "ERROR: $APP not found"
    exit 1
fi

# Hide Homebrew's lib directory so the bundle can't use Homebrew-installed libraries.
BREW_LIB="$(brew --prefix)/lib"
BREW_LIB_BACKUP="/tmp/homebrew_lib_backup"
sudo mv "$BREW_LIB" "$BREW_LIB_BACKUP"

# GitHub Actions macOS runners are VMs with no Metal GPU passthrough.
# GTK4's default Metal renderer produces a black window in this environment.
# Real users on physical Macs are unaffected by the following flag.
export GSK_RENDERER=cairo

# Baseline screenshot before the app opens.
screencapture "$BEFORE"

# Stream system-level logs in the background (AppKit, LaunchServices, CoreMIDI).
log stream --predicate 'process == "c7_app"' > "$SYS_LOG" 2>&1 &
LOG_PID=$!

# Launch application via the macOS open command. Identical to what Finder does.
open "$APP"

# Wait for the window to render.
# Unlikely that it would take over 15 seconds.
sleep 15

# Stop the background log stream.
kill $LOG_PID 2>/dev/null || true

# Screenshot after the wait period.
screencapture "$AFTER"

# Restore Homebrew's lib directory.
sudo mv "$BREW_LIB_BACKUP" "$BREW_LIB"

# The Verdict:
echo ""
echo "=== C7 Launch Log ==="
cat "$SYS_LOG"
echo ""

if pgrep -x "c7_app" > /dev/null; then
    echo "PASS: C7 launched and remained active for 15 seconds."
    exit 0
else
    echo "FAIL: C7 crashed or closed prematurely."

    echo ""
    echo "=== Crash Reports ==="
    CRASH_FOUND=0
    for f in "$CRASH_DIR"/c7_app*.ips "$CRASH_DIR"/c7_app*.crash; do
        [ -f "$f" ] || continue
        CRASH_FOUND=1
        echo "--- $f ---"
        cat "$f"
    done

    if [ "$CRASH_FOUND" -eq 0 ]; then
        echo "(no crash reports found in $CRASH_DIR)"
    fi

    exit 1
fi
