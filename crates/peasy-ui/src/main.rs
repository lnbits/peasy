use adw::prelude::*;
use anyhow::{Context, Result};
use clap::Parser;
use gtk::glib;
use peasy_client::{
    Choice, DEFAULT_OLLAMA_URL, DEFAULT_OPENAI_MODEL, KeyStore, LocalAction, LocalProposal,
    PeasyClient, ProviderSettings, ProviderStore, Resolution, ResolveStage, list_ollama_models,
    load_model_provider,
};
use peasy_core::{DiffKind, IpcRequest, IpcResponse, OperationStage, Proposal, ProposalChange};
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
}

enum ResolveMessage {
    Progress(ResolveStage),
    Finished(std::result::Result<Box<Resolution>, String>),
}

enum ApplyMessage {
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
    app.run();
    Ok(())
}

fn activate(app: &adw::Application, state: AppState) {
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
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Label::new(Some(title))));
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

fn initial_provider_selection(settings: Option<&ProviderSettings>, has_stored_key: bool) -> u32 {
    match settings {
        Some(ProviderSettings::OpenAi { .. }) => OPENAI_PROVIDER_INDEX,
        Some(ProviderSettings::Ollama { .. }) => OLLAMA_PROVIDER_INDEX,
        // Older installations may have an OpenAI key but no provider.json.
        None if has_stored_key => OPENAI_PROVIDER_INDEX,
        None => OLLAMA_PROVIDER_INDEX,
    }
}

