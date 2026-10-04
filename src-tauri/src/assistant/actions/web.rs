//! Web search, reading pages and the weather. Only search words and page addresses leave the Mac.

use super::*;


pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

pub(super) fn http() -> &'static reqwest::blocking::Client {
    static CLIENT: LazyLock<reqwest::blocking::Client> = LazyLock::new(|| {
        reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .build()
            .expect("http client")
    });
    &CLIENT
}

pub(super) fn host_of(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())).unwrap_or_else(|| url.to_string())
}

type Results = Vec<(String, String, String)>;
type SearchEngine<'a> = (&'a str, &'a reqwest::Url, fn(&str) -> Results);

/// Top results for `query`. Search pages are read directly (no account, key or download):
/// Brave first (its snippets carry dates, which helps with "latest"), DuckDuckGo if Brave
/// refuses, and as a last resort the Brave page opened in WebKit like a person would.
pub(super) fn web_search(query: &str, host: &dyn Host) -> Result<String> {
    static CACHE: Mutex<Vec<(Instant, String, String)>> = Mutex::new(Vec::new());
    const KEEP: Duration = Duration::from_secs(600);
    if query.is_empty() {
        bail!("No search words");
    }
    let key = query.to_lowercase();
    {
        let mut cache = CACHE.lock().unwrap();
        cache.retain(|(at, _, _)| at.elapsed() < KEEP);
        if let Some((_, _, hit)) = cache.iter().find(|(_, q, _)| *q == key) {
            mlog!("search: “{query}” answered from the last 10 minutes");
            return Ok(hit.clone());
        }
    }
    let out = search_fresh(query, host)?;
    CACHE.lock().unwrap().push((Instant::now(), key, out.clone()));
    Ok(out)
}

fn search_fresh(query: &str, host: &dyn Host) -> Result<String> {
    let brave = reqwest::Url::parse_with_params("https://search.brave.com/search", &[("q", query)])?;
    let ddg = reqwest::Url::parse_with_params("https://html.duckduckgo.com/html/", &[("q", query)])?;
    let engines: [SearchEngine<'_>; 2] = [("Brave", &brave, parse_brave_results), ("DuckDuckGo", &ddg, parse_search_results)];
    let today = chrono::Local::now().format("%A %-d %B %Y");
    for (name, url, parse) in engines {
        let results = http()
            .get(url.clone())
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text())
            .map(|html| parse(&html));
        match results {
            Ok(results) if !results.is_empty() => {
                let mut out = format!("Search results for “{query}” (today is {today}):\n");
                for (i, (title, snippet, link)) in results.iter().take(6).enumerate() {
                    out.push_str(&format!("{}. {title} ({link}): {snippet}\n", i + 1));
                }
                // Most answers need more than a snippet: read the top pages now, together,
                // rather than spending another round asking for them one by one.
                for (link, text) in read_top_pages(&results) {
                    out.push_str(&format!("\nFrom {} ({link}):\n{}\n", host_of(&link), truncate(&text, 1100)));
                }
                return Ok(out);
            }
            // A 202 or a page without results: usually a "are you a robot" check.
            Ok(_) => mlog!("{name} search: no results on the page"),
            Err(e) => mlog!("{name} search: {e}"),
        }
    }
    let text = host.render_page(brave.as_str())?;
    let text = tidy_text(&text);
    if text.is_empty() {
        bail!("The search engines didn't answer; try again in a minute");
    }
    Ok(format!("Search results page for “{query}” (today is {today}):\n{text}"))
}

/// The readable text of the first two results worth reading, fetched at the same time and
/// given up on after a few seconds. Videos, PDFs and social media are skipped.
fn read_top_pages(results: &Results) -> Vec<(String, String)> {
    const SKIP: [&str; 9] = ["youtube.com", "youtu.be", "tiktok.com", "instagram.com", "facebook.com", "x.com", "twitter.com", "reddit.com/r/", ".pdf"];
    let links: Vec<String> = results.iter().map(|r| r.2.clone()).filter(|l| !SKIP.iter().any(|s| l.contains(s))).take(2).collect();
    let (tx, rx) = std::sync::mpsc::channel();
    for (i, link) in links.iter().enumerate() {
        let (tx, link) = (tx.clone(), link.clone());
        std::thread::spawn(move || {
            if let Ok((_, text)) = fetch_page(&link) {
                if text.len() >= THIN_PAGE {
                    let _ = tx.send((i, link, text));
                }
            }
        });
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut pages = Vec::new();
    while let Ok(page) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        pages.push(page);
        if pages.len() == links.len() {
            break;
        }
    }
    pages.sort_by_key(|p| p.0);
    pages.into_iter().map(|(_, l, t)| (l, t)).collect()
}

