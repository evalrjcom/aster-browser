//! Basic non-JavaScript HTML form support. Not a complete HTML Living Standard implementation.
use scraper::{Html, Selector, ElementRef};
use url::Url;

#[derive(Clone, Debug)]
pub struct Control {
    pub name: String,
    pub label: String,
    pub kind: String,
    pub value: String,
    pub options: Vec<(String, String)>,
    pub checked: bool,
    pub disabled: bool,
    pub required: bool,
}
#[derive(Clone, Debug)]
pub struct Form {
    pub action: String,
    pub method: String,
    pub enctype: String,
    pub controls: Vec<Control>,
}
fn attr(el: &ElementRef<'_>, name: &str) -> String {
    el.value().attr(name).unwrap_or("").to_string()
}
fn label(el: &ElementRef<'_>) -> String {
    let placeholder = attr(el, "placeholder");
    if !placeholder.is_empty() { return placeholder; }
    let aria = attr(el, "aria-label");
    if !aria.is_empty() { return aria; }
    let name = attr(el, "name");
    if !name.is_empty() { return name; }
    attr(el, "id")
}
pub fn parse(html: &str, base: &str) -> Vec<Form> {
    let document = Html::parse_document(html);
    let form_sel = Selector::parse("form").unwrap();
    let control_sel = Selector::parse("input,textarea,select,button").unwrap();
    let option_sel = Selector::parse("option").unwrap();
    let mut forms = Vec::new();
    for form in document.select(&form_sel) {
        let action_attr = attr(&form, "action");
        let action = Url::parse(base).ok().and_then(|u| u.join(&action_attr).ok())
            .map(|u| u.to_string()).unwrap_or_else(|| base.to_owned());
        let method = if attr(&form, "method").eq_ignore_ascii_case("post") { "POST" } else { "GET" }.to_string();
        let enctype = { let e = attr(&form, "enctype"); if e.is_empty() { "application/x-www-form-urlencoded".to_string() } else { e } };
        let mut controls = Vec::new();
        for el in form.select(&control_sel) {
            let tag = el.value().name();
            let kind = if tag == "input" {
                let t = attr(&el, "type").to_ascii_lowercase();
                if t.is_empty() { "text".to_string() } else { t }
            } else if tag == "button" {
                let t = attr(&el, "type").to_ascii_lowercase();
                if t.is_empty() { "submit".to_string() } else { t }
            } else { tag.to_string() };
            if matches!(kind.as_str(), "reset" | "button" | "image") { continue; }
            let mut value = attr(&el, "value");
            let mut options = Vec::new();
            if kind == "textarea" { value = el.text().collect::<String>(); }
            if kind == "select" {
                for opt in el.select(&option_sel) {
                    let text = opt.text().collect::<String>().trim().to_owned();
                    let val = opt.value().attr("value").unwrap_or(&text).to_owned();
                    let selected = opt.value().attr("selected").is_some();
                    if selected || options.is_empty() { value = val.clone(); }
                    options.push((val, text));
                }
            }
            if kind == "checkbox" || kind == "radio" {
                if value.is_empty() { value = "on".into(); }
            }
            controls.push(Control {
                name: attr(&el, "name"),
                label: label(&el),
                kind, value, options,
                checked: el.value().attr("checked").is_some(),
                disabled: el.value().attr("disabled").is_some(),
                required: el.value().attr("required").is_some(),
            });
        }
        forms.push(Form { action, method, enctype, controls });
    }
    forms
}
pub fn pairs(form: &Form, submit_index: Option<usize>) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for (i, control) in form.controls.iter().enumerate() {
        if control.disabled || control.name.is_empty() { continue; }
        if matches!(control.kind.as_str(), "file" | "reset" | "button") { continue; }
        if control.kind == "submit" && Some(i) != submit_index { continue; }
        if matches!(control.kind.as_str(), "checkbox" | "radio") && !control.checked { continue; }
        result.push((control.name.clone(), control.value.clone()));
    }
    result
}
pub fn submit(form: &Form, submit_index: Option<usize>) -> Result<(String, String, String), String> {
    let mut url = Url::parse(&form.action).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https") { return Err("Only HTTP(S) form actions are allowed".into()); }
    if form.controls.iter().any(|c| c.kind == "file") {
        return Err("File upload requires a file picker and is not implemented yet".into());
    }
    let fields = pairs(form, submit_index);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (compatible; AsterBrowser/0.2)")
        .build().map_err(|e| e.to_string())?;
    let response = if form.method == "POST" {
        if form.enctype.starts_with("multipart/form-data") {
            let mut multipart = reqwest::blocking::multipart::Form::new();
            for (name, value) in fields { multipart = multipart.text(name, value); }
            client.post(url).multipart(multipart).send()
        } else if form.enctype.starts_with("text/plain") {
            let body = fields.iter().map(|(k,v)| format!("{k}={v}\r\n")).collect::<String>();
            client.post(url).header("Content-Type", "text/plain").body(body).send()
        } else {
            client.post(url).form(&fields).send()
        }
    } else {
        url.query_pairs_mut().extend_pairs(fields.iter().map(|(k,v)| (k.as_str(),v.as_str())));
        client.get(url).send()
    }.map_err(|e| e.to_string())?;
    let status = response.status().to_string();
    let final_url = response.url().to_string();
    let html = response.text().map_err(|e| e.to_string())?;
    Ok((final_url, html, status))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_duckduckgo_style_search() {
        let forms = parse(r#"<form action="/lite/" method="post">
          <input name="q" type="text" placeholder="Search">
          <input type="hidden" name="kl" value="us-en">
          <input type="submit" name="btnG" value="Search"></form>"#, "https://lite.duckduckgo.com/lite/");
        assert_eq!(forms[0].method, "POST");
        assert_eq!(forms[0].action, "https://lite.duckduckgo.com/lite/");
        assert_eq!(forms[0].controls.len(), 3);
        assert_eq!(pairs(&forms[0], Some(2)).len(), 3);
        assert_eq!(pairs(&forms[0], None).len(), 2);
    }
    #[test]
    fn skips_disabled_and_unchecked() {
        let mut f = parse(r#"<form><input name="a" value="1" disabled>
          <input name="b" type="checkbox"><input name="c" value="yes"></form>"#, "https://example.org/");
        assert_eq!(pairs(&f[0], None), vec![("c".into(), "yes".into())]);
        f[0].controls[1].checked = true;
        assert_eq!(pairs(&f[0], None).len(), 2);
    }
    #[test]
    fn resolves_relative_actions() {
        assert_eq!(parse("<form action='../search'></form>", "https://example.org/a/b").remove(0).action, "https://example.org/search");
    }
}
