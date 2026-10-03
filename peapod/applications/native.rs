//! Installed XDG desktop entries only. Model text never selects an executable.
use crate::cancellation::Cancellation;
use crate::resource_native::*;
use nix::libc;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, serde::Serialize)]
pub struct Application {
    pub desktop_id: String,
    pub name: String,
    #[serde(skip)]
    path: PathBuf,
    #[serde(skip)]
    contents: String,
}

fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    {
        roots.push(home.join("applications"));
    }
    let dirs = std::env::var_os("XDG_DATA_DIRS")
        .unwrap_or_else(|| "/run/current-system/sw/share:/usr/local/share:/usr/share".into());
    roots.extend(
        std::env::split_paths(&dirs)
            .filter(|p| p.is_absolute())
            .map(|p| p.join("applications")),
    );
    roots
}
fn parse(contents: &str) -> Option<String> {
    parse_for(
        contents,
        &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        crate::i18n::language(),
    )
}
fn parse_for(contents: &str, desktop: &str, language: &str) -> Option<String> {
    let mut entry = false;
    let mut values = BTreeMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            entry = line == "[Desktop Entry]";
            continue;
        }
        if entry
            && !line.starts_with('#')
            && let Some((key, value)) = line.split_once('=')
        {
            values.insert(key.trim(), value.trim());
        }
    }
    if values.get("Type") != Some(&"Application")
        || values.get("Hidden") == Some(&"true")
        || values.get("NoDisplay") == Some(&"true")
    {
        return None;
    }
    let desktops: Vec<_> = desktop.split(':').filter(|v| !v.is_empty()).collect();
    if values
        .get("OnlyShowIn")
        .is_some_and(|v| !v.split(';').any(|d| desktops.contains(&d)))
        || values
            .get("NotShowIn")
            .is_some_and(|v| v.split(';').any(|d| desktops.contains(&d)))
    {
        return None;
    }
    // GIO handles Exec field codes and D-Bus activation; we never parse Exec.
    if !values.contains_key("Exec") && values.get("DBusActivatable") != Some(&"true") {
        return None;
    }
    let key = format!("Name[{language}]");
    let name = *values.get(key.as_str()).or_else(|| values.get("Name"))?;
    if name.is_empty() || name.len() > 160 || name.chars().any(char::is_control) {
        return None;
    }
    Some(name.replace("\\s", " "))
}
fn scan(
    root: &Path,
    directory: &Path,
    depth: usize,
    visited: &mut usize,
    seen: &mut BTreeSet<String>,
    apps: &mut Vec<Application>,
) -> Result<()> {
    if depth > 3 {
        return Ok(());
    }
    let entries = match fs::read_dir(directory) {
        Ok(e) => e,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let mut entries = entries.take(2049).collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        *visited += 1;
        if *visited > 4096 {
            bail!("Too many desktop entries; narrow the installed application set");
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            scan(root, &path, depth + 1, visited, seen, apps)?;
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
            continue;
        }
        let id = path.strip_prefix(root)?.to_string_lossy().replace('/', "-");
        if crate::resources::identifier(&id, 160).is_err() || !seen.insert(id.clone()) {
            continue;
        }
        let Ok(file) = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        else {
            continue;
        };
        if !file.metadata()?.is_file() || file.metadata()?.len() > 16384 {
            continue;
        }
        let mut contents = String::new();
        if file.take(16385).read_to_string(&mut contents).is_err() || contents.len() > 16384 {
            continue;
        }
        if let Some(name) = parse(&contents) {
            apps.push(Application {
                desktop_id: id,
                name,
                path,
                contents,
            });
        }
    }
    Ok(())
}
pub fn installed() -> Result<Vec<Application>> {
    let mut apps = Vec::new();
    let mut seen = BTreeSet::new();
    let mut visited = 0;
    for root in roots() {
        scan(&root, &root, 0, &mut visited, &mut seen, &mut apps)?;
    }
    apps.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(apps)
}
pub fn inspect() -> Result<Value> {
    Ok(serde_json::to_value(installed()?)?)
}
fn find(id: &str) -> Result<Application> {
    crate::resources::identifier(id, 160)?;
    installed()?
        .into_iter()
        .find(|app| app.desktop_id == id)
        .context("Application is no longer installed; refresh and try again")
}
pub fn snapshot(id: &str) -> Result<Value> {
    let app = find(id)?;
    // Internal precondition; never sent as model context. Detect changed entries.
    Ok(json!({"desktop_id":app.desktop_id,"name":app.name,"path":app.path,"contents":app.contents}))
}
pub fn launch(id: &str, runner: &dyn ResourceRunner) -> Result<()> {
    if nix::unistd::geteuid().is_root() {
        bail!("Desktop applications must not be launched as root");
    }
    let app = find(id)?;
    runner.run(
        Tool::Gio,
        &[
            "launch",
            app.path.to_str().context("Invalid application path")?,
        ],
    )?;
    Ok(())
}
// A desktop application outlives this request. Do not use the bounded command
// helper's output pipes/process-group cleanup, which could wait for or kill it.
pub fn run_launcher(path: &Path, args: &[&str]) -> Result<String> {
    use std::process::Stdio;
    Cancellation::current().check()?;
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("The desktop could not open this application");
            }
            return Ok(String::new());
        }
        if Cancellation::current().is_cancelled() || started.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("The desktop did not confirm the application launch");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn normalise(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ac) in a.chars().enumerate() {
        let mut next = vec![i + 1];
        for (j, bc) in b.iter().enumerate() {
            next.push(
                (previous[j + 1] + 1)
                    .min(next[j] + 1)
                    .min(previous[j] + usize::from(ac != *bc)),
            );
        }
        previous = next;
    }
    previous[b.len()]
}
/// Typo tolerance is generic. Ambiguous matches stay choices, never guesses.
pub fn matching(apps: &[Application], hint: &str) -> Vec<Application> {
    let hint = normalise(hint);
    if hint.is_empty() || hint.len() > 160 {
        return vec![];
    }
    let mut scored: Vec<_> = apps
        .iter()
        .filter_map(|app| {
            let name = normalise(&app.name);
            let score = if name == hint {
                0
            } else if hint.chars().count() >= 4
                && (name.starts_with(&hint)
                    || app
                        .name
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|word| normalise(word).starts_with(&hint)))
            {
                1
            } else {
                distance(&hint, &name) + 1
            };
            (score <= 3).then_some((score, app))
        })
        .collect();
    scored.sort_by_key(|(score, _)| *score);
    let Some(best) = scored.first().map(|(score, _)| *score) else {
        return vec![];
    };
    scored
        .into_iter()
        .filter(|(score, _)| *score == best)
        .take(8)
        .map(|(_, app)| app.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app(name: &str) -> Application {
        Application {
            name: name.into(),
            desktop_id: format!("{name}.desktop"),
            path: PathBuf::new(),
            contents: String::new(),
        }
    }
    #[test]
    fn discovery_masks_hidden_overrides_and_skips_special_files() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user");
        let system = dir.path().join("system");
        fs::create_dir_all(&user).unwrap();
        fs::create_dir_all(&system).unwrap();
        let desktop = "[Desktop Entry]\nType=Application\nName=Fixture\nExec=false\n";
        fs::write(system.join("fixture.desktop"), desktop).unwrap();
        fs::write(
            user.join("fixture.desktop"),
            format!("{desktop}Hidden=true\n"),
        )
        .unwrap();
        symlink("/dev/zero", user.join("device.desktop")).unwrap();
        symlink("/missing/target", user.join("broken.desktop")).unwrap();
        fs::write(system.join("other.desktop"), desktop).unwrap();
        let (mut seen, mut count, mut apps) = (BTreeSet::new(), 0, Vec::new());
        scan(&user, &user, 0, &mut count, &mut seen, &mut apps).unwrap();
        scan(&system, &system, 0, &mut count, &mut seen, &mut apps).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].desktop_id, "other.desktop");
    }
    #[test]
    fn launcher_does_not_wait_for_or_kill_launched_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("completed");
        let started = std::time::Instant::now();
        run_launcher(
            Path::new("/bin/sh"),
            &[
                "-c",
                "(sleep 1; printf done > \"$1\") &",
                "fixture",
                marker.to_str().unwrap(),
            ],
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(900));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fs::read_to_string(marker).unwrap(), "done");
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(
            cancel
                .scope(|| run_launcher(Path::new("/missing"), &[]))
                .is_err()
        );
    }
    #[test]
    fn names_are_data_and_hidden_entries_are_not_launchable() {
        assert_eq!(
            parse("[Desktop Entry]\nType=Application\nName=Telegram\nExec=telegram %u"),
            Some("Telegram".into())
        );
        assert!(
            parse("[Desktop Entry]\nType=Application\nName=Telegram\nExec=telegram\nHidden=true")
                .is_none()
        );
        assert!(
            parse("[Desktop Entry]\nType=Link\nName=Telegram\nURL=https://example.com").is_none()
        );
        assert!(find("../../bin/sh").is_err());
    }
    #[test]
    fn desktop_visibility_and_localised_names_follow_the_session() {
        let contents = "[Desktop Entry]\nType=Application\nName=Browser\nName[es]=Navegador\nExec=browser\nOnlyShowIn=GNOME;\n";
        assert_eq!(
            parse_for(contents, "GNOME", "es").as_deref(),
            Some("Navegador")
        );
        assert!(parse_for(contents, "KDE", "es").is_none());
        assert!(parse_for(&contents.replace("OnlyShowIn", "NotShowIn"), "GNOME", "en").is_none());
    }
    #[test]
    fn typo_matching_preserves_ambiguity() {
        let apps = [app("Telegram"), app("Terminal")];
        assert_eq!(matching(&apps, "telegra")[0].name, "Telegram");
        assert!(matching(&apps, "unrelated").is_empty());
        assert_eq!(
            matching(&[app("Telegram"), app("Telegram Desktop")], "telegra").len(),
            2
        );
    }
}