/// Brave Search's results page: (title, snippet, address) of each web result.
pub(super) fn parse_brave_results(html: &str) -> Results {
    static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<a href="(https?://[^"]+)""#).unwrap());
    static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"class="title[^"]*"[^>]*title="([^"]*)""#).unwrap());
    static SNIPPET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)class="content[^"]*"[^>]*>(.*?)</div>"#).unwrap());
    let mut out = Vec::new();
    for block in html.split(r#"data-type="web""#).skip(1) {
        let (Some(link), Some(title)) = (LINK.captures(block), TITLE.captures(block)) else { continue };
        let snippet = SNIPPET.captures(block).map(|c| inline_text(&c[1])).unwrap_or_default();
        out.push((inline_text(&title[1]), snippet, decode_entities(&link[1])));
        if out.len() == 8 {
            break;
        }
    }
    out
}

/// (title, snippet, address) of each result, ads left out.
pub(super) fn parse_search_results(html: &str) -> Results {
    static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)^[^>]*?href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap());
    static SNIPPET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)class="result__snippet"[^>]*>(.*?)</(?:a|td|div)>"#).unwrap());
    let mut out = Vec::new();
    for piece in html.split(r#"class="result__a""#).skip(1) {
        let Some(link) = LINK.captures(piece) else { continue };
        let Some(href) = result_address(&decode_entities(&link[1])) else { continue };
        let snippet = SNIPPET.captures(piece).map(|c| inline_text(&c[1])).unwrap_or_default();
        out.push((inline_text(&link[2]), snippet, href));
        if out.len() == 8 {
            break;
        }
    }
    out
}

/// The real address behind a result link (DuckDuckGo sometimes wraps it); None for ads.
pub(super) fn result_address(href: &str) -> Option<String> {
    let full = if href.starts_with("//") { format!("https:{href}") } else { href.to_string() };
    let url = reqwest::Url::parse(&full).ok()?;
    if url.host_str().is_some_and(|h| h.ends_with("duckduckgo.com")) {
        if url.path().starts_with("/y.js") {
            return None;
        }
        return url.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v.into_owned());
    }
    Some(full)
}

pub(super) fn read_webpage(url: &str, host: &dyn Host) -> Result<String> {
    let url = if url.starts_with("http://") || url.starts_with("https://") { url.to_string() } else { format!("https://{url}") };
    let fetched = fetch_page(&url);
    match &fetched {
        Ok((_, text)) if text.len() >= THIN_PAGE => {}
        // Little or no text, or the site turned the request away: let WebKit run the page.
        _ => {
            mlog!("{url}: {} — opening it in WebKit", fetched.as_ref().map_or_else(|e| e.to_string(), |(_, t)| format!("{} chars of text", t.len())));
            if let Ok(text) = host.render_page(&url) {
                if text.len() > fetched.as_ref().map_or(0, |(_, t)| t.len()) {
                    return Ok(format!("{url}\n\n{}", tidy_text(&text)));
                }
            }
        }
    }
    let (title, text) = fetched?;
    if text.is_empty() {
        bail!("The page has no readable text");
    }
    Ok(format!("{title}\n{url}\n\n{text}"))
}

/// Title and readable text of a page.
pub(super) fn fetch_page(url: &str) -> Result<(String, String)> {
    let resp = http().get(url).send()?;
    if !resp.status().is_success() {
        bail!("The site answered {}", resp.status());
    }
    let kind = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if !kind.is_empty() && !kind.contains("html") && !kind.contains("text") {
        bail!("That's not a web page ({kind})");
    }
    let html = resp.text()?;
    Ok((page_title(&html), page_text(&html)))
}

pub(super) fn page_title(html: &str) -> String {
    static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap());
    TITLE.captures(html).map(|c| inline_text(&c[1])).unwrap_or_default()
}

