//! The user's own browser (Chrome, Arc, Brave, Edge, Safari…), so they stay signed in: opening
//! sites and searches, and managing tabs, with AppleScript. Firefox has no AppleScript for tabs,
//! so it gets keyboard shortcuts instead. Nothing here reads or clicks inside a page.

use super::*;

pub(super) const BROWSERS: [&str; 9] =
    ["Safari", "Google Chrome", "Arc", "Brave Browser", "Microsoft Edge", "Vivaldi", "Chromium", "Opera", "Safari Technology Preview"];

fn running(app: &str) -> bool {
    Command::new("pgrep").args(["-xq", app]).status().is_ok_and(|s| s.success())
}

/// A browser named the way people say it ("chrome", "brave") as its app name.
fn app_name(said: &str) -> Option<&'static str> {
    let s = said.to_lowercase();
    let s = s.trim().trim_end_matches(" browser");
    Some(match s {
        "" => return None,
        "brave" => "Brave Browser",
        "chrome" | "google chrome" | "google" => "Google Chrome",
        "edge" | "microsoft edge" => "Microsoft Edge",
        "safari" => "Safari",
        "arc" => "Arc",
        "firefox" | "mozilla firefox" => "Firefox",
        "vivaldi" => "Vivaldi",
        "opera" => "Opera",
        "chromium" => "Chromium",
        _ => return None,
    })
}

/// The browser the user is looking at, or else one that is open.
pub(super) fn active_browser() -> Option<&'static str> {
    let front = screen::front_app_name();
    if front.as_deref() == Some("Firefox") {
        return Some("Firefox");
    }
    if let Some(b) = BROWSERS.iter().find(|b| front.as_deref() == Some(**b)) {
        return Some(b);
    }
    BROWSERS.into_iter().chain(["Firefox"]).find(|b| running(b))
}

/// The browser to use: the one asked for, else the one in use.
fn pick(said: &str) -> Result<&'static str> {
    match app_name(said) {
        Some(b) => Ok(b),
        None if !said.trim().is_empty() => bail!("{said} isn't a browser I can control"),
        None => active_browser().ok_or_else(|| anyhow!("No web browser is open")),
    }
}

fn is_safari(b: &str) -> bool {
    b.starts_with("Safari")
}

/// AppleScript's name for the tab in front: Safari says "current tab", the others "active tab".
fn tab_ref(b: &str) -> &'static str {
    if is_safari(b) { "current tab of front window" } else { "active tab of front window" }
}

/// Runs AppleScript against `browser` (one of the fixed names above, so safe to put in the
/// script); anything from the AI goes in `args`.
fn tell(browser: &str, body: &str, args: &[&str]) -> Result<String> {
    osascript(&format!("on run argv\ntell application \"{browser}\"\n{body}\nend tell\nend run"), args)
}

fn activate(browser: &str) {
    let _ = osascript(&format!("tell application \"{browser}\" to activate"), &[]);
    std::thread::sleep(Duration::from_millis(250));
}

/// Runs JavaScript in the front tab and returns what it gives back.
fn js(browser: &str, code: &str) -> Result<String> {
    let body = if is_safari(browser) {
        "return do JavaScript (item 1 of argv) in current tab of front window".to_string()
    } else {
        "return execute active tab of front window javascript (item 1 of argv)".to_string()
    };
    tell(browser, &body, &[code]).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("JavaScript") && (msg.contains("turned off") || msg.contains("Allow JavaScript") || msg.contains("Apple Events")) {
            let how = if is_safari(browser) {
                "in Safari choose Settings → Advanced → Show features for web developers, then Develop → Allow JavaScript from Apple Events"
            } else {
                "in the browser's menu bar choose View → Developer → Allow JavaScript from Apple Events"
            };
            anyhow!("{browser} doesn't let Murmur work inside pages yet. To allow it, {how}. Tabs and opening sites still work")
        } else {
            e
        }
    })
}

/// Waits for the front tab to finish loading (up to ten seconds).
fn wait_loaded(browser: &str) {
    std::thread::sleep(Duration::from_millis(500));
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        match js(browser, "document.readyState") {
            Ok(s) if s.trim() == "complete" => break,
            Ok(_) => std::thread::sleep(Duration::from_millis(300)),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(1200));
                break;
            }
        }
    }
    // Pages often fill in a moment after "load".
    std::thread::sleep(Duration::from_millis(400));
}

