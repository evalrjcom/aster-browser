mod forms;
use fltk::{
    app, button::Button, dialog, enums::{Align, CallbackTrigger, Color, Font},
    frame::Frame, group::Flex, input::Input, prelude::*, text::{TextBuffer, TextDisplay},
    window::Window,
};
use std::{cell::RefCell, rc::Rc, sync::mpsc, thread, time::Duration};
use url::Url;

#[derive(Clone)]
struct Page { url: String, title: String, body: String, status: String, forms: Vec<forms::Form> }

fn fetch_page(input: String) -> Page {
    let url = if input.contains("://") { input } else if input.contains('.') && !input.contains(' ') {
        format!("https://{input}")
    } else {
        let mut u = Url::parse("https://duckduckgo.com/").unwrap();
        u.query_pairs_mut().append_pair("q", &input);
        u.to_string()
    };
    let error = |message: String| Page { url: url.clone(), title: "Error".into(), body: message.clone(), status: message, forms: Vec::new() };
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
    let html = match response.text() { Ok(t) => t, Err(e) => return error(e.to_string()) };
    let title = extract_title(&html).unwrap_or_else(|| final_url.clone());
    let body = plain_text(&html);
    Page { url: final_url.clone(), title, body, forms: forms::parse(&html, &final_url), status: format!("HTTP {code} · HTML forms available · JavaScript disabled") }
}

fn submit_form(form: forms::Form, selected: Option<usize>) -> Page {
    match forms::submit(&form, selected) {
        Ok((url, html, status)) => {
            let title = extract_title(&html).unwrap_or_else(|| url.clone());
            let body = plain_text(&html);
            let forms = forms::parse(&html, &url);
            Page { url, title, body, forms, status: format!("HTTP {status} · form submitted") }
        }
        Err(message) => Page {
            url: form.action, title: "Form error".into(), body: message.clone(),
            status: message, forms: Vec::new()
        }
    }
}

fn edit_form(form: &mut forms::Form) -> Option<usize> {
    let mut submit = None;
    for (i, field) in form.controls.iter_mut().enumerate() {
        if field.disabled { continue; }
        match field.kind.as_str() {
            "hidden" | "file" => continue,
            "submit" => {
                if submit.is_none() { submit = Some(i); }
            }
            "checkbox" | "radio" => {
                let value = dialog::input_default(
                    &format!("{} (checkbox/radio: yes or no)", field.label),
                    if field.checked { "yes" } else { "no" }
                )?;
                field.checked = matches!(value.trim().to_ascii_lowercase().as_str(), "yes" | "y" | "true" | "1");
            }
            _ => {
                let options = if field.kind == "select" {
                    format!(" — choices: {}", field.options.iter().map(|(v,l)| format!("{l} ({v})")).collect::<Vec<_>>().join(", "))
                } else { String::new() };
                let value = dialog::input_default(
                    &format!("{} [{}]{}", field.label, field.kind, options),
                    &field.value
                )?;
                if field.required && value.trim().is_empty() {
                    dialog::alert_default("This field is required.");
                    return None;
                }
                field.value = value;
            }
        }
    }
    Some(submit.unwrap_or(usize::MAX))
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
    address.set_value("https://example.org");
    let mut go = Button::default().with_label("Go");
    toolbar.fixed(&go, 65);
    let mut form_button = Button::default().with_label("Forms");
    toolbar.fixed(&form_button, 80);
    toolbar.end();
    column.fixed(&toolbar, 38);
    let mut display = TextDisplay::default();
    display.set_text_font(Font::Courier);
    display.set_text_size(16);
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
    let active_forms = Rc::new(RefCell::new(Vec::<forms::Form>::new()));
    let navigate = {
        let tx = tx.clone();
        let wake_tx = wake_tx.clone();
        let history = history.clone();
        move |input: String| {
            history.borrow_mut().push(input.clone());
            let tx = tx.clone();
            let wake = wake_tx.clone();
            thread::spawn(move || {
                let page = fetch_page(input);
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
        let navigate = navigate.clone();
        let history = history.clone();
        back.set_callback(move |_| {
            let mut h = history.borrow_mut();
            if h.len() > 1 {
                h.pop();
                if let Some(previous) = h.pop() {
                    drop(h);
                    navigate(previous);
                }
            }
        });
    }
    {
        let active_forms = active_forms.clone();
        let tx = tx.clone();
        let wake = wake_tx.clone();
        form_button.set_callback(move |_| {
            let available = active_forms.borrow().clone();
            if available.is_empty() {
                dialog::message_default("This page contains no HTML forms.");
                return;
            }
            let form_number = if available.len() == 1 { 0 } else {
                let answer = dialog::input_default(
                    &format!("Choose form number (1-{})", available.len()), "1"
                );
                match answer.and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) if n >= 1 && n <= available.len() => n - 1,
                    _ => return
                }
            };
            let mut form = available[form_number].clone();
            let selected = match edit_form(&mut form) { Some(i) => i, None => return };
            let selected = if selected == usize::MAX { None } else { Some(selected) };
            let tx = tx.clone();
            let wake = wake.clone();
            thread::spawn(move || {
                let page = submit_form(form, selected);
                let _ = tx.send(page);
                wake.send(());
            });
        });
    }
    navigate("https://lite.duckduckgo.com/lite/".into());
    while app.wait() {
        if wake_rx.recv().is_some() {
            while let Ok(page) = rx.try_recv() {
                *active_forms.borrow_mut() = page.forms.clone();
                address.set_value(&page.url);
                win.set_label(&format!("{} — Aster Browser", page.title));
                buffer.set_text(&page.body);
                status.set_label(&page.status);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn title() { assert_eq!(extract_title("<title>Hi &amp; bye</title>").as_deref(), Some("Hi & bye")); }
    #[test] fn text() { assert!(!plain_text("<head><title>secret</title></head><p>Hello</p><script>evil()</script>").contains("evil")); }
}
