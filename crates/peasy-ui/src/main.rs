mod chat_format;
mod chat_history;
mod chat_ui;
use peasy_core::i18n::{tr, tr_args};
mod ollama_models;
mod update;
use adw::prelude::*;
use anyhow::{Context, Result};
use clap::Parser;
use gtk::glib;
use peasy_client::{
    Choice, DEFAULT_OPENAI_MODEL, KeyStore, LocalAction, LocalProposal, PeasyClient,
    ProviderSettings, ProviderStore, Resolution, ResolveStage, load_model_provider,
};
use peasy_core::{DiffKind, IpcRequest, IpcResponse, OperationStage, Proposal, ProposalChange};
mod restore;
mod tasks;
use std::cell::{Cell, RefCell};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

#[derive(Clone, Debug, Parser)]
#[command(name = "peasy-ui")]
struct Args {
    #[arg(long, default_value = "/run/peasy/peasy.sock", hide = true)]
    socket: PathBuf,
    #[arg(long, hide = true)]
    engine: Option<PathBuf>,
    #[arg(long)]
    settings: bool,
}

type ChatPending = Option<(String, Vec<peasy_client::chat::attachments::Attachment>)>;

#[derive(Clone)]
struct AppState {
    args: Args,
    keys: KeyStore,
    providers: ProviderStore,
    client: Rc<RefCell<Option<Arc<PeasyClient>>>>,
    tasks: Rc<RefCell<tasks::Tasks>>,
    pending: Rc<RefCell<Option<String>>>,
    applying: Rc<Cell<bool>>,
    closing_apply: Rc<Cell<bool>>,
    request: Rc<RefCell<String>>,
    reviewed_change: Rc<RefCell<Option<ProposalChange>>>,
    conversation: Rc<RefCell<peasy_client::chat::Conversation>>,
    history: chat_history::Worker,
    history_id: Rc<RefCell<Option<String>>>,
    history_notice: Rc<RefCell<Option<String>>>,
    history_notice_widget: Rc<RefCell<glib::WeakRef<gtk::Label>>>,
    chat_mode: Rc<Cell<peasy_client::chat::Mode>>,
    chat_task_reply: Rc<Cell<bool>>,
    chat_pending: Rc<RefCell<ChatPending>>,
    chat_attachments: Rc<RefCell<Vec<peasy_client::chat::attachments::Attachment>>>,
    return_to_ollama_settings: bool,
}

enum ResolveMessage {
    Progress(ResolveStage),
    Finished(std::result::Result<Box<Resolution>, String>),
}

enum ApplyMessage {
    Resumed(Box<Resolution>),
    Progress(OperationStage),
    Finished(std::result::Result<peasy_core::ApplyResult, String>),
}

fn main() -> Result<()> {
    let args = Args::parse();
    let keys = KeyStore::discover()?;
    let providers = ProviderStore::discover()?;
    let engine = args
        .engine
        .clone()
        .or_else(|| std::env::var_os("PEASY_ENGINE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/run/current-system/sw/lib/peasy/peasy-engine.wasm"));
    let initial = load_model_provider(&providers, &keys)
        .unwrap_or(None)
        .map(|provider| PeasyClient::with_provider(args.socket.clone(), &engine, provider))
        .transpose()?;
    let state = AppState {
        args,
        keys,
        providers,
        client: Rc::new(RefCell::new(initial.map(Arc::new))),
        tasks: Default::default(),
        pending: Default::default(),
        applying: Default::default(),
        closing_apply: Default::default(),
        request: Default::default(),
        reviewed_change: Default::default(),
        conversation: Default::default(),
        history: chat_history::Worker::new(peasy_client::chat::history::HistoryStore::discover()?),
        history_id: Default::default(),
        history_notice: Default::default(),
        history_notice_widget: Default::default(),
        chat_mode: Default::default(),
        chat_task_reply: Default::default(),
        chat_pending: Default::default(),
        chat_attachments: Default::default(),
        return_to_ollama_settings: false,
    };
    let application_id = if state.args.settings {
        "io.github.peasy.Peasy.Settings"
    } else {
        "io.github.peasy.Peasy"
    };
    let app = adw::Application::builder()
        .application_id(application_id)
        .build();
    app.connect_activate(move |app| activate(app, state.clone()));
    // Clap already consumed our flags; GTK must not parse --settings/--socket again.
    app.run_with_args(&["peasy-ui"]);
    Ok(())
}

fn activate(app: &adw::Application, state: AppState) {
    gtk::Widget::set_default_direction(if peasy_core::i18n::is_rtl() {
        gtk::TextDirection::Rtl
    } else {
        gtk::TextDirection::Ltr
    });
    if let Some(window) = app.windows().into_iter().next() {
        let window = window
            .downcast::<adw::ApplicationWindow>()
            .expect("Peasy owns an Adwaita application window");
        if state.applying.get() {
            window.present();
            return;
        }
        if state.args.settings || state.client.borrow().is_none() {
            show_provider_settings(&window, state);
        } else {
            show_prompt(&window, state);
        }
        window.present();
        return;
    }
    let _hold = app.hold();
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Peasy")
        .default_width(440)
        .resizable(false)
        .build();
    let close_state = state.clone();
    window.connect_close_request(move |window| {
        close_state.tasks.borrow_mut().close();
        if close_state.applying.get() {
            request_apply_cancellation(window, close_state.clone());
        } else {
            close_state.chat_pending.borrow_mut().take();
            discard_pending(&close_state);
            clear_panel_status();
        }
        window.set_visible(false);
        glib::Propagation::Stop
    });
    if state.args.settings || state.client.borrow().is_none() {
        show_provider_settings(&window, state);
    } else {
        show_prompt(&window, state);
    }
    window.present();
}

fn discard_pending(state: &AppState) {
    if let Some(token) = state.pending.borrow_mut().take() {
        discard_token(state, token);
    }
}

fn discard_token(state: &AppState, token: String) {
    if let Some(client) = state.client.borrow().as_ref() {
        client.discard_continuation(&token);
    }
    let ipc = peasy_client::IpcClient::new(state.args.socket.clone());
    std::thread::spawn(move || {
        let _ = ipc.request(&IpcRequest::Cancel { proposal: token });
    });
}

