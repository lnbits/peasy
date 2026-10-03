//! A small safe Markdown renderer: no HTML, scripts, embedded remote media or
//! executable links. Everything is escaped before becoming GTK label markup.
use adw::prelude::*;
use gtk::glib;
fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}
fn link(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && !url
            .chars()
            .any(|c| c.is_control() || c == '"' || c == '<' || c == '>')
}
fn inline(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix('`')
            && let Some(end) = rest.find('`')
        {
            result.push_str(&format!("<tt>{}</tt>", escape(&rest[..end])));
            remaining = &rest[end + 1..];
            continue;
        }
        if let Some(rest) = remaining.strip_prefix("**")
            && let Some(end) = rest.find("**")
        {
            result.push_str(&format!("<b>{}</b>", escape(&rest[..end])));
            remaining = &rest[end + 2..];
            continue;
        }
        if let Some(rest) = remaining.strip_prefix('[')
            && let Some(mid) = rest.find("](")
            && let Some(end) = rest[mid + 2..].find(')')
        {
            let url = &rest[mid + 2..mid + 2 + end];
            if link(url) {
                result.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape(url),
                    escape(&rest[..mid])
                ));
                remaining = &rest[mid + 3 + end..];
                continue;
            }
        }
        let ch = remaining.chars().next().unwrap();
        result.push_str(&escape(&ch.to_string()));
        remaining = &remaining[ch.len_utf8()..];
    }
    result
}
pub fn render(text: &str) -> gtk::Box {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let mut code = false;
    let mut block = String::new();
    let flush = |body: &gtk::Box, block: &mut String, code: bool| {
        if block.is_empty() {
            return;
        }
        let label = gtk::Label::new(None);
        label.set_selectable(true);
        label.set_xalign(0.0);
        label.set_halign(gtk::Align::Fill);
        label.set_wrap(!code);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        if code {
            label.set_text(block.trim_end());
            label.set_direction(gtk::TextDirection::Ltr);
            label.add_css_class("monospace");
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .child(&label)
                .build();
            body.append(&scroll);
        } else {
            label.set_markup(&inline(block.trim_end()));
            body.append(&label);
        }
        block.clear();
    };
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            flush(&body, &mut block, code);
            code = !code;
            continue;
        }
        if !code && line.trim().is_empty() {
            flush(&body, &mut block, false);
            continue;
        }
        if !code
            && let Some(title) = line
                .strip_prefix("### ")
                .or_else(|| line.strip_prefix("## "))
                .or_else(|| line.strip_prefix("# "))
        {
            flush(&body, &mut block, false);
            let label = gtk::Label::new(None);
            label.set_markup(&format!("<b>{}</b>", inline(title)));
            label.set_wrap(true);
            label.set_xalign(0.0);
            label.set_selectable(true);
            body.append(&label);
            continue;
        }
        if !code && let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            block.push_str("• ");
            block.push_str(item);
        } else {
            block.push_str(line);
        }
        block.push('\n');
    }
    flush(&body, &mut block, code);
    body
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_never_injects_markup_or_executable_links() {
        let text = inline(
            "<script> **bold** `a<b` [source](https://example.com?a=1&b=2) [bad](file:///etc/passwd) [bad](javascript:evil)",
        );
        assert!(text.contains("&lt;script&gt;"));
        assert!(text.contains("<b>bold</b>"));
        assert!(text.contains("<tt>a&lt;b</tt>"));
        assert!(text.contains("a=1&amp;b=2"));
        assert!(!text.contains("href=\"file:"));
        assert!(!text.contains("href=\"javascript:"));
    }
}
