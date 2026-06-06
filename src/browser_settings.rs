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
/// Uses a minimal read-merge-write because we want to preserve any other
/// sections the user already authored (shortcuts, ui, …).
fn persist_chromium_path(new_value: Option<&str>) -> std::io::Result<()> {
    let path = crate::config::config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    // Strip any prior `chromium_path` line under `[browser]` so we don't
    // emit duplicates. Cheap line filter, not a full TOML rewrite.
    let mut out = String::new();
    let mut in_browser_section = false;
    let mut wrote_replacement = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_browser_section = trimmed == "[browser]";
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_browser_section
            && trimmed.starts_with("chromium_path")
            && trimmed.contains('=')
        {
            if let Some(value) = new_value {
                out.push_str(&format!("chromium_path = \"{}\"\n", escape_toml(value)));
                wrote_replacement = true;
            }
            // else: drop the line (clearing the override).
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }

    if !wrote_replacement {
        if let Some(value) = new_value {
            if !out.contains("[browser]") {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("\n[browser]\n");
            }
            out.push_str(&format!("chromium_path = \"{}\"\n", escape_toml(value)));
        }
    }

    std::fs::write(&path, out)
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
