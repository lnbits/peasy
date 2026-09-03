use adw::prelude::*;
use anyhow::{Context, Result};
use clap::Parser;
use gtk::glib;
use peasy_client::{
    Choice, DEFAULT_OLLAMA_URL, DEFAULT_OPENAI_MODEL, KeyStore, LocalAction, LocalProposal,
    PeasyClient, ProviderSettings, ProviderStore, Resolution, ResolveStage, list_ollama_models,
    load_model_provider,
};
use peasy_core::{DiffKind, Proposal};
use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

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
}

enum ResolveMessage {
    Progress(ResolveStage),
    Finished(std::result::Result<Resolution, String>),
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
    if let Some(window) = app.active_window() {
        let window = window
            .downcast::<adw::ApplicationWindow>()
            .expect("Peasy owns an Adwaita application window");
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
    window.connect_close_request(|window| {
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

fn show_provider_settings(window: &adw::ApplicationWindow, state: AppState) {
    let (root, body) = page("Peasy settings");
    let heading = gtk::Label::new(Some("AI provider"));
    heading.add_css_class("title-3");
    heading.set_halign(gtk::Align::Start);
    body.append(&heading);

    let provider = gtk::DropDown::from_strings(&["OpenAI", "Ollama (local)"]);
    body.append(&provider);

    let openai_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let key_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Enter a new OpenAI API key")
        .show_peek_icon(true)
        .build();
    openai_box.append(&key_entry);
    let key_note = gtk::Label::new(Some(if state.keys.load().ok().flatten().is_some() {
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
    remove_key.set_sensitive(state.keys.load().ok().flatten().is_some());
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

    if let Ok(Some(settings)) = state.providers.load() {
        match settings {
            ProviderSettings::OpenAi { model } => openai_model.set_text(&model),
            ProviderSettings::Ollama { model, .. } => {
                provider.set_selected(1);
                ollama_model.set_text(&model);
            }
        }
    }
    stack.set_visible_child_name(if provider.selected() == 1 {
        "ollama"
    } else {
        "openai"
    });
    let stack_clone = stack.clone();
    provider.connect_selected_notify(move |provider| {
        stack_clone.set_visible_child_name(if provider.selected() == 1 {
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
        "Export your complete NixOS configuration and everything managed by Peasy.",
    ));
    export_text.set_wrap(true);
    export_text.set_xalign(0.0);
    export_copy.append(&export_text);
    let export_note = gtk::Label::new(Some(
        "Creates a portable folder with restore instructions. Credentials are not included.",
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
        ollama_status.clone(),
        ollama_model.clone(),
        detected_models.clone(),
    );
    let detected_clone = detected_models.clone();
    let ollama_status_clone = ollama_status.clone();
    let ollama_model_clone = ollama_model.clone();
    refresh.connect_clicked(move |_| {
        refresh_ollama_models(
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
            let settings = if provider.selected() == 1 {
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

const HOST_CONFIGURATION_POINTER: &str = "/etc/peasy/host-configuration-path";
const PEASY_MODULE_POINTER: &str = "/etc/peasy/module-import-path";
const PEASY_SOURCE: &str = "/run/current-system/sw/share/peasy/source";
const MAX_CONFIGURATION_BYTES: u64 = 4 * 1024 * 1024;
const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXPORT_ENTRIES: usize = 4096;
const EXPORT_DIRECTORY: &str = "peasy-system-config";

#[derive(Clone, Debug)]
struct ConfigurationExport {
    source: PathBuf,
    peasy_module: PathBuf,
    peasy_source: PathBuf,
}

#[derive(Default)]
struct ExportSize {
    bytes: u64,
    entries: usize,
}

fn configuration_export(_socket: &Path) -> Result<ConfigurationExport> {
    configuration_export_from(
        Path::new(HOST_CONFIGURATION_POINTER),
        Path::new(PEASY_MODULE_POINTER),
        Path::new(PEASY_SOURCE),
    )
}

fn configuration_export_from(
    pointer: &Path,
    peasy_module_pointer: &Path,
    peasy_source: &Path,
) -> Result<ConfigurationExport> {
    let source = configured_source_path(pointer)?;
    let metadata = fs::metadata(&source)
        .with_context(|| format!("reading metadata for {}", source.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("{} is not a regular configuration file", source.display());
    }
    if metadata.len() > MAX_CONFIGURATION_BYTES {
        anyhow::bail!("configuration is larger than 4 MiB");
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .context("configured source has no portable file name")?;
    if name != "configuration.nix" {
        anyhow::bail!("portable system export currently requires a configuration.nix host source");
    }
    let peasy_module = configured_path(peasy_module_pointer, "/nix/store/peasy/nix/module.nix")?;
    if !peasy_source.is_dir() {
        anyhow::bail!("installed Peasy source is unavailable for the portable export");
    }
    Ok(ConfigurationExport {
        source,
        peasy_module,
        peasy_source: peasy_source.to_owned(),
    })
}

fn write_configuration_export(export: &ConfigurationExport, parent: &Path) -> Result<PathBuf> {
    if !parent.is_dir() {
        anyhow::bail!("export destination is not a directory");
    }
    let source_root = export
        .source
        .parent()
        .context("configured source has no parent directory")?;
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(source_root)?) {
        anyhow::bail!("choose a destination outside the active configuration directory");
    }
    let destination = parent.join(EXPORT_DIRECTORY);
    if destination.exists() {
        anyhow::bail!(
            "{} already exists; rename it or choose another folder",
            destination.display()
        );
    }
    let temporary = parent.join(format!(".{EXPORT_DIRECTORY}-{}", std::process::id()));
    if temporary.exists() {
        anyhow::bail!("a temporary Peasy export already exists");
    }

    fs::create_dir(&temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700))?;
    let result: Result<()> = (|| {
        let host_destination = temporary.join("host");
        let mut size = ExportSize::default();
        copy_configuration_directory(source_root, &host_destination, &mut size)?;
        copy_configuration_directory(&export.peasy_source, &temporary.join("peasy"), &mut size)?;
        make_host_configuration_portable(
            &host_destination.join("configuration.nix"),
            &export.peasy_module,
        )?;
        write_private_file(
            &temporary.join("configuration.nix"),
            br#"# Exported by Peasy. The complete host configuration is under ./host.
{ lib, ... }:
{
  imports = [
    ./host/configuration.nix
  ];
  services.peasy.hostConfiguration = lib.mkForce "/etc/nixos/configuration.nix";
  services.peasy.managedModule = lib.mkForce "/etc/nixos/host/.peasy/peasy-managed.nix";
}
"#,
        )?;
        write_private_file(
            &temporary.join("README.txt"),
            br#"PEASY NIXOS SYSTEM EXPORT

This folder contains the complete host configuration tree under host/, including
host/.peasy/peasy-managed.nix, plus a copy of Peasy under peasy/. API keys and provider
credentials are deliberately excluded.

To restore on another NixOS machine:

  1. Review the files for machine-specific settings and private values.
  2. From this folder, back up and replace the destination configuration:

       sudo cp -a /etc/nixos /etc/nixos.before-peasy-restore
       sudo cp -a configuration.nix host peasy /etc/nixos/

  3. When restoring to different hardware, replace the bundled hardware module:

       nixos-generate-config --show-hardware-config | sudo tee /etc/nixos/host/hardware-configuration.nix >/dev/null

  4. Rebuild:

       sudo nixos-rebuild switch --no-flake

Absolute imports outside the original configuration directory must also be
made available on the new machine or changed to portable paths before rebuilding.
"#,
        )?;
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    Ok(destination)
}

fn make_host_configuration_portable(path: &Path, peasy_module: &Path) -> Result<()> {
    let peasy_root = peasy_module
        .parent()
        .and_then(Path::parent)
        .context("Peasy module path has no source root")?;
    let peasy_root = peasy_root
        .to_str()
        .context("Peasy module source path is not UTF-8")?;
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let portable = contents.replace(peasy_root, "/etc/nixos/peasy");
    fs::write(path, portable)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn copy_configuration_directory(
    source: &Path,
    destination: &Path,
    size: &mut ExportSize,
) -> Result<()> {
    fs::create_dir(destination)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o700))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        size.entries += 1;
        if size.entries > MAX_EXPORT_ENTRIES {
            anyhow::bail!("configuration tree contains more than {MAX_EXPORT_ENTRIES} entries");
        }
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.is_dir() {
            copy_configuration_directory(&source_path, &destination_path, size)?;
        } else if metadata.is_file() {
            size.bytes = size.bytes.saturating_add(metadata.len());
            if size.bytes > MAX_EXPORT_BYTES {
                anyhow::bail!("configuration tree is larger than 64 MiB");
            }
            let contents = fs::read(&source_path)
                .with_context(|| format!("reading {}", source_path.display()))?;
            write_private_file(&destination_path, &contents)?;
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(&source_path)?;
            if target.is_absolute() {
                if is_nix_build_result_link(&source_path, &target) {
                    continue;
                }
                anyhow::bail!(
                    "{} is an absolute symlink and cannot be exported portably",
                    source_path.display()
                );
            }
            std::os::unix::fs::symlink(target, destination_path)?;
        } else {
            anyhow::bail!(
                "{} is not a portable configuration file",
                source_path.display()
            );
        }
    }
    Ok(())
}

fn is_nix_build_result_link(path: &Path, target: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    target.starts_with("/nix/store")
        && name.is_some_and(|name| name == "result" || name.starts_with("result-"))
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn configured_source_path(pointer: &Path) -> Result<PathBuf> {
    configured_path(pointer, "/etc/nixos/configuration.nix")
}

fn configured_path(pointer: &Path, fallback: &str) -> Result<PathBuf> {
    let source = match fs::metadata(pointer) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.len() > 4096 {
                anyhow::bail!("configured export pointer is invalid");
            }
            let mut value = String::new();
            OpenOptions::new()
                .read(true)
                .open(pointer)?
                .take(4097)
                .read_to_string(&mut value)?;
            if value.len() > 4096 {
                anyhow::bail!("configured export pointer is invalid");
            }
            value.trim().to_owned()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fallback.to_owned(),
        Err(error) => return Err(error.into()),
    };
    if source.is_empty() || source.len() > 4096 || source.chars().any(char::is_control) {
        anyhow::bail!("configured export path is invalid");
    }
    let source = PathBuf::from(source);
    if !source.is_absolute() {
        anyhow::bail!("configured export path must be absolute");
    }
    Ok(source)
}

fn refresh_ollama_models(
    status: gtk::Label,
    model_entry: gtk::Entry,
    detected: Rc<RefCell<Vec<String>>>,
) {
    status.set_text("Checking local Ollama…");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ =
            tx.send(list_ollama_models(DEFAULT_OLLAMA_URL).map_err(|error| format!("{error:#}")));
    });
    glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
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
    });
}

fn show_prompt(window: &adw::ApplicationWindow, state: AppState) {
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
    body.append(&entry);
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
        let request = entry_clone.text().trim().to_owned();
        if request.is_empty() {
            return;
        }
        button.set_sensitive(false);
        status_clone.set_text("Understanding request…");
        write_panel_status("…thinking");
        let Some(client) = state_clone.client.borrow().clone() else {
            status_clone.set_text("AI provider is not configured. Open Peasy settings.");
            return;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = client
                .resolve_with_progress(&request, move |stage| {
                    let _ = progress_tx.send(ResolveMessage::Progress(stage));
                })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(ResolveMessage::Finished(result));
        });
        let window = window_clone.clone();
        let state = state_clone.clone();
        let button = button.clone();
        let status = status_clone.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
            Ok(ResolveMessage::Progress(stage)) => {
                status.set_text(stage.message());
                write_panel_status(stage.panel_message());
                glib::ControlFlow::Continue
            }
            Ok(ResolveMessage::Finished(Ok(resolution))) => {
                show_resolution(&window, state.clone(), resolution);
                glib::ControlFlow::Break
            }
            Ok(ResolveMessage::Finished(Err(error))) => {
                status.set_text(&error);
                write_panel_status("Peasy needs attention");
                button.set_sensitive(true);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                status.set_text("The request worker stopped unexpectedly.");
                write_panel_status("Peasy request stopped");
                button.set_sensitive(true);
                glib::ControlFlow::Break
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
        Resolution::Proposal(proposal) => show_proposal(window, state, proposal),
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
            std::thread::spawn(move || {
                let _ = tx.send(
                    client
                        .select(choice, index)
                        .map_err(|error| format!("{error:#}")),
                );
            });
            let window = window_clone.clone();
            let state = state_clone.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
                Ok(Ok(resolution)) => {
                    show_resolution(&window, state.clone(), resolution);
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
                        "The package-selection worker stopped unexpectedly.",
                    );
                    glib::ControlFlow::Break
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

fn show_apply_progress(window: &adw::ApplicationWindow, title: &str) -> gtk::Label {
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
    let status = gtk::Label::new(Some("Starting the NixOS build…"));
    status.set_halign(gtk::Align::Start);
    status.set_wrap(true);
    status.add_css_class("heading");
    progress.append(&status);
    body.append(&progress);

    let explanation = gtk::Label::new(Some(
        "Peasy is validating, building, and activating your new system generation. This can take a few minutes; this window will update when it finishes.",
    ));
    explanation.set_halign(gtk::Align::Start);
    explanation.set_wrap(true);
    explanation.set_xalign(0.0);
    explanation.add_css_class("dim-label");
    body.append(&explanation);
    show_content(window, &root, 440, -1);
    status
}

fn show_proposal(window: &adw::ApplicationWindow, state: AppState, proposal: Proposal) {
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
        let Some(client) = state.client.borrow().clone() else {
            show_error_message(
                &window_apply,
                state.clone(),
                "AI provider is not configured. Open Peasy settings.",
            );
            return;
        };
        let progress = show_apply_progress(&window_apply, &proposal.title);
        write_panel_status(&format!("…applying {}", proposal.title));
        let proposal = proposal.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(
                client
                    .apply(&proposal)
                    .map_err(|error| format!("{error:#}")),
            );
        });
        let window = window_apply.clone();
        let state = state.clone();
        let started = Instant::now();
        glib::timeout_add_local(Duration::from_millis(80), move || match rx.try_recv() {
            Ok(Ok(result)) => {
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
            Ok(Err(error)) => {
                show_error_message(&window, state.clone(), &error);
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => {
                let elapsed = started.elapsed();
                if elapsed >= Duration::from_secs(30) {
                    progress.set_text(
                        "Still building… NixOS may be downloading or compiling packages.",
                    );
                } else if elapsed >= Duration::from_secs(2) {
                    progress.set_text("Validating and building the new NixOS generation…");
                }
                glib::ControlFlow::Continue
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                show_error_message(
                    &window,
                    state.clone(),
                    "The configuration worker stopped unexpectedly.",
                );
                glib::ControlFlow::Break
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
    std::thread::spawn(move || {
        let result = client
            .apply_local(&proposal, password.as_deref())
            .map_err(|error| format!("{error:#}"));
        let _ = tx.send(result);
    });
    let window = window.clone();
    glib::timeout_add_local(Duration::from_millis(80), move || match rx.try_recv() {
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
    });
}

fn show_message(window: &adw::ApplicationWindow, state: AppState, message: &str) {
    clear_panel_status();
    render_message(window, state, message);
}

fn show_error_message(window: &adw::ApplicationWindow, state: AppState, message: &str) {
    write_panel_status("Peasy needs attention");
    render_message(window, state, message);
}

fn render_message(window: &adw::ApplicationWindow, state: AppState, message: &str) {
    let (root, body) = page("Peasy");
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_halign(gtk::Align::Start);
    body.append(&label);
    let done = gtk::Button::with_label("Done");
    done.set_halign(gtk::Align::End);
    let window_clone = window.clone();
    done.connect_clicked(move |_| show_prompt(&window_clone, state.clone()));
    body.append(&done);
    show_content(window, &root, 440, -1);
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
    fn configuration_export_contains_host_tree_and_peasy_managed_module() {
        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("source");
        fs::create_dir(&host).unwrap();
        let source = host.join("configuration.nix");
        let pointer = temp.path().join("host-configuration-path");
        let original_peasy_root = temp.path().join("original-peasy");
        let original_module = original_peasy_root.join("nix/module.nix");
        let contents = format!(
            "{{ pkgs, ... }}: {{ imports = [ {} ./.peasy/peasy-managed.nix ]; environment.systemPackages = [ pkgs.vlc ]; }}\n",
            original_module.display()
        );
        fs::write(&source, &contents).unwrap();
        fs::write(host.join("hardware-configuration.nix"), b"{ ... }: {}\n").unwrap();
        std::os::unix::fs::symlink(
            "/nix/store/00000000000000000000000000000000-build-result",
            host.join("result"),
        )
        .unwrap();
        fs::create_dir(host.join(".peasy")).unwrap();
        fs::write(
            host.join(".peasy/peasy-managed.nix"),
            b"{ pkgs, ... }: { environment.systemPackages = [ pkgs.firefox ]; }\n",
        )
        .unwrap();
        fs::write(&pointer, format!("{}\n", source.display())).unwrap();
        let peasy_source = temp.path().join("peasy-source");
        fs::create_dir_all(peasy_source.join("nix")).unwrap();
        fs::write(peasy_source.join("nix/module.nix"), b"{ ... }: {}\n").unwrap();
        let module_pointer = temp.path().join("module-import-path");
        fs::write(
            &module_pointer,
            original_module.to_string_lossy().as_bytes(),
        )
        .unwrap();

        let export = configuration_export_from(&pointer, &module_pointer, &peasy_source).unwrap();
        let selected = temp.path().join("selected");
        fs::create_dir(&selected).unwrap();
        let destination = write_configuration_export(&export, &selected).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("host/configuration.nix")).unwrap(),
            contents.replace(original_peasy_root.to_str().unwrap(), "/etc/nixos/peasy")
        );
        assert!(
            destination
                .join("host/hardware-configuration.nix")
                .is_file()
        );
        assert!(
            fs::read_to_string(destination.join("host/.peasy/peasy-managed.nix"))
                .unwrap()
                .contains("pkgs.firefox")
        );
        assert!(
            fs::read_to_string(destination.join("configuration.nix"))
                .unwrap()
                .contains("./host/configuration.nix")
        );
        assert!(destination.join("README.txt").is_file());
        assert!(destination.join("peasy/nix/module.nix").is_file());
        assert!(!destination.join("host/result").exists());
    }

    #[test]
    fn configuration_export_rejects_relative_and_oversized_sources() {
        let temp = tempfile::tempdir().unwrap();
        let pointer = temp.path().join("host-configuration-path");
        fs::write(&pointer, "../configuration.nix\n").unwrap();
        assert!(
            configuration_export_from(
                &pointer,
                Path::new("/missing-module-pointer"),
                Path::new("/missing-peasy-source"),
            )
            .is_err()
        );

        let source = temp.path().join("configuration.nix");
        fs::write(&source, vec![b'x'; MAX_CONFIGURATION_BYTES as usize + 1]).unwrap();
        fs::write(&pointer, source.to_string_lossy().as_bytes()).unwrap();
        assert!(
            configuration_export_from(
                &pointer,
                Path::new("/missing-module-pointer"),
                Path::new("/missing-peasy-source"),
            )
            .is_err()
        );
    }
}
