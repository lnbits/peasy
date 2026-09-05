use anyhow::{Context, Result};
use clap::Parser;
use peasy_client::{
    Choice, DEFAULT_OLLAMA_URL, DEFAULT_OPENAI_MODEL, KeyStore, LocalAction, LocalProposal,
    PeasyClient, ProviderSettings, ProviderStore, Resolution, ResolveStage, list_ollama_models,
    load_model_provider, sync_live_theme_from_file,
};
use peasy_core::{DiffKind, DiffLine, Proposal};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, BufRead, IsTerminal, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct TerminalAgent(Child);

impl TerminalAgent {
    fn start() -> Result<Option<Self>> {
        if !io::stdin().is_terminal() {
            return Ok(None);
        }
        let executable = std::env::var_os("PEASY_PKTTYAGENT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/current-system/sw/bin/pkttyagent"));
        let (mut ready, notify) = UnixStream::pair()?;
        ready.set_read_timeout(Some(Duration::from_secs(10)))?;
        let child = Command::new(executable)
            .args([
                "--process",
                &std::process::id().to_string(),
                "--fallback",
                "--notify-fd",
                "0",
            ])
            .stdin(Stdio::from(OwnedFd::from(notify)))
            .spawn()
            .context("starting terminal authorization agent")?;
        let mut agent = Self(child);
        // The notification descriptor closes after registration with Polkit.
        let mut byte = [0];
        let _ = std::io::Read::read(&mut ready, &mut byte)
            .context("waiting for terminal authorization agent")?;
        if agent.0.try_wait()?.is_some() {
            anyhow::bail!("terminal authorization agent could not start");
        }
        Ok(Some(agent))
    }
}