fn show_provider_settings(window: &adw::ApplicationWindow, state: AppState) {
    state.tasks.borrow_mut().close();
    discard_pending(&state);
    let (root, body) = page("Peasy settings");
    let heading = gtk::Label::new(Some("AI provider"));
    heading.add_css_class("title-3");
    heading.set_halign(gtk::Align::Start);
    body.append(&heading);
    add_system_status_button(&body, window, &state);

    let provider = gtk::DropDown::from_strings(&["OpenAI", "Ollama (local)"]);
    let settings = state.providers.load().ok().flatten();
    let has_stored_key = state.keys.load().ok().flatten().is_some();
    provider.set_selected(initial_provider_selection(
        settings.as_ref(),
        has_stored_key,
    ));
    body.append(&provider);

    let openai_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let key_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Enter a new OpenAI API key")
        .show_peek_icon(true)
        .build();
    openai_box.append(&key_entry);
    let key_note = gtk::Label::new(Some(if has_stored_key {
        "A key is stored privately. Leave this empty to keep it."
    } else {
        "Your key is stored privately for this desktop user."
    }));
    key_note.set_wrap(true);
    key_note.set_halign(gtk::Align::Start);
    openai_box.append(&key_note);
    let openai_model = gtk::Entry::builder()
        .placeholder_text("OpenAI model")
        .text(DEFAULT_OPENAI_MODEL)
        .build();
    openai_box.append(&openai_model);
    let remove_key = gtk::Button::with_label("Remove stored OpenAI key");
    remove_key.add_css_class("destructive-action");
    remove_key.set_halign(gtk::Align::Start);
    remove_key.set_sensitive(has_stored_key);
    openai_box.append(&remove_key);

    let ollama_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let endpoint = gtk::Label::new(Some("Local Ollama · http://127.0.0.1:11434"));
    endpoint.set_halign(gtk::Align::Start);
    ollama_box.append(&endpoint);
    let ollama_model = gtk::Entry::builder()
        .placeholder_text("Detecting installed models…")
        .build();
    ollama_box.append(&ollama_model);
    let ollama_status = gtk::Label::new(Some("Checking Ollama…"));
    ollama_status.set_wrap(true);
    ollama_status.set_halign(gtk::Align::Start);
    ollama_box.append(&ollama_status);
    let refresh = gtk::Button::with_label("Refresh installed models");
    refresh.set_halign(gtk::Align::Start);
    ollama_box.append(&refresh);

    let stack = gtk::Stack::new();
    stack.add_named(&openai_box, Some("openai"));
    stack.add_named(&ollama_box, Some("ollama"));
    body.append(&stack);

    if let Some(settings) = settings {
        match settings {
            ProviderSettings::OpenAi { model } => openai_model.set_text(&model),
            ProviderSettings::Ollama { model, .. } => {
                ollama_model.set_text(&model);
            }
        }
    }
    stack.set_visible_child_name(if provider.selected() == OLLAMA_PROVIDER_INDEX {
        "ollama"
    } else {
        "openai"
    });
    let stack_clone = stack.clone();
    provider.connect_selected_notify(move |provider| {
        stack_clone.set_visible_child_name(if provider.selected() == OLLAMA_PROVIDER_INDEX {
            "ollama"
        } else {
            "openai"
        });
    });

    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.set_margin_top(4);
    separator.set_margin_bottom(4);
    body.append(&separator);
    let export_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    export_row.set_valign(gtk::Align::Center);
    let export_copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
    export_copy.set_hexpand(true);
    let export_text = gtk::Label::new(Some(
        "Export your configuration.nix host and Peasy settings with restore instructions. Flake hosts are not supported by this export yet.",
    ));
    export_text.set_wrap(true);
    export_text.set_xalign(0.0);
    export_copy.append(&export_text);
    let export_note = gtk::Label::new(Some(
        "Creates a private configuration backup with a file inventory. Common secret files and Git history are excluded; configuration files may still contain private values. Review before sharing.",
    ));
    export_note.set_wrap(true);
    export_note.set_xalign(0.0);
    export_note.add_css_class("dim-label");
    export_copy.append(&export_note);
    export_row.append(&export_copy);
    let download_config = gtk::Button::with_label("Export system");
    download_config.set_valign(gtk::Align::Center);
    export_row.append(&download_config);
    body.append(&export_row);

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
                export_status.set_text(&format!("Could not prepare config: {error:#}"));
                return;
            }
        };
        let dialog = gtk::FileDialog::builder()
            .title("Choose where to export your NixOS system")
            .accept_label("Export here")
            .build();
        let status = export_status.clone();
        dialog.select_folder(
            Some(&export_window),
            None::<&gtk::gio::Cancellable>,
            move |result| match result {
                Ok(folder) => match folder.path() {
                    Some(folder) => match write_configuration_export(&export, &folder) {
                        Ok(path) => status.set_text(&format!(
                            "System configuration exported to {}.",
                            path.display()
                        )),
                        Err(error) => {
                            status.set_text(&format!("Could not export system: {error:#}"))
                        }
                    },
                    None => status.set_text("Choose a local folder for the system export."),
                },
                Err(error)
                    if error.matches(gtk::DialogError::Cancelled)
                        || error.matches(gtk::DialogError::Dismissed) => {}
                Err(error) => status.set_text(&format!("Could not open Save dialog: {error}")),
            },
        );
    });
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    if state.client.borrow().is_some() {
        let cancel = gtk::Button::with_label("Cancel");
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
    let save = gtk::Button::with_label("Save provider");
    save.add_css_class("suggested-action");
    buttons.append(&save);
    body.append(&buttons);

    let detected_models = Rc::new(RefCell::new(Vec::<String>::new()));
    refresh_ollama_models(
        &state,
        ollama_status.clone(),
        ollama_model.clone(),
        detected_models.clone(),
    );
    let detected_clone = detected_models.clone();
    let ollama_status_clone = ollama_status.clone();
    let ollama_model_clone = ollama_model.clone();
    let refresh_state = state.clone();
    refresh.connect_clicked(move |_| {
        refresh_ollama_models(
            &refresh_state,
            ollama_status_clone.clone(),
            ollama_model_clone.clone(),
            detected_clone.clone(),
        );
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
            status_for_remove.set_text("Stored OpenAI key removed.");
        }
        Err(error) => status_for_remove.set_text(&format!("{error:#}")),
    });

    let window_clone = window.clone();
    save.connect_clicked(move |_| {
        let result = (|| -> Result<()> {
            let settings = if provider.selected() == OLLAMA_PROVIDER_INDEX {
                let model = ollama_model.text().trim().to_owned();
                if !detected_models.borrow().iter().any(|found| found == &model) {
                    anyhow::bail!(
                        "Choose an installed Ollama model. Press Refresh after pulling a model."
                    );
                }
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
    show_content(window, &root, 480, 570);
}

fn engine_path(args: &Args) -> PathBuf {
    args.engine
        .clone()
        .or_else(|| std::env::var_os("PEASY_ENGINE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/run/current-system/sw/lib/peasy/peasy-engine.wasm"))
}

mod export;
use export::{configuration_export, write_configuration_export};

fn refresh_ollama_models(
    state: &AppState,
    status: gtk::Label,
    model_entry: gtk::Entry,
    detected: Rc<RefCell<Vec<String>>>,
) {
    status.set_text("Checking local Ollama…");
    let task = state.tasks.borrow_mut().start();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        task.work.scope(|| {
            let _ = tx
                .send(list_ollama_models(DEFAULT_OLLAMA_URL).map_err(|error| format!("{error:#}")));
        })
    });
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if task.view.is_cancelled() {
            return glib::ControlFlow::Break;
        }
        let result = rx.try_recv();
        if !matches!(result, Err(mpsc::TryRecvError::Empty)) {
            task.view.cancel();
        }
        match result {
            Ok(Ok(models)) => {
                *detected.borrow_mut() = models.clone();
                if models.is_empty() {
                    status.set_text(
                    "Ollama is running but has no models. Run `ollama pull MODEL`, then press Refresh.",
                );
                } else {
                    if model_entry.text().trim().is_empty()
                        || !models
                            .iter()
                            .any(|model| model == model_entry.text().as_str())
                    {
                        model_entry.set_text(&models[0]);
                    }
                    status.set_text(&format!("Installed: {}", models.join(", ")));
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                detected.borrow_mut().clear();
                status.set_text(&format!(
                    "{error}\nEnable services.ollama and start it, then press Refresh."
                ));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                status.set_text("Ollama model check stopped unexpectedly.");
                glib::ControlFlow::Break
            }
        }
    });
}

fn show_prompt(window: &adw::ApplicationWindow, state: AppState) {
    state.tasks.borrow_mut().close();
    discard_pending(&state);
    let request = take_panel_request().ok().flatten();
    if request.is_none() {
        clear_panel_status();
    }
    let (root, body, header) = page_with_header("Peasy");
    let settings = gtk::Button::builder()
        .icon_name("applications-system-symbolic")
        .tooltip_text("AI provider settings")
        .build();
    header.pack_end(&settings);
    let settings_window = window.clone();
    let settings_state = state.clone();
    settings
        .connect_clicked(move |_| show_provider_settings(&settings_window, settings_state.clone()));
    let tagline = gtk::Label::new(Some("Tell your computer what you want."));
    tagline.add_css_class("title-3");
    tagline.set_halign(gtk::Align::Start);
    body.append(&tagline);
    let entry = gtk::Entry::builder()
        .placeholder_text("install telegram…")
        .hexpand(true)
        .build();
    entry.set_text(&state.request.borrow());
    body.append(&entry);
    add_system_status_button(&body, window, &state);
    let recovery_notice = gtk::Label::new(None);
    recovery_notice.set_wrap(true);
    recovery_notice.set_halign(gtk::Align::Start);
    body.append(&recovery_notice);
    let ipc = peasy_client::IpcClient::new(state.args.socket.clone());
    let (notice_tx, notice_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = notice_tx.send(ipc.request(&IpcRequest::Inspect));
    });
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match notice_rx.try_recv() {
            Ok(Ok(IpcResponse::Inspection { status })) => {
                if let Some(info) = status.recovery {
                    recovery_notice.set_text(&format!(
                        "{} Open System status and recovery for details.",
                        info.message
                    ));
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            _ => glib::ControlFlow::Break,
        }
    });

    let status = gtk::Label::new(None);
    status.set_halign(gtk::Align::Start);
    status.set_wrap(true);
    body.append(&status);
    let send = gtk::Button::with_label("Send");
    send.add_css_class("suggested-action");
    send.set_halign(gtk::Align::End);
    body.append(&send);
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
        status_clone.set_text("Understanding request…");
        write_panel_status("…thinking");
        let Some(client) = state_clone.client.borrow().clone() else {
            status_clone.set_text("AI provider is not configured. Open Peasy settings.");
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
                    status.set_text(stage.message());
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
                    status.set_text("The request worker stopped unexpectedly.");
                    write_panel_status("Peasy request stopped");
                    button.set_sensitive(true);
                    glib::ControlFlow::Break
                }
            }
        });
    });
    let send_clone = send.clone();
    entry.connect_activate(move |_| send_clone.emit_clicked());
    show_content(window, &root, 440, -1);
    if let Some(request) = request {
        entry.set_text(&request);
        send.emit_clicked();
    }
}

