#!/usr/bin/env bash
# Build a portable AppImage that runs on any glibc >= the build host's.
#
# Two flavours selected via $CMUX_APPIMAGE_FLAVOUR (default: core):
#   * core: cmux-app + cmux + cmux-generate + agent-browser + cmuxd-remote +
#           ghostty .so + GTK4 / libc++ deps. ~120 MB.
#   * full: core + bundled Chrome-for-Testing + skills + CLAUDE.md. ~360 MB.
#
# Output: dist/cmux-<flavour>-x86_64.AppImage
#
# Tooling auto-downloaded into $BUILD_TOOLS_DIR on first run:
#   - linuxdeploy (AppImage bootstrap)
#   - linuxdeploy-plugin-gtk (pulls libgtk-4 + glib + cairo + …)
#   - appimagetool (final image assembly)
#
# Build host requirements:
#   - glibc ideally <= the oldest target distro's glibc (build on
#     Ubuntu 22.04 if you want compatibility back to Debian 12)
#   - Rust toolchain, zig 0.15.2 (or pre-built ghostty-internal.a)
#   - patchelf, file, wget, curl
#   - fuse2 (for appimagetool runtime; can be disabled with
#     APPIMAGE_EXTRACT_AND_RUN=1)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
FLAVOUR="${CMUX_APPIMAGE_FLAVOUR:-core}"

case "${FLAVOUR}" in
    core|full) ;;
    *)
        echo "ERROR: invalid CMUX_APPIMAGE_FLAVOUR='${FLAVOUR}'. Use 'core' or 'full'." >&2
        exit 1
        ;;
esac

BUILD_TOOLS_DIR="${REPO_ROOT}/.build-tools"
APP_DIR="${REPO_ROOT}/dist/AppDir-${FLAVOUR}"
DIST_DIR="${REPO_ROOT}/dist"
TARGET_DIR="${REPO_ROOT}/target/release"

mkdir -p "${BUILD_TOOLS_DIR}" "${DIST_DIR}"

# --- Tooling -----------------------------------------------------------------

LINUXDEPLOY="${BUILD_TOOLS_DIR}/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_GTK="${BUILD_TOOLS_DIR}/linuxdeploy-plugin-gtk.sh"
APPIMAGETOOL="${BUILD_TOOLS_DIR}/appimagetool-x86_64.AppImage"

download_if_missing() {
    local url="$1" dest="$2"
    if [[ ! -f "${dest}" ]]; then
        echo "==> downloading $(basename "${dest}")"
        curl --fail --location --progress-bar -o "${dest}" "${url}"
        chmod +x "${dest}"
    fi
}

download_if_missing \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage" \
    "${LINUXDEPLOY}"

download_if_missing \
    "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh" \
    "${LINUXDEPLOY_GTK}"

download_if_missing \
    "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage" \
    "${APPIMAGETOOL}"

# --- Build cmux release binaries --------------------------------------------

if [[ ! -f "${TARGET_DIR}/cmux-app" ]] \
    || [[ ! -f "${TARGET_DIR}/agent-browser" ]]; then
    echo "==> cargo build --release (workspace)"
    (cd "${REPO_ROOT}" && cargo build --release --workspace)
fi

# cmuxd-remote (Go) — optional (remote-SSH mode only). Skip if Go is absent
# rather than aborting the whole image; the copy below is already guarded.
if [[ ! -f "${REPO_ROOT}/daemon/remote/cmuxd-remote" ]]; then
    if command -v go >/dev/null 2>&1; then
        echo "==> building cmuxd-remote (Go)"
        "${REPO_ROOT}/scripts/install-cmuxd-remote.sh"
    else
        echo "==> skipping cmuxd-remote (Go not installed; remote-SSH mode unavailable)"
    fi
fi
CMUXD_REMOTE_BIN="$(find "${HOME}/.local/share/cmux/bin" -name 'cmuxd-remote-*' -print -quit 2>/dev/null || true)"

# --- AppDir scaffolding ------------------------------------------------------

rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/usr/bin" "${APP_DIR}/usr/lib" "${APP_DIR}/usr/share/cmux/scripts"

cp "${TARGET_DIR}/cmux-app"      "${APP_DIR}/usr/bin/cmux-app.bin"
cp "${TARGET_DIR}/cmux"          "${APP_DIR}/usr/bin/cmux"
cp "${TARGET_DIR}/cmux-generate" "${APP_DIR}/usr/bin/cmux-generate"
cp "${TARGET_DIR}/agent-browser" "${APP_DIR}/usr/bin/agent-browser"

if [[ -n "${CMUXD_REMOTE_BIN}" && -f "${CMUXD_REMOTE_BIN}" ]]; then
    cp "${CMUXD_REMOTE_BIN}" "${APP_DIR}/usr/bin/cmuxd-remote"
fi

# Ghostty shared lib — already statically linked into cmux-app, but we copy
# the .so alongside for dlopen consumers (libghostty-vt and friends).
for so in \
    "${REPO_ROOT}/ghostty/zig-out/lib/ghostty-internal.so" \
    "${REPO_ROOT}/ghostty/zig-out/lib/libghostty-vt.so"