/// Sites people name without their address: "open youtube" means the site, not a search for it.
const SITES: [(&str, &str); 28] = [
    ("youtube", "www.youtube.com"),
    ("gmail", "mail.google.com"),
    ("google", "www.google.com"),
    ("google maps", "maps.google.com"),
    ("maps", "maps.google.com"),
    ("google drive", "drive.google.com"),
    ("drive", "drive.google.com"),
    ("netflix", "www.netflix.com"),
    ("hotstar", "www.hotstar.com"),
    ("jio hotstar", "www.hotstar.com"),
    ("jiohotstar", "www.hotstar.com"),
    ("amazon", "www.amazon.in"),
    ("flipkart", "www.flipkart.com"),
    ("github", "github.com"),
    ("linkedin", "www.linkedin.com"),
    ("twitter", "x.com"),
    ("x", "x.com"),
    ("facebook", "www.facebook.com"),
    ("instagram", "www.instagram.com"),
    ("reddit", "www.reddit.com"),
    ("wikipedia", "www.wikipedia.org"),
    ("whatsapp", "web.whatsapp.com"),
    ("chatgpt", "chatgpt.com"),
    ("chat gpt", "chatgpt.com"),
    ("gpt", "chatgpt.com"),
    ("perplexity", "www.perplexity.ai"),
    ("claude", "claude.ai"),
    ("spotify", "open.spotify.com"),
];

/// Search pages of sites where "search X for Y" should land on the site's own results.
fn site_search(host: &str, query: &str) -> Option<String> {
    let base = match host.trim_start_matches("www.") {
        "youtube.com" => "https://www.youtube.com/results?search_query=",
        "google.com" => "https://www.google.com/search?q=",
        "amazon.in" => "https://www.amazon.in/s?k=",
        "amazon.com" => "https://www.amazon.com/s?k=",
        "flipkart.com" => "https://www.flipkart.com/search?q=",
        "github.com" => "https://github.com/search?q=",
        "reddit.com" => "https://www.reddit.com/search/?q=",
        "wikipedia.org" | "en.wikipedia.org" => "https://en.wikipedia.org/w/index.php?search=",
        "open.spotify.com" => "https://open.spotify.com/search/",
        "x.com" | "twitter.com" => "https://x.com/search?q=",
        // AI chats that take the question in the address and answer it straight away.
        "chatgpt.com" => "https://chatgpt.com/?q=",
        "perplexity.ai" => "https://www.perplexity.ai/search?q=",
        "claude.ai" => "https://claude.ai/new?q=",
        _ => return None,
    };
    let q: String = reqwest::Url::parse_with_params("https://q.invalid/", &[("q", query)]).ok()?.query()?.trim_start_matches("q=").to_string();
    Some(format!("{base}{q}"))
}

/// The address to open for `target` (a site, an address or words to search for), searching
/// that site for `query` when one is given. Returns the address and whether it names a site.
fn address(target: &str, query: &str) -> Result<(String, bool)> {
    // Stray quotes and commas from a garbled tool call ("amazon.com', ").
    let junk = |c: char| c.is_whitespace() || matches!(c, '\'' | '"' | ',' | '“' | '”' | '‘' | '’');
    let t = target.trim_matches(junk);
    let query = query.trim_matches(junk);
    if t.is_empty() && query.is_empty() {
        bail!("Nothing to open");
    }
    let lower = t.to_lowercase();
    let known = SITES.iter().find(|(name, _)| *name == lower.trim_end_matches(".com")).map(|(_, host)| host.to_string());
    let is_address = !t.contains(char::is_whitespace) && t.contains('.');
    let site = if t.starts_with("http://") || t.starts_with("https://") {
        Some(t.to_string())
    } else if let Some(host) = known {
        Some(format!("https://{host}"))
    } else if is_address {
        Some(format!("https://{t}"))
    } else {
        None
    };
    let google = |q: &str| reqwest::Url::parse_with_params("https://www.google.com/search", &[("q", q)]).map(|u| u.to_string());
    Ok(match (site, query.is_empty()) {
        (Some(url), true) => (url, true),
        (Some(url), false) => {
            let host = reqwest::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
            match site_search(&host, query) {
                Some(found) => (found, true),
                // A site without a known search page: let Google search within it.
                None => (google(&format!("site:{host} {query}"))?, false),
            }
        }
        (None, _) => (google(&format!("{t} {query}").trim().to_string())?, false),
    })
}