impl Drop for TerminalAgent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Parser)]
#[command(name = "peasy", about = "Tell your computer what you want.")]
struct Args {
    #[arg(value_name = "REQUEST", trailing_var_arg = true)]
    request: Vec<String>,
    #[arg(long, default_value = "/run/peasy/peasy.sock", hide = true)]
    socket: PathBuf,
    #[arg(long, hide = true)]
    engine: Option<PathBuf>,
    #[arg(long)]
    setup_key: bool,
    #[arg(long)]
    setup_provider: bool,
    #[arg(long, hide = true)]
    panel_worker: bool,
    #[arg(long, hide = true)]
    sync_theme: bool,
    #[arg(long, default_value = "/etc/peasy/theme.json", hide = true)]
    theme_state: PathBuf,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum PanelCommand {
    Request { text: String },
    Select { index: usize },
    Apply { password: Option<String> },
    Cancel,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.sync_theme {
        let gsettings = std::env::var_os("PEASY_GSETTINGS")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/current-system/sw/bin/gsettings"));
        if let Err(error) = sync_live_theme_from_file(&args.theme_state, &gsettings) {
            eprintln!("Peasy could not synchronize this desktop's appearance: {error:#}");
            return Err(error);
        }
        return Ok(());
    }
    let keys = KeyStore::discover()?;
    let providers = ProviderStore::discover()?;
    if args.panel_worker {
        let result = (|| -> Result<()> {
            let provider = load_model_provider(&providers, &keys)?
                .context("AI provider is not configured. Open Peasy settings")?;
            let engine = args
                .engine
                .or_else(|| std::env::var_os("PEASY_ENGINE").map(PathBuf::from))
                .unwrap_or_else(|| {
                    PathBuf::from("/run/current-system/sw/lib/peasy/peasy-engine.wasm")
                });
            let client = PeasyClient::with_provider(args.socket, &engine, provider)?;
            panel_worker(&client)
        })();
        if let Err(error) = result {
            let message = error
                .to_string()
                .chars()
                .filter(|character| !character.is_control())
                .take(1000)
                .collect::<String>();
            send_panel_event(&json!({ "event": "error", "message": message }))?;
        }
        return Ok(());
    }
    if args.setup_key {
        println!("Welcome to Peasy\n\nTell your computer what you want.\n");
        let key = rpassword::prompt_password("OpenAI API key: ")?;
        keys.save(&key)?;
        let model = match providers.load()? {
            Some(ProviderSettings::OpenAi { model }) => model,
            _ => DEFAULT_OPENAI_MODEL.into(),
        };
        providers.save(&ProviderSettings::OpenAi { model })?;
        println!("API key saved privately.");
        if args.request.is_empty() {
            return Ok(());
        }
    }
    if args.setup_provider || load_model_provider(&providers, &keys)?.is_none() {
        configure_provider(&keys, &providers)?;
        if args.setup_provider && args.request.is_empty() {
            return Ok(());
        }
    }
    let provider = load_model_provider(&providers, &keys)?
        .context("AI provider is not configured. Run peasy --setup-provider")?;
    let engine = args
        .engine
        .or_else(|| std::env::var_os("PEASY_ENGINE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/run/current-system/sw/lib/peasy/peasy-engine.wasm"));
    let client = PeasyClient::with_provider(args.socket, &engine, provider)?;
    if args.request.is_empty() {
        interactive(&client)
    } else {
        handle(&client, &args.request.join(" "))
    }
}

fn configure_provider(keys: &KeyStore, providers: &ProviderStore) -> Result<()> {
    println!("Peasy AI provider\n\n1. OpenAI\n2. Ollama (local)\n");
    print!("Choose [1/2]: ");
    io::stdout().flush()?;
    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;
    match choice.trim() {
        "1" | "openai" | "OpenAI" => {
            let existing = keys.load()?.is_some();
            let key = rpassword::prompt_password(if existing {
                "New OpenAI API key (leave empty to keep current): "
            } else {
                "OpenAI API key: "
            })?;
            if !key.trim().is_empty() {
                keys.save(&key)?;
            }
            keys.load()?.context("an OpenAI API key is required")?;
            print!("Model [{DEFAULT_OPENAI_MODEL}]: ");
            io::stdout().flush()?;
            let mut model = String::new();
            io::stdin().read_line(&mut model)?;
            let model = if model.trim().is_empty() {
                DEFAULT_OPENAI_MODEL.into()
            } else {
                model.trim().to_owned()
            };
            providers.save(&ProviderSettings::OpenAi { model })?;
            println!("OpenAI selected. The key is stored privately.");
        }
        "2" | "ollama" | "Ollama" => {
            println!("Checking {DEFAULT_OLLAMA_URL}…");
            let models = list_ollama_models(DEFAULT_OLLAMA_URL).with_context(
                || "Ollama is not ready. Enable services.ollama, start it, and pull a model",
            )?;
            if models.is_empty() {
                anyhow::bail!(
                    "Ollama has no installed models. Run `ollama pull MODEL`, then try again"
                );
            }
            for (index, model) in models.iter().enumerate() {
                println!("{}. {}", index + 1, model);
            }
            print!("Choose model [1]: ");
            io::stdout().flush()?;
            let mut selected = String::new();
            io::stdin().read_line(&mut selected)?;
            let index = if selected.trim().is_empty() {
                0
            } else {
                selected
                    .trim()
                    .parse::<usize>()
                    .context("model choice must be a number")?
                    .checked_sub(1)
                    .context("model choice starts at 1")?
            };
            let model = models
                .get(index)
                .context("model choice is out of range")?
                .clone();
            providers.save(&ProviderSettings::ollama(model.clone())?)?;
            println!("Local Ollama model {model} selected.");
        }
        _ => anyhow::bail!("choose 1 for OpenAI or 2 for Ollama"),
    }
    Ok(())
}

fn panel_worker(client: &PeasyClient) -> Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let first = read_panel_command(&mut input)?;
    let PanelCommand::Request { text } = first else {
        anyhow::bail!("the panel worker expected a request");
    };
    let resolution = client.resolve_with_progress(&text, |stage| {
        let _ = send_panel_progress(stage);
    })?;
    finish_panel(client, resolution, &mut input)
}

