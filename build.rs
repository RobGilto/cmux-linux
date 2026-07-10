use std::env;
use std::path::PathBuf;

fn main() {
    // Get the absolute path to the project directory
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let ghostty_lib_path = format!("{}/ghostty/zig-out/lib", manifest_dir);

    // Static link pre-built ghostty-internal.a (built by scripts/setup-linux.sh).
    // Upstream renamed the artifact from libghostty.a to ghostty-internal.a
    // when libghostty-vt was split off; the embedded API still lives here.
    // The file is emitted without the standard `lib` prefix, so pass the full
    // path to the linker rather than relying on the -lNAME search convention.
    println!("cargo:rustc-link-search=native={}", ghostty_lib_path);
    println!(
        "cargo:rustc-link-arg={}/ghostty-internal.a",
        ghostty_lib_path
    );
    // Rebuild when the archive itself changes — e.g. after a `zig build`
    // inside ghostty/. Without this, cargo will reuse the previously linked
    // binary even after the archive is regenerated, silently shipping stale
    // ghostty symbols.
    println!(
        "cargo:rerun-if-changed={}/ghostty-internal.a",
        ghostty_lib_path
    );

    // Note: ghostty-internal.a is a CombinedArchive that already bundles
    // simdutf.o and libhighway.a. The fork's earlier build.rs linked them
    // separately to chase AVX-512 SIGILL issues on older CPUs; with the
    // combined archive that produces duplicate-symbol link errors.
    // If a future ghostty refactor splits these back out, restore the
    // mtime-tracking lookup in the zig-cache here.

    // The legacy `stubs.o` (and its source `stubs.c`) provided empty no-op
    // implementations of glslang_*, spvc_*, and dcimgui symbols back when
    // ghostty exposed them as unresolved externs. The combined ghostty-internal
    // archive now ships real implementations, so linking stubs.o produces
    // duplicate-symbol errors. Keep the source file in tree for now in case a
    // future ghostty build configuration drops these deps again.

    // Link the GLAD loader object file — provides gladLoaderLoadGLContext and
    // gladLoaderUnloadGLContext which are needed by ghostty's OpenGL renderer.
    // The .o is checked into the repo; build.rs does not recompile it from
    // ghostty/vendor/glad/src/gl.c. If glad ever needs regenerating, do so
    // out-of-band and commit the new .o.
    println!("cargo:rustc-link-arg={}/glad.o", manifest_dir);
    println!("cargo:rerun-if-changed={}/glad.o", manifest_dir);

    // ghostty-internal.a requires these system libraries at link time.
    //
    // C++ ABI: zig builds the bundled C++ deps (glslang, dcimgui, SPIRV-Cross)
    // against libc++, NOT libstdc++ — symbols are in `std::__1::*`. Linking the
    // GNU libstdc++ ABI here produces "vtable / method not found" errors.
    // Resolve by pulling in LLVM's libc++ + libc++abi. On Debian/Ubuntu these
    // come from `libc++-dev libc++abi-dev`; on Fedora from
    // `libcxx-devel libcxxabi-devel`.
    println!("cargo:rustc-link-lib=dylib=GL");
    println!("cargo:rustc-link-lib=dylib=c++");
    println!("cargo:rustc-link-lib=dylib=c++abi");
    println!("cargo:rustc-link-lib=dylib=gcc_s"); // unwind helpers shared with libc++abi
    println!("cargo:rustc-link-lib=dylib=fontconfig");
    println!("cargo:rustc-link-lib=dylib=freetype");

    // Try to link the versioned onig library if dev package isn't installed
    if std::process::Command::new("pkg-config")
        .args(["--exists", "oniguruma"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        println!("cargo:rustc-link-lib=dylib=onig");
    } else if std::path::Path::new("/usr/lib/x86_64-linux-gnu/libonig.so.5").exists() {
        // Link to the versioned library file directly
        println!("cargo:rustc-link-arg=/usr/lib/x86_64-linux-gnu/libonig.so.5");
    }

    // glslang is optional - ghostty can work without it
    // We'll skip it for now since it's not installed

    // Use pkg-config for GTK4/GLib system libraries that libghostty.a needs
    // at link time if they are not fully bundled in the static archive.
    // This is a soft best-effort; link errors reveal which ones are needed.
    if std::process::Command::new("pkg-config")
        .args(["--exists", "gtk4"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        // Emit link-search dirs from the .pc file location (handles extracted dev packages).
        // pkg-config --variable=pcfiledir emits the directory containing the .pc file; the
        // sibling directory (../lib or the pkgconfig parent) contains the .so linker stubs.
        for pkg in &["gtk4", "graphene-gobject-1.0"] {
            let pcdir_out = std::process::Command::new("pkg-config")
                .args(["--variable=pcfiledir", pkg])
                .output();
            if let Ok(out) = pcdir_out {
                let pcdir = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !pcdir.is_empty() {
                    // pkgconfig dir is typically .../lib/x86_64-linux-gnu/pkgconfig;
                    // the parent contains the .so symlinks.
                    let libdir = std::path::Path::new(&pcdir)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !libdir.is_empty() {
                        println!("cargo:rustc-link-search=native={libdir}");
                    }
                }
            }
        }

        let gtk4_libs = std::process::Command::new("pkg-config")
            .args(["--libs", "gtk4"])
            .output()
            .expect("pkg-config gtk4 failed");
        let flags = String::from_utf8_lossy(&gtk4_libs.stdout);
        for flag in flags.split_whitespace() {
            if let Some(lib) = flag.strip_prefix("-l") {
                println!("cargo:rustc-link-lib=dylib={lib}");
            } else if let Some(path) = flag.strip_prefix("-L") {
                println!("cargo:rustc-link-search=native={path}");
            }
        }
    }

    // Re-run bindgen when ghostty.h changes (Plan 02 already patched it)
    println!("cargo:rerun-if-changed=ghostty.h");

    let bindings = bindgen::Builder::default()
        .header("ghostty.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        // Needed for types that reference C integer types
        .allowlist_item("ghostty_.*")
        .allowlist_item("GHOSTTY_.*")
        // Block the two display lifecycle exports: they are declared in
        // ghostty.h (held over from the fork's earlier pinned SHA) but no
        // longer defined in ghostty-internal.a. Blocking them in bindgen
        // prevents any future Rust caller from compile-succeeding into a
        // link-time `undefined reference` error. Restore once Phase C
        // re-exports them — see myc task #1 (see docs/phase-c-plan.md §1).
        .blocklist_function("ghostty_surface_display_realized")
        .blocklist_function("ghostty_surface_display_unrealized")
        .generate()
        .expect("Unable to generate ghostty bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("ghostty_sys.rs"))
        .expect("Couldn't write ghostty_sys.rs");
}
