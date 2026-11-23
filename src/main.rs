mod forms;
use fltk::{
    app, browser::HoldBrowser, button::Button, enums::{Align, CallbackTrigger, Color, Font},
    frame::Frame, group::Flex, input::Input, prelude::*, text::{TextBuffer, TextDisplay},
    window::Window,
};
use std::{cell::RefCell, rc::Rc, sync::{mpsc, atomic::{AtomicU64, Ordering}}, thread, time::Duration};
use url::Url;
use scraper::{Html, Selector};

#[derive(Clone)]
struct Page { request_id: u64, url: String, title: String, body: String, status: String, forms: Vec<forms::Form>, links: Vec<(String, String)> }

// Search prefixes: ddg:, google:, bing:, brave:, yahoo:, ecosia:, startpage:.
fn resolve_address(input: &str) -> String {
    let input = input.trim();
    if input.is_empty() { return "https://lite.duckduckgo.com/lite/".into(); }
    if let Some((prefix, query)) = input.split_once(':') {
        let base = match prefix.to_ascii_lowercase().as_str() {
            "ddg" | "duckduckgo" => Some("https://lite.duckduckgo.com/lite/"),
            "google" => Some("https://www.google.com/search"),
            "bing" => Some("https://www.bing.com/search"),
            "brave" => Some("https://search.brave.com/search"),
            "yahoo" => Some("https://search.yahoo.com/search"),
            "ecosia" => Some("https://www.ecosia.org/search"),
            "startpage" => Some("https://www.startpage.com/sp/search"),
            _ => None,
        };
        if let Some(base) = base {
            let mut url = Url::parse(base).unwrap();
            url.query_pairs_mut().append_pair("q", query.trim());
            return url.to_string();
        }
    }
    if input.contains("://") { return input.to_string(); }
    if !input.contains(char::is_whitespace) && (input.contains('.') || input.starts_with("localhost")) {
        return format!("https://{input}");
    }
    let mut url = Url::parse("https://lite.duckduckgo.com/lite/").unwrap();
    url.query_pairs_mut().append_pair("q", input);
    url.to_string()
}
fn unwrap_search_redirect(input: &str) -> String {
    let mut current = input.to_string();
    for _ in 0..3 {
        let parsed = match Url::parse(&current) { Ok(url) => url, Err(_) => break };
        let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
        let is_ddg = matches!(host.as_str(), "duckduckgo.com" | "www.duckduckgo.com" | "links.duckduckgo.com" | "lite.duckduckgo.com");
        let is_google = host == "www.google.com" || host == "google.com";
        let is_bing = host == "www.bing.com" || host == "bing.com";
        let candidate = if is_ddg && parsed.path().starts_with("/l/") {
            parsed.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v.into_owned())
        } else if is_google && parsed.path() == "/url" {
            parsed.query_pairs().find(|(k, _)| k == "q" || k == "url").map(|(_, v)| v.into_owned())
        } else if is_bing && parsed.path() == "/ck/a" {
            // Bing uses an encoded redirect token; do not guess or decode unsupported formats.
            None
        } else { None };
        let Some(candidate) = candidate else { break };
        let Ok(target) = Url::parse(&candidate) else { break };
        if !matches!(target.scheme(), "http" | "https") { break; }
        current = target.to_string();
    }
    current
}
fn fetch_page(input: String) -> Page { fetch_page_inner(unwrap_search_redirect(&resolve_address(&input)), 0) }

fn fetch_page_inner(input: String, redirect_depth: u8) -> Page {
    let url = unwrap_search_redirect(&resolve_address(&input));
    let error = |message: String| Page { request_id: 0, url: url.clone(), title: "Error".into(), body: message.clone(), status: message, forms: Vec::new(), links: Vec::new() };
    let parsed = match Url::parse(&url) { Ok(u) => u, Err(e) => return error(e.to_string()) };
    if !matches!(parsed.scheme(), "http" | "https") { return error("Only HTTP and HTTPS are supported".into()); }
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(25))
        .user_agent("AsterBrowser/0.1")
        .build() {
            Ok(c) => c, Err(e) => return error(e.to_string())
        };
    let response = match client.get(parsed).send() { Ok(r) => r, Err(e) => return error(e.to_string()) };
    let code = response.status();
    let final_url = response.url().to_string();
    // FIX: reject oversized responses before allocating a page-sized string.
    if response.content_length().is_some_and(|n| n > 8 * 1024 * 1024) {
        return error("Page exceeds 8 MiB safety limit".into());
    }
    let bytes = match response.bytes() { Ok(b) => b, Err(e) => return error(e.to_string()) };
    if bytes.len() > 8 * 1024 * 1024 { return error("Page exceeds 8 MiB safety limit".into()); }
    let html = String::from_utf8_lossy(&bytes).into_owned();
    if redirect_depth == 0 {
        if let Some(target) = duckduckgo_lite_redirect(&final_url, &html) {
            return fetch_page_inner(target, 1);
        }
    }
    let title = extract_title(&html).unwrap_or_else(|| final_url.clone());
    let body = plain_text(&html);
    Page { request_id: 0, url: final_url.clone(), title, body, forms: forms::parse(&html, &final_url), links: extract_links(&html, &final_url), status: format!("HTTP {code} · HTML forms available · JavaScript disabled") }
}

