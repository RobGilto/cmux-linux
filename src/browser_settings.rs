//! Browser settings dialog: lets the user override which Chromium binary
//! cmux passes to agent-browser for the preview pane.
//!
//! Reads the current value from `~/.config/cmux/config.toml`'s
//! `[browser].chromium_path` field, displays it in an Entry, and writes back
//! a minimal patch on save. The dialog is intentionally simple — full
//! preferences live in the text editor invoked by win.preferences.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::app_state::AppState;

/// Open the browser-settings dialog parented to `parent`.
pub fn show_dialog(parent: &gtk4::ApplicationWindow, state: Rc<RefCell<AppState>>) {
    let current = state
        .borrow()
        .chromium_path_override
        .clone()
        .unwrap_or_default();
    let detected = crate::browser::bundled_chromium_path();

    let dialog = gtk4::Window::builder()
        .title("Browser Settings")
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .default_height(0)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let heading = gtk4::Label::new(Some("Chromium binary"));
    heading.set_xalign(0.0);
    heading.add_css_class("title-4");
    vbox.append(&heading);

    let hint = gtk4::Label::new(Some(
        "Path to the Chromium or Chrome executable used for the browser \
         preview pane. Leave empty to let cmux auto-detect (bundled binary, \
         then $PATH, then Flatpak wrappers).",
    ));
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    hint.add_css_class("dim-label");
    vbox.append(&hint);

    let bundled_label = gtk4::Label::new(Some(&format!(
        "Bundled binary location: {}",
        detected.display()
    )));
    bundled_label.set_xalign(0.0);
    bundled_label.add_css_class("monospace");
    bundled_label.add_css_class("dim-label");
    vbox.append(&bundled_label);

    let path_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some("/usr/bin/chromium or /path/to/chrome"));
    entry.set_text(&current);
    entry.set_hexpand(true);
    path_row.append(&entry);

    let browse_btn = gtk4::Button::with_label("Browse…");
    path_row.append(&browse_btn);
    vbox.append(&path_row);

    let status = gtk4::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    vbox.append(&status);

    let button_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    button_row.set_halign(gtk4::Align::End);
    let cancel_btn = gtk4::Button::with_label("Cancel");
    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    button_row.append(&cancel_btn);
    button_row.append(&save_btn);
    vbox.append(&button_row);

    dialog.set_child(Some(&vbox));

    // Browse handler.
    {
        let entry = entry.clone();
        let dialog_for_browse = dialog.clone();
        browse_btn.connect_clicked(move |_| {
            let chooser = gtk4::FileChooserNative::new(
                Some("Pick a Chromium binary"),
                Some(&dialog_for_browse),
                gtk4::FileChooserAction::Open,
                Some("Select"),
                Some("Cancel"),
            );
            let entry = entry.clone();
            chooser.connect_response(move |dlg, resp| {
                if resp == gtk4::ResponseType::Accept {
                    if let Some(file) = dlg.file() {
                        if let Some(path) = file.path() {
                            entry.set_text(&path.display().to_string());
                        }
                    }
                }
                dlg.destroy();
            });
            chooser.show();
        });
    }

    // Cancel handler.
    {
        let dialog_for_cancel = dialog.clone();
        cancel_btn.connect_clicked(move |_| dialog_for_cancel.close());
    }

    // Save handler: write the value into AppState (so the next BrowserManager
    // honors it without a restart) AND persist to config.toml.
    {
        let dialog_for_save = dialog.clone();
        let status_for_save = status.clone();
        let state_for_save = state.clone();
        let entry_for_save = entry.clone();
        save_btn.connect_clicked(move |_| {
            let raw = entry_for_save.text().to_string();
            let trimmed = raw.trim();
            let new_value = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };

            if let Some(ref p) = new_value {
                let pb = std::path::PathBuf::from(p);
                if !pb.is_file() {
                    status_for_save.set_text(&format!(
                        "Path is not a file: {} (saved anyway — fix before next browser open)",
                        p
                    ));
                }
            }

            state_for_save.borrow_mut().chromium_path_override = new_value.clone();

            match persist_chromium_path(new_value.as_deref()) {
                Ok(()) => {
                    status_for_save.set_text("Saved. Close any open browser pane to apply.");
                    dialog_for_save.close();
                }
                Err(e) => {
                    status_for_save.set_text(&format!("Failed to save: {}", e));
                }
            }
        });
    }

    dialog.present();
}

/// Persist the chromium-path override to `~/.config/cmux/config.toml`.
///
/// Reads the existing file, replaces (or removes) the `chromium_path` line
/// inside the `[browser]` section, and writes the result back. If no
/// `[browser]` section exists, one is appended at EOF. If the section
/// exists but contains no `chromium_path` line, the new line is inserted
/// **immediately after the section header** so we don't accidentally
/// append it under a later section that follows `[browser]`.
///
/// Atomic write via a sibling temp file + `rename`. The temp file inherits
/// the parent directory's perms; `fs::write` would otherwise create the
/// real file with `mode=0666 & ~umask` and a brief truncated state where
/// a concurrent reader could see an empty config.
fn persist_chromium_path(new_value: Option<&str>) -> std::io::Result<()> {
    let path = crate::config::config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let mut out = String::new();
    let mut in_browser_section = false;
    let mut wrote_replacement = false;

    // Closure: emit the chromium_path line once. No-op if already written
    // or if new_value is None (the user cleared the override).
    let emit = |out: &mut String, wrote: &mut bool| {
        if *wrote {
            return;
        }
        if let Some(value) = new_value {
            out.push_str(&format!(
                "chromium_path = \"{}\"\n",
                escape_toml(value)
            ));
            *wrote = true;
        }
    };

    for line in existing.lines() {
        let trimmed = line.trim();

        // Section header: detect [browser] entry/exit. When we're LEAVING
        // an open [browser] section without having written the key, inject
        // the line right before the new header. This handles the
        // empty-[browser]-then-[other] case where the section had no
        // entries to anchor an in-loop insertion.
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_browser_section && !wrote_replacement {
                emit(&mut out, &mut wrote_replacement);
            }
            in_browser_section = trimmed == "[browser]";
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Existing chromium_path inside [browser]: replace (or drop if
        // new_value is None — clearing the override).
        if in_browser_section && is_chromium_path_key(trimmed) {
            emit(&mut out, &mut wrote_replacement);
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    // EOF inside [browser] section that never had a chromium_path entry.
    if in_browser_section && !wrote_replacement {
        emit(&mut out, &mut wrote_replacement);
    }

    // No [browser] section anywhere in the file.
    if !wrote_replacement {
        if let Some(value) = new_value {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("\n[browser]\nchromium_path = \"");
            out.push_str(&escape_toml(value));
            out.push_str("\"\n");
        }
    }

    write_atomic(&path, out.as_bytes())
}

/// Return true iff the trimmed line is exactly `chromium_path` followed by
/// optional whitespace and an `=`. Prevents `chromium_path_extra` and
/// `chromium_pathfinder` from being silently dropped by the rewrite.
fn is_chromium_path_key(trimmed: &str) -> bool {
    let rest = match trimmed.strip_prefix("chromium_path") {
        Some(r) => r,
        None => return false,
    };
    let rest = rest.trim_start();
    rest.starts_with('=')
}

/// Atomic write via temp-file + rename. Preserves the existing file's
/// permission bits so users who `chmod 0600` their config.toml to keep
/// secrets out of group/world read don't silently lose that protection
/// after a settings save.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp = parent.join(format!(
        ".{}.cmux-settings.{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("config"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        // Best-effort: leave no orphan .tmp lying around next to the user's config.
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
