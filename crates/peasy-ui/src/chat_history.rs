//! A single bounded disk worker; the list holds headers, never 50 conversations.
use crate::*;
use peasy_client::chat::history::{Entry, HistoryStore, Loaded, Snapshot};

enum Job {
    Save(String, Snapshot),
    List,
    Load(String),
    Delete(String),
    Clear,
}
enum Output {
    Saved,
    Entries(Vec<Entry>),
    Loaded(Loaded),
}
type Response = Result<Output, String>;
#[derive(Clone)]
pub struct Worker {
    tx: mpsc::SyncSender<(Job, mpsc::Sender<Response>)>,
}
impl Worker {
    pub fn new(store: HistoryStore) -> Self {
        // At most one active operation and two queued compact snapshots.
        let (tx, rx) = mpsc::sync_channel::<(Job, mpsc::Sender<Response>)>(2);
        std::thread::spawn(move || {
            while let Ok((job, reply)) = rx.recv() {
                let result = match job {
                    Job::Save(id, snapshot) => store.save(&id, snapshot).map(|_| Output::Saved),
                    Job::List => store.list().map(Output::Entries),
                    Job::Load(id) => store.load(&id).map(Output::Loaded),
                    Job::Delete(id) => store.delete(&id).map(|_| Output::Saved),
                    Job::Clear => store.clear().map(|_| Output::Saved),
                };
                let _ = reply.send(result.map_err(|e| format!("{e:#}")));
            }
        });
        Self { tx }
    }
    fn send(&self, job: Job) -> Result<mpsc::Receiver<Response>> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .try_send((job, tx))
            .map_err(|_| anyhow::anyhow!("Chat history is busy; try again shortly"))?;
        Ok(rx)
    }
}
fn receive(rx: mpsc::Receiver<Response>, done: impl FnOnce(Response) + 'static) {
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(60), move || {
        let result = match rx.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err("Chat history worker stopped".into()),
        };
        done.take().unwrap()(result);
        glib::ControlFlow::Break
    });
}
fn save_job(state: &AppState) -> Result<Option<Job>> {
    let Some(snapshot) = Snapshot::capture(&state.conversation.borrow())? else {
        return Ok(None);
    };
    let id = state
        .history_id
        .borrow_mut()
        .get_or_insert_with(HistoryStore::new_id)
        .clone();
    Ok(Some(Job::Save(id, snapshot)))
}
fn save_error(state: &AppState, error: &str) {
    let message = tr_args("Could not save chat history: {error}", &[("error", error)]);
    *state.history_notice.borrow_mut() = Some(message.clone());
    if let Some(label) = state.history_notice_widget.borrow().upgrade() {
        label.set_text(&message);
        label.set_visible(true);
    }
}
pub fn autosave(state: &AppState) {
    let state = state.clone();
    let result =
        save_job(&state).and_then(|job| job.map(|job| state.history.send(job)).transpose());
    match result {
        Ok(Some(rx)) => receive(rx, move |result| {
            if let Err(error) = result {
                save_error(&state, &error);
            }
        }),
        Err(error) => save_error(&state, &error.to_string()),
        Ok(None) => {}
    }
}
fn save_then(
    window: &adw::ApplicationWindow,
    state: AppState,
    next: impl FnOnce(&adw::ApplicationWindow, AppState) + 'static,
) {
    match save_job(&state).and_then(|job| job.map(|job| state.history.send(job)).transpose()) {
        Ok(None) => next(window, state),
        Ok(Some(rx)) => {
            state.tasks.borrow_mut().close();
            show_working(window, "Saving conversation…");
            let window = window.downgrade();
            receive(rx, move |result| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                match result {
                    Ok(_) => next(&window, state),
                    Err(error) => {
                        *state.history_notice.borrow_mut() = Some(tr_args(
                            "Could not save chat history: {error}",
                            &[("error", &error)],
                        ));
                        chat_ui::show(&window, state);
                    }
                }
            });
        }
        Err(error) => {
            *state.history_notice.borrow_mut() = Some(tr_args(
                "Could not save chat history: {error}",
                &[("error", &error.to_string())],
            ));
            chat_ui::show(window, state);
        }
    }
}
fn reset(state: &AppState) {
    state.conversation.borrow_mut().turns.clear();
    state.history_id.borrow_mut().take();
    state.history_notice.borrow_mut().take();
    state.chat_pending.borrow_mut().take();
    state.chat_task_reply.set(false);
    state.chat_mode.set(peasy_client::chat::Mode::Auto);
    state.request.borrow_mut().clear();
    state.chat_attachments.borrow_mut().clear();
    state.reviewed_change.borrow_mut().take();
    if let Some(client) = state.client.borrow().as_ref() {
        client.forget_conversation();
    }
}
pub fn new_chat(window: &adw::ApplicationWindow, state: AppState) {
    save_then(window, state, |window, state| {
        reset(&state);
        chat_ui::show(window, state);
    });
}
pub fn show(window: &adw::ApplicationWindow, state: AppState) {
    save_then(window, state, list);
}
fn list(window: &adw::ApplicationWindow, state: AppState) {
    state.tasks.borrow_mut().close();
    match state.history.send(Job::List) {
        Ok(rx) => {
            show_working(window, "Loading chat history…");
            let window = window.downgrade();
            receive(rx, move |result| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                match result {
                    Ok(Output::Entries(entries)) => render(&window, state, entries),
                    Err(error) => show_error_message(&window, state, &error),
                    _ => show_error_message(&window, state, "Unexpected chat history response"),
                }
            });
        }
        Err(error) => show_error_message(window, state, &error.to_string()),
    }
}
fn render(window: &adw::ApplicationWindow, state: AppState, entries: Vec<Entry>) {
    let (root, body, header) = page_with_header("Chat history");
    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text(tr("Back to chat"))
        .build();
    header.pack_start(&back);
    let w = window.downgrade();
    let s = state.clone();
    back.connect_clicked(move |_| {
        if let Some(w) = w.upgrade() {
            chat_ui::show(&w, s.clone());
        }
    });
    let note = gtk::Label::new(Some(&tr(
        "Your last 50 chats are saved on this computer. Text and links only; reattach files or images when needed. Older chats are removed automatically.",
    )));
    note.set_wrap(true);
    note.set_xalign(0.0);
    body.append(&note);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
    if entries.is_empty() {
        rows.append(&gtk::Label::new(Some(&tr("No saved chats yet."))));
    }
    for entry in &entries {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let open = gtk::Button::new();
        open.set_hexpand(true);
        open.set_tooltip_text(Some(&entry.title));
        let details = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let title = gtk::Label::new(Some(&entry.title));
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_max_width_chars(50);
        details.append(&title);
        if let Ok(date) = glib::DateTime::from_unix_local((entry.updated / 1000) as i64) {
            let label = gtk::Label::new(date.format("%x %X").ok().as_deref());
            label.set_xalign(0.0);
            label.add_css_class("dim-label");
            details.append(&label);
        }
        open.set_child(Some(&details));
        row.append(&open);
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(tr("Delete chat"))
            .build();
        row.append(&remove);
        let w = window.downgrade();
        let s = state.clone();
        let id = entry.id.clone();
        open.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                load(&w, s.clone(), id.clone());
            }
        });
        let w = window.downgrade();
        let s = state.clone();
        let id = entry.id.clone();
        remove.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                confirm_delete(&w, s.clone(), Some(id.clone()));
            }
        });
        rows.append(&row);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&rows)
        .build();
    body.append(&scroll);
    let clear = gtk::Button::with_label(&tr("Delete all chats"));
    clear.add_css_class("destructive-action");
    clear.set_halign(gtk::Align::Start);
    clear.set_sensitive(!entries.is_empty());
    body.append(&clear);
    let w = window.downgrade();
    clear.connect_clicked(move |_| {
        if let Some(w) = w.upgrade() {
            confirm_delete(&w, state.clone(), None);
        }
    });
    show_content(window, &root, MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT);
}
fn load(window: &adw::ApplicationWindow, state: AppState, id: String) {
    match state.history.send(Job::Load(id.clone())) {
        Ok(rx) => {
            show_working(window, "Loading conversation…");
            let window = window.downgrade();
            receive(rx, move |result| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                match result {
                    Ok(Output::Loaded(loaded)) => {
                        reset(&state);
                        *state.history_id.borrow_mut() = Some(id);
                        *state.conversation.borrow_mut() = loaded.conversation;
                        if loaded.incomplete {
                            *state.history_notice.borrow_mut() = Some(tr(
                                "This saved chat has limited earlier context. Attachment contents were not saved; reattach files if needed.",
                            ));
                        }
                        chat_ui::show(&window, state);
                    }
                    Err(error) => show_error_message(&window, state, &error),
                    _ => show_error_message(&window, state, "Unexpected chat history response"),
                }
            });
        }
        Err(error) => show_error_message(window, state, &error.to_string()),
    }
}
fn confirm_delete(window: &adw::ApplicationWindow, state: AppState, id: Option<String>) {
    let dialog = adw::AlertDialog::builder()
        .heading(tr(if id.is_some() {
            "Delete chat?"
        } else {
            "Delete all chats?"
        }))
        .body(tr(
            "This removes the saved conversation text from Peasy. This cannot be undone.",
        ))
        .build();
    dialog.add_response("cancel", &tr("Cancel"));
    dialog.add_response("delete", &tr("Delete"));
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let w = window.downgrade();
    dialog.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }
        let Some(window) = w.upgrade() else {
            return;
        };
        let clears_current = id.is_none() || state.history_id.borrow().as_ref() == id.as_ref();
        let job = id.clone().map_or(Job::Clear, Job::Delete);
        match state.history.send(job) {
            Ok(rx) => {
                show_working(&window, "Deleting chat history…");
                let w = window.downgrade();
                let state = state.clone();
                receive(rx, move |result| {
                    let Some(window) = w.upgrade() else {
                        return;
                    };
                    match result {
                        Ok(_) => {
                            if clears_current {
                                reset(&state);
                            }
                            list(&window, state);
                        }
                        Err(error) => show_error_message(&window, state, &error),
                    }
                });
            }
            Err(error) => show_error_message(&window, state.clone(), &error.to_string()),
        }
    });
    dialog.present(Some(window));
}