/// The main text of an HTML page: menus, scripts and boilerplate dropped, paragraphs kept.
pub fn page_text(html: &str) -> String {
    static NOISE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        ["head", "script", "style", "noscript", "svg", "nav", "header", "footer", "aside", "form", "iframe", "template"]
            .iter()
            .map(|t| Regex::new(&format!(r"(?is)<{t}\b.*?</{t}>")).unwrap())
            .collect()
    });
    static MAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<(?:article|main)\b[^>]*>(.*)</(?:article|main)>").unwrap());
    static BREAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>|</(?:p|div|li|h[1-6]|tr|section|blockquote|pre|dd|dt)>").unwrap());
    static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
    let mut html = COMMENT.replace_all(html, "").into_owned();
    for re in NOISE.iter() {
        html = re.replace_all(&html, " ").into_owned();
    }
    // Prefer the article itself when the page marks it.
    let body = match MAIN.captures(&html) {
        Some(m) if m[1].len() > 2000 => m[1].to_string(),
        _ => html,
    };
    let text = BREAK.replace_all(&body, "\n");
    tidy_text(&strip_tags(&text))
}

/// Keeps lines that read like sentences (menus and buttons are a word or two).
pub(super) fn tidy_text(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| l.split(' ').count() >= 5)
        .collect();
    lines.join("\n")
}

pub(super) fn strip_tags(html: &str) -> String {
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    decode_entities(&TAG.replace_all(html, " "))
}

/// A one-line piece of HTML (a title or snippet) as plain text.
pub(super) fn inline_text(html: &str) -> String {
    strip_tags(html).split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn decode_entities(s: &str) -> String {
    static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(#[xX][0-9a-fA-F]+|#[0-9]+|[a-zA-Z]+);").unwrap());
    ENTITY
        .replace_all(s, |c: &regex::Captures| {
            let e = &c[1];
            let ch = if let Some(hex) = e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = e.strip_prefix('#') {
                dec.parse().ok().and_then(char::from_u32)
            } else {
                match e {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    "ndash" => Some('–'),
                    "mdash" => Some('—'),
                    "hellip" => Some('…'),
                    "lsquo" => Some('‘'),
                    "rsquo" => Some('’'),
                    "ldquo" => Some('“'),
                    "rdquo" => Some('”'),
                    "deg" => Some('°'),
                    _ => None,
                }
            };
            ch.map_or_else(|| c[0].to_string(), String::from)
        })
        .into_owned()
}

/// Weather from wttr.in (no account or key). An empty place means where the connection is.
pub(super) fn weather(place: &str) -> Result<String> {
    let mut url = reqwest::Url::parse("https://wttr.in/")?;
    if !place.is_empty() {
        url.path_segments_mut().map_err(|_| anyhow!("bad place"))?.pop_if_empty().push(place);
    }
    url.set_query(Some("format=j1"));
    let v: Value = http().get(url).send()?.error_for_status()?.json()?;
    let f = uses_fahrenheit();
    let temp = |c: &Value, key: &str| {
        let (k, unit) = if f { (format!("{key}F"), "°F") } else { (format!("{key}C"), "°C") };
        format!("{}{unit}", c[k.as_str()].as_str().unwrap_or("?"))
    };
    let now = &v["current_condition"][0];
    let area = &v["nearest_area"][0];
    let name = [area["areaName"][0]["value"].as_str(), area["country"][0]["value"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(", ");
    let wind = if f { format!("{} mph", now["windspeedMiles"].as_str().unwrap_or("?")) } else { format!("{} km/h", now["windspeedKmph"].as_str().unwrap_or("?")) };
    let mut out = format!(
        "Weather in {name}. Now: {}, {} (feels like {}), humidity {}%, wind {wind}.\n",
        now["weatherDesc"][0]["value"].as_str().unwrap_or("").trim(),
        temp(now, "temp_"),
        temp(now, "FeelsLike"),
        now["humidity"].as_str().unwrap_or("?"),
    );
    for (day, label) in v["weather"].as_array().into_iter().flatten().zip(["Today", "Tomorrow"]) {
        let hours = day["hourly"].as_array().cloned().unwrap_or_default();
        let rain = hours.iter().filter_map(|h| h["chanceofrain"].as_str()?.parse::<u32>().ok()).max().unwrap_or(0);
        let midday = hours.get(4).and_then(|h| h["weatherDesc"][0]["value"].as_str()).unwrap_or("").trim().to_string();
        out.push_str(&format!(
            "{label}: {midday}, high {}, low {}, chance of rain {rain}%.\n",
            temp(day, "maxtemp"),
            temp(day, "mintemp")
        ));
    }
    Ok(out)
}

pub(super) fn uses_fahrenheit() -> bool {
    static F: LazyLock<bool> =
        LazyLock::new(|| run_cmd(Command::new("defaults").args(["read", "-g", "AppleTemperatureUnit"]), Duration::from_secs(3)).is_ok_and(|s| s.contains("Fahrenheit")));
    *F
}