fn finish_panel(
    client: &PeasyClient,
    mut resolution: Resolution,
    input: &mut impl BufRead,
) -> Result<()> {
    loop {
        match resolution {
            Resolution::Choose(choice) => {
                send_panel_event(&json!({
                    "event": "choice",
                    "candidates": choice.candidates,
                }))?;
                resolution = match read_panel_command(input)? {
                    PanelCommand::Select { index } => {
                        client.select_with_progress(choice, index, |stage| {
                            let _ = send_panel_progress(stage);
                        })?
                    }
                    PanelCommand::Cancel => {
                        send_panel_event(&json!({ "event": "cancelled" }))?;
                        return Ok(());
                    }
                    _ => anyhow::bail!("the panel worker expected a package selection"),
                };
            }
            Resolution::Proposal(proposal) => {
                send_panel_event(&json!({
                    "event": "review",
                    "title": proposal.title,
                    "diff": proposal.diff,
                    "password_required": false,
                }))?;
                match read_panel_command(input)? {
                    PanelCommand::Apply { password: None } => {
                        send_panel_event(&json!({
                            "event": "progress",
                            "message": "Testing and applying the NixOS configuration…"
                        }))?;
                        let result = client.apply(&proposal)?;
                        if !result.activated {
                            anyhow::bail!(result.message);
                        }
                        send_panel_event(&json!({
                            "event": "done",
                            "message": result.message,
                        }))?;
                        return Ok(());
                    }
                    PanelCommand::Cancel => {
                        send_panel_event(&json!({ "event": "cancelled" }))?;
                        return Ok(());
                    }
                    _ => anyhow::bail!("the panel worker expected apply or cancel"),
                }
            }
            Resolution::LocalProposal(proposal) => {
                let password_required = matches!(
                    &proposal.action,
                    LocalAction::Wifi {
                        password: None,
                        password_required: true,
                        ..
                    }
                );
                send_panel_event(&json!({
                    "event": "review",
                    "title": proposal.title,
                    "diff": proposal.diff,
                    "password_required": password_required,
                }))?;
                match read_panel_command(input)? {
                    PanelCommand::Apply { password } => {
                        if let Some(password) = &password
                            && (password.is_empty()
                                || password.len() > 256
                                || password.chars().any(char::is_control))
                        {
                            anyhow::bail!("invalid Wi-Fi password");
                        }
                        let progress = match &proposal.action {
                            LocalAction::Wifi { .. } => "Connecting to Wi-Fi…",
                            LocalAction::Bluetooth { .. } => "Connecting Bluetooth device…",
                            LocalAction::Calendar { .. } => "Opening calendar event…",
                            LocalAction::HyprlandSetting { .. } => "Changing Hyprland setting…",
                            LocalAction::HyprlandDispatch { .. } => "Controlling Hyprland…",
                        };
                        send_panel_event(&json!({ "event": "progress", "message": progress }))?;
                        let result = client.apply_local(&proposal, password.as_deref())?;
                        send_panel_event(&json!({
                            "event": "done",
                            "message": result.message,
                        }))?;
                        return Ok(());
                    }
                    PanelCommand::Cancel => {
                        send_panel_event(&json!({ "event": "cancelled" }))?;
                        return Ok(());
                    }
                    _ => anyhow::bail!("the panel worker expected continue or cancel"),
                }
            }
            Resolution::Explain(message) => {
                send_panel_event(&json!({ "event": "done", "message": message }))?;
                return Ok(());
            }
            Resolution::Cancel => {
                send_panel_event(&json!({ "event": "cancelled" }))?;
                return Ok(());
            }
        }
    }
}

fn read_panel_command(input: &mut impl BufRead) -> Result<PanelCommand> {
    let mut line = String::new();
    if std::io::Read::take(input, 8193).read_line(&mut line)? == 0 || line.len() > 8192 {
        anyhow::bail!("invalid panel command");
    }
    serde_json::from_str(&line).context("invalid panel command")
}

fn send_panel_progress(stage: ResolveStage) -> Result<()> {
    send_panel_event(&json!({ "event": "progress", "message": stage.message() }))
}

fn send_panel_event(event: &Value) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, event)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

fn interactive(client: &PeasyClient) -> Result<()> {
    println!("Peasy\nTell your computer what you want.\n");
    loop {
        print!("Peasy › ");
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if matches!(line, "quit" | "exit") {
            break;
        }
        if !line.is_empty()
            && let Err(error) = handle(client, line)
        {
            eprintln!("{error:#}");
        }
    }
    Ok(())
}

fn handle(client: &PeasyClient, request: &str) -> Result<()> {
    let resolution = client.resolve(request)?;
    finish(client, resolution)
}

