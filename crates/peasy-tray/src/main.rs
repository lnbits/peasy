use anyhow::Result;
use clap::Parser;
use ksni::blocking::TrayMethods;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const APPINDICATOR_UUID: &str = "appindicatorsupport@rgcjonas.gmail.com";

#[derive(Debug, Parser)]
#[command(name = "peasy-tray")]
struct Args {
    #[arg(long, default_value = "peasy-ui")]
    ui: PathBuf,
    #[arg(long)]
    gnome_extensions: Option<PathBuf>,
}

#[derive(Debug)]
struct PeasyTray {
    ui: PathBuf,
}

impl PeasyTray {
    fn open(&self) {
        if let Err(error) = Command::new(&self.ui).spawn() {
            eprintln!("Could not open Peasy: {error}");
        }
    }
}

impl ksni::Tray for PeasyTray {
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        eprintln!(
            "Peasy: no StatusNotifier tray host is available yet; you can still open Peasy from the application menu."
        );
        // Close this registration before the supervisor retries. In particular,
        // a failed registration need not be followed by another owner change.
        false
    }

    fn id(&self) -> String {
        "io.github.peasy.Peasy".into()
    }

    fn title(&self) -> String {
        "Peasy".into()
    }

    fn icon_name(&self) -> String {
        "io.github.peasy.Peasy".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [16, 22, 32].into_iter().map(mint_circle).collect()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.open();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{MenuItem, StandardItem};
        vec![
            StandardItem {
                label: "Open Peasy".into(),
                icon_name: "io.github.peasy.Peasy".into(),
                activate: Box::new(|tray: &mut Self| tray.open()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn mint_circle(size: i32) -> ksni::Icon {
    let center = size as f64 / 2.0;
    let radius = size as f64 * 0.36;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 + 0.5 - center;
            let dy = y as f64 + 0.5 - center;
            let coverage = (radius + 0.75 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            data.extend_from_slice(&[(coverage * 255.0).round() as u8, 0x5c, 0xd6, 0x98]);
        }
    }
    ksni::Icon {
        width: size,
        height: size,
        data,
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let current = std::env::var("XDG_CURRENT_DESKTOP").ok();
    let session = std::env::var("XDG_SESSION_DESKTOP").ok();
    let legacy = std::env::var("DESKTOP_SESSION").ok();
    let desktop = peasy_core::DesktopEnvironment::detect(
        [current.as_deref(), session.as_deref(), legacy.as_deref()],
        false,
        true,
    );
    if desktop == peasy_core::DesktopEnvironment::Gnome
        && let Some(program) = args.gnome_extensions
    {
        // A fixed, declarative session integration step. User/model text is never involved.
        let _ = Command::new(program)
            .args(["enable", APPINDICATOR_UUID])
            .status();
    }
    run_tray(args.ui)
}

fn run_tray(ui: PathBuf) -> ! {
    let mut retry_delay = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        match (PeasyTray { ui: ui.clone() })
            .assume_sni_available(true)
            .spawn()
        {
            Ok(handle) => {
                // ksni closes the service when watcher_offline returns false.
                // Checking the local handle also catches an ended service task;
                // no polling traffic is sent to a healthy desktop's session bus.
                while !handle.is_closed() {
                    std::thread::sleep(Duration::from_secs(1));
                }
                handle.shutdown().wait();
            }
            Err(error) => eprintln!("Peasy: could not start the tray service: {error}"),
        }
        if started.elapsed() >= Duration::from_secs(10) {
            retry_delay = Duration::from_secs(1);
        }
        eprintln!("Peasy: retrying tray registration in {retry_delay:?}");
        std::thread::sleep(retry_delay);
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_circle_has_argb_pixels_and_transparent_corners() {
        for size in [16, 22, 32] {
            let icon = mint_circle(size);
            assert_eq!(icon.width, size);
            assert_eq!(icon.height, size);
            assert_eq!(icon.data.len(), (size * size * 4) as usize);
            assert_eq!(&icon.data[..4], &[0, 0x5c, 0xd6, 0x98]);

            let centre = ((size / 2 * size + size / 2) * 4) as usize;
            assert_eq!(&icon.data[centre..centre + 4], &[255, 0x5c, 0xd6, 0x98]);
        }
    }

    #[test]
    fn brand_assets_match_tray_colour() {
        for asset in [
            include_str!("../../../assets/io.github.peasy.Peasy.svg"),
            include_str!("../../../assets/peasy-wordmark.svg"),
            include_str!("../../../assets/gnome-shell-extension/stylesheet.css"),
        ] {
            assert!(asset.contains("#5cd698"));
        }
    }
}