fn request_apply_cancellation(window: &adw::ApplicationWindow, state: AppState) {
    if state.closing_apply.replace(true) {
        return;
    }
    let Some(token) = state.pending.borrow().clone() else {
        return;
    };
    if let Some(client) = state.client.borrow().as_ref() {
        client.discard_continuation(&token);
    }
    let ipc = peasy_client::IpcClient::new(state.args.socket.clone());
    show_working(
        window,
        "Requesting cancellation… If activation has started, it will finish safely in the background.",
    );
    let (tx, rx) = mpsc::channel();
    let expected_token = token.clone();
    std::thread::spawn(move || {
        let _ = tx.send(
            ipc.request(&IpcRequest::Cancel { proposal: token })
                .and_then(|r| match r {
                    IpcResponse::Cancelled { activation_started } => Ok(activation_started),
                    _ => anyhow::bail!("unexpected cancellation response"),
                }),
        );
    });
    let window = window.clone();
    glib::timeout_add_local(Duration::from_millis(80), move || {
        if !state.applying.get() || state.pending.borrow().as_ref() != Some(&expected_token) {
            return glib::ControlFlow::Break;
        }
        match rx.try_recv() {
            Ok(result) => {
                let message = match result {
                    Ok(false) => "Cancelling the build and restoring the previous configuration…",
                    Ok(true) => {
                        "System activation has already started. It will finish safely in the background."
                    }
                    Err(_) => {
                        "Cancellation could not be confirmed. Waiting for the operation to finish safely…"
                    }
                };
                show_working(&window, message);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

fn page(title: &str) -> (gtk::Box, gtk::Box) {
    let (root, body, _) = page_with_header(title);
    (root, body)
}

fn page_with_header(title: &str) -> (gtk::Box, gtk::Box, adw::HeaderBar) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_direction(if peasy_core::i18n::is_rtl() {
        gtk::TextDirection::Rtl
    } else {
        gtk::TextDirection::Ltr
    });
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Label::new(Some(&tr(title)))));
    root.append(&header);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(24);
    body.set_margin_bottom(24);
    body.set_margin_start(28);
    body.set_margin_end(28);
    body.set_vexpand(true);
    root.append(&body);
    (root, body, header)
}

fn show_content(window: &adw::ApplicationWindow, root: &gtk::Box, width: i32, height: i32) {
    window.set_content(Some(root));
    window.set_default_size(width, height);
}

const OPENAI_PROVIDER_INDEX: u32 = 0;
const OLLAMA_PROVIDER_INDEX: u32 = 1;
const MAIN_WINDOW_WIDTH: i32 = 680;
const MAIN_WINDOW_HEIGHT: i32 = 760;

fn initial_provider_selection(settings: Option<&ProviderSettings>, has_stored_key: bool) -> u32 {
    match settings {
        Some(ProviderSettings::OpenAi { .. }) => OPENAI_PROVIDER_INDEX,
        Some(ProviderSettings::Ollama { .. }) => OLLAMA_PROVIDER_INDEX,
        // Older installations may have an OpenAI key but no provider.json.
        None if has_stored_key => OPENAI_PROVIDER_INDEX,
        None => OLLAMA_PROVIDER_INDEX,
    }
}

fn show_provider_settings(window: &adw::ApplicationWindow, mut state: AppState) {
    let return_to_ollama = state.return_to_ollama_settings;
    state.return_to_ollama_settings = false;
    state.tasks.borrow_mut().close();
    discard_pending(&state);
    let (root, body) = page("Peasy settings");
    let pages = gtk::Stack::builder()
        .vexpand(true)
        .vhomogeneous(false)
        .hhomogeneous(false)
        .build();
    let switcher = gtk::StackSwitcher::builder()
        .stack(&pages)
        .halign(gtk::Align::Center)
        .build();
    body.append(&switcher);
    body.append(&pages);
    let provider_page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    let system_page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    pages.add_titled(&provider_page, Some("provider"), &tr("AI provider"));
    pages.add_titled(&system_page, Some("system"), &tr("Backups and updates"));
    add_system_status_button(&system_page, window, &state);

    let provider = gtk::DropDown::from_strings(&["OpenAI", &tr("Ollama (local)")]);
    let settings = state.providers.load().ok().flatten();
    let has_stored_key = state.keys.load().ok().flatten().is_some();
    provider.set_selected(initial_provider_selection(
        settings.as_ref(),
        has_stored_key,
    ));
    if return_to_ollama {
        provider.set_selected(1);
    }
    provider_page.append(&provider);

    let openai_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let key_entry = gtk::PasswordEntry::builder()
        .placeholder_text(tr("Enter a new OpenAI API key"))
        .show_peek_icon(true)
        .build();
    openai_box.append(&key_entry);
    let key_note = gtk::Label::new(Some(&tr(if has_stored_key {
        "A key is stored privately. Leave this empty to keep it."
    } else {
        "Your key is stored privately for this desktop user."
    })));
    key_note.set_wrap(true);
    key_note.set_halign(gtk::Align::Start);
    openai_box.append(&key_note);
    let openai_model = gtk::Entry::builder()
        .placeholder_text(tr("OpenAI model"))
        .text(DEFAULT_OPENAI_MODEL)
        .build();
    openai_box.append(&openai_model);
    let remove_key = gtk::Button::with_label(&tr("Remove stored OpenAI key"));
    remove_key.add_css_class("destructive-action");
    remove_key.set_halign(gtk::Align::Start);
    remove_key.set_sensitive(has_stored_key);
    openai_box.append(&remove_key);

    let ollama_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let endpoint = gtk::Label::new(Some(&tr("Local Ollama · http://127.0.0.1:11434")));
    endpoint.set_halign(gtk::Align::Start);
    ollama_box.append(&endpoint);
    let saved_model = match settings.as_ref() {
        Some(ProviderSettings::Ollama { model, .. }) => Some(model.clone()),
        _ => None,
    };
    let ollama_selector = ollama_models::ModelSelector::new(window, &state, saved_model);
    ollama_box.append(ollama_selector.widget());

    let stack = gtk::Stack::new();
    stack.set_vhomogeneous(false);
    stack.set_hhomogeneous(false);
    stack.set_vexpand(false);
    stack.set_valign(gtk::Align::Start);
    stack.add_named(&openai_box, Some("openai"));
    stack.add_named(&ollama_box, Some("ollama"));
    provider_page.append(&stack);

    if let Some(settings) = settings {
        match settings {
            ProviderSettings::OpenAi { model } => openai_model.set_text(&model),
            ProviderSettings::Ollama { .. } => {}
        }
    }
    stack.set_visible_child_name(if provider.selected() == OLLAMA_PROVIDER_INDEX {
        "ollama"
    } else {
        "openai"
    });
    if provider.selected() == OLLAMA_PROVIDER_INDEX {
        ollama_selector.activate();
    }
    let selector_for_provider = ollama_selector.clone();
    let stack_clone = stack.clone();
    provider.connect_selected_notify(move |provider| {
        stack_clone.set_visible_child_name(if provider.selected() == OLLAMA_PROVIDER_INDEX {
            selector_for_provider.activate();
            "ollama"
        } else {
            selector_for_provider.deactivate();
            "openai"
        });
    });

    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.set_margin_top(4);
    separator.set_margin_bottom(4);
    system_page.append(&separator);
    let export_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    export_row.set_valign(gtk::Align::Center);
    let export_copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
    export_copy.set_hexpand(true);
    let export_text = gtk::Label::new(Some(&tr(
        "Back up Peasy software and appearance settings for another NixOS machine, keeping its hardware configuration. Supports traditional and flake hosts.",
    )));
    export_text.set_wrap(true);
    export_text.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    export_copy.append(&export_text);
    let export_note = gtk::Label::new(Some(&tr(
        "Service setups, network profiles and AppImages need review on the destination. Original host files are archived separately when readable.",
    )));
    export_note.set_wrap(true);
    export_note.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    export_note.add_css_class("dim-label");
    export_copy.append(&export_note);
    export_row.append(&export_copy);
    let download_config = gtk::Button::with_label(&tr("Export backup"));
    download_config.set_valign(gtk::Align::Center);
    let export_actions = gtk::Box::new(gtk::Orientation::Vertical, 6);
    export_actions.set_valign(gtk::Align::Center);
    export_actions.append(&download_config);
    export_row.append(&export_actions);
    let restore_backup = gtk::Button::with_label(&tr("Restore backup"));
    restore_backup.set_valign(gtk::Align::Center);
    export_actions.append(&restore_backup);
    let restore_window = window.clone();
    let restore_state = state.clone();
    restore_backup.connect_clicked(move |_| choose_backup(&restore_window, restore_state.clone()));
    system_page.append(&export_row);
    update::add_controls(&system_page, window, &state);

    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_halign(gtk::Align::Start);
    body.append(&status);

    let export_window = window.clone();
    let export_status = status.clone();
    let export_socket = state.args.socket.clone();
    download_config.connect_clicked(move |_| {
        let export = match configuration_export(&export_socket) {
            Ok(export) => export,
            Err(error) => {
                export_status.set_text(&tr_args("Could not prepare config: {error}", &[("error", &format!("{error:#}"))]));
                return;
            }
        };
        let dialog = gtk::FileDialog::builder()
            .title(tr("Choose where to save your Peasy backup"))
            .accept_label(tr("Export here"))
            .build();
        let status = export_status.clone();
        dialog.select_folder(
            Some(&export_window),
            None::<&gtk::gio::Cancellable>,
            move |result| match result {
                Ok(folder) => match folder.path() {
                    Some(folder) => match write_configuration_export(&export, &folder) {
                        Ok(path) => status.set_text(&tr_args("Peasy backup exported to {path}. Use Restore backup to review and apply it. Read README.txt for scope and archive availability.", &[("path", &path.display().to_string())])),
                        Err(error) => {
                            status.set_text(&tr_args("Could not export system: {error}", &[("error", &format!("{error:#}"))]))
                        }
                    },
                    None => status.set_text(&tr("Choose a local folder for the system export.")),
                },
                Err(error)
                    if error.matches(gtk::DialogError::Cancelled)
                        || error.matches(gtk::DialogError::Dismissed) => {}
                Err(error) => status.set_text(&tr_args("Could not open Save dialog: {error}", &[("error", &format!("{error:#}"))])),
            },
        );
    });
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    if state.client.borrow().is_some() {
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        let window_clone = window.clone();
        let state_clone = state.clone();
        cancel.connect_clicked(move |_| {
            if state_clone.client.borrow().is_some() {
                show_prompt(&window_clone, state_clone.clone());
            } else {
                show_provider_settings(&window_clone, state_clone.clone());
            }
        });
        buttons.append(&cancel);
    }
    let save = gtk::Button::with_label(&tr("Save provider"));
    save.add_css_class("suggested-action");
    buttons.append(&save);
    body.append(&buttons);
    let save_for_tab = save.clone();
    pages.connect_visible_child_name_notify(move |pages| {
        save_for_tab.set_visible(pages.visible_child_name().as_deref() == Some("provider"));
    });

    let keys_for_remove = state.keys.clone();
    let providers_for_remove = state.providers.clone();
    let client_for_remove = state.client.clone();
    let status_for_remove = status.clone();
    remove_key.connect_clicked(move |button| match keys_for_remove.remove() {
        Ok(()) => {
            button.set_sensitive(false);
            if !matches!(
                providers_for_remove.load(),
                Ok(Some(ProviderSettings::Ollama { .. }))
            ) {
                *client_for_remove.borrow_mut() = None;
            }
            status_for_remove.set_text(&tr("Stored OpenAI key removed."));
        }
        Err(error) => status_for_remove.set_text(&format!("{error:#}")),
    });

    let window_clone = window.clone();
    save.connect_clicked(move |_| {
        let result = (|| -> Result<()> {
            let settings = if provider.selected() == OLLAMA_PROVIDER_INDEX {
                let model = ollama_selector.selected_model().context(tr(
                    "Choose an installed model or wait for its download to finish.",
                ))?;
                ProviderSettings::ollama(model)?
            } else {
                if !key_entry.text().trim().is_empty() {
                    state.keys.save(key_entry.text().as_str())?;
                }
                state
                    .keys
                    .load()?
                    .context("Enter an OpenAI API key before selecting OpenAI")?;
                ProviderSettings::OpenAi {
                    model: openai_model.text().trim().to_owned(),
                }
            };
            state.providers.save(&settings)?;
            let model_provider = load_model_provider(&state.providers, &state.keys)?
                .context("AI provider was not saved")?;
            let client = PeasyClient::with_provider(
                state.args.socket.clone(),
                &engine_path(&state.args),
                model_provider,
            )?;
            *state.client.borrow_mut() = Some(Arc::new(client));
            Ok(())
        })();
        match result {
            Ok(()) => show_prompt(&window_clone, state.clone()),
            Err(error) => status.set_text(&format!("{error:#}")),
        }
    });
    show_content(window, &root, MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT);
}

fn engine_path(args: &Args) -> PathBuf {
    args.engine
        .clone()
        .or_else(|| std::env::var_os("PEASY_ENGINE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/run/current-system/sw/lib/peasy/peasy-engine.wasm"))
}

mod export;
use export::{configuration_export, write_configuration_export};

fn show_prompt(window: &adw::ApplicationWindow, state: AppState) {
    if state.return_to_ollama_settings {
        show_provider_settings(window, state);
        return;
    }
    chat_ui::show(window, state);
}

// Main requests and inline replies use the same worker, progress and cancellation path.
fn connect_request_submit(
    window: &adw::ApplicationWindow,
    state: &AppState,
    entry: &gtk::Entry,
    send: &gtk::Button,
    status: &gtk::Label,
) {
    let window_clone = window.clone();
    let state_clone = state.clone();
    let entry_clone = entry.clone();
    let status_clone = status.clone();
    send.connect_clicked(move |button| {
        if !button.is_sensitive() {
            return;
        }
        let request = entry_clone.text().trim().to_owned();
        if request.is_empty() {
            return;
        }
        *state_clone.request.borrow_mut() = request.clone();
        state_clone.reviewed_change.borrow_mut().take();
        button.set_sensitive(false);
        status_clone.set_text(&tr("Understanding request…"));
        write_panel_status("…thinking");
        let Some(client) = state_clone.client.borrow().clone() else {
            status_clone.set_text(&tr("AI provider is not configured. Open Peasy settings."));
            return;
        };
        let (tx, rx) = mpsc::channel();
        let task = state_clone.tasks.borrow_mut().start();
        std::thread::spawn(move || {
            task.work.scope(|| {
                let progress_tx = tx.clone();
                let result = client
                    .resolve_with_progress(&request, move |stage| {
                        let _ = progress_tx.send(ResolveMessage::Progress(stage));
                    })
                    .map(Box::new)
                    .map_err(|error| format!("{error:#}"));
                let _ = tx.send(ResolveMessage::Finished(result));
            })
        });
        let window = window_clone.clone();
        let state = state_clone.clone();
        let button = button.clone();
        let status = status_clone.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let result = rx.try_recv();
            if task.view.is_cancelled() {
                return match result {
                    Ok(ResolveMessage::Finished(Ok(resolution))) => {
                        if let Resolution::Proposal(p) = *resolution {
                            discard_token(&state, p.id);
                        }
                        glib::ControlFlow::Break
                    }
                    Ok(ResolveMessage::Finished(Err(_)))
                    | Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                    _ => glib::ControlFlow::Continue,
                };
            }
            if matches!(
                result,
                Ok(ResolveMessage::Finished(_)) | Err(mpsc::TryRecvError::Disconnected)
            ) {
                task.view.cancel();
            }
            match result {
                Ok(ResolveMessage::Progress(stage)) => {
                    status.set_text(&tr(stage.message()));
                    write_panel_status(stage.panel_message());
                    glib::ControlFlow::Continue
                }
                Ok(ResolveMessage::Finished(Ok(resolution))) => {
                    show_resolution(&window, state.clone(), *resolution);
                    glib::ControlFlow::Break
                }
                Ok(ResolveMessage::Finished(Err(error))) => {
                    show_error_message(&window, state.clone(), &error);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    status.set_text(&tr("The request worker stopped unexpectedly."));
                    write_panel_status("Peasy request stopped");
                    button.set_sensitive(true);
                    glib::ControlFlow::Break
                }
            }
        });
    });
    let send_clone = send.clone();
    entry.connect_activate(move |_| send_clone.emit_clicked());
}