/// The page in the browser's front tab, read for the AI (signed out, as a fresh visit sees it).
pub(super) fn read_tab(said_browser: &str, host: &dyn Host) -> Result<String> {
    let browser = pick(said_browser)?;
    let now = where_now(browser);
    let url = now.rsplit_once(" (").map(|(_, u)| u.trim_end_matches(')').to_string()).unwrap_or_default();
    if !url.starts_with("http") {
        bail!("There's no web page open in {browser}");
    }
    super::web::read_webpage(&url, host)
}

/// Title and address of the front tab.
fn where_now(browser: &str) -> String {
    let script = if is_safari(browser) {
        "return (name of current tab of front window) & \" (\" & (URL of current tab of front window) & \")\""
    } else {
        "return (title of active tab of front window) & \" (\" & (URL of active tab of front window) & \")\""
    };
    tell(browser, script, &[]).unwrap_or_default()
}

/// The tab in front once it has loaded: its title and address.
fn after(browser: &str) -> String {
    wait_loaded(browser);
    format!(" Now showing: {}", where_now(browser))
}

/// Opens a site or a search: in a new tab (default) or in the tab in front.
pub(super) fn open(target: &str, query: &str, said_browser: &str, new_tab: bool) -> Result<String> {
    let (url, is_address) = address(target, query)?;
    let browser = match app_name(said_browser) {
        Some(b) => Some(b),
        None if said_browser.trim().is_empty() => active_browser(),
        None => bail!("{said_browser} isn't a browser I can control"),
    };
    let what = if is_address { url.clone() } else { format!("a web search for “{}”", format!("{} {}", target.trim(), query.trim()).trim()) };
    let Some(browser) = browser else {
        // No browser open: the default one opens it.
        run_cmd(Command::new("open").arg(&url), Duration::from_secs(10))?;
        return Ok(format!("Opened {what}"));
    };
    let in_place = !new_tab && browser != "Firefox" && tell(browser, &format!("set URL of {} to (item 1 of argv)", tab_ref(browser)), &[&url]).is_ok();
    if !in_place {
        run_cmd(Command::new("open").args(["-a", browser]).arg(&url), Duration::from_secs(10)).map_err(|_| anyhow!("Couldn't open {browser}; is it installed?"))?;
    }
    activate(browser);
    if browser == "Firefox" {
        return Ok(format!("Opened {what} in Firefox"));
    }
    Ok(format!("Opened {what} in {browser}{}.{}", if in_place { " (same tab)" } else { ", new tab" }, after(browser)))
}

/// A blank new tab.
pub(super) fn new_tab(said_browser: &str) -> Result<String> {
    let browser = pick(said_browser)?;
    activate(browser);
    screen::keys("cmd+t", browser)?;
    Ok(format!("Opened a new tab in {browser}"))
}

/// The open tabs of the front window: (number, title, address), and which is in front.
type BrowserTabs = (usize, Vec<(usize, String, String)>);

fn tab_list(browser: &str) -> Result<BrowserTabs> {
    // (`tab` can't be the separator: inside `tell` it means the browser's tab.)
    let (title, current) = if is_safari(browser) { ("name", "index of current tab of front window") } else { ("title", "active tab index of front window") };
    let out = tell(
        browser,
        &format!(
            "if (count of windows) is 0 then return \"0\"
            set out to ({current}) as text
            set n to count of tabs of front window
            repeat with i from 1 to n
                set t to tab i of front window
                set out to out & linefeed & i & (character id 9) & ({title} of t) & (character id 9) & (URL of t)
            end repeat
            return out"
        ),
        &[],
    )?;
    let mut lines = out.lines();
    let current = lines.next().and_then(|l| l.trim().parse().ok()).unwrap_or(0);
    let tabs = lines
        .filter_map(|l| {
            let mut p = l.splitn(3, '\t');
            Some((p.next()?.trim().parse().ok()?, p.next()?.to_string(), p.next().unwrap_or_default().to_string()))
        })
        .collect();
    Ok((current, tabs))
}

pub(super) fn tabs(said_browser: &str) -> Result<String> {
    let browser = pick(said_browser)?;
    if browser == "Firefox" {
        bail!("Firefox doesn't let other apps list its tabs");
    }
    let (current, tabs) = tab_list(browser)?;
    if tabs.is_empty() {
        return Ok(format!("{browser} has no windows open"));
    }
    let mut out = format!("{} tabs in {browser}:\n", tabs.len());
    for (i, title, url) in &tabs {
        out.push_str(&format!("{i}. {}{} ({}){}\n", truncate(title, 70), "", host_of(url), if *i == current { " ← in front" } else { "" }));
    }
    Ok(out)
}