fn finish(client: &PeasyClient, resolution: Resolution) -> Result<()> {
    match resolution {
        Resolution::Proposal(proposal) => confirm_and_apply(client, proposal),
        Resolution::LocalProposal(proposal) => confirm_and_apply_local(client, proposal),
        Resolution::Choose(choice) => {
            let index = choose(&choice)?;
            finish(client, client.select(choice, index)?)
        }
        Resolution::Explain(message) => {
            println!("{message}");
            Ok(())
        }
        Resolution::Cancel => {
            println!("Cancelled.");
            Ok(())
        }
    }
}

fn choose(choice: &Choice) -> Result<usize> {
    println!("I found:\n");
    for (index, candidate) in choice.candidates.iter().enumerate() {
        println!(
            "  {}. {}{}\n     {}",
            index + 1,
            candidate.name,
            if candidate.version.is_empty() {
                String::new()
            } else {
                format!(" {}", candidate.version)
            },
            candidate.attribute
        );
    }
    print!(
        "\nWhich one would you like? [1-{}] ",
        choice.candidates.len()
    );
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let number: usize = line.trim().parse().context("invalid package choice")?;
    if number == 0 || number > choice.candidates.len() {
        anyhow::bail!("invalid package choice");
    }
    Ok(number - 1)
}

fn confirm_and_apply(client: &PeasyClient, proposal: Proposal) -> Result<()> {
    println!("Proposed change:\n\n{}\n", proposal.title);
    print_diff(&proposal.diff);
    print!("\nApply this change? [y/N] ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    if !matches!(line.trim(), "y" | "Y" | "yes" | "YES") {
        println!("Cancelled.");
        return Ok(());
    }
    println!("\nTesting configuration...");
    let _agent = TerminalAgent::start()?;
    let result = client.apply(&proposal)?;
    if result.configuration_valid {
        println!("✓ Configuration valid");
    }
    if result.build_successful {
        println!("✓ Build successful");
    }
    if result.activated {
        println!("✓ Activated\n\n{}", result.message);
    } else {
        anyhow::bail!(result.message);
    }
    Ok(())
}

fn print_diff(diff: &[DiffLine]) {
    let colour = io::stdout().is_terminal();
    for line in diff {
        let (sign, ansi) = match line.kind {
            DiffKind::Context => (' ', ""),
            DiffKind::Add => ('+', if colour { "\x1b[32m" } else { "" }),
            DiffKind::Remove => ('-', if colour { "\x1b[31m" } else { "" }),
        };
        let reset = if colour && line.kind != DiffKind::Context {
            "\x1b[0m"
        } else {
            ""
        };
        println!("{ansi}{sign} {}{reset}", line.text);
    }
}

fn confirm_and_apply_local(client: &PeasyClient, proposal: LocalProposal) -> Result<()> {
    println!("Proposed action:\n\n{}\n", proposal.title);
    print_diff(&proposal.diff);
    print!("\nContinue? [y/N] ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    if !matches!(line.trim(), "y" | "Y" | "yes" | "YES") {
        println!("Cancelled.");
        return Ok(());
    }
    let supplied_password = match &proposal.action {
        LocalAction::Wifi {
            password: None,
            password_required: true,
            ..
        } => Some(rpassword::prompt_password("Wi-Fi password (kept local): ")?),
        _ => None,
    };
    let progress = match &proposal.action {
        LocalAction::Wifi { .. } => "Connecting to Wi-Fi...",
        LocalAction::Bluetooth { .. } => "Connecting Bluetooth device...",
        LocalAction::Calendar { .. } => "Opening calendar event...",
        LocalAction::HyprlandSetting { .. } => "Changing Hyprland setting...",
        LocalAction::HyprlandDispatch { .. } => "Controlling Hyprland...",
    };
    println!("\n{progress}");
    let result = client.apply_local(&proposal, supplied_password.as_deref())?;
    if result.completed {
        println!("✓ {}", result.message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_commands_are_closed_typed_messages() {
        assert!(matches!(
            serde_json::from_str::<PanelCommand>(
                r#"{"action":"request","text":"install telegram"}"#
            )
            .unwrap(),
            PanelCommand::Request { text } if text == "install telegram"
        ));
        assert!(matches!(
            serde_json::from_str::<PanelCommand>(r#"{"action":"select","index":0}"#).unwrap(),
            PanelCommand::Select { index: 0 }
        ));
        assert!(
            serde_json::from_str::<PanelCommand>(
                r#"{"action":"apply","password":null,"command":"sh"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<PanelCommand>(r#"{"action":"shell"}"#).is_err());
    }
}
