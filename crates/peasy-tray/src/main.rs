use anyhow::Result;
use clap::Parser;
use ksni::blocking::TrayMethods;
use std::path::PathBuf;
use std::process::Command;

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
            data.extend_from_slice(&[(coverage * 255.0).round() as u8, 0xbf, 0xff, 0xd4]);
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
    if let Some(program) = args.gnome_extensions {
        // A fixed, declarative session integration step. User/model text is never involved.
        let _ = Command::new(program)
            .args(["enable", APPINDICATOR_UUID])
            .status();
    }
    let _handle = PeasyTray { ui: args.ui }
        .assume_sni_available(true)
        .spawn()?;
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_circle_has_argb_pixels_and_transparent_corners() {
        let icon = mint_circle(16);
        assert_eq!(icon.width, 16);
        assert_eq!(icon.height, 16);
        assert_eq!(icon.data.len(), 16 * 16 * 4);
        assert_eq!(&icon.data[..4], &[0, 0xbf, 0xff, 0xd4]);

        let centre = (8 * 16 + 8) * 4;
        assert_eq!(&icon.data[centre..centre + 4], &[255, 0xbf, 0xff, 0xd4]);
    }
}
