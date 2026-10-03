//! Opt-in live local-model check. It interprets requests, never applies tasks or
//! opens applications. Run against a dedicated Ollama server, not during CI.
use anyhow::{Result, bail};
use peasy_client::{
    ModelProvider, PeasyClient,
    chat::{Answer, Conversation, Mode, Reply, Request, Turn},
};
use std::path::PathBuf;
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .unwrap_or_else(|| "http://127.0.0.1:11434".into());
    let model = args.next().unwrap_or_else(|| "qwen3:0.6b".into());
    let engine = PathBuf::from(std::env::var("PEASY_TEST_ENGINE")?);
    let client = PeasyClient::with_provider(
        std::env::temp_dir().join("peasy-chat-smoke-unused.sock"),
        &engine,
        ModelProvider::Ollama {
            base_url: url,
            model,
        },
    )?;
    let mut conversation = Conversation::default();
    for (prompt, expected) in [
        ("What is NixOS? Explain in one sentence.", "answer"),
        (
            "What does declarative mean in that explanation? One sentence please.",
            "answer",
        ),
        ("install telegram", "task"),
        ("change theme colour to purple", "task"),
        ("set the speaker volume to 30%", "task"),
        ("create an image of a red bicycle", "answer"),
    ] {
        let started = std::time::Instant::now();
        let reply = client.converse(Request {
            conversation: conversation.clone(),
            prompt: prompt.into(),
            attachments: vec![],
            mode: Mode::Auto,
        })?;
        let (kind, answer) = match reply {
            Reply::Answer(answer) => ("answer", answer),
            Reply::Task(_) => ("task", Answer::default()),
            Reply::Applications(_) => ("applications", Answer::default()),
        };
        println!("{kind}: {prompt} ({:.1}s)", started.elapsed().as_secs_f64());
        if kind != expected {
            bail!("Expected {expected}, got {kind}");
        }
        if kind == "answer" && conversation.turns.len() < 2 {
            conversation.push(Turn {
                user: prompt.into(),
                attachments: vec![],
                answer,
            });
        } else {
            conversation.turns.clear();
        }
    }
    Ok(())
}
