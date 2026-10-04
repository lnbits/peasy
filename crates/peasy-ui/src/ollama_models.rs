//! Settings-owned model inventory and downloads; dropdown labels are never IDs.
use crate::{AppState, run_system_request, tasks::Task};
use adw::prelude::*;
use gtk::glib;
use peasy_client::ollama_setup::{self, SetupAction};
use peasy_client::{
    DEFAULT_OLLAMA_URL,
    ollama_models::{
        InstalledModel, ModelCatalogue, PullProgress, pull_ollama_model, recommended_models,
        remove_ollama_model,
    },
};
use peasy_core::i18n::{tr, tr_args};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};

#[derive(Clone, Debug)]
struct Row {
    name: String,
    size: Option<u64>,
    installed: bool,
    downloadable: bool,
}

fn rows(catalogue: &ModelCatalogue, installed: &[InstalledModel], saved: Option<&str>) -> Vec<Row> {
    let mut rows = catalogue
        .models()
        .iter()
        .map(|model| {
            let found = installed.iter().find(|m| m.name == model.name);
            Row {
                name: model.name.clone(),
                size: found.and_then(|m| m.size).or(Some(model.size)),
                installed: found.is_some(),
                downloadable: true,
            }
        })
        .collect::<Vec<_>>();
    for model in installed {
        if !rows.iter().any(|row| row.name == model.name) {
            rows.push(Row {
                name: model.name.clone(),
                size: model.size,
                installed: true,
                downloadable: false,
            });
        }
    }
    if let Some(saved) = saved
        && !rows.iter().any(|row| row.name == saved)
    {
        rows.push(Row {
            name: saved.into(),
            size: None,
            installed: false,
            downloadable: false,
        });
    }
    rows
}

fn size_label(bytes: Option<u64>, approximate: bool) -> String {
    let Some(bytes) = bytes else {
        return tr("Size unknown");
    };
    let size = if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else {
        format!("{} MB", bytes / 1_000_000)
    };
    if approximate {
        format!("~{size}")
    } else {
        size
    }
}

impl Row {
    fn label(&self) -> String {
        let status = if self.installed {
            tr("Installed")
        } else if self.downloadable {
            tr("Download")
        } else {
            tr("Unavailable")
        };
        format!(
            "{} · {} · {}",
            self.name,
            size_label(self.size, !self.installed),
            status
        )
    }
}

#[derive(Clone)]
enum Operation {
    Refresh(bool),
    Download(String),
    Remove(String),
}

struct Inventory {
    models: Vec<InstalledModel>,
    catalogue: ModelCatalogue,
}

enum Event {
    Starting,
    Progress(PullProgress),
    Finished(Result<Inventory, Failure>),
}

struct Failure {
    message: String,
    setup: Option<SetupAction>,
}

pub(super) struct ModelSelector {
    root: gtk::Box,
    dropdown: gtk::DropDown,
    status: gtk::Label,
    progress: gtk::ProgressBar,
    refresh: gtk::Button,
    cancel: gtk::Button,
    remove: gtk::Button,
    setup: gtk::Button,
    setup_action: Cell<Option<SetupAction>>,
    catalogue: RefCell<ModelCatalogue>,
    rows: RefCell<Vec<Row>>,
    previous: RefCell<Option<String>>,
    updating: Cell<bool>,
    busy: Cell<bool>,
    refresh_pending: Cell<bool>,
    task: RefCell<Option<Task>>,
    state: AppState,
}