fn show_resolution(window: &adw::ApplicationWindow, state: AppState, resolution: Resolution) {
    match resolution {
        Resolution::Proposal(proposal) => show_proposal(window, state, *proposal),
        Resolution::LocalProposal(proposal) => show_local_proposal(window, state, proposal),
        Resolution::Choose(choice) => show_choices(window, state, choice),
        Resolution::Explain(message) => show_message(window, state, &message),
        Resolution::Clarify(message) => {
            if state.chat_pending.borrow().is_some() {
                chat_ui::complete_task(window, state, &message, true);
                return;
            }
            clear_panel_status();
            state.reviewed_change.borrow_mut().take();
            state.request.borrow_mut().clear();
            render_message(window, state, &message, MessageKind::Reply);
        }
        Resolution::Cancel => show_message(window, state, "Cancelled."),
    }
}

fn show_choices(window: &adw::ApplicationWindow, state: AppState, choice: Choice) {
    let (root, body) = page("Choose a package");
    let intro = gtk::Label::new(Some(choice.intro.as_deref().unwrap_or(&tr(
        "Available on this system, with the best matches first:",
    ))));
    intro.set_wrap(true);
    intro.set_halign(gtk::Align::Start);
    intro.set_selectable(true);
    intro.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    let guidance = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .max_content_height(220)
        .propagate_natural_height(true)
        .child(&intro)
        .build();
    body.append(&guidance);
    let shared = Arc::new(Mutex::new(Some(choice)));
    let candidates = shared
        .lock()
        .expect("choice mutex poisoned")
        .as_ref()
        .expect("choice missing")
        .candidates
        .clone();
    let choices = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for (index, candidate) in candidates.into_iter().enumerate() {
        let button = gtk::Button::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let name = gtk::Label::new(Some(&if index == 0 {
            tr_args("Best match · {name}", &[("name", &candidate.name)])
        } else {
            candidate.name.clone()
        }));
        name.set_halign(gtk::Align::Start);
        name.add_css_class("heading");
        content.append(&name);
        if !candidate.version.is_empty() {
            let version = gtk::Label::new(Some(&tr_args(
                "Version {version}",
                &[("version", &candidate.version)],
            )));
            version.set_halign(gtk::Align::Start);
            version.add_css_class("dim-label");
            content.append(&version);
        }
        let attribute = gtk::Label::new(Some(&candidate.attribute));
        attribute.set_direction(gtk::TextDirection::Ltr);
        attribute.set_halign(gtk::Align::Start);
        attribute.add_css_class("monospace");
        attribute.add_css_class("dim-label");
        content.append(&attribute);
        if !candidate.description.is_empty() {
            let description = gtk::Label::new(Some(&candidate.description));
            description.set_halign(gtk::Align::Start);
            description.set_wrap(true);
            description.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
            description.add_css_class("dim-label");
            content.append(&description);
        }
        button.set_child(Some(&content));
        let selected = Arc::clone(&shared);
        let window_clone = window.clone();
        let state_clone = state.clone();
        button.connect_clicked(move |button| {
            button.set_sensitive(false);
            let Some(choice) = selected.lock().expect("choice mutex poisoned").take() else {
                return;
            };
            let Some(client) = state_clone.client.borrow().clone() else {
                return;
            };
            show_working(&window_clone, "Preparing the configuration change…");
            write_panel_status("…preparing change");
            let (tx, rx) = mpsc::channel();
            let task = state_clone.tasks.borrow_mut().start();
            std::thread::spawn(move || {
                task.work.scope(|| {
                    let progress_tx = tx.clone();
                    let result = client
                        .select_with_progress(choice, index, move |stage| {
                            let _ = progress_tx.send(ResolveMessage::Progress(stage));
                        })
                        .map(Box::new)
                        .map_err(|error| format!("{error:#}"));
                    let _ = tx.send(ResolveMessage::Finished(result));
                })
            });
            let window = window_clone.clone();
            let state = state_clone.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let result = rx.try_recv();
                if task.view.is_cancelled() {
                    return match result {
                        Ok(ResolveMessage::Finished(Ok(resolution))) => {
                            if let Resolution::Proposal(p) = *resolution {
                                discard_token(&state, p.id);
                            }
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) | Ok(ResolveMessage::Progress(_)) => {
                            glib::ControlFlow::Continue
                        }
                        _ => glib::ControlFlow::Break,
                    };
                }
                if matches!(
                    result,
                    Ok(ResolveMessage::Finished(_)) | Err(mpsc::TryRecvError::Disconnected)
                ) {
                    task.view.cancel();
                }
                match result {
                    Ok(ResolveMessage::Progress(stage)) => {
                        show_working(&window, stage.message());
                        glib::ControlFlow::Continue
                    }
                    Ok(ResolveMessage::Finished(Ok(resolution))) => {
                        show_resolution(&window, state.clone(), *resolution);
                        glib::ControlFlow::Break
                    }
                    Ok(ResolveMessage::Finished(Err(error))) => {
                        show_error_message(&window, state.clone(), &error);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        show_error_message(
                            &window,
                            state.clone(),
                            "The package-selection worker stopped unexpectedly.",
                        );
                        glib::ControlFlow::Break
                    }
                }
            });
        });
        choices.append(&button);
    }
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(260)
        .vexpand(true)
        .child(&choices)
        .build();
    body.append(&scroller);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let window_clone = window.clone();
    cancel.connect_clicked(move |_| cancel_review(&window_clone, state.clone()));
    body.append(&cancel);
    show_content(window, &root, 480, -1);
}

