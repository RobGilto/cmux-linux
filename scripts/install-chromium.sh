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
trap 'rm -rf "${TMP_DIR}"' EXIT

echo "Downloading ${ZIP_URL}"
curl --fail --location --progress-bar "${ZIP_URL}" -o "${TMP_DIR}/chrome.zip"

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

# Move the whole directory contents into CMUX_CHROMIUM_DIR so shared libs sit
# alongside the chrome binary.
rm -rf "${CMUX_CHROMIUM_DIR}"
mkdir -p "$(dirname "${CMUX_CHROMIUM_DIR}")"
mv "${SRC_DIR}" "${CMUX_CHROMIUM_DIR}"

chmod 755 "${TARGET_BIN}"
echo
echo "Installed: ${TARGET_BIN}"
echo
echo "cmux browser preview will pick this up automatically. To override with"
echo "a different binary, edit ~/.config/cmux/config.toml:"
echo
echo "    [browser]"
echo "    chromium_path = \"/path/to/your/chrome\""
