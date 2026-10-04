use crate::*;
use peasy_client::chat::{Answer, Mode, Reply, Request, Turn, attachments::Attachment};

enum Event {
    Stage(ResolveStage),
    Finished(Result<Outcome, String>),
}
enum Outcome {
    Chat(Reply),
    Task(Box<Resolution>),
}

fn message(body: &gtk::Box, who: &str, text: &str, formatted: bool) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("card");
    card.set_margin_bottom(6);
    let inside = gtk::Box::new(gtk::Orientation::Vertical, 8);
    inside.set_margin_top(14);
    inside.set_margin_bottom(14);
    inside.set_margin_start(16);
    inside.set_margin_end(16);
    let name = gtk::Label::new(Some(who));
    name.add_css_class("heading");
    name.set_halign(gtk::Align::Start);
    inside.append(&name);
    if formatted {
        inside.append(&crate::chat_format::render(text));
    } else {
        let label = gtk::Label::new(Some(text));
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_selectable(true);
        label.set_xalign(0.0);
        inside.append(&label);
    }
    card.append(&inside);
    body.append(&card);
}
fn answer_widgets(
    body: &gtk::Box,
    answer: &Answer,
    window: &adw::ApplicationWindow,
    state: &AppState,
) {
    message(body, "Peasy", &answer.message, true);
    // Source titles supplement inline citations and remain accessible to screen readers.
    for (title, url) in &answer.sources {
        let link = gtk::LinkButton::with_label(url, title);
        link.set_halign(gtk::Align::Start);
        body.append(&link);
    }
    if let Some(task) = &answer.suggested_task {
        let button = gtk::Button::with_label(&tr("Review suggested task"));
        button.set_tooltip_text(Some(task));
        button.set_halign(gtk::Align::Start);
        body.append(&button);
        let task = task.clone();
        let state = state.clone();
        let window = window.clone();
        button.connect_clicked(move |_| {
            *state.request.borrow_mut() = task.clone();
            state.chat_mode.set(Mode::Task);
            state.chat_task_reply.set(false);
            show(&window, state.clone());
        });
    }
    if answer.earlier_omitted {
        let note = gtk::Label::new(Some(&tr(
            "Earlier messages were left out to fit this model. Start a new conversation if the topic has changed.",
        )));
        note.set_wrap(true);
        note.add_css_class("dim-label");
        body.append(&note);
    }
}
pub fn complete_task(window: &adw::ApplicationWindow, state: AppState, text: &str, reply: bool) {
    if let Some((prompt, attachments)) = state.chat_pending.borrow_mut().take() {
        state.conversation.borrow_mut().push(Turn {
            user: prompt,
            attachments,
            answer: Answer {
                message: tr(text),
                ..Default::default()
            },
        });
    }
    chat_history::autosave(&state);
    state.chat_task_reply.set(reply);
    state.request.borrow_mut().clear();
    clear_panel_status();
    show(window, state);
}
pub fn show(window: &adw::ApplicationWindow, state: AppState) {
    state.tasks.borrow_mut().close();
    discard_pending(&state);
    clear_panel_status();
    let panel = take_panel_request().ok().flatten();
    if panel.is_some() {
        state.chat_mode.set(Mode::Task);
    }
    let (root, body, header) = page_with_header("Peasy");
    let settings = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text(tr("Settings"))
        .build();
    header.pack_end(&settings);
    let w = window.clone();
    let s = state.clone();
    settings.connect_clicked(move |_| show_provider_settings(&w, s.clone()));
    let history = gtk::Button::builder()
        .icon_name("document-open-recent-symbolic")
        .tooltip_text(tr("Chat history"))
        .build();
    header.pack_start(&history);
    let w = window.clone();
    let s = state.clone();
    history.connect_clicked(move |_| chat_history::show(&w, s.clone()));
    let new_chat = gtk::Button::builder()
        .icon_name("document-new-symbolic")
        .tooltip_text(tr("New conversation"))
        .build();
    header.pack_start(&new_chat);
    let w = window.clone();
    let s = state.clone();
    new_chat.connect_clicked(move |_| chat_history::new_chat(&w, s.clone()));
    let history_notice = gtk::Label::new(state.history_notice.borrow().as_deref());
    history_notice.set_wrap(true);
    history_notice.set_xalign(0.0);
    history_notice.set_visible(state.history_notice.borrow().is_some());
    *state.history_notice_widget.borrow_mut() = history_notice.downgrade();
    body.append(&history_notice);
    let tagline = gtk::Label::new(Some(&tr("Tell your computer what you want.")));
    tagline.add_css_class("title-2");
    tagline.set_wrap(true);
    tagline.set_halign(gtk::Align::Start);
    body.append(&tagline);
    let recovery_notice = gtk::Label::new(None);
    recovery_notice.set_wrap(true);
    recovery_notice.set_xalign(0.0);
    body.append(&recovery_notice);
    let ipc = peasy_client::IpcClient::new(state.args.socket.clone());
    let notice_task = state.tasks.borrow_mut().start();
    let work = notice_task.work.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work.scope(|| ipc.request(&IpcRequest::Inspect)));
    });
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if notice_task.view.is_cancelled() {
            return glib::ControlFlow::Break;
        }
        match rx.try_recv() {
            Ok(Ok(IpcResponse::Inspection { status })) => {
                notice_task.view.cancel();
                if let Some(info) = status.recovery {
                    recovery_notice.set_text(&tr_args(
                        "{message} Open System status and recovery for details.",
                        &[("message", &info.message)],
                    ));
                }
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            _ => {
                notice_task.view.cancel();
                glib::ControlFlow::Break
            }
        }
    });
    let messages = gtk::Box::new(gtk::Orientation::Vertical, 12);
    messages.set_vexpand(true);
    if state.conversation.borrow().turns.is_empty() {
        let welcome = gtk::Label::new(Some(&tr(
            "Ask a question, write something, or tell Peasy what to do.",
        )));
        welcome.set_wrap(true);
        welcome.set_xalign(0.0);
        welcome.add_css_class("dim-label");
        messages.append(&welcome);
    }
    for turn in &state.conversation.borrow().turns {
        let names = turn
            .attachments
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        message(
            &messages,
            &tr("You"),
            &if names.is_empty() {
                turn.user.clone()
            } else {
                format!("{}\n📎 {names}", turn.user)
            },
            false,
        );
        answer_widgets(&messages, &turn.answer, window, &state);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .min_content_height(220)
        .child(&messages)
        .build();
    body.append(&scroll);
    let adjustment = scroll.vadjustment();
    adjustment.connect_changed(move |a| {
        let adjustment = a.clone();
        glib::idle_add_local_once(move || {
            adjustment.set_value((adjustment.upper() - adjustment.page_size()).max(0.0))
        });
    });
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    modes.add_css_class("linked");
    let auto = gtk::ToggleButton::with_label(&tr("Auto"));
    let ask = gtk::ToggleButton::with_label(&tr("Ask"));
    let task_mode = gtk::ToggleButton::with_label(&tr("Task"));
    ask.set_group(Some(&auto));
    task_mode.set_group(Some(&auto));
    for (button, mode) in [
        (&auto, Mode::Auto),
        (&ask, Mode::Ask),
        (&task_mode, Mode::Task),
    ] {
        button.set_active(state.chat_mode.get() == mode);
        modes.append(button);
        let mode_state = state.chat_mode.clone();
        button.connect_toggled(move |b| {
            if b.is_active() {
                mode_state.set(mode);
            }
        });
    }
    body.append(&modes);
    let provider = match state.providers.load().ok().flatten() {
        Some(ProviderSettings::Ollama { model, .. }) => {
            format!("Ollama · {model} · {}", tr("On this computer"))
        }
        Some(ProviderSettings::OpenAi { model }) => format!(
            "OpenAI · {model} · {}",
            tr("Messages and attachments are sent to OpenAI")
        ),
        None => tr("Messages and attachments are sent to OpenAI"),
    };
    let provider_label = gtk::Label::new(Some(&provider));
    provider_label.add_css_class("dim-label");
    provider_label.set_wrap(true);
    provider_label.set_xalign(0.0);
    body.append(&provider_label);
    let attachment_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    body.append(&attachment_box);
    populate_attachments(&attachment_box, &state);
    let entry = gtk::TextView::new();
    entry.set_wrap_mode(gtk::WrapMode::WordChar);
    entry.set_top_margin(10);
    entry.set_bottom_margin(10);
    entry.set_left_margin(10);
    entry.set_right_margin(10);
    entry.set_accepts_tab(false);
    entry
        .buffer()
        .set_text(panel.as_deref().unwrap_or(&state.request.borrow()));
    let draft = state.request.clone();
    entry.buffer().connect_changed(move |buffer| {
        *draft.borrow_mut() = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
    });
    entry.set_tooltip_text(Some(&tr(
        "Ask a question or describe a task. Enter sends; Shift+Enter adds a line.",
    )));
    if state.chat_task_reply.get() {
        let label = gtk::Label::new(Some(&tr("Reply")));
        label.set_xalign(0.0);
        body.append(&label);
    }
    let input_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(76)
        .max_content_height(160)
        .propagate_natural_height(true)
        .child(&entry)
        .build();
    input_scroll.add_css_class("card");
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&input_scroll));
    let placeholder = gtk::Label::new(Some(&tr("Ask a question or describe a task…")));
    placeholder.set_wrap(true);
    placeholder.set_xalign(0.0);
    placeholder.set_valign(gtk::Align::Start);
    placeholder.set_margin_top(10);
    placeholder.set_margin_start(10);
    placeholder.set_margin_end(10);
    placeholder.add_css_class("dim-label");
    placeholder.set_can_target(false);
    placeholder.set_visible(entry.buffer().char_count() == 0);
    overlay.add_overlay(&placeholder);
    entry.buffer().connect_changed(move |buffer| {
        placeholder.set_visible(buffer.char_count() == 0);
    });
    body.append(&overlay);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_selectable(true);
    status.set_xalign(0.0);
    body.append(&status);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let attach = gtk::Button::builder()
        .icon_name("mail-attachment-symbolic")
        .tooltip_text(tr("Attach files or images"))
        .build();
    controls.append(&attach);
    let recovery = gtk::Button::with_label(&tr("System status and recovery"));
    controls.append(&recovery);
    let w = window.clone();
    let s = state.clone();
    recovery.connect_clicked(move |_| run_system_request(&w, s.clone(), IpcRequest::Inspect));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    controls.append(&spacer);
    let stop = gtk::Button::with_label(&tr("Stop"));
    stop.set_visible(false);
    controls.append(&stop);
    let send = gtk::Button::with_label(&tr("Send"));
    send.add_css_class("suggested-action");
    controls.append(&send);
    body.append(&controls);
    let w = window.clone();
    let s = state.clone();
    let ab = attachment_box.clone();
    let st = status.clone();
    let send_for_attach = send.downgrade();
    attach.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title(tr("Attach files or images"))
            .build();
        let s = s.clone();
        let ab = ab.clone();
        let st = st.clone();
        let Some(send) = send_for_attach.upgrade() else {
            return;
        };
        dialog.open(Some(&w), None::<&gtk::gio::Cancellable>, move |result| {
            if !send.is_sensitive() {
                return;
            }
            if let Ok(file) = result
                && let Some(path) = file.path()
            {
                send.set_sensitive(false);
                st.set_text(&tr("Loading attachment…"));
                let task = s.tasks.borrow_mut().start();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(Attachment::read(&path));
                });
                glib::timeout_add_local(Duration::from_millis(60), move || {
                    if task.view.is_cancelled() {
                        return glib::ControlFlow::Break;
                    }
                    match rx.try_recv() {
                        Ok(result) => {
                            task.view.cancel();
                            send.set_sensitive(true);
                            st.set_text("");
                            match result {
                                Ok(file) => {
                                    if s.chat_attachments.borrow().len() >= 4
                                        || s.chat_attachments
                                            .borrow()
                                            .iter()
                                            .map(Attachment::bytes)
                                            .sum::<usize>()
                                            + file.bytes()
                                            > 16 * 1024 * 1024
                                    {
                                        st.set_text(&tr(
                                            "Attach at most four files, up to 16 MB total.",
                                        ));
                                    } else {
                                        s.chat_attachments.borrow_mut().push(file);
                                        populate_attachments(&ab, &s);
                                    }
                                }
                                Err(e) => st.set_text(&e.to_string()),
                            }
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => {
                            task.view.cancel();
                            send.set_sensitive(true);
                            st.set_text(&tr("Could not read attachment."));
                            glib::ControlFlow::Break
                        }
                    }
                });
            }
        });
    });
    let running: Rc<RefCell<Option<tasks::Task>>> = Default::default();
    let run = running.clone();
    stop.connect_clicked(move |button| {
        if let Some(task) = run.borrow().as_ref() {
            task.work.cancel();
            button.set_sensitive(false);
        }
    });
    let w = window.clone();
    let s = state.clone();
    let text = entry.clone();
    let stop_copy = stop.clone();
    let status_copy = status.clone();
    let messages_copy = messages.clone();
    let attach_copy = attach.clone();
    let mode_copy = modes.clone();
    let rec_copy = recovery.clone();
    let new_copy = new_chat.clone();
    let history_copy = history.clone();
    let settings_copy = settings.clone();
    let attachments_copy = attachment_box.clone();
    send.connect_clicked(move |button| {
        if !button.is_sensitive() {
            return;
        }
        let buffer = text.buffer();
        let prompt = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .trim()
            .to_string();
        if prompt.is_empty() {
            return;
        }
        let Some(client) = s.client.borrow().clone() else {
            status_copy.set_text(&tr("AI provider is not configured. Open Peasy settings."));
            return;
        };
        let attachments = s.chat_attachments.borrow().clone();
        let request = Request {
            conversation: s.conversation.borrow().clone(),
            prompt: prompt.clone(),
            attachments: attachments.clone(),
            mode: s.chat_mode.get(),
        };
        let stored_prompt = peasy_client::chat::redacted_message(&prompt);
        *s.chat_pending.borrow_mut() = Some((stored_prompt.clone(), attachments));
        *s.request.borrow_mut() = prompt.clone();
        message(&messages_copy, &tr("You"), &stored_prompt, false);

        attachments_copy.set_sensitive(false);
        button.set_sensitive(false);
        text.set_sensitive(false);
        attach_copy.set_sensitive(false);
        mode_copy.set_sensitive(false);
        rec_copy.set_sensitive(false);
        new_copy.set_sensitive(false);
        history_copy.set_sensitive(false);
        settings_copy.set_sensitive(false);
        stop_copy.set_visible(true);
        stop_copy.set_sensitive(true);
        status_copy.set_text(&tr("Thinking…"));
        let task = s.tasks.borrow_mut().start();
        *running.borrow_mut() = Some(task.clone());
        let direct_task = request.mode == Mode::Task;
        let clarification = s.chat_task_reply.replace(false) && request.mode != Mode::Ask;
        let (tx, rx) = mpsc::channel();
        let worker = task.clone();
        std::thread::spawn(move || {
            let result = worker.work.scope(|| -> Result<Outcome> {
                let reply = if clarification {
                    Reply::Task(prompt)
                } else {
                    client.converse(request)?
                };
                match reply {
                    Reply::Task(request) => {
                        if !clarification && !direct_task {
                            client.clear_task_clarification();
                        }
                        let updates = tx.clone();
                        Ok(Outcome::Task(Box::new(client.resolve_with_progress(
                            &request,
                            move |stage| {
                                let _ = updates.send(Event::Stage(stage));
                            },
                        )?)))
                    }
                    Reply::Applications(apps) if apps.len() == 1 => {
                        client.open_application(&apps[0].desktop_id)?;
                        Ok(Outcome::Chat(Reply::Answer(Answer {
                            message: tr_args("Opening {name}…", &[("name", &apps[0].name)]),
                            ..Default::default()
                        })))
                    }
                    other => Ok(Outcome::Chat(other)),
                }
            });
            let _ = tx.send(Event::Finished(result.map_err(|e| format!("{e:#}"))));
        });
        let w = w.clone();
        let s = s.clone();
        let status = status_copy.clone();
        let send = button.clone();
        let text = text.clone();
        let stop = stop_copy.clone();
        let attach = attach_copy.clone();
        let modes = mode_copy.clone();
        let recovery = rec_copy.clone();
        let new_chat = new_copy.clone();
        let history = history_copy.clone();
        let settings = settings_copy.clone();
        let attachment_box = attachments_copy.clone();
        let messages = messages_copy.clone();
        let pending_card = messages.last_child();
        glib::timeout_add_local(Duration::from_millis(60), move || {
            let result = rx.try_recv();
            if task.view.is_cancelled() {
                return match result {
                    Ok(Event::Finished(Ok(Outcome::Task(res)))) => {
                        if let Resolution::Proposal(p) = *res {
                            discard_token(&s, p.id);
                        }
                        glib::ControlFlow::Break
                    }
                    Ok(Event::Finished(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                        glib::ControlFlow::Break
                    }
                    _ => glib::ControlFlow::Continue,
                };
            }
            match result {
                Ok(Event::Stage(stage)) => {
                    status.set_text(&tr(stage.message()));
                    glib::ControlFlow::Continue
                }
                Ok(Event::Finished(result)) => {
                    task.view.cancel();
                    attachment_box.set_sensitive(true);
                    stop.set_visible(false);
                    send.set_sensitive(true);
                    text.set_sensitive(true);
                    attach.set_sensitive(true);
                    modes.set_sensitive(true);
                    recovery.set_sensitive(true);
                    new_chat.set_sensitive(true);
                    history.set_sensitive(true);
                    settings.set_sensitive(true);
                    match result {
                        Ok(Outcome::Task(res)) => {
                            s.chat_attachments.borrow_mut().clear();
                            show_resolution(&w, s.clone(), *res);
                        }
                        Ok(Outcome::Chat(Reply::Answer(answer))) => {
                            if let Some((prompt, attachments)) = s.chat_pending.borrow_mut().take()
                            {
                                s.conversation.borrow_mut().push(Turn {
                                    user: prompt,
                                    attachments,
                                    answer,
                                });
                            }
                            chat_history::autosave(&s);
                            s.chat_attachments.borrow_mut().clear();
                            s.request.borrow_mut().clear();
                            show(&w, s.clone());
                        }
                        Ok(Outcome::Chat(Reply::Applications(apps))) => {
                            show_applications(&w, s.clone(), apps)
                        }
                        Ok(Outcome::Chat(Reply::Task(_))) => unreachable!(),
                        Err(error) => {
                            if let Some(card) = &pending_card {
                                messages.remove(card);
                            }
                            s.chat_pending.borrow_mut().take();
                            if clarification {
                                s.chat_task_reply.set(true);
                            }
                            status.set_text(&if task.work.is_cancelled() {
                                tr("Cancelled.")
                            } else {
                                tr(peasy_client::connectivity::offline_message(&error)
                                    .unwrap_or(&error))
                            });
                        }
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    task.view.cancel();
                    s.chat_pending.borrow_mut().take();
                    attachment_box.set_sensitive(true);
                    stop.set_visible(false);
                    send.set_sensitive(true);
                    text.set_sensitive(true);
                    attach.set_sensitive(true);
                    modes.set_sensitive(true);
                    recovery.set_sensitive(true);
                    new_chat.set_sensitive(true);
                    history.set_sensitive(true);
                    settings.set_sensitive(true);
                    status.set_text(&tr("The request stopped unexpectedly. Please try again."));
                    glib::ControlFlow::Break
                }
            }
        });
    });
    let composing = Rc::new(Cell::new(false));
    let preedit = composing.clone();
    entry.connect_preedit_changed(move |_, text| preedit.set(!text.is_empty()));
    let keys = gtk::EventControllerKey::new();
    keys.set_name(Some("peasy-composer"));
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let send_key = send.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !composing.get()
            && key == gtk::gdk::Key::Return
            && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
        {
            if let Some(send) = send_key.upgrade() {
                send.emit_clicked();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(keys);
    window.set_resizable(true);
    show_content(window, &root, MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT);
    entry.grab_focus();
    if panel.is_some() {
        send.emit_clicked();
    }
}
fn populate_attachments(container: &gtk::Box, state: &AppState) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for (index, file) in state.chat_attachments.borrow().iter().enumerate() {
        let button = gtk::Button::with_label(&format!("📎 {}  ×", file.name));
        button.set_halign(gtk::Align::Start);
        let state = state.clone();
        let container_copy = container.downgrade();
        button.connect_clicked(move |_| {
            if index < state.chat_attachments.borrow().len() {
                state.chat_attachments.borrow_mut().remove(index);
            }
            if let Some(container) = container_copy.upgrade() {
                populate_attachments(&container, &state);
            }
        });
        container.append(&button);
    }
}
fn show_applications(
    window: &adw::ApplicationWindow,
    state: AppState,
    apps: Vec<peasy_core::resource_native::applications::Application>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(tr("Which application?"))
        .body(tr(
            "More than one installed application matches. Choose the one to open.",
        ))
        .build();
    dialog.add_response("cancel", &tr("Cancel"));
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("cancel"));
    for (i, app) in apps.iter().enumerate() {
        dialog.add_response(&i.to_string(), &app.name);
    }
    let window_copy = window.clone();
    dialog.connect_response(None, move |_, response| {
        let Some(app) = response.parse::<usize>().ok().and_then(|i| apps.get(i)) else {
            complete_task(&window_copy, state.clone(), "Cancelled.", false);
            return;
        };
        let Some(client) = state.client.borrow().clone() else {
            return;
        };
        let app = app.clone();
        let (tx, rx) = mpsc::channel();
        let task = state.tasks.borrow_mut().start();
        let work = task.work.clone();
        if let Some(content) = window_copy.content() {
            content.set_sensitive(false);
        }
        let name = app.name.clone();
        std::thread::spawn(move || {
            let _ = tx.send(
                work.scope(|| client.open_application(&app.desktop_id))
                    .map_err(|e| e.to_string()),
            );
        });
        let state = state.clone();
        let window = window_copy.clone();
        glib::timeout_add_local(Duration::from_millis(60), move || {
            if task.view.is_cancelled() {
                return glib::ControlFlow::Break;
            }
            match rx.try_recv() {
                Ok(result) => {
                    task.view.cancel();
                    let message = match result {
                        Ok(()) => tr_args("Opening {name}…", &[("name", &name)]),
                        Err(e) => e,
                    };
                    complete_task(&window, state.clone(), &message, false);
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => {
                    task.view.cancel();
                    complete_task(
                        &window,
                        state.clone(),
                        "The request stopped unexpectedly. Please try again.",
                        false,
                    );
                    glib::ControlFlow::Break
                }
            }
        });
    });
    dialog.present(Some(window));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut result = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(value) = child {
            result.extend(descendants(&value));
            child = value.next_sibling();
        }
        result
    }
    fn button(window: &adw::ApplicationWindow, label: &str) -> gtk::Button {
        descendants(window.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| {
                b.label().as_deref() == Some(label) || b.tooltip_text().as_deref() == Some(label)
            })
            .unwrap_or_else(|| panic!("Missing button: {label}"))
    }
    fn drain() {
        let context = glib::MainContext::default();
        while context.pending() {
            context.iteration(false);
        }
    }
    #[test]
    #[ignore = "requires Xvfb; scripts/check-rust.sh supplies a display"]
    fn conversation_view_preserves_history_drafts_and_review_handoff() {
        adw::init().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let state = AppState {
            args: Args {
                socket: directory.path().join("absent.sock"),
                engine: None,
                settings: false,
            },
            keys: KeyStore::at(directory.path().join("key")),
            providers: ProviderStore::at(directory.path().join("provider")),
            client: Default::default(),
            tasks: Default::default(),
            pending: Default::default(),
            applying: Default::default(),
            closing_apply: Default::default(),
            request: Default::default(),
            reviewed_change: Default::default(),
            conversation: Default::default(),
            history: chat_history::Worker::new(peasy_client::chat::history::HistoryStore::at(
                directory.path().join("history"),
            )),
            history_id: Default::default(),
            history_notice: Default::default(),
            history_notice_widget: Default::default(),
            chat_mode: Default::default(),
            chat_task_reply: Default::default(),
            chat_pending: Default::default(),
            chat_attachments: Default::default(),
            return_to_ollama_settings: false,
        };
        state.conversation.borrow_mut().push(Turn{user:"Why is my computer slow?".into(),attachments:vec![],answer:Answer{message:"## Observations\nA **short sample** is not proof.\n\n```nix\nzramSwap.enable = true;\n```\n[source](https://example.com)".into(),suggested_task:Some("enable zram".into()),..Default::default()}});
        let app = adw::Application::builder()
            .application_id("io.github.peasy.ConversationTest")
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk::gio::Cancellable>).unwrap();
        let window = adw::ApplicationWindow::builder().application(&app).build();
        show(&window, state.clone());
        window.present();
        drain();
        if let Ok(path) = std::env::var("PEASY_TEST_SCREENSHOT") {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                drain();
                std::thread::sleep(Duration::from_millis(10));
            }
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            let node = snapshot.to_node().expect("rendered conversation");
            let texture = window.renderer().unwrap().render_texture(&node, None);
            texture.save_to_png(path).unwrap();
        }
        assert_eq!(state.conversation.borrow().turns.len(), 1);
        assert!(button(&window, &tr("Ask")).is_visible());
        let text = descendants(window.upcast_ref())
            .into_iter()
            .find_map(|w| w.downcast::<gtk::TextView>().ok())
            .unwrap();
        text.buffer().set_text("draft follow-up");
        assert_eq!(&*state.request.borrow(), "draft follow-up");
        let controllers = text.observe_controllers();
        let keys = (0..controllers.n_items())
            .filter_map(|i| {
                controllers
                    .item(i)?
                    .downcast::<gtk::EventControllerKey>()
                    .ok()
            })
            .find(|k| k.name().as_deref() == Some("peasy-composer"))
            .unwrap();
        text.emit_by_name::<()>("preedit-changed", &[&"中"]);
        assert!(!keys.emit_by_name::<bool>(
            "key-pressed",
            &[
                &gtk::gdk::Key::Return,
                &0u32,
                &gtk::gdk::ModifierType::empty()
            ]
        ));
        text.emit_by_name::<()>("preedit-changed", &[&""]);
        assert!(!keys.emit_by_name::<bool>(
            "key-pressed",
            &[
                &gtk::gdk::Key::Return,
                &0u32,
                &gtk::gdk::ModifierType::SHIFT_MASK
            ]
        ));
        assert!(keys.emit_by_name::<bool>(
            "key-pressed",
            &[
                &gtk::gdk::Key::Return,
                &0u32,
                &gtk::gdk::ModifierType::empty()
            ]
        ));

        button(&window, &tr("Review suggested task")).emit_clicked();
        drain();
        assert_eq!(state.chat_mode.get(), Mode::Task);
        assert_eq!(&*state.request.borrow(), "enable zram");
        assert!(state.pending.borrow().is_none());
        *state.chat_pending.borrow_mut() = Some(("enable zram".into(), vec![]));
        complete_task(&window, state.clone(), "Which setting?", true);
        drain();
        assert!(state.chat_task_reply.get());
        assert_eq!(state.conversation.borrow().turns.len(), 2);
        *state.chat_pending.borrow_mut() = Some(("remove an application".into(), vec![]));
        cancel_review(&window, state.clone());
        drain();
        assert!(state.chat_pending.borrow().is_none());
        assert_eq!(
            state
                .conversation
                .borrow()
                .turns
                .last()
                .unwrap()
                .answer
                .message,
            tr("Cancelled.")
        );
        button(&window, &tr("New conversation")).emit_clicked();
        for _ in 0..200 {
            drain();
            if state.conversation.borrow().turns.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(state.conversation.borrow().turns.is_empty());
        assert!(!state.chat_task_reply.get());
        assert!(state.request.borrow().is_empty());

        // History reopens only the selected transcript, without resuming any
        // old task or replaying attached files. Repeated navigation must drop
        // old composers, not retain one GTK tree per historical chat.
        for _ in 0..3 {
            let old_composer = descendants(window.upcast_ref())
                .into_iter()
                .find_map(|w| w.downcast::<gtk::TextView>().ok())
                .unwrap()
                .downgrade();
            button(&window, &tr("Chat history")).emit_clicked();
            for _ in 0..500 {
                drain();
                if descendants(window.upcast_ref())
                    .iter()
                    .any(|w| w.tooltip_text().as_deref() == Some("Why is my computer slow?"))
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if let Ok(path) = std::env::var("PEASY_TEST_SCREENSHOT") {
                for _ in 0..20 {
                    drain();
                    std::thread::sleep(Duration::from_millis(10));
                }
                let paintable = gtk::WidgetPaintable::new(Some(&window));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
                let node = snapshot.to_node().unwrap();
                window
                    .renderer()
                    .unwrap()
                    .render_texture(&node, None)
                    .save_to_png(format!("{path}.history.png"))
                    .unwrap();
            }
            assert!(
                old_composer.upgrade().is_none(),
                "old chat composer leaked across navigation"
            );
            button(&window, "Why is my computer slow?").emit_clicked();
            for _ in 0..500 {
                drain();
                if descendants(window.upcast_ref())
                    .iter()
                    .any(|w| w.is::<gtk::TextView>())
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(state.conversation.borrow().turns.len(), 3);
            assert!(state.chat_pending.borrow().is_none());
            assert!(!state.chat_task_reply.get());
            assert_eq!(state.chat_mode.get(), Mode::Auto);
            assert!(state.history_id.borrow().is_some());
        }

        button(&window, &tr("Chat history")).emit_clicked();
        for _ in 0..500 {
            drain();
            if descendants(window.upcast_ref())
                .iter()
                .any(|w| w.tooltip_text().as_deref() == Some("Why is my computer slow?"))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        button(&window, &tr("Delete all chats")).emit_clicked();
        drain();
        let dialog = window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        dialog.emit_by_name::<()>("response", &[&"delete"]);
        dialog.close();
        for _ in 0..500 {
            drain();
            if descendants(window.upcast_ref())
                .iter()
                .filter_map(|w| w.clone().downcast::<gtk::Label>().ok())
                .any(|l| l.text() == tr("No saved chats yet."))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(state.conversation.borrow().turns.is_empty());
        assert!(state.history_id.borrow().is_none());
        button(&window, &tr("Back to chat")).emit_clicked();
        drain();

        // Settings retain the chat window size and expose both provider forms
        // and maintenance actions without a scrolling page or clipped footer.
        state
            .providers
            .save(&ProviderSettings::OpenAi {
                model: "gpt-5-mini".into(),
            })
            .unwrap();
        show_provider_settings(&window, state.clone());
        state.tasks.borrow_mut().close();
        let widgets = descendants(window.upcast_ref());
        let pages = widgets
            .iter()
            .find_map(|w| {
                w.clone()
                    .downcast::<gtk::Stack>()
                    .ok()
                    .filter(|s| s.child_by_name("provider").is_some())
            })
            .unwrap();
        let provider = widgets
            .iter()
            .find_map(|w| w.clone().downcast::<gtk::DropDown>().ok())
            .unwrap();
        let model = widgets
            .iter()
            .find_map(|w| {
                w.clone()
                    .downcast::<gtk::Entry>()
                    .ok()
                    .filter(|e| e.text() == "gpt-5-mini")
            })
            .unwrap();
        model.set_text("draft-model");
        for (page, selected, suffix) in [
            ("provider", OPENAI_PROVIDER_INDEX, "openai"),
            ("provider", OLLAMA_PROVIDER_INDEX, "ollama"),
            ("system", OLLAMA_PROVIDER_INDEX, "system"),
            ("provider", OPENAI_PROVIDER_INDEX, "openai-return"),
        ] {
            provider.set_selected(selected);
            pages.set_visible_child_name(page);
            for _ in 0..20 {
                drain();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(window.default_width(), MAIN_WINDOW_WIDTH, "{suffix}");
            assert_eq!(window.default_height(), MAIN_WINDOW_HEIGHT);
            assert!(
                !widgets
                    .iter()
                    .any(|w| w.is::<gtk::ScrolledWindow>() && w.is_mapped()),
                "{suffix} has a scrolling settings page"
            );
            let content = window.content().unwrap();
            assert!(
                content
                    .measure(gtk::Orientation::Vertical, MAIN_WINDOW_WIDTH)
                    .0
                    <= MAIN_WINDOW_HEIGHT,
                "{suffix} settings exceed the chat height"
            );
            let save = button(&window, &tr("Save provider"));
            assert_eq!(save.is_visible(), page == "provider");
            let bottom = if page == "provider" {
                save
            } else {
                button(&window, &tr("Check for updates"))
            };
            let bounds = bottom.compute_bounds(&window).unwrap();
            assert!(bounds.y() + bounds.height() <= MAIN_WINDOW_HEIGHT as f32);
            if let Ok(path) = std::env::var("PEASY_TEST_SCREENSHOT") {
                let paintable = gtk::WidgetPaintable::new(Some(&window));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
                let node = snapshot.to_node().unwrap();
                window
                    .renderer()
                    .unwrap()
                    .render_texture(&node, None)
                    .save_to_png(format!("{path}.{suffix}.png"))
                    .unwrap();
            }
        }
        assert_eq!(model.text(), "draft-model");
        // Bootstrap return routing stays in Ollama settings, even with a saved
        // OpenAI provider and without a loaded AI client.
        let mut bootstrap = state.clone();
        bootstrap.return_to_ollama_settings = true;
        cancel_review(&window, bootstrap.clone());
        state.tasks.borrow_mut().close();
        drain();
        let selected = descendants(window.upcast_ref())
            .into_iter()
            .find_map(|w| w.downcast::<gtk::DropDown>().ok())
            .unwrap();
        assert_eq!(selected.selected(), OLLAMA_PROVIDER_INDEX);
        show_message(&window, bootstrap, "Ollama started.");
        button(&window, &tr("Done")).emit_clicked();
        state.tasks.borrow_mut().close();
        drain();
        let selected = descendants(window.upcast_ref())
            .into_iter()
            .find_map(|w| w.downcast::<gtk::DropDown>().ok())
            .unwrap();
        assert_eq!(selected.selected(), OLLAMA_PROVIDER_INDEX);
        state.tasks.borrow_mut().close();
        window.close();
        drain();
    }
}