fn show_working(window: &adw::ApplicationWindow, message: &str) {
    let (root, body) = page("Peasy");
    let progress = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    progress.set_halign(gtk::Align::Center);
    progress.set_valign(gtk::Align::Center);
    progress.set_vexpand(true);
    let spinner = adw::Spinner::new();
    spinner.set_size_request(20, 20);
    spinner.set_valign(gtk::Align::Center);
    progress.append(&spinner);
    let label = gtk::Label::new(Some(&tr(message)));
    label.set_halign(gtk::Align::Center);
    label.set_valign(gtk::Align::Center);
    label.set_wrap(true);
    progress.append(&label);
    body.append(&progress);
    show_content(window, &root, 440, -1);
}

fn show_apply_progress(
    window: &adw::ApplicationWindow,
    title: &str,
    state: AppState,
) -> (gtk::Label, gtk::Button) {
    let (root, body) = page("Applying change");
    let title = gtk::Label::new(Some(title));
    title.add_css_class("title-3");
    title.set_halign(gtk::Align::Start);
    title.set_wrap(true);
    body.append(&title);

    let progress = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    progress.set_margin_top(18);
    let spinner = adw::Spinner::new();
    spinner.set_size_request(24, 24);
    spinner.set_valign(gtk::Align::Center);
    progress.append(&spinner);
    let status = gtk::Label::new(Some(&tr(OperationStage::Authorizing.message())));
    status.set_halign(gtk::Align::Start);
    status.set_wrap(true);
    status.add_css_class("heading");
    progress.append(&status);
    body.append(&progress);

    let explanation = gtk::Label::new(Some(&tr(
        "Peasy is validating, building, and activating your new system generation. Closing this window cancels the build and restores your configuration. If activation has already started, it will finish safely in the background.",
    )));
    explanation.set_halign(gtk::Align::Start);
    explanation.set_wrap(true);
    explanation.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    explanation.add_css_class("dim-label");
    body.append(&explanation);
    let cancel = gtk::Button::with_label(&tr("Cancel change"));
    let window_cancel = window.clone();
    cancel.connect_clicked(move |_| request_apply_cancellation(&window_cancel, state.clone()));
    body.append(&cancel);
    show_content(window, &root, 440, -1);
    (status, cancel)
}

