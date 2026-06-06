#!/usr/bin/env bash
# Download a self-contained Chromium build (Chrome for Testing) and install it
# at $XDG_DATA_HOME/cmux/chromium/ so the cmux browser preview pane can use
# it without depending on a system Chrome/Chromium install.
#
# Idempotent: if the binary already exists, exits 0 without re-downloading.
# Override the destination with CMUX_CHROMIUM_DIR.
#
# Architecture: linux-x64 only. Other arches need a different CfT channel.

set -euo pipefail

CMUX_CHROMIUM_DIR="${CMUX_CHROMIUM_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/cmux/chromium}"
TARGET_BIN="${CMUX_CHROMIUM_DIR}/chrome"

if [[ -f "${TARGET_BIN}" ]]; then
    echo "Chromium already installed: ${TARGET_BIN}"
    echo "Remove the directory and re-run to upgrade."
    exit 0
fi

# Serialize concurrent runs. flock(2) is inode-keyed, so we must NOT delete
# the lock file on exit — a `rm -f` on the held path frees the inode and the
# next process re-creates a *different* inode, silently breaking mutual
# exclusion. Use a stable path under XDG_RUNTIME_DIR (per-user tmpfs,
# cleaned by the session manager) and leave it in place.
mkdir -p "$(dirname "${CMUX_CHROMIUM_DIR}")"
LOCK_FILE="${XDG_RUNTIME_DIR:-/tmp}/cmux-chromium-install.lock"
TMP_DIR=""
cleanup() {
    [[ -n "${TMP_DIR}" ]] && rm -rf "${TMP_DIR}"
    # NOTE: deliberately do NOT remove LOCK_FILE — see comment above.
}
trap cleanup EXIT INT TERM
exec 9>"${LOCK_FILE}"
if ! flock -n 9; then
    echo "Another install is already in progress (lock: ${LOCK_FILE})." >&2
    exit 1
fi

# Re-check the target after acquiring the lock — a concurrent run that
# finished between our line-16 check and the flock above already installed
# Chromium, and we should report success rather than re-download.
if [[ -f "${TARGET_BIN}" ]]; then
    echo "Chromium already installed by a concurrent run: ${TARGET_BIN}"
    exit 0
fi

ARCH="$(uname -m)"
if [[ "${ARCH}" != "x86_64" ]]; then
    echo "ERROR: this installer only supports x86_64 (got ${ARCH})." >&2
    echo "Build a Chromium yourself and set [browser].chromium_path in" >&2
    echo "~/.config/cmux/config.toml to its absolute path." >&2
    exit 1
fi

for tool in curl jq unzip; do
    if ! command -v "${tool}" >/dev/null 2>&1; then
        echo "ERROR: '${tool}' is required (install via your package manager)." >&2
        exit 1
    fi
done

CHANNEL="Stable"
META_URL="https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json"

echo "Resolving latest Chrome for Testing ${CHANNEL} build…"
ZIP_URL="$(
    curl --fail --silent --show-error --location "${META_URL}" \
        | jq -r --arg channel "${CHANNEL}" \
            '.channels[$channel].downloads.chrome[]
             | select(.platform == "linux64")
             | .url'
)"

if [[ -z "${ZIP_URL}" || "${ZIP_URL}" == "null" ]]; then
    echo "ERROR: could not parse Chrome for Testing metadata at ${META_URL}." >&2
    exit 1
fi

mkdir -p "${CMUX_CHROMIUM_DIR}"
TMP_DIR="$(mktemp -d)"
# (cleanup trap registered up top handles both TMP_DIR and LOCK_FILE.)

echo "Downloading ${ZIP_URL}"
curl --fail --location --progress-bar "${ZIP_URL}" -o "${TMP_DIR}/chrome.zip"

# Integrity. Google publishes Chrome for Testing without per-build hashes in
# the `last-known-good-versions-with-downloads.json` endpoint (they live in
# `latest-versions-per-milestone-with-downloads.json`, not used here), so we
# rely on (a) TLS for transport integrity, (b) ZIP CRC verification to catch
# truncated/corrupted downloads, and (c) basic shape check. If you need
# stronger supply-chain guarantees, build Chromium yourself and point
# `[browser].chromium_path` in cmux config at your binary.
if ! unzip -t -q "${TMP_DIR}/chrome.zip" >/dev/null 2>&1; then
    echo "ERROR: downloaded chrome.zip failed CRC verification (truncated or corrupted)." >&2
    echo "Try re-running the installer." >&2
    exit 1
fi
ZIP_SIZE=$(stat -c %s "${TMP_DIR}/chrome.zip" 2>/dev/null || wc -c <"${TMP_DIR}/chrome.zip")
if [[ "${ZIP_SIZE}" -lt 10000000 ]]; then
    echo "ERROR: downloaded chrome.zip is suspiciously small (${ZIP_SIZE} bytes)." >&2
    exit 1
fi
ZIP_SHA256=$(sha256sum "${TMP_DIR}/chrome.zip" | awk '{print $1}')
echo "SHA-256: ${ZIP_SHA256}"
echo "(Record this hash if you want to verify a reproducible install later.)"

echo "Extracting…"
unzip -q "${TMP_DIR}/chrome.zip" -d "${TMP_DIR}"

# The zip layout is `chrome-linux64/chrome` plus shared libs.
SRC_DIR="${TMP_DIR}/chrome-linux64"
if [[ ! -d "${SRC_DIR}" ]]; then
    SRC_DIR="$(find "${TMP_DIR}" -maxdepth 2 -type d -name 'chrome-*' | head -1)"
fi
if [[ -z "${SRC_DIR}" || ! -f "${SRC_DIR}/chrome" ]]; then
    echo "ERROR: extracted archive does not look right — no chrome-linux64/chrome found." >&2
    exit 1
fi

# Stage the new tree at $CMUX_CHROMIUM_DIR.new, then atomic-swap into the
# target so an aborted install never leaves the user with no Chromium at all.
STAGING_DIR="${CMUX_CHROMIUM_DIR}.new"
rm -rf "${STAGING_DIR}"
mkdir -p "$(dirname "${STAGING_DIR}")"
mv "${SRC_DIR}" "${STAGING_DIR}"
chmod 755 "${STAGING_DIR}/chrome"

# Atomic swap. `mv -T` over an existing directory replaces it in one
# rename(2) call on the same filesystem. If a previous install exists we
# move it aside first so we can roll back on failure.
BACKUP_DIR=""
if [[ -d "${CMUX_CHROMIUM_DIR}" ]]; then
    BACKUP_DIR="${CMUX_CHROMIUM_DIR}.old.$$"
    mv -T "${CMUX_CHROMIUM_DIR}" "${BACKUP_DIR}"
fi
if ! mv -T "${STAGING_DIR}" "${CMUX_CHROMIUM_DIR}"; then
    echo "ERROR: failed to swap staged install into place" >&2
    if [[ -n "${BACKUP_DIR}" && -d "${BACKUP_DIR}" ]]; then
        mv -T "${BACKUP_DIR}" "${CMUX_CHROMIUM_DIR}" || true
        echo "Rolled back to previous install." >&2
    fi
    exit 1
fi
if [[ -n "${BACKUP_DIR}" && -d "${BACKUP_DIR}" ]]; then
    rm -rf "${BACKUP_DIR}"
fi

echo
echo "Installed: ${TARGET_BIN}"
echo
echo "cmux browser preview will pick this up automatically. To override with"
echo "a different binary, edit ~/.config/cmux/config.toml:"
echo
echo "    [browser]"
echo "    chromium_path = \"/path/to/your/chrome\""