fn duckduckgo_lite_redirect(url: &str, html: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.host_str()? != "duckduckgo.com" { return None; }
    if !html.contains("non-JavaScript site") { return None; }
    let mut target = Url::parse("https://lite.duckduckgo.com/lite/").ok()?;
    for (key, value) in parsed.query_pairs() {
        if key == "q" { target.query_pairs_mut().append_pair("q", &value); }
    }
    Some(target.to_string())
}

fn submit_form(form: forms::Form, selected: Option<usize>) -> Page {
    match forms::submit(&form, selected) {
        Ok((url, html, status)) => {
            let title = extract_title(&html).unwrap_or_else(|| url.clone());
            let body = plain_text(&html);
            let forms = forms::parse(&html, &url);
            Page { request_id: 0, links: extract_links(&html, &url), url, title, body, forms, status: format!("HTTP {status} · form submitted") }
        }
        Err(message) => Page {
            request_id: 0, url: form.action, title: "Form error".into(), body: message.clone(),
            status: message, forms: Vec::new(), links: Vec::new()
        }
    }
}

fn extract_links(html: &str, base: &str) -> Vec<(String, String)> {
    let doc = Html::parse_document(html);
    let all = Selector::parse("a[href]").unwrap();
    let base = match Url::parse(base) { Ok(u) => u, Err(_) => return Vec::new() };
    let duckduckgo = base.host_str().map(|h| h == "duckduckgo.com" || h.ends_with(".duckduckgo.com")).unwrap_or(false);
    let mut links = Vec::new();
    for a in doc.select(&all) {
        if duckduckgo && !a.value().classes().any(|c| c == "result-link" || c == "result__a") {
            continue;
        }
        let title = a.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() { continue; }
        let href = a.value().attr("href").unwrap_or("");
        let url = match base.join(href) { Ok(u) if matches!(u.scheme(), "http" | "https") => unwrap_search_redirect(u.as_str()), _ => continue };
        if !links.iter().any(|(_, u)| *u == url) {
            links.push((title, url));
        }
        if links.len() >= 25 { break; }
    }
    links
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let end = lower[open_end..].find("</title>")? + open_end;
    Some(decode_entities(html[open_end..end].trim()))
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">")
        .replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

// This is a minimal display-oriented extractor, not a standards-compliant HTML parser.
fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    let bytes = html.as_bytes();
    let mut hidden: Option<String> = None;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if html[i..].starts_with("<!--") {
                if let Some(n) = html[i + 4..].find("-->") { i += n + 7; continue; }
                break;
            }
            if let Some(n) = html[i..].find('>') {
                let tag = html[i + 1..i + n].trim().to_ascii_lowercase();
                let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
                if hidden.is_none() && matches!(name, "script" | "style" | "head" | "svg") && !tag.starts_with('/') {
                    hidden = Some(name.to_owned());
                } else if hidden.as_deref() == Some(name) && tag.starts_with('/') {
                    hidden = None;
                } else if hidden.is_none() && matches!(name, "p" | "div" | "br" | "h1" | "h2" | "h3" | "li" | "section" | "article" | "tr") {
                    if !out.ends_with('\n') { out.push('\n'); }
                }
                i += n + 1;
                continue;
            }
        }
        let next = html[i..].find('<').map(|n| i + n).unwrap_or(html.len());
        if next == i { i += 1; continue; }
        if hidden.is_none() {
            let chunk = decode_entities(&html[i..next]);
            let cleaned = chunk.split_whitespace().collect::<Vec<_>>().join(" ");
            if !cleaned.is_empty() { out.push_str(&cleaned); out.push(' '); }
        }
        i = next;
    }
    out.trim().to_string()
}