fn show_proposal(window: &adw::ApplicationWindow, state: AppState, proposal: Proposal) {
    *state.pending.borrow_mut() = Some(proposal.id.clone());
    *state.reviewed_change.borrow_mut() = Some(proposal.change.clone());
    write_panel_status(&format!("…review {}", proposal.title));
    let (root, body) = page("Review change");
    let title = gtk::Label::new(Some(&proposal.title));
    title.add_css_class("title-3");
    title.set_halign(gtk::Align::Start);
    title.set_wrap(true);
    body.append(&title);
    body.append(&diff_view(&proposal.diff));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let apply = gtk::Button::with_label(&tr("Apply"));
    apply.add_css_class("suggested-action");
    actions.append(&cancel);
    actions.append(&apply);
    body.append(&actions);
    let window_cancel = window.clone();
    let state_cancel = state.clone();
    cancel.connect_clicked(move |_| cancel_review(&window_cancel, state_cancel.clone()));
    let window_apply = window.clone();
    apply.connect_clicked(move |button| {
        button.set_sensitive(false);
        let client = if state.return_to_ollama_settings {
            None
        } else {
            state.client.borrow().clone()
        };
        let ipc = peasy_client::IpcClient::new(state.args.socket.clone());
        let (progress, cancel_progress) =
            show_apply_progress(&window_apply, &proposal.title, state.clone());
        state.applying.set(true);
        state.closing_apply.set(false);
        write_panel_status(&format!("…applying {}", proposal.title));
        let proposal = proposal.clone();
        let resume_task = state.tasks.borrow_mut().start();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress = move |stage| {
                let _ = progress_tx.send(ApplyMessage::Progress(stage));
            };
            let result = if let Some(client) = client {
                match client.apply_with_progress(&proposal, progress) {
                    Ok(result) if result.activated => {
                        match resume_task
                            .work
                            .scope(|| client.resume_after_apply(&proposal))
                        {
                            Ok(Some(next)) => {
                                let _ = tx.send(ApplyMessage::Resumed(Box::new(next)));
                                return;
                            }
                            Ok(None) => Ok(result),
                            Err(error) => Err(anyhow::anyhow!(
                                "Change activated, but resuming the request failed: {error}"
                            )),
                        }
                    }
                    other => other,
                }
            } else {
                ipc.request_with_progress(
                    &IpcRequest::ApplyWithProgress {
                        proposal: proposal.id,
                    },
                    progress,
                )
                .and_then(|r| match r {
                    IpcResponse::Applied { result } => Ok(result),
                    _ => anyhow::bail!("unexpected apply response"),
                })
            };
            let _ = tx.send(ApplyMessage::Finished(
                result.map_err(|error| format!("{error:#}")),
            ));
        });
        let window = window_apply.clone();
        let state = state.clone();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            let result = rx.try_recv();
            if matches!(
                result,
                Ok(ApplyMessage::Finished(_))
                    | Ok(ApplyMessage::Resumed(_))
                    | Err(mpsc::TryRecvError::Disconnected)
            ) {
                state.applying.set(false);
                state.pending.borrow_mut().take();
            }
            match result {
                Ok(ApplyMessage::Resumed(next)) => {
                    if resume_task.view.is_cancelled() {
                        if let Resolution::Proposal(p) = *next {
                            discard_token(&state, p.id);
                        }
                    } else {
                        show_resolution(&window, state.clone(), *next);
                    }
                    glib::ControlFlow::Break
                }
                Ok(ApplyMessage::Finished(Ok(result))) => {
                    let message = if result.activated {
                        tr_args(
                            "✓ Configuration valid\n✓ Build successful\n✓ Activated\n\n{message}",
                            &[("message", &result.message)],
                        )
                    } else {
                        result.message
                    };
                    if result.activated {
                        show_message(&window, state.clone(), &message);
                    } else {
                        show_error_message(&window, state.clone(), &message);
                    }
                    glib::ControlFlow::Break
                }
                Ok(ApplyMessage::Finished(Err(error))) => {
                    if state.closing_apply.get() && error == "Operation cancelled" {
                        show_message(
                            &window,
                            state.clone(),
                            "Cancelled. The previous configuration is unchanged.",
                        );
                    } else {
                        show_error_message(&window, state.clone(), &error);
                    }
                    glib::ControlFlow::Break
                }
                Ok(ApplyMessage::Progress(stage)) => {
                    progress.set_text(&tr(stage.message()));
                    cancel_progress.set_sensitive(!matches!(
                        stage,
                        OperationStage::Activating | OperationStage::Completed
                    ));
                    glib::ControlFlow::Continue
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    show_error_message(
                        &window,
                        state.clone(),
                        "The configuration worker stopped unexpectedly.",
                    );
                    glib::ControlFlow::Break
                }
            }
        });
    });
    show_content(window, &root, 480, -1);
}

fn diff_view(lines: &[peasy_core::DiffLine]) -> gtk::ScrolledWindow {
    let diff_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    for line in lines {
        let sign = match line.kind {
            DiffKind::Context => ' ',
            DiffKind::Add => '+',
            DiffKind::Remove => '-',
        };
        let label = gtk::Label::new(Some(&format!("{sign} {}", line.text)));
        label.set_direction(gtk::TextDirection::Ltr);
        label.add_css_class("monospace");
        match line.kind {
            DiffKind::Add => label.add_css_class("success"),
            DiffKind::Remove => label.add_css_class("error"),
            DiffKind::Context => {}
        }
        label.set_halign(gtk::Align::Start);
        label.set_selectable(true);
        diff_box.append(&label);
    }
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(140)
        .child(&diff_box)
        .build()
}

fn show_local_proposal(window: &adw::ApplicationWindow, state: AppState, proposal: LocalProposal) {
    write_panel_status(&format!("…review {}", proposal.title));
    let (root, body) = page("Review action");
    let title = gtk::Label::new(Some(&proposal.title));
    title.add_css_class("title-3");
    title.set_halign(gtk::Align::Start);
    title.set_wrap(true);
    body.append(&title);
    body.append(&diff_view(&proposal.diff));
    let status = gtk::Label::new(None);
    status.set_halign(gtk::Align::Start);
    status.set_wrap(true);
    body.append(&status);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let apply = gtk::Button::with_label(&tr("Continue"));
    apply.add_css_class("suggested-action");
    actions.append(&cancel);
    actions.append(&apply);
    body.append(&actions);
    let window_cancel = window.clone();
    let state_cancel = state.clone();
    cancel.connect_clicked(move |_| cancel_review(&window_cancel, state_cancel.clone()));
    let window_apply = window.clone();
    apply.connect_clicked(move |button| {
        button.set_sensitive(false);
        if proposal.password_required() {
            show_wifi_password(&window_apply, state.clone(), proposal.clone());
        } else {
            apply_local(
                &window_apply,
                state.clone(),
                proposal.clone(),
                None,
                status.clone(),
            );
        }
    });
    show_content(window, &root, 480, -1);
}

