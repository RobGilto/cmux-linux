// Build script: a panic here fails the build, which is the correct
// behavior — the no-panic policy applies to the app, not to build.rs.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;
use std::path::PathBuf;

fn main() {
    // Get the absolute path to the project directory
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let ghostty_lib_path = format!("{}/ghostty/zig-out/lib", manifest_dir);

    // Detect the TARGET os (not the host) — in a build script `cfg!(target_os)`
    // reflects the machine running the script, so cross-compiling to Apple
    // Silicon from a Linux CI box would silently take the Linux link path.
    // CARGO_CFG_TARGET_OS is the target triple's os, which is what we want.
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| "linux".to_string());

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

    // Link the GLAD loader — provides gladLoaderLoadGLContext and
    // gladLoaderUnloadGLContext which ghostty's OpenGL renderer needs.
    //
    // The checked-in glad.o is an x86_64 Linux ELF object, so it can only be
    // linked into a Linux build. For macOS (or any non-Linux target) we
    // compile ghostty/vendor/glad/src/gl.c from source with the `cc` crate so
    // the object matches the target's format (Mach-O on macOS). Linux keeps
    // linking the prebuilt .o unchanged — no behavior change on the platform
    // this build was proven on.
    if target_os == "linux" {
        println!("cargo:rustc-link-arg={}/glad.o", manifest_dir);
        println!("cargo:rerun-if-changed={}/glad.o", manifest_dir);
    } else {
        cc::Build::new()
            .file(format!("{manifest_dir}/ghostty/vendor/glad/src/gl.c"))
            .include(format!("{manifest_dir}/ghostty/vendor/glad/include"))
            .compile("glad");
        println!("cargo:rerun-if-changed={manifest_dir}/ghostty/vendor/glad/src/gl.c");
    }

    // ghostty-internal.a requires these system libraries at link time.
    // The set is platform-specific: Linux links the Mesa/GL + GNU/libc++ ABI
    // stack, macOS links Apple frameworks and lets Homebrew's pkg-config
    // resolve the rest. Gate on the TARGET os so cross-builds pick correctly.
    if target_os == "macos" {
        // Apple frameworks the ghostty renderer + font stack pull in. Apple's
        // OpenGL is deprecated-but-present through current macOS; GtkGLArea on
        // the Quartz GDK backend still creates its context through it. Metal
        // migration would replace this arm. CoreText/CoreFoundation back the
        // font rasterization ghostty uses instead of fontconfig/freetype.
        println!("cargo:rustc-link-lib=framework=OpenGL");
        println!("cargo:rustc-link-lib=framework=CoreText");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        // libc++ ships with the macOS SDK; the toolchain links it by default,
        // so no explicit -lc++ / -lc++abi / -lgcc_s (gcc_s does not exist on
        // macOS — its unwinder is part of the system libunwind).
        // oniguruma comes from Homebrew; resolve its prefix rather than a
        // hardcoded Debian path. `brew --prefix oniguruma` → <prefix>/lib.
        if let Ok(out) = std::process::Command::new("brew")
            .args(["--prefix", "oniguruma"])
            .output()
        {
            let prefix = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !prefix.is_empty() && std::path::Path::new(&prefix).exists() {
                println!("cargo:rustc-link-search=native={prefix}/lib");
                println!("cargo:rustc-link-lib=dylib=onig");
            }
        } else if std::process::Command::new("pkg-config")
            .args(["--exists", "oniguruma"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            println!("cargo:rustc-link-lib=dylib=onig");
        }
    } else {
        // Linux (and other unix): the original Mesa/GL + libc++ ABI stack.
        //
        // C++ ABI: zig builds the bundled C++ deps (glslang, dcimgui,
        // SPIRV-Cross) against libc++, NOT libstdc++ — symbols are in
        // `std::__1::*`. Linking the GNU libstdc++ ABI here produces "vtable /
        // method not found" errors. Resolve by pulling in LLVM's libc++ +
        // libc++abi. On Debian/Ubuntu these come from `libc++-dev
        // libc++abi-dev`; on Fedora from `libcxx-devel libcxxabi-devel`.
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
        .generate()
        .expect("Unable to generate ghostty bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("ghostty_sys.rs"))
        .expect("Couldn't write ghostty_sys.rs");
}