fn show_resolution(window: &adw::ApplicationWindow, state: AppState, resolution: Resolution) {
    match resolution {
        Resolution::Proposal(proposal) => show_proposal(window, state, *proposal),
        Resolution::LocalProposal(proposal) => show_local_proposal(window, state, proposal),
        Resolution::Choose(choice) => show_choices(window, state, choice),
        Resolution::Explain(message) => show_message(window, state, &message),
        Resolution::Cancel => show_message(window, state, "Cancelled."),
    }
}

fn show_choices(window: &adw::ApplicationWindow, state: AppState, choice: Choice) {
    let (root, body) = page("Choose a package");
    let intro = gtk::Label::new(Some(
        choice
            .intro
            .as_deref()
            .unwrap_or("Available on this system, with the best matches first:"),
    ));
    intro.set_wrap(true);
    intro.set_halign(gtk::Align::Start);
    body.append(&intro);
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
            format!("Best match · {}", candidate.name)
        } else {
            candidate.name.clone()
        }));
        name.set_halign(gtk::Align::Start);
        name.add_css_class("heading");
        content.append(&name);
        if !candidate.version.is_empty() {
            let version = gtk::Label::new(Some(&format!("Version {}", candidate.version)));
            version.set_halign(gtk::Align::Start);
            version.add_css_class("dim-label");
            content.append(&version);
        }
        let attribute = gtk::Label::new(Some(&candidate.attribute));
        attribute.set_halign(gtk::Align::Start);
        attribute.add_css_class("monospace");
        attribute.add_css_class("dim-label");
        content.append(&attribute);
        if !candidate.description.is_empty() {
            let description = gtk::Label::new(Some(&candidate.description));
            description.set_halign(gtk::Align::Start);
            description.set_wrap(true);
            description.set_xalign(0.0);
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
    let cancel = gtk::Button::with_label("Cancel");
    let window_clone = window.clone();
    cancel.connect_clicked(move |_| show_prompt(&window_clone, state.clone()));
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
    let label = gtk::Label::new(Some(message));
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
    let status = gtk::Label::new(Some(OperationStage::Authorizing.message()));
    status.set_halign(gtk::Align::Start);
    status.set_wrap(true);
    status.add_css_class("heading");
    progress.append(&status);
    body.append(&progress);

    let explanation = gtk::Label::new(Some(
        "Peasy is validating, building, and activating your new system generation. Closing this window cancels the build and restores your configuration. If activation has already started, it will finish safely in the background.",
    ));
    explanation.set_halign(gtk::Align::Start);
    explanation.set_wrap(true);
    explanation.set_xalign(0.0);
    explanation.add_css_class("dim-label");
    body.append(&explanation);
    let cancel = gtk::Button::with_label("Cancel change");
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
    let cancel = gtk::Button::with_label("Cancel");
    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    actions.append(&cancel);
    actions.append(&apply);
    body.append(&actions);
    let window_cancel = window.clone();
    let state_cancel = state.clone();
    cancel.connect_clicked(move |_| show_prompt(&window_cancel, state_cancel.clone()));
    let window_apply = window.clone();
    apply.connect_clicked(move |button| {
        button.set_sensitive(false);
        let client = state.client.borrow().clone();
        let ipc = peasy_client::IpcClient::new(state.args.socket.clone());
        let (progress, cancel_progress) =
            show_apply_progress(&window_apply, &proposal.title, state.clone());
        state.applying.set(true);
        state.closing_apply.set(false);
        write_panel_status(&format!("…applying {}", proposal.title));
        let proposal = proposal.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress = move |stage| {
                let _ = progress_tx.send(ApplyMessage::Progress(stage));
            };
            let result = if let Some(client) = client {
                client.apply_with_progress(&proposal, progress)
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
                Ok(ApplyMessage::Finished(_)) | Err(mpsc::TryRecvError::Disconnected)
            ) {
                state.applying.set(false);
                state.pending.borrow_mut().take();
            }
            match result {
                Ok(ApplyMessage::Finished(Ok(result))) => {
                    let message = if result.activated {
                        format!(
                            "✓ Configuration valid\n✓ Build successful\n✓ Activated\n\n{}",
                            result.message
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
                    progress.set_text(stage.message());
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
    let cancel = gtk::Button::with_label("Cancel");
    let apply = gtk::Button::with_label("Continue");
    apply.add_css_class("suggested-action");
    actions.append(&cancel);
    actions.append(&apply);
    body.append(&actions);
    let window_cancel = window.clone();
    let state_cancel = state.clone();
    cancel.connect_clicked(move |_| show_prompt(&window_cancel, state_cancel.clone()));
    let window_apply = window.clone();
    apply.connect_clicked(move |button| {
        button.set_sensitive(false);
        if matches!(
            &proposal.action,
            LocalAction::Wifi {
                password: None,
                password_required: true,
                ..
            }
        ) {
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
    let message = gtk::Label::new(Some(
        "Enter the network password. It stays on this machine and is not sent to the AI provider.",
    ));
    message.set_wrap(true);
    message.set_halign(gtk::Align::Start);
    body.append(&message);
    let password = gtk::PasswordEntry::builder()
        .placeholder_text("Wi-Fi password")
        .show_peek_icon(true)
        .build();
    body.append(&password);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    body.append(&status);
    let connect = gtk::Button::with_label("Connect");
    connect.add_css_class("suggested-action");
    connect.set_halign(gtk::Align::End);
    body.append(&connect);
    let window_connect = window.clone();
    connect.connect_clicked(move |button| {
        let secret = password.text().to_string();
        if secret.is_empty() {
            status.set_text("Enter the Wi-Fi password.");
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
        status.set_text("AI provider is not configured. Open Peasy settings.");
        return;
    };
    status.set_text(match &proposal.action {
        LocalAction::Wifi { .. } => "…connecting to Wi-Fi",
        LocalAction::Bluetooth { .. } => "…connecting Bluetooth device",
        LocalAction::Calendar { .. } => "…opening calendar event",
        LocalAction::HyprlandSetting { .. } => "…changing Hyprland setting",
        LocalAction::HyprlandDispatch { .. } => "…controlling Hyprland",
    });
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
                show_message(&window, state.clone(), &format!("✓ {}", result.message));
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

fn show_message(window: &adw::ApplicationWindow, state: AppState, message: &str) {
    clear_panel_status();
    state.reviewed_change.borrow_mut().take();
    state.request.borrow_mut().clear();
    render_message(window, state, message, false);
}

fn show_error_message(window: &adw::ApplicationWindow, state: AppState, message: &str) {
    write_panel_status("Peasy needs attention");
    render_message(window, state, message, true);
}

fn render_message(window: &adw::ApplicationWindow, state: AppState, message: &str, error: bool) {
    let (root, body) = page("Peasy");
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_halign(gtk::Align::Start);
    body.append(&label);
    if error {
        let copy = gtk::Button::with_label("Copy diagnostics");
        let diagnostics = message.to_owned();
        let display = gtk::prelude::WidgetExt::display(window);
        copy.connect_clicked(move |_| display.clipboard().set_text(&diagnostics));
        body.append(&copy);
        let retry = gtk::Button::with_label(if state.reviewed_change.borrow().is_some() {
            "Review again"
        } else {
            "Retry request"
        });
        let retry_window = window.clone();
        let retry_state = state.clone();
        retry.connect_clicked(move |_| review_again(&retry_window, retry_state.clone()));
        body.append(&retry);
        add_system_status_button(&body, window, &state);
    }
    let done = gtk::Button::with_label("Done");
    done.set_halign(gtk::Align::End);
    let window_clone = window.clone();
    done.connect_clicked(move |_| show_prompt(&window_clone, state.clone()));
    body.append(&done);
    show_content(window, &root, 440, -1);
}

fn add_system_status_button(body: &gtk::Box, window: &adw::ApplicationWindow, state: &AppState) {
    let button = gtk::Button::with_label("System status and recovery");
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
        Some(ProposalChange::Recovery { .. }) => IpcRequest::ProposeRecovery,
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
        "Peasy is finishing existing work before updating.\n\n".to_owned()
    } else {
        "".to_owned()
    };
    message.push_str(&format!(
        "Service version: {}\nProtocol: {}\nRunning executable: {}\nPackage source: {}",
        status.version, status.protocol, status.executable, status.nixpkgs
    ));
    if let Some(recovery) = status.recovery {
        message.push_str(&format!(
            "\n\n{}\nActive generation: {}\nPrevious generation: {}\nRequested packages: {}",
            recovery.message,
            recovery
                .active_generation
                .as_deref()
                .unwrap_or("unavailable"),
            recovery
                .previous_generation
                .as_deref()
                .unwrap_or("unavailable"),
            recovery.intended_packages.join(", ")
        ));
        if !recovery.intended_change.is_empty() {
            message.push_str("\n\nIntended configuration change (up to 100 lines):\n");
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
            let recover = gtk::Button::with_label("Review restoring the previous generation");
            let w = window.clone();
            let s = state.clone();
            recover.connect_clicked(move |_| {
                run_system_request(&w, s.clone(), IpcRequest::ProposeRecovery)
            });
            body.append(&recover);
        }
    } else if status.applying {
        message.push_str("\n\nA system change is running.");
    } else {
        message.push_str("\n\nNo interrupted operation needs recovery.");
    }
    let label = gtk::Label::new(Some(&message));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_selectable(true);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(240)
        .max_content_height(500)
        .child(&label)
        .build();
    body.append(&scroll);
    let refresh = gtk::Button::with_label("Refresh status");
    let w = window.clone();
    let s = state.clone();
    refresh.connect_clicked(move |_| run_system_request(&w, s.clone(), IpcRequest::Inspect));
    body.append(&refresh);
    let done = gtk::Button::with_label("Back");
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
        let safe = message
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