fn show_wifi_password(window: &adw::ApplicationWindow, state: AppState, proposal: LocalProposal) {
    let (root, body) = page("Wi-Fi password");
    let message = gtk::Label::new(Some(&tr(
        "Enter the network password. It stays on this machine and is not sent to the AI provider.",
    )));
    message.set_wrap(true);
    message.set_halign(gtk::Align::Start);
    body.append(&message);
    let password = gtk::PasswordEntry::builder()
        .placeholder_text(tr("Wi-Fi password"))
        .show_peek_icon(true)
        .build();
    body.append(&password);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    body.append(&status);
    let connect = gtk::Button::with_label(&tr("Connect"));
    connect.add_css_class("suggested-action");
    connect.set_halign(gtk::Align::End);
    body.append(&connect);
    let window_connect = window.clone();
    connect.connect_clicked(move |button| {
        let secret = password.text().to_string();
        if secret.is_empty() {
            status.set_text(&tr("Enter the Wi-Fi password."));
            return;
        }
        button.set_sensitive(false);
        apply_local(
            &window_connect,
            state.clone(),
            proposal.clone(),
            Some(secret),
            status.clone(),
        );
    });
    show_content(window, &root, 440, -1);
}

fn apply_local(
    window: &adw::ApplicationWindow,
    state: AppState,
    proposal: LocalProposal,
    password: Option<String>,
    status: gtk::Label,
) {
    let Some(client) = state.client.borrow().clone() else {
        status.set_text(&tr("AI provider is not configured. Open Peasy settings."));
        return;
    };
    status.set_text(&tr(match &proposal.action {
        LocalAction::Resources { .. } => "Applying the reviewed resource change…",
        LocalAction::Network { .. } => "Changing network connections…",
        LocalAction::Wifi { .. } => "…connecting to Wi-Fi",
        LocalAction::Bluetooth { .. } => "…connecting Bluetooth device",
        LocalAction::Calendar { .. } => "…opening calendar event",
        LocalAction::HyprlandSetting { .. } => "…changing Hyprland setting",
        LocalAction::HyprlandDispatch { .. } => "…controlling Hyprland",
    }));
    write_panel_status(status.text().as_str());
    let (tx, rx) = mpsc::channel();
    let task = state.tasks.borrow_mut().start();
    std::thread::spawn(move || {
        task.work.scope(|| {
            let result = client
                .apply_local(&proposal, password.as_deref())
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
        })
    });
    let window = window.clone();
    glib::timeout_add_local(Duration::from_millis(80), move || {
        if task.view.is_cancelled() {
            return glib::ControlFlow::Break;
        }
        let result = rx.try_recv();
        if !matches!(result, Err(mpsc::TryRecvError::Empty)) {
            task.view.cancel();
        }
        match result {
            Ok(Ok(result)) => {
                if let Some(trial) = result.display_trial {
                    show_display_confirmation(&window, state.clone(), trial);
                } else {
                    show_message(&window, state.clone(), &format!("✓ {}", result.message));
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                show_error_message(&window, state.clone(), &error);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                show_error_message(
                    &window,
                    state.clone(),
                    "The desktop-action worker stopped unexpectedly.",
                );
                glib::ControlFlow::Break
            }
        }
    });
}

fn show_display_confirmation(
    window: &adw::ApplicationWindow,
    state: AppState,
    trial: peasy_core::display_trial::DisplayTrial,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(tr("Keep these display settings?"))
        .build();
    dialog.add_response("revert", &tr("Revert"));
    dialog.add_response("keep", &tr("Keep settings"));
    dialog.set_default_response(Some("revert"));
    dialog.set_close_response("revert");
    dialog.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
    let trial = Rc::new(RefCell::new(Some(trial)));
    let pending = trial.clone();
    let weak_dialog = dialog.downgrade();
    glib::timeout_add_local(Duration::from_millis(200), move || {
        let Some(dialog) = weak_dialog.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let remaining = pending.borrow().as_ref().map(|t| t.seconds_remaining());
        match remaining {
            Some(0) => {
                dialog.close();
                glib::ControlFlow::Break
            }
            Some(seconds) => {
                dialog.set_body(&tr_args(
                    "Previous settings will be restored in {seconds} seconds.",
                    &[("seconds", &seconds.to_string())],
                ));
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        }
    });
    let response_window = window.clone();
    dialog.connect_response(None, move |_, response| {
        let Some(trial) = trial.borrow_mut().take() else {
            return;
        };
        let keep = response == "keep";
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(trial.finish(keep).map_err(|e| format!("{e:#}")));
        });
        let window = response_window.clone();
        let state = state.clone();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            match rx.try_recv() {
                Ok(Ok(message)) => show_message(&window, state.clone(), &message),
                Ok(Err(error)) => show_error_message(&window, state.clone(), &error),
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(_) => show_error_message(
                    &window,
                    state.clone(),
                    "Display safety worker disconnected.",
                ),
            }
            glib::ControlFlow::Break
        });
    });
    dialog.set_body(&tr(
        "Previous settings will be restored in 20 seconds unless you keep them.",
    ));
    dialog.present(Some(window));
}

fn cancel_review(window: &adw::ApplicationWindow, state: AppState) {
    if state.chat_pending.borrow().is_some() {
        state.reviewed_change.borrow_mut().take();
        chat_ui::complete_task(window, state, "Cancelled.", false);
    } else {
        show_prompt(window, state);
    }
}

fn show_message(window: &adw::ApplicationWindow, state: AppState, message: &str) {
    if state.chat_pending.borrow().is_some() {
        state.reviewed_change.borrow_mut().take();
        chat_ui::complete_task(window, state, message, false);
        return;
    }
    clear_panel_status();
    state.reviewed_change.borrow_mut().take();
    state.request.borrow_mut().clear();
    render_message(window, state, message, MessageKind::Notice);
}

fn show_error_message(window: &adw::ApplicationWindow, state: AppState, message: &str) {
    write_panel_status("Peasy needs attention");
    render_message(window, state, message, MessageKind::Error);
}

enum MessageKind {
    Notice,
    Reply,
    Error,
}

