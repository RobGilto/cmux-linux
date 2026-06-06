# Contributing to cmux for Linux

## Prerequisites

- Linux (any modern distribution; tested on Debian/Ubuntu, Fedora, Arch)
- Rust toolchain (install via [rustup](https://rustup.rs))
- Zig 0.15.2 — install via [mise](https://mise.jdx.dev) (`mise use -g zig@0.15.2`), [asdf](https://asdf-vm.com), or download from [ziglang.org](https://ziglang.org/download/)
- GTK4 development headers and libclang
  - Debian/Ubuntu: `sudo apt-get install libgtk-4-dev libclang-dev`
  - Fedora/RHEL: `sudo dnf install gtk4-devel clang-devel`
  - Arch: `sudo pacman -S gtk4 clang`
- Go 1.22+ (only required for building the SSH remote daemon)

## Getting Started

1. Clone the repository:
   ```bash
   git clone https://github.com/bradwilson331/cmux-linux.git
   cd cmux-linux
   ```

2. Initialize the ghostty submodule:
   ```bash
   git submodule update --init ghostty
   ```

   The `homebrew-cmux` and `vendor/bonsplit` submodules are macOS-only and may be
   left uninitialized for Linux work.

3. Build the embedded ghostty library and Rust app:
   ```bash
   ./scripts/setup-linux.sh                            # builds ghostty-internal.a
   cargo build --release --bin cmux --bin cmux-app
   ```

4. (Optional) Build the SSH remote daemon used by SSH workspaces:
   ```bash
   ./scripts/install-cmuxd-remote.sh
   ```

5. Run the app from the build tree:
   ```bash
   ./target/release/cmux-app
   ```

## Development Scripts

| Script | Description |
|--------|-------------|
| `./scripts/setup-linux.sh` | Builds `ghostty-internal.a` from the ghostty submodule |
| `./scripts/install-cmuxd-remote.sh` | Builds and installs the Go SSH remote daemon to `~/.local/share/cmux/bin/` |
| `./scripts/build_remote_daemon_release_assets.sh` | Multi-arch cmuxd-remote release builds |
| `./packaging/scripts/build-deb.sh` | Produces `dist/cmux_<version>_amd64.deb` |
| `./packaging/scripts/build-rpm.sh` | Produces `dist/cmux-<version>-1.x86_64.rpm` |
| `./packaging/scripts/validate-deb.sh` / `validate-rpm.sh` | Smoke-tests the package layout |

## Rebuilding ghostty

If you make changes to the ghostty submodule, rebuild the static library:

```bash
cd ghostty
zig build \
    -Dapp-runtime=none \
    -Doptimize=ReleaseFast \
    -Dcpu=baseline \
    -Dgtk-x11=true \
    -Dgtk-wayland=true
```

The artifact lands at `ghostty/zig-out/lib/ghostty-internal.a`; `build.rs` picks
it up automatically.

## Running Tests

Local test execution is intentionally avoided so unrelated runs do not contend
for the cmux socket. Use CI (GitHub Actions) for the Python `tests_v2/` socket
integration suite. The Swift `cmuxTests/` and `cmuxUITests/` directories are
legacy macOS XCTest bundles and do not run on Linux.

## Ghostty Submodule

The `ghostty` submodule points to [manaflow-ai/ghostty](https://github.com/manaflow-ai/ghostty),
a downstream fork of the upstream Ghostty project. The Linux build only depends
on this submodule; the macOS-specific submodules (`vendor/bonsplit`,
`homebrew-cmux`) are not used.

### Making changes to ghostty

```bash
cd ghostty
git checkout -b my-feature
# make changes
git add .
git commit -m "Description of changes"
git push origin my-feature      # 'origin' = manaflow-ai/ghostty
```

### Updating the pinned submodule SHA

```bash
cd ghostty
git fetch origin
git checkout origin/main
cd ..
git add ghostty
git commit -m "Update ghostty submodule"
```

See `docs/ghostty-fork.md` for fork-specific notes when they exist.

## License

By contributing to this repository, you agree that your contributions are
licensed under the project's GNU Affero General Public License v3.0 or later
(`AGPL-3.0-or-later`).
