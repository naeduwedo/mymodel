//! GTK4 shell for GGUF AI Engine (spec §2.1, §4).
//!
//! Same contract as the Windows shell: connect to the core over the local
//! transport and render the snapshot from `proto/ui.proto`. Every label,
//! severity and formatted value comes from the core, so the two shells cannot
//! describe the same core state differently.
//!
//! STATUS: builds and runs on Linux (GTK 4.22, Ubuntu 26.04, Rust 1.100).
//! `platforms/linux/verify.sh` starts a real core, launches this shell under
//! Xvfb, and fails unless the window survives eight seconds without logging a
//! critical error. The chat page is not implemented here yet; Overview and
//! Settings are.

use std::path::PathBuf;
use std::time::Duration;

use gguf_engine_protocol::pb;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Box as GtkBox, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow, Stack, StackSwitcher};

const APP_ID: &str = "engine.gguf.Shell";

/// Data root, resolved the same way the core does (spec §15).
fn data_root() -> PathBuf {
    if let Ok(value) = std::env::var("GGUF_AI_ENGINE_DATA_DIR") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home).join(".local/share/gguf-ai-engine")
}

fn severity_mark(severity: i32) -> &'static str {
    match pb::UiSeverity::try_from(severity) {
        Ok(pb::UiSeverity::Ok) => "OK",
        Ok(pb::UiSeverity::Warning) => "!",
        Ok(pb::UiSeverity::Error) => "FAIL",
        Ok(pb::UiSeverity::Disabled) => "-",
        _ => "?",
    }
}

fn clear(list: &ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

/// Append a two-line row: bold-ish title plus a wrapped detail line.
fn add_row(list: &ListBox, title: &str, detail: &str) {
    let row = ListBoxRow::new();
    let container = GtkBox::new(Orientation::Vertical, 2);
    container.set_margin_top(6);
    container.set_margin_bottom(6);
    container.set_margin_start(12);
    container.set_margin_end(12);

    let title_label = Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    container.append(&title_label);

    if !detail.is_empty() {
        let detail_label = Label::new(Some(detail));
        detail_label.set_xalign(0.0);
        detail_label.set_wrap(true);
        detail_label.add_css_class("dim-label");
        container.append(&detail_label);
    }

    row.set_child(Some(&container));
    list.append(&row);
}

fn render(tiles: &ListBox, features: &ListBox, settings: &ListBox, status: &Label, snapshot: &pb::UiSnapshot) {
    let overview = snapshot.overview.clone().unwrap_or_default();
    let settings_data = snapshot.settings.clone().unwrap_or_default();

    clear(tiles);
    for tile in &overview.tiles {
        let detail = if tile.detail.is_empty() {
            tile.value.clone()
        } else {
            format!("{}\n{}", tile.value, tile.detail)
        };
        add_row(
            tiles,
            &format!("[{}] {}", severity_mark(tile.severity), tile.label),
            &detail,
        );
    }

    clear(features);
    for feature in &overview.features {
        add_row(
            features,
            &format!("[{}] {} - {}", severity_mark(feature.severity), feature.label, feature.status_text),
            &feature.reason,
        );
    }

    clear(settings);
    for group in &settings_data.groups {
        add_row(settings, &group.title, "");
        for row in &group.rows {
            add_row(
                settings,
                &format!("{}: {}", row.label, row.value),
                &row.note,
            );
        }
    }

    status.set_text(&format!(
        "Snapshot {} - {} tile(s), {} capability row(s)",
        snapshot.generated_at_local,
        overview.tiles.len(),
        overview.features.len()
    ));
}

fn build_ui(app: &Application) {
    let stack = Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    let tiles = ListBox::new();
    let features = ListBox::new();
    let settings = ListBox::new();

    let overview_page = GtkBox::new(Orientation::Vertical, 8);
    let tiles_scroll = ScrolledWindow::new();
    tiles_scroll.set_child(Some(&tiles));
    tiles_scroll.set_vexpand(true);
    overview_page.append(&tiles_scroll);

    let capability_heading = Label::new(Some("Capabilities"));
    capability_heading.set_xalign(0.0);
    overview_page.append(&capability_heading);
    let features_scroll = ScrolledWindow::new();
    features_scroll.set_child(Some(&features));
    features_scroll.set_vexpand(true);
    overview_page.append(&features_scroll);

    let settings_scroll = ScrolledWindow::new();
    settings_scroll.set_child(Some(&settings));
    settings_scroll.set_vexpand(true);

    stack.add_titled(&overview_page, Some("overview"), "Overview");
    stack.add_titled(&settings_scroll, Some("settings"), "Settings");

    let switcher = StackSwitcher::new();
    switcher.set_stack(Some(&stack));

    let status = Label::new(Some("Waiting for the core..."));
    status.set_xalign(0.0);

    let root = GtkBox::new(Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);
    root.append(&switcher);
    root.append(&stack);
    root.append(&status);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("GGUF AI Engine")
        .default_width(960)
        .default_height(720)
        .child(&root)
        .build();
    window.present();

    poll_forever(tiles, features, settings, status);
}

/// Poll the core off the UI thread and render each snapshot on the GTK loop.
///
/// A dropped connection is a state, not an error dialog: the status line says
/// the core is not running and the last snapshot stays on screen.
fn poll_forever(tiles: ListBox, features: ListBox, settings: ListBox, status: Label) {
    let (tx, rx) = async_channel::unbounded::<Option<pb::UiSnapshot>>();
    let root = data_root();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return,
        };

        runtime.block_on(async move {
            loop {
                let snapshot = match gguf_engine_ipc::connect(&root).await {
                    Ok(client) => client.snapshot().await.ok(),
                    Err(_) => None,
                };
                if tx.send(snapshot).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    });

    glib::MainContext::default().spawn_local(async move {
        while let Ok(message) = rx.recv().await {
            match message {
                Some(snapshot) => render(&tiles, &features, &settings, &status, &snapshot),
                None => status
                    .set_text("Core not connected. Start it with: gguf-core --data-dir <DIR>"),
            }
        }
    });
}

fn main() {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run();
}