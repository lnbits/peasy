//! Provider-independent release check. No work is added to package installation.
use crate::{AppState, IpcRequest, IpcResponse, run_system_request};
use gtk::glib;
use gtk::prelude::*;
use peasy_core::i18n::{tr, tr_args};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};

pub(super) fn add_controls(body: &gtk::Box, window: &adw::ApplicationWindow, state: &AppState) {
    let label = gtk::Label::new(Some(&tr("Checking for Peasy updates…")));
    label.set_wrap(true);
    label.set_xalign(if peasy_core::i18n::is_rtl() { 1.0 } else { 0.0 });
    label.set_selectable(true);
    body.append(&label);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let check = gtk::Button::with_label(&tr("Check for updates"));
    let update = gtk::Button::with_label(&tr("Update Peasy"));
    update.add_css_class("suggested-action");
    update.set_visible(false);
    row.append(&check);
    row.append(&update);
    body.append(&row);
    let release = Rc::new(RefCell::new(None));
    let selected = release.clone();
    let owner = window.clone();
    let context = state.clone();
    update.connect_clicked(move |_| {
        if let Some(release) = selected.borrow().clone() {
            run_system_request(
                &owner,
                context.clone(),
                IpcRequest::ProposePeasyUpdate { release },
            );
        }
    });
    let context = state.clone();
    let initial = Cell::new(true);
    check.connect_clicked(move |button| {
        button.set_sensitive(false);
        update.set_visible(false);
        release.borrow_mut().take();
        label.set_text(&tr("Checking for Peasy updates…"));
        let task = context.tasks.borrow_mut().start();
        let socket = context.args.socket.clone();
        let force = !initial.replace(false);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = task.work.scope(|| {
                peasy_client::IpcClient::new(socket)
                    .request(&IpcRequest::CheckPeasyUpdate { force })
            });
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
        });
        let button = button.clone();
        let label = label.clone();
        let update = update.clone();
        let release = release.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if task.view.is_cancelled() {
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(Ok(IpcResponse::PeasyUpdate { status })) => {
                    label.set_text(&tr_args(
                        "Installed: Peasy {version}. {message}",
                        &[
                            ("version", &status.current_version),
                            ("message", &status.message),
                        ],
                    ));
                    update.set_visible(status.release.is_some());
                    *release.borrow_mut() = status.release;
                }
                Ok(Ok(_)) => label.set_text(&tr(
                    "The system service does not support update checks yet.",
                )),
                Ok(Err(error)) => label.set_text(
                    &peasy_client::connectivity::offline_message(&error)
                        .map(tr)
                        .unwrap_or_else(|| {
                            tr_args(
                                "Could not check for updates: {error}. You can try again later.",
                                &[("error", &error)],
                            )
                        }),
                ),
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    label.set_text(&tr("Update check stopped; try again."))
                }
            }
            button.set_sensitive(true);
            task.view.cancel();
            glib::ControlFlow::Break
        });
    });
    check.emit_clicked();
}
