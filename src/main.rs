mod forms;
use fltk::{
    app, button::{Button, CheckButton}, enums::{Align, CallbackTrigger, Color, Font},
    frame::Frame, group::{Flex, Pack, Scroll}, input::{Input, SecretInput}, menu::Choice, prelude::*, text::{TextBuffer, TextDisplay},
    window::Window,
};
use std::{cell::RefCell, rc::Rc, sync::mpsc, thread, time::Duration};
use url::Url;

#[derive(Clone)]
struct Page { url: String, title: String, body: String, status: String, forms: Vec<forms::Form> }

fn fetch_page(input: String) -> Page { fetch_page_inner(input, 0) }

fn fetch_page_inner(input: String, redirect_depth: u8) -> Page {
    let url = if input.contains("://") { input } else if input.contains('.') && !input.contains(' ') {
        format!("https://{input}")
    } else {
        let mut u = Url::parse("https://lite.duckduckgo.com/lite/").unwrap();
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
    if redirect_depth == 0 {
        if let Some(target) = duckduckgo_lite_redirect(&final_url, &html) {
            return fetch_page_inner(target, 1);
        }
    }
    let title = extract_title(&html).unwrap_or_else(|| final_url.clone());
    let body = plain_text(&html);
    Page { url: final_url.clone(), title, body, forms: forms::parse(&html, &final_url), status: format!("HTTP {code} · HTML forms available · JavaScript disabled") }
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
            Page { url, title, body, forms, status: format!("HTTP {status} · form submitted") }
        }
        Err(message) => Page {
            url: form.action, title: "Form error".into(), body: message.clone(),
            status: message, forms: Vec::new()
        }
    }
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
    toolbar.end();
    column.fixed(&toolbar, 38);
    let mut form_scroll = Scroll::default();
    let mut form_pack = Pack::default().with_size(980, 1);
    form_pack.set_spacing(5);
    form_pack.end();
    form_scroll.end();
    column.fixed(&form_scroll, 0);
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
    navigate("https://lite.duckduckgo.com/lite/".into());
    while app.wait() {
        if wake_rx.recv().is_some() {
            while let Ok(page) = rx.try_recv() {
                form_pack.clear();
                form_pack.begin();
                let form_count = page.forms.len();
                let mut total_height = 0;
                for original_form in page.forms.clone() {
                    let form_state = Rc::new(RefCell::new(original_form));
                    let count = form_state.borrow().controls.iter().filter(|c| !c.disabled && c.kind != "hidden").count();
                    if count == 0 { continue; }
                    let mut row = Pack::default().with_size(form_scroll.w() - 24, 1);
                    row.set_spacing(3);
                    let mut form_height = 0;
                    for (index, control) in form_state.borrow().controls.clone().into_iter().enumerate() {
                        if control.disabled || control.kind == "hidden" { continue; }
                        match control.kind.as_str() {
                            "submit" => {
                                let caption = if control.value.is_empty() { "Submit".to_string() } else { control.value.clone() };
                                let mut button = Button::default().with_size(row.w(), 32).with_label(&caption);
                                let state = form_state.clone();
                                let tx = tx.clone();
                                let wake = wake_tx.clone();
                                button.set_callback(move |_| {
                                    let form = state.borrow().clone();
                                    let tx = tx.clone();
                                    let wake = wake.clone();
                                    thread::spawn(move || {
                                        let result = submit_form(form, Some(index));
                                        let _ = tx.send(result);
                                        wake.send(());
                                    });
                                });
                                form_height += 35;
                            }
                            "checkbox" | "radio" => {
                                let mut check = CheckButton::default().with_size(row.w(), 28).with_label(&control.label);
                                check.set_value(control.checked);
                                let state = form_state.clone();
                                check.set_callback(move |w| state.borrow_mut().controls[index].checked = w.value());
                                form_height += 31;
                            }
                            "select" => {
                                let mut choice = Choice::default().with_size(row.w(), 30);
                                for (_, text) in &control.options { choice.add_choice(&text.replace('|', " ")); }
                                let selected = control.options.iter().position(|(v,_)| *v == control.value).unwrap_or(0);
                                choice.set_value(selected as i32);
                                let state = form_state.clone();
                                let options = control.options.clone();
                                choice.set_callback(move |w| {
                                    if let Some((value, _)) = options.get(w.value().max(0) as usize) {
                                        state.borrow_mut().controls[index].value = value.clone();
                                    }
                                });
                                form_height += 33;
                            }
                            "password" => {
                                let mut field = SecretInput::default().with_size(row.w(), 30);
                                field.set_value(&control.value);
                                field.set_tooltip(&control.label);
                                let state = form_state.clone();
                                field.set_trigger(CallbackTrigger::Changed);
                                field.set_callback(move |w| state.borrow_mut().controls[index].value = w.value());
                                form_height += 33;
                            }
                            _ => {
                                let mut field = Input::default().with_size(row.w(), 30);
                                field.set_value(&control.value);
                                field.set_tooltip(&control.label);
                                let state = form_state.clone();
                                field.set_trigger(CallbackTrigger::Changed);
                                field.set_callback(move |w| state.borrow_mut().controls[index].value = w.value());
                                form_height += 33;
                            }
                        }
                    }
                    // A form without a submit control can still be submitted with this button.
                    if !form_state.borrow().controls.iter().any(|c| c.kind == "submit" && !c.disabled) {
                        let mut button = Button::default().with_size(row.w(), 32).with_label("Submit");
                        let state = form_state.clone();
                        let tx = tx.clone();
                        let wake = wake_tx.clone();
                        button.set_callback(move |_| {
                            let form = state.borrow().clone();
                            let tx = tx.clone();
                            let wake = wake.clone();
                            thread::spawn(move || {
                                let result = submit_form(form, None);
                                let _ = tx.send(result);
                                wake.send(());
                            });
                        });
                        form_height += 35;
                    }
                    row.end();
                    row.resize(row.x(), row.y(), row.w(), form_height.max(35));
                    total_height += form_height.max(35) + 5;
                }
                form_pack.end();
                form_pack.resize(form_pack.x(), form_pack.y(), form_scroll.w() - 24, total_height.max(1));
                form_scroll.resize(form_scroll.x(), form_scroll.y(), form_scroll.w(), total_height.min(230));
                column.fixed(&form_scroll, total_height.min(230));
                column.recalc();
                form_scroll.redraw();
                status.set_label(&format!("{} · {} form(s) · JavaScript disabled", page.status, form_count));
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
    #[test] fn title() { assert_eq!(extract_title("<title>Hi &amp; bye</title>").as_deref(), Some("Hi & bye")); }
    #[test] fn text() { assert!(!plain_text("<head><title>secret</title></head><p>Hello</p><script>evil()</script>").contains("evil")); }
}