do
    if [[ -f "${so}" ]]; then
        cp "${so}" "${APP_DIR}/usr/lib/"
    fi
done

# Packaging scripts the menu actions invoke (e.g. install-chromium.sh).
cp "${REPO_ROOT}/scripts/install-chromium.sh" \
   "${APP_DIR}/usr/share/cmux/scripts/install-chromium.sh"
chmod +x "${APP_DIR}/usr/share/cmux/scripts/install-chromium.sh"

# Skills + CLAUDE.md for the bundled skill-discovery path.
for skill in cmux cmux-browser; do
    if [[ -d "${REPO_ROOT}/skills/${skill}" ]]; then
        mkdir -p "${APP_DIR}/usr/share/cmux/skills/${skill}"
        cp -r "${REPO_ROOT}/skills/${skill}/." \
              "${APP_DIR}/usr/share/cmux/skills/${skill}/"
    fi
done
if [[ -f "${REPO_ROOT}/packaging/CLAUDE.md" ]]; then
    cp "${REPO_ROOT}/packaging/CLAUDE.md" "${APP_DIR}/usr/share/cmux/CLAUDE.md"
fi

# Desktop entry + icon. linuxdeploy requires these.
mkdir -p "${APP_DIR}/usr/share/applications" \
         "${APP_DIR}/usr/share/icons/hicolor/256x256/apps"
cp "${REPO_ROOT}/packaging/desktop/com.cmux_lx.terminal.desktop" \
   "${APP_DIR}/usr/share/applications/cmux.desktop"
cp "${REPO_ROOT}/packaging/icons/hicolor/256x256/apps/com.cmux_lx.terminal.png" \
   "${APP_DIR}/usr/share/icons/hicolor/256x256/apps/cmux.png"

# AppImage runtime needs the desktop file + icon at AppDir root too.
cp "${APP_DIR}/usr/share/applications/cmux.desktop" "${APP_DIR}/cmux.desktop"
cp "${APP_DIR}/usr/share/icons/hicolor/256x256/apps/cmux.png" "${APP_DIR}/cmux.png"

# Override Exec=, Icon= in the embedded desktop entry to match AppImage names.
sed -i 's|^Exec=.*|Exec=cmux-app|' \
    "${APP_DIR}/cmux.desktop" \
    "${APP_DIR}/usr/share/applications/cmux.desktop"
sed -i 's|^Icon=.*|Icon=cmux|' \
    "${APP_DIR}/cmux.desktop" \
    "${APP_DIR}/usr/share/applications/cmux.desktop"

