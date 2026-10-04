//! Finding, opening and listing files and folders. Nothing here changes a file.

use super::*;
use std::path::PathBuf;


/// A folder named the way people say it: "Desktop", "my downloads", "~/Work", a full path.
pub(super) fn resolve_folder(folder: &str) -> Result<std::path::PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home folder"))?;
    let name = folder.trim().trim_start_matches("my ").trim_end_matches('/');
    let path = match name.to_lowercase().as_str() {
        "" | "desktop" => home.join("Desktop"),
        "documents" => home.join("Documents"),
        "downloads" => home.join("Downloads"),
        "home" | "~" => home,
        "applications" => "/Applications".into(),
        _ => match name.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None if name.starts_with('/') => name.into(),
            // A folder name on its own: look for it in the usual places.
            None => [home.join("Desktop"), home.join("Documents"), home.clone()]
                .iter()
                .map(|dir| dir.join(name))
                .find(|p| p.is_dir())
                .ok_or_else(|| anyhow!("There's no folder called {name} on the Desktop, in Documents or in the home folder"))?,
        },
    };
    if !path.is_dir() {
        bail!("{} isn't a folder", path.display());
    }
    Ok(path)
}

pub(super) fn list_folder(folder: &str) -> Result<String> {
    let path = resolve_folder(folder)?;
    let (mut folders, mut files) = (Vec::new(), Vec::new());
    for entry in std::fs::read_dir(&path)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // Apps are folders on disk but files to people.
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir()) && !name.ends_with(".app");
        if is_dir { folders.push(name) } else { files.push(name) }
    }
    folders.sort_by_key(|n| n.to_lowercase());
    files.sort_by_key(|n| n.to_lowercase());
    let list = |items: &[String]| if items.is_empty() { "none".to_string() } else { items.iter().take(60).cloned().collect::<Vec<_>>().join(", ") };
    Ok(format!(
        "{} contains {} folder{} and {} file{}.\nFolders: {}\nFiles: {}",
        path.display(),
        folders.len(),
        if folders.len() == 1 { "" } else { "s" },
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        list(&folders),
        list(&files)
    ))
}

/// Paths under the home folder whose name contains `name`, best guesses first.
fn search(name: &str) -> Result<Vec<String>> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home folder"))?;
    let found = run_cmd(Command::new("mdfind").arg("-onlyin").arg(&home).args(["-name", name]), Duration::from_secs(10))?;
    let skip = ["/Library/", "/node_modules/", "/.", "/target/", "/dist/", "/build/"];
    let want = name.to_lowercase();
    let mut paths: Vec<String> = found.lines().filter(|p| !skip.iter().any(|s| p.contains(s))).map(str::to_string).collect();
    // An exact name first, then shortest paths: usually the file itself rather than something
    // deep inside a project.
    let file_name = |p: &str| p.rsplit('/').next().unwrap_or_default().to_lowercase();
    paths.sort_by_key(|p| {
        let f = file_name(p);
        let stem = f.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(f.clone());
        (if f == want || stem == want { 0 } else { 1 }, p.len())
    });
    Ok(paths)
}

pub(super) fn find_files(name: &str) -> Result<String> {
    if name.is_empty() {
        bail!("No file name");
    }
    let paths = search(name)?;
    if paths.is_empty() {
        return Ok(format!("No files named like “{name}”"));
    }
    Ok(format!("Files named like “{name}”:\n{}", paths.iter().take(8).cloned().collect::<Vec<_>>().join("\n")))
}

/// A file or folder from a full path, "~/…", or just its name (looked for on the Desktop, in
/// Downloads and Documents, then everywhere in the home folder).
pub(super) fn path_of(said: &str) -> Result<PathBuf> {
    let said = said.trim().trim_matches(['"', '“', '”']);
    if said.is_empty() {
        bail!("Which file?");
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home folder"))?;
    let direct = match said.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(said),
    };
    if direct.is_absolute() && direct.exists() {
        return Ok(direct);
    }
    if direct.is_absolute() {
        bail!("{} doesn't exist", direct.display());
    }
    if let Ok(folder) = resolve_folder(said) {
        return Ok(folder);
    }
    let want = said.to_lowercase();
    for dir in ["Desktop", "Downloads", "Documents"] {
        if let Ok(entries) = std::fs::read_dir(home.join(dir)) {
            for e in entries.flatten() {
                let n = e.file_name().to_string_lossy().to_lowercase();
                if n == want || n.rsplit_once('.').is_some_and(|(stem, _)| stem == want) {
                    return Ok(e.path());
                }
            }
        }
    }
    let found = search(said)?;
    match found.first() {
        Some(p) => Ok(PathBuf::from(p)),
        None => bail!("There's no file or folder called “{said}”"),
    }
}

pub(super) fn open_file(path: &str) -> Result<String> {
    let path = path_of(path)?;
    run_cmd(Command::new("open").arg(&path), Duration::from_secs(10))?;
    Ok(format!("Opened {}", path.display()))
}

/// Shows the file selected in a Finder window.
pub(super) fn reveal(path: &str) -> Result<String> {
    let path = path_of(path)?;
    run_cmd(Command::new("open").arg("-R").arg(&path), Duration::from_secs(10))?;
    Ok(format!("Showing {} in Finder", path.display()))
}

/// The newest files: in a folder ("the latest download"), or anywhere in the home folder.
pub(super) fn recent(folder: &str) -> Result<String> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let within = if folder.trim().is_empty() { None } else { Some(resolve_folder(folder)?) };
    match &within {
        Some(dir) => {
            for e in std::fs::read_dir(dir)?.flatten() {
                if e.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                if let Ok(m) = e.metadata() {
                    files.push((m.created().or_else(|_| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()));
                }
            }
        }
        None => {
            let home = dirs::home_dir().ok_or_else(|| anyhow!("no home folder"))?;
            let found = run_cmd(
                Command::new("mdfind").arg("-onlyin").arg(&home).arg("kMDItemLastUsedDate >= $time.today(-7) && kMDItemContentTypeTree != public.folder"),
                Duration::from_secs(10),
            )?;
            let skip = ["/Library/", "/node_modules/", "/.", "/target/", "/dist/"];
            for p in found.lines().filter(|p| !skip.iter().any(|s| p.contains(s))) {
                if let Ok(m) = std::fs::metadata(p) {
                    files.push((m.accessed().or_else(|_| m.modified()).unwrap_or(std::time::UNIX_EPOCH), PathBuf::from(p)));
                }
            }
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    if files.is_empty() {
        return Ok("No recent files".into());
    }
    let now = std::time::SystemTime::now();
    let ago = |t: std::time::SystemTime| {
        let s = now.duration_since(t).unwrap_or_default().as_secs();
        match s {
            s if s < 3600 => format!("{} min ago", s / 60),
            s if s < 86400 => format!("{} h ago", s / 3600),
            s => format!("{} days ago", s / 86400),
        }
    };
    let list: Vec<String> = files.iter().take(10).map(|(t, p)| format!("{} ({})", p.display(), ago(*t))).collect();
    Ok(format!("Newest first:\n{}", list.join("\n")))
}
