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

mod chat;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use chat::{SessionStats, Transcript};

use gguf_engine_protocol::pb;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, Entry, Label, ListBox, ListBoxRow,
    Orientation, ScrolledWindow, Stack, StackSwitcher,
};

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

/// Events a chat task sends back to the GTK loop.
enum ChatEvent {
    /// A token arrived.
    Delta(String),
    /// The stream ended normally.
    Done(SessionStats),
    /// The stream failed.
    Error(String),
    /// The user stopped it.
    Stopped,
}

/// Build the chat page.
///
/// Returns the page plus the widgets the caller needs to keep alive. The
/// streaming task talks to the UI over an `async_channel`: GTK widgets may only
/// be touched on the main loop, so the worker thread never holds one.
fn build_chat_page(
    runtime: &'static tokio::runtime::Runtime,
) -> (GtkBox, Entry, Entry, Button, Button, Label, ListBox) {
    let model_entry = Entry::new();
    model_entry.set_placeholder_text(Some("repository id, e.g. ggml-org/models"));
    model_entry.set_hexpand(true);

    let model_row = GtkBox::new(Orientation::Horizontal, 8);
    let model_label = Label::new(Some("Model"));
    model_row.append(&model_label);
    model_row.append(&model_entry);

    let transcript_list = ListBox::new();
    let transcript_scroll = ScrolledWindow::new();
    transcript_scroll.set_child(Some(&transcript_list));
    transcript_scroll.set_vexpand(true);

    let prompt_entry = Entry::new();
    prompt_entry.set_placeholder_text(Some("Ask something..."));
    prompt_entry.set_hexpand(true);

    let send_button = Button::with_label("Send");
    send_button.add_css_class("suggested-action");
    let stop_button = Button::with_label("Stop");
    stop_button.set_sensitive(false);
    let clear_button = Button::with_label("Clear");

    let input_row = GtkBox::new(Orientation::Horizontal, 8);
    input_row.append(&prompt_entry);
    input_row.append(&send_button);
    input_row.append(&stop_button);
    input_row.append(&clear_button);

    let status = Label::new(Some("Choose a model that is already downloaded."));
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    status.set_wrap(true);

    let page = GtkBox::new(Orientation::Vertical, 8);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page.append(&model_row);
    page.append(&transcript_scroll);
    page.append(&input_row);
    page.append(&status);

    let (tx, rx) = async_channel::unbounded::<ChatEvent>();
    let transcript = Rc::new(RefCell::new(Transcript::new()));
    let current_turn: Rc<RefCell<Option<Label>>> = Rc::new(RefCell::new(None));
    let running: Rc<RefCell<Option<tokio::task::JoinHandle<()>>>> = Rc::new(RefCell::new(None));
    let root = data_root();

    // Send.
    {
        let transcript = transcript.clone();
        let current_turn = current_turn.clone();
        let running = running.clone();
        let list = transcript_list.clone();
        let model_entry = model_entry.clone();
        let prompt_entry = prompt_entry.clone();
        let send_button = send_button.clone();
        let stop_button = stop_button.clone();
        let status = status.clone();
        let tx = tx.clone();
        let scroller = transcript_scroll.clone();
        let root = root.clone();

        send_button.clone().connect_clicked(move |_| {
            let model = model_entry.text().to_string();
            let prompt = prompt_entry.text().to_string();

            let request = {
                let transcript = transcript.borrow();
                match transcript.build_request(&model, &prompt, 256, 0.7) {
                    Ok(request) => request,
                    Err(err) => {
                        status.set_text(&err.to_string());
                        return;
                    }
                }
            };

            // The prompt is committed to the history only now that a request
            // exists, and the pending copy is not stored twice.
            let _ = transcript.borrow_mut().add_user(&prompt);
            prompt_entry.set_text("");

            add_row(&list, "You", &prompt);
            let answer = Label::new(Some(""));
            answer.set_xalign(0.0);
            answer.set_wrap(true);
            let answer_row = ListBoxRow::new();
            let answer_box = GtkBox::new(Orientation::Vertical, 2);
            answer_box.set_margin_top(6);
            answer_box.set_margin_bottom(6);
            answer_box.set_margin_start(12);
            answer_box.set_margin_end(12);
            let speaker = Label::new(Some("Assistant"));
            speaker.set_xalign(0.0);
            speaker.add_css_class("dim-label");
            answer_box.append(&speaker);
            answer_box.append(&answer);
            answer_row.set_child(Some(&answer_box));
            list.append(&answer_row);
            *current_turn.borrow_mut() = Some(answer.clone());

            send_button.set_sensitive(false);
            stop_button.set_sensitive(true);
            status.set_text("Generating...");

            let tx = tx.clone();
            let root = root.clone();
            let handle = runtime.spawn(async move {
                let client = match gguf_engine_ipc::connect(&root).await {
                    Ok(client) => client,
                    Err(err) => {
                        let _ = tx.send(ChatEvent::Error(format!("no core: {err}"))).await;
                        return;
                    }
                };

                let mut stream = match client.chat(request).await {
                    Ok(stream) => stream,
                    Err(err) => {
                        let _ = tx.send(ChatEvent::Error(err.to_string())).await;
                        return;
                    }
                };

                use futures_util::StreamExt as _;
                let mut stats = SessionStats::default();
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(chunk) => {
                            if chunk.done {
                                stats.completed = true;
                                break;
                            }
                            if stats.record_delta(&chunk.delta) {
                                let _ = tx.send(ChatEvent::Delta(chunk.delta)).await;
                            }
                        }
                        Err(err) => {
                            let _ = tx.send(ChatEvent::Error(err.to_string())).await;
                            return;
                        }
                    }
                }
                let _ = tx.send(ChatEvent::Done(stats)).await;
            });

            *running.borrow_mut() = Some(handle);
            let _ = scroller;
        });
    }

    // Stop: aborting the task drops the stream, and the core unloads the model.
    {
        let running = running.clone();
        let current_turn = current_turn.clone();
        let transcript = transcript.clone();
        let send_button = send_button.clone();
        let stop_button = stop_button.clone();
        let status = status.clone();
        let tx = tx.clone();

        stop_button.clone().connect_clicked(move |_| {
            if let Some(handle) = running.borrow_mut().take() {
                handle.abort();
            }
            let partial = current_turn
                .borrow()
                .as_ref()
                .map(|label| label.text().to_string())
                .unwrap_or_default();
            transcript.borrow_mut().add_assistant(&partial);

            send_button.set_sensitive(true);
            stop_button.set_sensitive(false);
            status.set_text("Stopped; the partial answer was kept.");
            let _ = tx.try_send(ChatEvent::Stopped);
        });
    }

    // Clear: forget the conversation and what is on screen.
    {
        let transcript = transcript.clone();
        let current_turn = current_turn.clone();
        let list = transcript_list.clone();
        let status = status.clone();

        clear_button.connect_clicked(move |_| {
            transcript.borrow_mut().clear();
            clear(&list);
            *current_turn.borrow_mut() = None;
            status.set_text("Conversation cleared.");
        });
    }

    // Drain the worker's events on the GTK loop.
    {
        let current_turn = current_turn.clone();
        let transcript = transcript.clone();
        let running = running.clone();
        let send_button = send_button.clone();
        let stop_button = stop_button.clone();
        let status = status.clone();
        let model_entry = model_entry.clone();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = rx.recv().await {
                match event {
                    ChatEvent::Delta(delta) => {
                        if let Some(label) = current_turn.borrow().as_ref() {
                            label.set_text(&format!("{}{}", label.text(), delta));
                        }
                    }
                    ChatEvent::Done(stats) => {
                        let partial = current_turn
                            .borrow()
                            .as_ref()
                            .map(|label| label.text().to_string())
                            .unwrap_or_default();
                        transcript.borrow_mut().add_assistant(&partial);
                        running.borrow_mut().take();
                        send_button.set_sensitive(true);
                        stop_button.set_sensitive(false);
                        let turns = transcript.borrow().messages().len();
                        status.set_text(&format!("{} tokens; {turns} turns in context.", stats.chunks));
                    }
                    ChatEvent::Error(message) => {
                        running.borrow_mut().take();
                        send_button.set_sensitive(true);
                        stop_button.set_sensitive(false);
                        status.set_text(&format!("Failed: {message}"));
                    }
                    ChatEvent::Stopped => {}
                }
                let _ = model_entry;
            }
        });
    }

    (
        page,
        model_entry,
        prompt_entry,
        send_button,
        stop_button,
        status,
        transcript_list,
    )
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

    // The chat task needs a runtime that outlives every request. It is leaked on
    // purpose: its lifetime is the process's, and dropping it would abort
    // in-flight streams while the window is still open.
    let runtime: &'static tokio::runtime::Runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("building the tokio runtime"),
    ));

    let (chat_page, _model, _prompt, _send, _stop, _status, _list) = build_chat_page(runtime);

    stack.add_titled(&overview_page, Some("overview"), "Overview");
    stack.add_titled(&chat_page, Some("chat"), "Chat");
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