# AppRun wrapper: picks GDK backend (mirrors packaging/scripts/cmux-app-wrapper.sh)
# and exec's the real binary.
cat > "${APP_DIR}/AppRun" << 'APPRUN'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
export XDG_DATA_DIRS="${HERE}/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
# Append bundled libs (libc++, libghostty) AFTER the system search path so
# they only resolve if the host lacks them. Prepending would clobber host
# libgsystemd/libcrypto/libdbus with our build-host's copies, which crash
# at dl-init on hosts with a different glibc.
export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}:${HERE}/usr/lib"
# Mirror the .deb / .rpm wrapper's GDK-backend selection.
if [[ -z "${GDK_BACKEND:-}" ]]; then
    if command -v nvidia-smi >/dev/null 2>&1 && [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
        export GDK_BACKEND=x11
    fi
fi
# GDK binds the GLES API at EGL init by default and then cannot hand out the
# desktop OpenGL context libghostty's renderer requires, so GLArea realize
# fails with "Unable to create a GL context" on NVIDIA proprietary drivers
# (regardless of backend). gl-prefer-gl makes GDK bind desktop GL up front.
if command -v nvidia-smi >/dev/null 2>&1; then
    export GDK_DEBUG="${GDK_DEBUG:+${GDK_DEBUG},}gl-prefer-gl"
fi
exec "${HERE}/usr/bin/cmux-app.bin" "$@"
APPRUN
chmod +x "${APP_DIR}/AppRun"

# Replace the inline cmux-app wrapper with a tiny shim that re-execs through
# AppRun semantics (so $TERMINAL and similar env land correctly).
cat > "${APP_DIR}/usr/bin/cmux-app" << 'WRAPPER'
#!/usr/bin/env bash
exec "$(dirname "$(readlink -f "${0}")")/cmux-app.bin" "$@"
WRAPPER
chmod +x "${APP_DIR}/usr/bin/cmux-app"

# --- linuxdeploy + GTK plugin pull libs --------------------------------------

cd "${REPO_ROOT}"

export LINUXDEPLOY_PLUGIN_GTK_GTK_VERSION=4
export DEPLOY_GTK_VERSION=4
export NO_STRIP=1

# Step 1: let linuxdeploy + GTK plugin populate the AppDir's libs/icons.
# We do NOT ask for --output appimage here because linuxdeploy assembles
# the image immediately and would miss the libc++/chromium/skills we add
# below. We invoke appimagetool ourselves at the end.
"${LINUXDEPLOY}" \
    --appdir "${APP_DIR}" \
    --executable "${APP_DIR}/usr/bin/cmux-app.bin" \
    --executable "${APP_DIR}/usr/bin/cmux" \
    --executable "${APP_DIR}/usr/bin/cmux-generate" \
    --executable "${APP_DIR}/usr/bin/agent-browser" \
    --plugin gtk \
    --desktop-file "${APP_DIR}/usr/share/applications/cmux.desktop" \
    --icon-file "${APP_DIR}/usr/share/icons/hicolor/256x256/apps/cmux.png"

# Step 2: libc++ + libc++abi. linuxdeploy-plugin-gtk pulls libgtk-4 et al but
# not the libc++ ABI shipped by clang-built ghostty deps. Bundle them
# manually so the AppImage runs on distros that only have libstdc++.
for lib in libc++.so.1 libc++abi.so.1 libunwind.so.1; do
    src=""
    for cand in /usr/lib64 /usr/lib /usr/lib/x86_64-linux-gnu; do
        if [[ -f "${cand}/${lib}" ]]; then
            src="${cand}/${lib}"
            break
        fi
    done
    if [[ -n "${src}" ]]; then
        cp -L "${src}" "${APP_DIR}/usr/lib/"
    fi
done

# Step 2b: minimal-bundle policy. linuxdeploy-plugin-gtk over-bundles
# aggressively (libsystemd, libcrypto, libgnutls, libdbus, libGL, libffi,
# libnettle, libwayland, libglib, …). On a host whose glibc / dbus /
# systemd differs even slightly from the build host's, those bundled libs
# crash at dl-init because their .init_array entries call versioned glibc
# symbols the host's ld.so resolves differently. Pick a minimal allowlist
# of libs we actually need bundled (libc++ for the cmux ghostty deps,
# libghostty-*), and drop everything else so the binary's RUNPATH walks
# past $ORIGIN/../lib into /lib64.
BUNDLE_LIB="${APP_DIR}/usr/lib"
KEEP_LIBS=(
    libc++.so.1 libc++abi.so.1 libunwind.so.1
    libghostty-vt.so ghostty-internal.so
)
if [[ -d "${BUNDLE_LIB}" ]]; then
    for f in "${BUNDLE_LIB}"/*; do
        base="$(basename "${f}")"
        keep=0
        for kept in "${KEEP_LIBS[@]}"; do
            if [[ "${base}" == "${kept}" || "${base}" == "${kept%.*}".* ]]; then
                keep=1
                break
            fi
        done
        if [[ "${keep}" -eq 0 ]]; then
            rm -rf "${f}"
        fi
    done
fi

# Step 2c: strip RUNPATH from the executables so they cannot accidentally
# pick up *any* bundled lib whose name happens to collide with the host's —
# only the explicit KEEP_LIBS above are still resolvable via LD_LIBRARY_PATH
# set by AppRun.
if command -v patchelf >/dev/null 2>&1; then
    for exe in cmux-app.bin cmux cmux-generate agent-browser cmuxd-remote; do
        if [[ -f "${APP_DIR}/usr/bin/${exe}" ]]; then
            patchelf --remove-rpath "${APP_DIR}/usr/bin/${exe}" 2>/dev/null || true
        fi
    done
fi

# Step 3 (full flavour): bundle Chrome-for-Testing inside the AppDir BEFORE
# packing — otherwise the squashfs misses it.
if [[ "${FLAVOUR}" == "full" ]]; then
    if [[ ! -f "${APP_DIR}/usr/share/cmux/chromium/chrome" ]]; then
        echo "==> bundling Chrome-for-Testing for full flavour"
        CMUX_CHROMIUM_DIR="${APP_DIR}/usr/share/cmux/chromium" \
            "${REPO_ROOT}/scripts/install-chromium.sh"
    fi
    # Point the AppRun at the bundled binary so resolve_chromium_path finds
    # it via the AGENT_BROWSER_EXECUTABLE_PATH override hook.
    if ! grep -q AGENT_BROWSER_EXECUTABLE_PATH "${APP_DIR}/AppRun"; then
        sed -i '/^exec /i export AGENT_BROWSER_EXECUTABLE_PATH=\"${HERE}/usr/share/cmux/chromium/chrome\"' \
            "${APP_DIR}/AppRun"
    fi
fi

# Step 4: assemble the final AppImage from the now-populated AppDir.
OUT="${DIST_DIR}/cmux-${FLAVOUR}-x86_64.AppImage"
rm -f "${OUT}"
echo "==> assembling AppImage with appimagetool"
ARCH=x86_64 "${APPIMAGETOOL}" "${APP_DIR}" "${OUT}"

chmod +x "${OUT}"
echo
echo "Built: ${OUT}"
echo "Size:  $(du -h "${OUT}" | cut -f1)"