fn render_message(
    window: &adw::ApplicationWindow,
    state: AppState,
    message: &str,
    kind: MessageKind,
) {
    let (root, body) = page("Peasy");
    let displayed = if matches!(kind, MessageKind::Error) {
        peasy_client::connectivity::offline_message(message).unwrap_or(message)
    } else {
        message
    };
    let label = gtk::Label::new(Some(&tr(displayed)));
    label.set_wrap(true);
    label.set_halign(gtk::Align::Start);
    label.set_selectable(true);
    label.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(100)
        .max_content_height(420)
        .propagate_natural_height(true)
        .child(&label)
        .build();
    body.append(&scroll);
    if matches!(kind, MessageKind::Error) {
        let copy = gtk::Button::with_label(&tr("Copy diagnostics"));
        let diagnostics = message.to_owned();
        let display = gtk::prelude::WidgetExt::display(window);
        copy.connect_clicked(move |_| display.clipboard().set_text(&diagnostics));
        body.append(&copy);
        let retry = gtk::Button::with_label(&tr(if state.reviewed_change.borrow().is_some() {
            "Review again"
        } else {
            "Retry request"
        }));
        let retry_window = window.clone();
        let retry_state = state.clone();
        retry.connect_clicked(move |_| review_again(&retry_window, retry_state.clone()));
        body.append(&retry);
        add_system_status_button(&body, window, &state);
    }
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    controls.set_halign(gtk::Align::End);
    let reply_entry = if matches!(kind, MessageKind::Reply) {
        let reply_label = gtk::Label::new(Some(&tr("Reply")));
        reply_label.set_halign(gtk::Align::Start);
        body.append(&reply_label);
        let entry = gtk::Entry::builder()
            .placeholder_text(tr("Type your reply…"))
            .hexpand(true)
            .build();
        body.append(&entry);
        let status = gtk::Label::new(None);
        status.set_halign(gtk::Align::Start);
        status.set_wrap(true);
        body.append(&status);
        let send = gtk::Button::with_label(&tr("Send reply"));
        send.add_css_class("suggested-action");
        connect_request_submit(window, &state, &entry, &send, &status);
        controls.append(&send);
        Some(entry)
    } else {
        None
    };
    let done = gtk::Button::with_label(&tr("Done"));
    let window_clone = window.clone();
    let final_message = message.to_owned();
    done.connect_clicked(move |_| {
        if state.chat_pending.borrow().is_some() {
            chat_ui::complete_task(&window_clone, state.clone(), &final_message, false);
        } else {
            show_prompt(&window_clone, state.clone());
        }
    });
    controls.prepend(&done);
    body.append(&controls);
    show_content(window, &root, 440, -1);
    if let Some(entry) = reply_entry {
        entry.grab_focus();
    }
}

fn add_system_status_button(body: &gtk::Box, window: &adw::ApplicationWindow, state: &AppState) {
    let button = gtk::Button::with_label(&tr("System status and recovery"));
    let window = window.clone();
    let state = state.clone();
    button
        .connect_clicked(move |_| run_system_request(&window, state.clone(), IpcRequest::Inspect));
    body.append(&button);
}

fn review_again(window: &adw::ApplicationWindow, state: AppState) {
    let change = state.reviewed_change.borrow().clone();
    let request = match change {
        None => {
            show_prompt(window, state);
            return;
        }
        Some(ProposalChange::Resources { plan, .. }) => {
            IpcRequest::ProposeResources { change: plan }
        }
        Some(ProposalChange::Recovery { .. }) => IpcRequest::ProposeRecovery,
        Some(ProposalChange::PeasyUpdate { release }) => IpcRequest::ProposePeasyUpdate { release },
        Some(ProposalChange::Restore { backup, mode }) => {
            IpcRequest::ProposeRestore { backup, mode }
        }
        Some(ProposalChange::Package {
            operation, package, ..
        }) => match operation {
            peasy_core::PackageOperation::Install => IpcRequest::ProposeInstall { package },
            peasy_core::PackageOperation::Remove => IpcRequest::ProposeRemove { package },
        },
        Some(ProposalChange::Setup { operation, setup }) => match operation {
            peasy_core::PackageOperation::Install => IpcRequest::ProposeSetup {
                package: setup.package,
                setup: setup.settings,
            },
            peasy_core::PackageOperation::Remove => IpcRequest::ProposeRemove {
                package: setup.package,
            },
        },
        Some(ProposalChange::Pea { pin, enable }) => IpcRequest::ProposePea { pin, enable },
        Some(ProposalChange::Network { plan }) => IpcRequest::ProposeNetwork { plan },
        Some(ProposalChange::Theme { theme }) => IpcRequest::ProposeTheme { theme },
        Some(ProposalChange::AppImage { operation, package }) => match operation {
            peasy_core::PackageOperation::Install => IpcRequest::ProposeAppImageInstall { package },
            peasy_core::PackageOperation::Remove => IpcRequest::ProposeRemove {
                package: package.id,
            },
        },
    };
    run_system_request(window, state, request);
}

fn choose_backup(window: &adw::ApplicationWindow, state: AppState) {
    state.tasks.borrow_mut().close();
    discard_pending(&state);
    state.reviewed_change.borrow_mut().take();
    let task = state.tasks.borrow_mut().start();
    let dialog = gtk::FileDialog::builder()
        .title(tr("Choose a Peasy backup folder"))
        .accept_label(tr("Open backup"))
        .build();
    let w = window.clone();
    dialog.select_folder(
        Some(window),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            if task.view.is_cancelled() {
                return;
            }
            let path = match result {
                Ok(folder) => match folder.path() {
                    Some(path) => path,
                    None => {
                        show_error_message(&w, state.clone(), "Choose a local backup folder.");
                        return;
                    }
                },
                Err(error)
                    if error.matches(gtk::DialogError::Cancelled)
                        || error.matches(gtk::DialogError::Dismissed) =>
                {
                    return;
                }
                Err(error) => {
                    show_error_message(
                        &w,
                        state.clone(),
                        &tr_args(
                            "Could not open backup: {error}",
                            &[("error", &format!("{error:#}"))],
                        ),
                    );
                    return;
                }
            };
            show_working(&w, "Validating backup…");
            let worker = task.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let result = worker.work.scope(|| restore::read_backup(&path));
                let _ = tx.send(result.map_err(|e| format!("{e:#}")));
            });
            let w = w.clone();
            let state = state.clone();
            let view = task.view.clone();
            glib::timeout_add_local(Duration::from_millis(80), move || {
                if view.is_cancelled() {
                    return glib::ControlFlow::Break;
                }
                match rx.try_recv() {
                    Ok(Ok(backup)) => show_restore_options(&w, state.clone(), backup),
                    Ok(Err(error)) => show_error_message(&w, state.clone(), &error),
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(_) => show_error_message(
                        &w,
                        state.clone(),
                        "Backup validation stopped unexpectedly.",
                    ),
                }
                glib::ControlFlow::Break
            });
        },
    );
}

fn show_restore_options(window: &adw::ApplicationWindow, state: AppState, backup: restore::Backup) {
    let (root, body) = page("Restore backup");
    let summary = gtk::Label::new(Some(&tr_args(
        "{packages} standalone packages, {peas} pea instructions and saved appearance settings. Your hardware configuration stays in place.",
        &[
            ("packages", &backup.portable.packages.len().to_string()),
            ("peas", &backup.portable.peas.len().to_string()),
        ],
    )));
    summary.set_wrap(true);
    summary.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    body.append(&summary);
    let mode = gtk::DropDown::from_strings(&[
        &tr("Merge with current settings"),
        &tr("Replace portable settings"),
    ]);
    body.append(&mode);
    let explanation = gtk::Label::new(Some(&tr(
        "Merge keeps current packages and adds the saved selection; saved appearance choices take precedence. Replace uses only the backup's standalone package, appearance and pea selections. Both modes keep this machine's service setups, network profiles and AppImages.",
    )));
    explanation.set_wrap(true);
    explanation.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    body.append(&explanation);
    let deferred = gtk::Label::new(Some(&backup.deferred));
    deferred.set_wrap(true);
    deferred.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    deferred.set_selectable(true);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&deferred)
        .max_content_height(180)
        .propagate_natural_height(true)
        .build();
    body.append(&scroll);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let w = window.clone();
    let s = state.clone();
    cancel.connect_clicked(move |_| show_provider_settings(&w, s.clone()));
    let review = gtk::Button::with_label(&tr("Review restore"));
    review.add_css_class("suggested-action");
    let w = window.clone();
    review.connect_clicked(move |_| {
        run_system_request(
            &w,
            state.clone(),
            IpcRequest::ProposeRestore {
                backup: backup.portable.clone(),
                mode: if mode.selected() == 0 {
                    peasy_core::RestoreMode::Merge
                } else {
                    peasy_core::RestoreMode::Replace
                },
            },
        )
    });
    actions.append(&cancel);
    actions.append(&review);
    body.append(&actions);
    show_content(window, &root, 580, -1);
}

