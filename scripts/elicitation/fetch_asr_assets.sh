#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCK_FILE="${SCRIPT_DIR}/asr-assets.lock"

if [[ ! -f "${LOCK_FILE}" ]]; then
    echo "ERROR: Lockfile not found at ${LOCK_FILE}" >&2
    exit 1
fi

VERSION="$(jq -r '.version' "${LOCK_FILE}")"
TARBALL_URL="$(jq -r '.tarball_url' "${LOCK_FILE}")"
EXPECTED_TARBALL_SHA256="$(jq -r '.tarball_sha256' "${LOCK_FILE}")"

TKS_CACHE_ROOT="${TKS_MODEL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/tks/models}"
DEFAULT_TARGET_DIR="${TKS_CACHE_ROOT}/sherpa-onnx-web/${VERSION}"
TARGET_DIR="${1:-${DEFAULT_TARGET_DIR}}"

mkdir -p "${TARGET_DIR}"

# Check if all files already exist and match their expected hashes
all_valid=true
file_count="$(jq '.files | length' "${LOCK_FILE}")"
for ((i=0; i<file_count; i++)); do
    file_name="$(jq -r ".files[$i].name" "${LOCK_FILE}")"
    expected_hash="$(jq -r ".files[$i].sha256" "${LOCK_FILE}")"
    file_path="${TARGET_DIR}/${file_name}"
    if [[ ! -f "${file_path}" ]]; then
        all_valid=false
        break
    fi
    actual_hash="$(sha256sum "${file_path}" | awk '{print $1}')"
    if [[ "${actual_hash}" != "${expected_hash}" ]]; then
        echo "Hash mismatch for existing ${file_name}: expected ${expected_hash}, got ${actual_hash}" >&2
        all_valid=false
        break
    fi
done

if [[ "${all_valid}" == "true" ]]; then
    echo "ASR assets for ${VERSION} already present and verified at ${TARGET_DIR}"
    exit 0
fi

echo "Fetching ASR assets for ${VERSION} into ${TARGET_DIR}..."
TMP_DIR="$(mktemp -d /tmp/tks-asr-fetch.XXXXXX)"
cleanup() {
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

TARBALL_PATH="${TMP_DIR}/archive.tar.bz2"
echo "Downloading ${TARBALL_URL}..."
curl -fsSL "${TARBALL_URL}" -o "${TARBALL_PATH}"

echo "Verifying tarball SHA-256..."
ACTUAL_TARBALL_SHA256="$(sha256sum "${TARBALL_PATH}" | awk '{print $1}')"
if [[ "${ACTUAL_TARBALL_SHA256}" != "${EXPECTED_TARBALL_SHA256}" ]]; then
    echo "ERROR: Tarball SHA-256 mismatch! Expected ${EXPECTED_TARBALL_SHA256}, got ${ACTUAL_TARBALL_SHA256}" >&2
    exit 1
fi

echo "Extracting archive..."
mkdir -p "${TMP_DIR}/extracted"
tar -xjf "${TARBALL_PATH}" -C "${TMP_DIR}/extracted"

for ((i=0; i<file_count; i++)); do
    file_name="$(jq -r ".files[$i].name" "${LOCK_FILE}")"
    expected_hash="$(jq -r ".files[$i].sha256" "${LOCK_FILE}")"
    
    # Locate file in extracted tree
    found_file="$(find "${TMP_DIR}/extracted" -type f -name "${file_name}" | head -n 1)"
    if [[ -z "${found_file}" || ! -f "${found_file}" ]]; then
        echo "ERROR: File ${file_name} not found in extracted archive" >&2
        exit 1
    fi

    actual_hash="$(sha256sum "${found_file}" | awk '{print $1}')"
    if [[ "${actual_hash}" != "${expected_hash}" ]]; then
        echo "ERROR: File ${file_name} hash mismatch! Expected ${expected_hash}, got ${actual_hash}" >&2
        exit 1
    fi

    cp "${found_file}" "${TARGET_DIR}/${file_name}"
    echo "Installed and verified: ${file_name}"
done

echo "Successfully verified and installed all ASR assets at ${TARGET_DIR}"