impl ModelSelector {
    pub fn new(
        window: &adw::ApplicationWindow,
        state: &AppState,
        saved: Option<String>,
    ) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let dropdown = gtk::DropDown::from_strings(&[]);
        dropdown.set_enable_search(true);
        dropdown.set_selected(gtk::INVALID_LIST_POSITION);
        dropdown.set_tooltip_text(Some(&tr("Select a model to use or download")));
        root.append(&dropdown);
        let note = gtk::Label::new(Some(&tr(
            "Selecting a missing model downloads it. Sizes are approximate downloads; running models needs additional memory.",
        )));
        note.set_wrap(true);
        note.set_halign(gtk::Align::Start);
        note.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
        root.append(&note);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_selectable(true);
        status.set_halign(gtk::Align::Start);
        root.append(&status);
        let progress = gtk::ProgressBar::new();
        progress.set_visible(false);
        root.append(&progress);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let refresh = gtk::Button::with_label(&tr("Refresh models"));
        let remove = gtk::Button::with_label(&tr("Remove model"));
        remove.set_sensitive(false);
        let cancel = gtk::Button::with_label(&tr("Cancel download"));
        cancel.set_visible(false);
        actions.append(&refresh);
        actions.append(&cancel);
        actions.append(&remove);
        root.append(&actions);
        let setup = gtk::Button::new();
        setup.add_css_class("suggested-action");
        setup.set_halign(gtk::Align::Start);
        setup.set_visible(false);
        root.append(&setup);
        let selector = Rc::new(Self {
            root,
            dropdown,
            status,
            progress,
            refresh,
            cancel,
            remove,
            setup,
            setup_action: Cell::new(None),
            catalogue: RefCell::new(ModelCatalogue::bundled()),
            rows: Default::default(),
            previous: RefCell::new(saved),
            updating: Cell::new(false),
            busy: Cell::new(false),
            refresh_pending: Cell::new(false),
            task: Default::default(),
            state: state.clone(),
        });
        let weak = Rc::downgrade(&selector);
        let owner = window.downgrade();
        selector.setup.connect_clicked(move |_| {
            if let Some(selector) = weak.upgrade()
                && let Some(window) = owner.upgrade()
                && let Some(action) = selector.setup_action.get()
                && !selector.busy.get()
            {
                let mut state = selector.state.clone();
                state.return_to_ollama_settings = true;
                run_system_request(&window, state, action.request());
            }
        });
        let weak = Rc::downgrade(&selector);
        selector.dropdown.connect_selected_notify(move |_| {
            let Some(selector) = weak.upgrade() else {
                return;
            };
            if selector.updating.get() || selector.busy.get() {
                return;
            }
            let row = selector
                .rows
                .borrow()
                .get(selector.dropdown.selected() as usize)
                .cloned();
            selector
                .remove
                .set_sensitive(row.as_ref().is_some_and(|r| r.installed));
            if let Some(row) = row {
                if row.installed {
                    *selector.previous.borrow_mut() = Some(row.name);
                    selector
                        .status
                        .set_text(&tr("Choose a model, then Save provider to use it."));
                } else if row.downloadable {
                    selector.start(Operation::Download(row.name));
                } else {
                    selector.status.set_text(&tr(
                        "This model is no longer installed. Choose another model.",
                    ));
                }
            }
        });
        let weak = Rc::downgrade(&selector);
        selector.refresh.connect_clicked(move |_| {
            if let Some(selector) = weak.upgrade() {
                selector.start(Operation::Refresh(true));
            }
        });
        let weak = Rc::downgrade(&selector);
        selector.cancel.connect_clicked(move |_| {
            if let Some(selector) = weak.upgrade()
                && let Some(task) = selector.task.borrow().as_ref()
            {
                task.work.cancel();
                selector.cancel.set_sensitive(false);
            }
        });
        let weak = Rc::downgrade(&selector);
        selector.remove.connect_clicked(move |_| {
            if let Some(selector) = weak.upgrade() {
                selector.confirm_remove();
            }
        });
        selector
    }

    pub fn activate(self: &Rc<Self>) {
        if self.busy.get() {
            self.refresh_pending.set(
                self.task
                    .borrow()
                    .as_ref()
                    .is_some_and(|task| task.work.is_cancelled()),
            );
            return;
        }
        self.start(Operation::Refresh(false));
    }

    pub fn deactivate(&self) {
        self.refresh_pending.set(false);
        if let Some(task) = self.task.borrow().as_ref() {
            task.work.cancel();
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn selected_model(&self) -> Option<String> {
        if self.busy.get() {
            return None;
        }
        self.rows
            .borrow()
            .get(self.dropdown.selected() as usize)
            .filter(|row| row.installed)
            .map(|row| row.name.clone())
    }

    fn populate(&self, installed: &[InstalledModel], selected: Option<&str>) {
        self.updating.set(true);
        let entries = rows(&self.catalogue.borrow(), installed, selected);
        let index = selected.and_then(|name| {
            entries
                .iter()
                .position(|row| row.name == name && row.installed)
        });
        let labels: Vec<_> = entries.iter().map(Row::label).collect();
        *self.rows.borrow_mut() = entries;
        self.dropdown.set_model(Some(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        self.dropdown
            .set_selected(index.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
        self.updating.set(false);
        self.remove.set_sensitive(index.is_some());
    }

    fn check_removal(&self, name: &str) -> anyhow::Result<()> {
        if matches!(self.state.providers.load()?, Some(peasy_client::ProviderSettings::Ollama { model, .. }) if model == name)
        {
            anyhow::bail!(tr(
                "Save a different provider or model before removing the active model."
            ));
        }
        Ok(())
    }

    fn confirm_remove(self: &Rc<Self>) {
        let Some(name) = self.selected_model() else {
            return;
        };
        if let Err(error) = self.check_removal(&name) {
            self.status.set_text(&error.to_string());
            return;
        }
        let mut body = tr(
            "This removes the model from local Ollama for all apps using this service. You can download it again later.",
        );
        if name == "qwen3:0.6b" {
            body.push_str("\n\n");
            body.push_str(&tr("On Peasy ISO installations, NixOS restores the bundled model when Ollama restarts. Removing it does not free its Nix store space."));
        }
        let dialog = adw::AlertDialog::builder()
            .heading(tr_args("Remove {model}?", &[("model", &name)]))
            .body(body)
            .build();
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("remove", &tr("Remove model"));
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response == "remove"
                && let Some(selector) = weak.upgrade()
            {
                // Recheck saved settings after confirmation; never delete the
                // active model merely because a different row was selected.
                if let Err(error) = selector.check_removal(&name) {
                    selector.status.set_text(&error.to_string());
                } else {
                    selector.start(Operation::Remove(name.clone()));
                }
            }
        });
        dialog.present(Some(&self.root));
    }

    pub(super) fn set_setup(&self, action: Option<SetupAction>) {
        self.setup_action.set(action);
        self.setup.set_visible(action.is_some());
        if let Some(action) = action {
            self.setup.set_label(&tr(match action {
                SetupAction::Install => "Install and enable Ollama",
                SetupAction::Start => "Start Ollama",
            }));
        }
    }

    fn start(self: &Rc<Self>, operation: Operation) {
        let download = match &operation {
            Operation::Download(name) => Some(name.clone()),
            _ => None,
        };
        let removing = matches!(operation, Operation::Remove(_));
        if self.busy.replace(true) {
            return;
        }
        self.set_setup(None);
        self.dropdown.set_sensitive(false);
        if download.is_none() {
            self.updating.set(true);
            self.rows.borrow_mut().clear();
            self.dropdown.set_model(None::<&gtk::StringList>);
            self.updating.set(false);
        }
        self.refresh.set_sensitive(false);
        self.remove.set_sensitive(false);
        self.cancel.set_sensitive(true);
        self.cancel.set_visible(download.is_some());
        self.progress.set_visible(download.is_some());
        self.progress.set_fraction(0.0);
        self.status.set_text(&match &operation {
            Operation::Download(model) => tr_args("Installing {model}…", &[("model", model)]),
            Operation::Remove(model) => tr_args("Removing {model}…", &[("model", model)]),
            Operation::Refresh(_) => tr("Checking local Ollama…"),
        });
        let task = self.state.tasks.borrow_mut().start();
        *self.task.borrow_mut() = Some(task.clone());
        // Bound progress buffering even when the window is busy or closed.
        let (tx, rx) = mpsc::sync_channel(8);
        let worker_task = task.clone();
        let catalogue = self.catalogue.borrow().clone();
        std::thread::spawn(move || {
            let result = worker_task.work.scope(|| -> anyhow::Result<Inventory> {
                let models = match &operation {
                    Operation::Download(model) => {
                        pull_ollama_model(DEFAULT_OLLAMA_URL, model, &catalogue, |progress| {
                            let _ = tx.try_send(Event::Progress(progress));
                        })?
                    }
                    Operation::Remove(model) => remove_ollama_model(DEFAULT_OLLAMA_URL, model)?,
                    Operation::Refresh(_) => ollama_setup::list_or_start_models(|| {
                        let _ = tx.try_send(Event::Starting);
                    })?,
                };
                let catalogue = if let Operation::Refresh(force) = operation {
                    recommended_models(force)
                } else {
                    catalogue
                };
                Ok(Inventory { models, catalogue })
            });
            let result = result.map_err(|error| Failure {
                setup: error
                    .downcast_ref::<ollama_setup::SetupRequired>()
                    .map(|required| required.0),
                message: format!("{error:#}"),
            });
            let _ = tx.send(Event::Finished(result));
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(selector) = weak.upgrade() else {
                task.work.cancel();
                return glib::ControlFlow::Break;
            };
            if task.view.is_cancelled() {
                return glib::ControlFlow::Break;
            }
            for _ in 0..16 {
                match rx.try_recv() {
                    Ok(Event::Starting) => selector.status.set_text(&tr("Starting local Ollama…")),
                    Ok(Event::Progress(event)) => {
                        if let (Some(total), Some(completed), Some(model)) =
                            (event.total, event.completed, download.as_ref())
                            && total > 0
                        {
                            let fraction = (completed as f64 / total as f64).clamp(0.0, 1.0);
                            selector.progress.set_fraction(fraction);
                            selector.status.set_text(&tr_args(
                                "Downloading {model}: {percent}% of current file",
                                &[
                                    ("model", model),
                                    ("percent", &format!("{:.0}", fraction * 100.0)),
                                ],
                            ));
                        } else {
                            selector.progress.pulse();
                            if let Some(model) = &download {
                                selector
                                    .status
                                    .set_text(&tr_args("Installing {model}…", &[("model", model)]));
                            }
                        }
                    }
                    Ok(Event::Finished(result)) => {
                        selector.busy.set(false);
                        selector.refresh.set_sensitive(true);
                        selector.cancel.set_visible(false);
                        selector.progress.set_visible(false);
                        selector.task.borrow_mut().take();
                        task.view.cancel();
                        if selector.refresh_pending.replace(false) {
                            selector.start(Operation::Refresh(false));
                            return glib::ControlFlow::Break;
                        }
                        match result {
                            Ok(Inventory { models, catalogue }) => {
                                *selector.catalogue.borrow_mut() = catalogue;
                                let selected = download
                                    .clone()
                                    .or_else(|| selector.previous.borrow().clone());
                                selector.populate(&models, selected.as_deref());
                                *selector.previous.borrow_mut() =
                                    selected.filter(|name| models.iter().any(|m| &m.name == name));
                                selector.dropdown.set_sensitive(true);
                                selector.status.set_text(&tr(if removing {
                                    "Model removed."
                                } else if download.is_some() {
                                    "Model installed. Save provider to use it."
                                } else {
                                    "Choose a model, then Save provider to use it."
                                }));
                            }
                            Err(Failure {
                                message: error,
                                setup,
                            }) => {
                                if !task.work.is_cancelled() {
                                    selector.set_setup(setup);
                                }
                                // An incomplete model is never considered ready or saved.
                                selector.updating.set(true);
                                let index = selector.previous.borrow().as_ref().and_then(|name| {
                                    selector
                                        .rows
                                        .borrow()
                                        .iter()
                                        .position(|r| r.installed && &r.name == name)
                                });
                                selector.dropdown.set_selected(
                                    index.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32),
                                );
                                selector.updating.set(false);
                                selector
                                    .dropdown
                                    .set_sensitive(!selector.rows.borrow().is_empty());
                                let message = if task.work.is_cancelled() {
                                    tr("Download cancelled.")
                                } else if let Some(action) = setup {
                                    tr(match action {
                                        SetupAction::Install => {
                                            "Ollama is not set up as a system service. Review installation below, then choose a model."
                                        }
                                        SetupAction::Start => {
                                            "Ollama is installed but stopped. Review starting it below, then choose a model."
                                        }
                                    })
                                } else if let Some(message) =
                                    peasy_client::connectivity::offline_message(&error)
                                {
                                    tr(message)
                                } else if removing {
                                    tr_args(
                                        "Removal could not be confirmed. Refresh the model list.\n{error}",
                                        &[("error", &error)],
                                    )
                                } else {
                                    error
                                };
                                selector
                                    .remove
                                    .set_sensitive(selector.selected_model().is_some());
                                selector.status.set_text(&message);
                            }
                        }
                        return glib::ControlFlow::Break;
                    }
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        selector.busy.set(false);
                        selector.refresh.set_sensitive(true);
                        selector
                            .dropdown
                            .set_sensitive(!selector.rows.borrow().is_empty());
                        selector.cancel.set_visible(false);
                        selector.progress.set_visible(false);
                        selector.task.borrow_mut().take();
                        task.view.cancel();
                        selector
                            .status
                            .set_text(&tr("Ollama model check stopped unexpectedly."));
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_preserves_custom_models_and_never_promotes_missing_models() {
        let installed = vec![
            InstalledModel {
                name: "qwen3:0.6b".into(),
                size: Some(524_000_000),
            },
            InstalledModel {
                name: "my-model:latest".into(),
                size: Some(123),
            },
        ];
        let entries = rows(
            &ModelCatalogue::bundled(),
            &installed,
            Some("old-model:latest"),
        );
        assert_eq!(entries.len(), 8);
        assert!(entries[0].installed);
        assert_eq!(entries[0].size, Some(524_000_000));
        assert!(!entries[1].installed);
        assert!(entries[1].downloadable);
        assert!(entries[6].installed);
        assert_eq!(entries[6].name, "my-model:latest");
        assert!(!entries[7].installed);
        assert!(!entries[7].downloadable);
    }
}
