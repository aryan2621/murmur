//! Apps and the Mac itself.

use super::*;

pub(super) fn open_app(name: &str) -> Result<String> {
    if name.is_empty() {
        bail!("No app name");
    }
    if run_cmd(Command::new("open").args(["-a", name]), Duration::from_secs(10)).is_ok() {
        return Ok(format!("Opened {name}"));
    }
    // Not an exact name ("VS Code", "chrome"): look for an app whose name contains it.
    let safe: String = name.chars().filter(|c| !matches!(c, '\'' | '"' | '\\' | '*')).collect();
    let query = format!("kMDItemContentType == 'com.apple.application-bundle' && kMDItemDisplayName == '*{safe}*'cd");
    let found = run_cmd(Command::new("mdfind").arg(query), Duration::from_secs(5)).unwrap_or_default();
    let app = found.lines().filter(|p| p.starts_with("/Applications") || p.starts_with("/System/Applications")).min_by_key(|p| p.len());
    let Some(app) = app else { bail!("There's no app called {name} on this Mac") };
    run_cmd(Command::new("open").arg(app), Duration::from_secs(10))?;
    let opened = app.rsplit('/').next().unwrap_or(app).trim_end_matches(".app");
    Ok(format!("Opened {opened}"))
}

pub(super) fn quit_app(name: &str) -> Result<String> {
    // "chrome" → "Google Chrome": the running app whose name matches.
    let name = screen::app_pid(name).map(|(_, n)| n).unwrap_or_else(|| name.to_string());
    osascript(
        r#"on run argv
            set n to item 1 of argv
            if application n is running then
                tell application n to quit
                return "Quit " & n & " (if it has unsaved work it will ask first)"
            end if
            return n & " isn't running"
        end run"#,
        &[&name],
    )
}

// --- The Mac ---------------------------------------------------------------------------------

pub(super) fn current_volume() -> Result<i64> {
    Ok(osascript("output volume of (get volume settings)", &[])?.trim().parse()?)
}

pub(super) fn set_volume(level: Option<i64>, change: Option<i64>) -> Result<String> {
    let target = match (level, change) {
        (Some(l), _) => l,
        (None, Some(c)) => current_volume()? + c,
        (None, None) => bail!("No volume given"),
    }
    .clamp(0, 100);
    if target == 0 {
        osascript("set volume with output muted", &[])?;
        return Ok("Muted".into());
    }
    osascript(&format!("set volume output volume {target}\nset volume without output muted"), &[])?;
    Ok(format!("Volume is now {target}%"))
}

pub(super) fn media(action: &str) -> Result<String> {
    let verb = match action {
        "play" => "play",
        "pause" | "stop" => "pause",
        "next" | "skip" => "next track",
        "previous" | "back" => "previous track",
        "toggle" | "play_pause" => "playpause",
        other => bail!("Unknown music action {other}"),
    };
    let running = |app: &str| Command::new("pgrep").args(["-xq", app]).status().is_ok_and(|s| s.success());
    let app = if running("Spotify") {
        "Spotify"
    } else if running("Music") || verb == "play" {
        "Music"
    } else {
        bail!("No music is playing (Spotify and Music are closed)");
    };
    osascript(&format!(r#"tell application "{app}" to {verb}"#), &[])?;
    if verb == "pause" {
        return Ok(format!("Paused {app}"));
    }
    std::thread::sleep(Duration::from_millis(600));
    let track = osascript(&format!(r#"tell application "{app}" to return (name of current track) & " by " & (artist of current track)"#), &[]);
    Ok(match track {
        Ok(t) if !t.trim().is_empty() => format!("{app} is playing {t}"),
        _ => format!("Done in {app}"),
    })
}

pub(super) fn mac(action: &str) -> Result<String> {
    match action {
        "lock_screen" => {
            osascript(r#"tell application "System Events" to keystroke "q" using {control down, command down}"#, &[])?;
            Ok("Screen locked".into())
        }
        "dark_mode" | "light_mode" => {
            let dark = action == "dark_mode";
            osascript(&format!(r#"tell application "System Events" to tell appearance preferences to set dark mode to {dark}"#), &[])?;
            Ok(format!("Switched to {} mode", if dark { "dark" } else { "light" }))
        }
        "screenshot" => {
            let dir = dirs::desktop_dir().ok_or_else(|| anyhow!("no Desktop folder"))?;
            let path = dir.join(format!("Screenshot {}.png", chrono::Local::now().format("%Y-%m-%d at %H.%M.%S")));
            run_cmd(Command::new("screencapture").arg("-x").arg(&path), Duration::from_secs(10))?;
            Ok(format!("Saved a screenshot to the Desktop: {}", path.display()))
        }
        other => bail!("Unknown Mac action {other}"),
    }
}

pub(super) fn mac_status() -> Result<String> {
    let mut out = Vec::new();
    if let Ok(batt) = run_cmd(Command::new("pmset").args(["-g", "batt"]), Duration::from_secs(3)) {
        static PCT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)%;\s*([a-zA-Z ]+?);\s*([^\n]*)").unwrap());
        match PCT.captures(&batt) {
            Some(c) => {
                let left = c[3].replace("present: true", "").replace("(no estimate)", "").trim().to_string();
                let left = if left.contains("remaining") { format!(", {}", left.replace(" remaining", " left")) } else { String::new() };
                out.push(format!("Battery {}%, {}{left}", &c[1], c[2].trim()))
            }
            None if batt.contains("AC Power") => out.push("On AC power, no battery".into()),
            None => {}
        }
    }
    if let Ok(v) = current_volume() {
        out.push(format!("Volume {v}%"));
    }
    if let Ok(df) = run_cmd(Command::new("df").args(["-H", "/System/Volumes/Data"]), Duration::from_secs(3)) {
        if let Some(cols) = df.lines().nth(1).map(|l| l.split_whitespace().collect::<Vec<_>>()) {
            if cols.len() > 3 {
                out.push(format!("{} free of {} on the disk", cols[3], cols[1]));
            }
        }
    }
    if out.is_empty() {
        bail!("Couldn't read the Mac's status");
    }
    Ok(out.join(". "))
}
