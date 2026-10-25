#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

FRONTEND_DIR="${1:-${REPO_ROOT}/src/gateway/elicitation_ui}"
GATEWAY_ROUTES="${2:-${REPO_ROOT}/src/gateway/routes/elicitation.rs}"

echo "Running static audio privacy & security audit (D-131, D-132, C-28, §6.7)..."
echo "Auditing frontend sources in: ${FRONTEND_DIR}"

violations=0

check_pattern() {
    local pattern="$1"
    local desc="$2"
    local files
    files="$(grep -rnE "${pattern}" "${FRONTEND_DIR}" || true)"
    if [[ -n "${files}" ]]; then
        echo "FAIL: Prohibited pattern detected (${desc}):" >&2
        echo "${files}" >&2
        violations=$((violations + 1))
    fi
}

# 1. Prohibited Web APIs that could persist or transmit audio
check_pattern '\bMediaRecorder\b' "MediaRecorder persistence"
check_pattern '\bindexedDB\b' "IndexedDB persistence"
check_pattern '\bWebSocket\b' "WebSocket external streaming"
check_pattern '\bsendBeacon\b' "sendBeacon telemetry"
check_pattern '\bcaches\.' "CacheStorage API audio retention"
check_pattern '\bcreateObjectURL\b' "createObjectURL blob exposure"

# 2. Check storage writes other than elicitation_id
storage_matches="$(grep -rnE '(localStorage|sessionStorage)\.(setItem|\[)' "${FRONTEND_DIR}" || true)"
if [[ -n "${storage_matches}" ]]; then
    # Filter out permissible elicitation_id key
    bad_storage="$(echo "${storage_matches}" | grep -v 'elicitation_id' || true)"
    if [[ -n "${bad_storage}" ]]; then
        echo "FAIL: Unauthorized storage write detected:" >&2
        echo "${bad_storage}" >&2
        violations=$((violations + 1))
    fi
fi

# 3. Check for Blob usage with audio types
blob_audio_matches="$(grep -rnE 'new[[:space:]]+Blob\([^)]*audio' "${FRONTEND_DIR}" || true)"
if [[ -n "${blob_audio_matches}" ]]; then
    echo "FAIL: Audio Blob construction detected:" >&2
    echo "${blob_audio_matches}" >&2
    violations=$((violations + 1))
fi

# 4. Check that Rust gateway accepts no audio upload bodies
if [[ -f "${GATEWAY_ROUTES}" ]]; then
    rust_audio_route="$(grep -rnE '(audio/|application/octet-stream)' "${GATEWAY_ROUTES}" | grep -E '(post|patch|put)' || true)"
    if [[ -n "${rust_audio_route}" ]]; then
        echo "FAIL: Rust gateway route accepts raw audio/binary body:" >&2
        echo "${rust_audio_route}" >&2
        violations=$((violations + 1))
    fi
fi

if [[ ${violations} -gt 0 ]]; then
    echo "Static audio safety check FAILED with ${violations} violations." >&2
    exit 1
fi

echo "Static audio safety check PASSED cleanly. 0 audio persistence/transmission vectors found."
exit 0