/// Finds a tab by number or by words in its title or address.
fn find_tab(browser: &str, which: &str) -> Result<(usize, String)> {
    let (_, tabs) = tab_list(browser)?;
    if let Ok(n) = which.trim().parse::<usize>() {
        return tabs.iter().find(|t| t.0 == n).map(|t| (t.0, t.1.clone())).ok_or_else(|| anyhow!("There's no tab {n}"));
    }
    let want = which.to_lowercase();
    let hit = tabs
        .iter()
        .find(|t| t.1.to_lowercase().contains(&want))
        .or_else(|| tabs.iter().find(|t| t.2.to_lowercase().contains(&want.replace(' ', ""))))
        .or_else(|| {
            // Any of the words.
            let words: Vec<&str> = want.split_whitespace().filter(|w| w.len() > 2).collect();
            tabs.iter().find(|t| words.iter().any(|w| t.1.to_lowercase().contains(w) || t.2.to_lowercase().contains(w)))
        });
    hit.map(|t| (t.0, t.1.clone())).ok_or_else(|| anyhow!("No open tab matches “{which}”"))
}

pub(super) fn switch_tab(which: &str, said_browser: &str) -> Result<String> {
    let browser = pick(said_browser)?;
    let (n, title) = find_tab(browser, which)?;
    let n = n.to_string();
    if is_safari(browser) {
        tell(browser, "set current tab of front window to tab ((item 1 of argv) as integer) of front window", &[&n])?;
    } else {
        tell(browser, "set active tab index of front window to ((item 1 of argv) as integer)", &[&n])?;
    }
    activate(browser);
    Ok(format!("Switched to the tab “{title}”"))
}

pub(super) fn close_tab(which: &str, said_browser: &str) -> Result<String> {
    let browser = pick(said_browser)?;
    if browser == "Firefox" {
        activate(browser);
        screen::keys("cmd+w", browser)?;
        return Ok("Closed the tab".into());
    }
    if which.trim().is_empty() {
        let title = where_now(browser);
        tell(browser, &format!("close {}", tab_ref(browser)), &[])?;
        return Ok(format!("Closed the tab {title}"));
    }
    let (n, title) = find_tab(browser, which)?;
    tell(browser, "close tab ((item 1 of argv) as integer) of front window", &[&n.to_string()])?;
    Ok(format!("Closed the tab “{title}”"))
}

/// back, forward or reload, with the browser's own shortcuts.
pub(super) fn navigate(action: &str, said_browser: &str) -> Result<String> {
    let browser = pick(said_browser)?;
    activate(browser);
    let combo = match action {
        "back" => "cmd+[",
        "forward" => "cmd+]",
        _ => "cmd+r",
    };
    screen::keys(combo, browser)?;
    if browser == "Firefox" {
        return Ok(format!("Went {action}"));
    }
    Ok(format!("Went {action}.{}", after(browser)))
}

/// Plays the top YouTube result for `query`: finds the first video on YouTube's results page
/// (no JavaScript in the browser, nothing clicked) and opens it, where it starts by itself.
pub(super) fn play_youtube(query: &str, said_browser: &str) -> Result<String> {
    static VIDEO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)"videoRenderer":\{"videoId":"([A-Za-z0-9_-]{11})".*?"title":\{"runs":\[\{"text":"((?:[^"\\]|\\.)*)""#).unwrap()
    });
    let query = query.trim();
    if query.is_empty() {
        bail!("Say what to play");
    }
    let url = reqwest::Url::parse_with_params("https://www.youtube.com/results", &[("search_query", query)])?;
    let html = http().get(url.as_str()).header("Accept-Language", "en").send()?.error_for_status()?.text()?;
    let Some(found) = VIDEO.captures(&html) else {
        // No video in the page (YouTube changed or asked for consent): show the results instead.
        let opened = open("youtube.com", query, said_browser, true)?;
        return Ok(format!("Couldn't pick a video, so opened YouTube's results for “{query}”. {opened}"));
    };
    let id = &found[1];
    let title: String = serde_json::from_str(&format!("\"{}\"", &found[2])).unwrap_or_else(|_| found[2].to_string());
    open(&format!("https://www.youtube.com/watch?v={id}"), "", said_browser, true)?;
    // "Chaiya Chaiya Full Video | Dil Se | Shah Rukh Khan" → "Chaiya Chaiya Full Video".
    let short = title.split(['|', '(', '[']).next().unwrap_or(&title).trim();
    Ok(format!("Playing “{}” on YouTube", truncate(short, 70)))
}