fn main() {
    let app = app::App::default();
    let mut win = Window::default().with_size(1050, 740).with_label("Aster Browser");
    let mut column = Flex::default_fill().column();
    column.set_margin(8);
    column.set_pad(6);
    let mut toolbar = Flex::default().row();
    let mut back = Button::default().with_label("Back");
    toolbar.fixed(&back, 65);
    let mut address = Input::default();
    address.set_value("https://lite.duckduckgo.com/lite/");
    let mut go = Button::default().with_label("Go");
    toolbar.fixed(&go, 65);
    toolbar.end();
    column.fixed(&toolbar, 38);
    let mut links_view = HoldBrowser::default();
    links_view.set_text_size(15);
    column.fixed(&links_view, 0);
    let mut display = TextDisplay::default();
    display.set_text_font(Font::Courier);
    display.set_text_size(15);
    display.set_color(Color::White);
    let mut status = Frame::default().with_label("Ready");
    status.set_align(Align::Left | Align::Inside);
    column.fixed(&status, 24);
    column.end();
    win.end();
    win.make_resizable(true);
    win.show();

    let mut buffer = TextBuffer::default();
    buffer.set_text("Enter a URL and press Go.");
    display.set_buffer(buffer.clone());

    let (tx, rx) = mpsc::channel::<Page>();
    let (wake_tx, wake_rx) = app::channel::<()>();
    let history = Rc::new(RefCell::new(Vec::<String>::new()));
    let current_url = Rc::new(RefCell::new(String::new()));
    let request_counter = std::sync::Arc::new(AtomicU64::new(0));

    let link_targets = Rc::new(RefCell::new(Vec::<String>::new()));
    let navigate = {
        let tx = tx.clone();
        let wake_tx = wake_tx.clone();
        let history = history.clone();
        let current_url = current_url.clone();
        let request_counter = request_counter.clone();
        move |input: String| {
            // FIX: history records the previous displayed page, not the requested input.
            let previous = current_url.borrow().clone();
            if !previous.is_empty() && previous != input { history.borrow_mut().push(previous); }
            let request_id = request_counter.fetch_add(1, Ordering::SeqCst) + 1;
            let tx = tx.clone();
            let wake = wake_tx.clone();
            thread::spawn(move || {
                let mut page = fetch_page(input);
                page.request_id = request_id;
                let _ = tx.send(page);
                wake.send(());
            });
        }
    };
    {
        let navigate = navigate.clone();
        let address = address.clone();
        go.set_callback(move |_| navigate(address.value()));
    }
    {
        let navigate = navigate.clone();
        let mut address_input = address.clone();
        let address_for_callback = address_input.clone();
        address_input.set_trigger(CallbackTrigger::EnterKey);
        address_input.set_callback(move |_| navigate(address_for_callback.value()));
    }
    {
        let tx = tx.clone();
        let wake = wake_tx.clone();
        let history = history.clone();
        let request_counter = request_counter.clone();
        back.set_callback(move |_| {
            // FIX: don't re-add the current page when navigating backward.
            if let Some(previous) = history.borrow_mut().pop() {
                let id = request_counter.fetch_add(1, Ordering::SeqCst) + 1;
                let tx = tx.clone();
                let wake = wake.clone();
                thread::spawn(move || {
                    let mut page = fetch_page(previous);
                    page.request_id = id;
                    let _ = tx.send(page);
                    wake.send(());
                });
            }
        });
    }
    {
        let targets = link_targets.clone();
        let navigate = navigate.clone();
        links_view.set_callback(move |list| {
            let selected = list.value();
            if selected > 0 {
                if let Some(url) = targets.borrow().get((selected - 1) as usize).cloned() {
                    navigate(url);
                }
            }
        });
    }
    navigate("https://lite.duckduckgo.com/lite/".into());
    while app.wait() {
        if wake_rx.recv().is_some() {
            while let Ok(page) = rx.try_recv() {
                // FIX: ignore stale responses from slower, earlier requests.
                if page.request_id != request_counter.load(Ordering::SeqCst) { continue; }
                *current_url.borrow_mut() = page.url.clone();
                links_view.clear();
                link_targets.borrow_mut().clear();
                for (title, url) in &page.links {
                    links_view.add(&title.replace('@', "@@").replace('\n', " "));
                    link_targets.borrow_mut().push(url.clone());
                }
                let list_height = if page.links.is_empty() { 0 } else { (page.links.len().min(8) as i32) * 25 + 8 };
                column.fixed(&links_view, list_height);
                column.recalc();
                status.set_label(&format!("{} · {} links", page.status, page.links.len()));
                address.set_value(&page.url);
                win.set_label(&format!("{} — Aster Browser", page.title));
                buffer.set_text(&page.body);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ddg_redirect() {
        assert_eq!(unwrap_search_redirect("https://duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.python.org%2F&rut=abc"), "https://www.python.org/");
    }
    #[test] fn google_redirect() {
        assert_eq!(unwrap_search_redirect("https://www.google.com/url?q=https%3A%2F%2Fexample.org%2F"), "https://example.org/");
    }
    #[test] fn search_prefixes() {
        assert!(resolve_address("bing: rust language").contains("www.bing.com/search?q=rust+language"));
        assert!(resolve_address("google: hello").contains("google.com/search?q=hello"));
        assert!(resolve_address("hello world").contains("lite.duckduckgo.com/lite/?q=hello+world"));
    }
    #[test] fn title() { assert_eq!(extract_title("<title>Hi &amp; bye</title>").as_deref(), Some("Hi & bye")); }
    #[test] fn links() { assert_eq!(extract_links("<a href=\"/docs\">Docs</a>", "https://example.org/")[0].1, "https://example.org/docs"); }
    #[test] fn text() { assert!(!plain_text("<head><title>secret</title></head><p>Hello</p><script>evil()</script>").contains("evil")); }
}