fn run_system_request(window: &adw::ApplicationWindow, state: AppState, request: IpcRequest) {
    state.tasks.borrow_mut().close();
    show_working(window, "Checking system state…");
    let task = state.tasks.borrow_mut().start();
    let ipc = peasy_client::IpcClient::new(state.args.socket.clone());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        task.work.scope(|| {
            let _ = tx.send(ipc.request(&request).map_err(|e| format!("{e:#}")));
        })
    });
    let window = window.clone();
    glib::timeout_add_local(Duration::from_millis(80), move || {
        let result = match rx.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => Err("System status worker stopped".into()),
            Ok(result) => result,
        };
        if task.view.is_cancelled() {
            if let Ok(IpcResponse::Proposal { proposal }) = result {
                discard_token(&state, proposal.id);
            }
            return glib::ControlFlow::Break;
        }
        task.view.cancel();
        match result {
            Ok(IpcResponse::Proposal { proposal }) => {
                show_proposal(&window, state.clone(), *proposal)
            }
            Ok(IpcResponse::Inspection { status }) => {
                show_system_status(&window, state.clone(), *status)
            }
            Ok(_) => show_error_message(&window, state.clone(), "Unexpected system response"),
            Err(error) => show_error_message(&window, state.clone(), &error),
        }
        glib::ControlFlow::Break
    });
}

fn show_system_status(
    window: &adw::ApplicationWindow,
    state: AppState,
    status: peasy_core::ServiceStatus,
) {
    let (root, body) = page("System status and recovery");
    let mut message = if status.restart_pending {
        tr("Peasy is finishing existing work before updating.\n\n")
    } else {
        "".to_owned()
    };
    message.push_str(&tr_args("Service version: {version}\nProtocol: {protocol}\nRunning executable: {executable}\nPackage source: {source}", &[("version", &status.version), ("protocol", &status.protocol.to_string()), ("executable", &status.executable), ("source", &status.nixpkgs)]));
    if let Some(recovery) = status.recovery {
        message.push_str(&tr_args(
            "\n\n{message}\nActive generation: {active}\nPrevious generation: {previous}\nRequested packages: {packages}",
            &[("message", &recovery.message), ("active", recovery.active_generation.as_deref().unwrap_or(&tr("unavailable"))),
              ("previous", recovery.previous_generation.as_deref().unwrap_or(&tr("unavailable"))),
              ("packages", &recovery.intended_packages.join(", "))],
        ));
        if !recovery.intended_change.is_empty() {
            message.push_str(&tr(
                "\n\nIntended configuration change (up to 100 lines):\n",
            ));
            for line in &recovery.intended_change {
                let prefix = match line.kind {
                    peasy_core::DiffKind::Add => "+",
                    peasy_core::DiffKind::Remove => "-",
                    peasy_core::DiffKind::Context => " ",
                };
                message.push_str(&format!("{prefix} {}\n", line.text));
            }
        }
        if recovery.needs_attention && recovery.previous_generation.is_some() && !status.applying {
            let recover = gtk::Button::with_label(&tr("Review restoring the previous generation"));
            let w = window.clone();
            let s = state.clone();
            recover.connect_clicked(move |_| {
                run_system_request(&w, s.clone(), IpcRequest::ProposeRecovery)
            });
            body.append(&recover);
        }
    } else if status.applying {
        message.push_str(&tr("\n\nA system change is running."));
    } else {
        message.push_str(&tr("\n\nNo interrupted operation needs recovery."));
    }
    let label = gtk::Label::new(Some(&message));
    label.set_wrap(true);
    label.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    label.set_selectable(true);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(240)
        .max_content_height(500)
        .child(&label)
        .build();
    body.append(&scroll);
    let refresh = gtk::Button::with_label(&tr("Refresh status"));
    let w = window.clone();
    let s = state.clone();
    refresh.connect_clicked(move |_| run_system_request(&w, s.clone(), IpcRequest::Inspect));
    body.append(&refresh);
    let done = gtk::Button::with_label(&tr("Back"));
    let w = window.clone();
    done.connect_clicked(move |_| {
        if state.client.borrow().is_some() {
            show_prompt(&w, state.clone());
        } else {
            show_provider_settings(&w, state.clone());
        }
    });
    body.append(&done);
    show_content(window, &root, 520, -1);
}

fn panel_runtime_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|path| path.join("peasy-user"))
}

fn take_panel_request() -> Result<Option<String>> {
    let Some(directory) = panel_runtime_directory() else {
        return Ok(None);
    };
    let path = directory.join("pending-request");
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                anyhow::bail!("Peasy panel request is not a regular file");
            }
            if metadata.len() > 4096 {
                anyhow::bail!("Peasy panel request is too large");
            }
            let mut request = String::new();
            OpenOptions::new()
                .read(true)
                .open(&path)?
                .take(4097)
                .read_to_string(&mut request)?;
            fs::remove_file(path)?;
            let request = request.trim().to_owned();
            Ok((!request.is_empty()).then_some(request))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_panel_status(message: &str) {
    let Some(directory) = panel_runtime_directory() else {
        return;
    };
    let result = (|| -> Result<()> {
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = directory.join(format!(".status-{}-{nonce}.tmp", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        let safe = tr(message)
            .chars()
            .filter(|character| !character.is_control())
            .take(180)
            .collect::<String>();
        file.write_all(safe.as_bytes())?;
        file.sync_all()?;
        fs::rename(temporary, directory.join("status"))?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("Could not update Peasy panel status: {error:#}");
    }
}

fn clear_panel_status() {
    if let Some(directory) = panel_runtime_directory() {
        let _ = fs::remove_file(directory.join("status"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_selection_defaults_to_ollama_for_new_users() {
        assert_eq!(
            initial_provider_selection(None, false),
            OLLAMA_PROVIDER_INDEX
        );
    }

    #[test]
    fn provider_selection_preserves_saved_choices() {
        let openai = ProviderSettings::openai_default();
        let ollama = ProviderSettings::ollama("test-model".into()).unwrap();
        for has_stored_key in [false, true] {
            assert_eq!(
                initial_provider_selection(Some(&openai), has_stored_key),
                OPENAI_PROVIDER_INDEX
            );
            assert_eq!(
                initial_provider_selection(Some(&ollama), has_stored_key),
                OLLAMA_PROVIDER_INDEX
            );
        }
    }

    #[test]
    fn provider_selection_preserves_legacy_openai_setup() {
        assert_eq!(
            initial_provider_selection(None, true),
            OPENAI_PROVIDER_INDEX
        );
    }
}
