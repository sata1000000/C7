#!/bin/bash

# This bash file tests the bridge runtime in the CI pipeline.
# If it fails, that doesn't particularly mean that there's a macOS bug, but it likely means there's a runtime bug.
#
# The `cargo build` command in the yml files usually catches compile-time errors. This script is for catching runtime errors.

# Make bash talk more.
set -euo pipefail

BRIDGE_PLUGIN="dist/C7.app/Contents/Resources/bridge/C7-Bridge.clap"
SYS_LOG="/tmp/C7_bridge_test.log"

# Check if the bridge was bundled successfully.
if [ ! -d "$BRIDGE_PLUGIN" ]; then
    echo "ERROR: $BRIDGE_PLUGIN not found"
    exit 1
fi

# Run the CLAP validator to simulate CLAP runtime and stress-test the plugin.
# Redirect the output to the system log file for inspection if it fails.
export CLAP_VALIDATOR_CONFIG="$(dirname "$0")/clap-validator.toml"
if clap-validator validate "$BRIDGE_PLUGIN" > "$SYS_LOG" 2>&1; then
    VALIDATION_PASSED=true
else
    VALIDATION_PASSED=false
fi

# The Verdict:
echo ""
echo "=== Validator Log ==="
cat "$SYS_LOG"
echo ""

if [ "$VALIDATION_PASSED" = true ]; then
    echo "PASS: C7 Bridge successfully validated and executed runtime loops."
    exit 0
else
    echo "FAIL: C7 Bridge crashed or failed ABI validation."
    exit 1
fi
