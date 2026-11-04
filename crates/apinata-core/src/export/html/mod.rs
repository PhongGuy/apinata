//! The collection as one self-contained, static HTML documentation page. Every collection
//! string passes through `text::escape` and every docs block through `text::docs`.

mod examples;
mod parts;
mod request;
mod text;

use super::{ExportNote, HtmlExport};
use crate::collection::{Collection, Folder};
use crate::resolve::is_pass_through;
use crate::sequence::sort_by_name_then_seq;
use parts::{auth_line, credential_notes, kv_table, method_badge};
use text::{docs, escape};

/// The inline stylesheet; it holds no `url(` or `@import`, so the page fetches nothing.
const STYLE: &str = include_str!("style.css");

/// The policy forbids every fetch and script; only the inline style and data images may load.
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:";

/// The HTML documentation page for `collection`, with notes on everything not carried over.
pub fn html(collection: &Collection) -> HtmlExport {
    let name = escape(collection.name());
    let mut page = String::new();
    page.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    page.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    page.push_str(&format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\">\n"
    ));
    page.push_str(&format!(
        "<title>{name}</title>\n<style>\n{STYLE}</style>\n"
    ));
    page.push_str("</head>\n<body>\n<header>\n");
    page.push_str(&format!("<h1>{name}</h1>\n"));
    let file = collection.file();
    page.push_str(&docs(&file.docs(), "c", 2));
    let headers = file.headers();
    page.push_str(&kv_table("Headers", &headers));
    if !is_pass_through(file.document().get(&["request", "auth"])) {
        page.push_str(&auth_line(&file.auth()));
    }
    page.push_str("</header>\n");
    let mut notes = Vec::new();
    credential_notes(&headers, "collection", &mut notes);
    let mut walk = Walk {
        collection,
        nav: String::new(),
        main: String::new(),
        notes,
    };
    walk.folder(collection.tree(), &[], &[]);
    page.push_str("<nav>\n<h2>Contents</h2>\n<ul>\n");
    page.push_str(&walk.nav);
    page.push_str("</ul>\n</nav>\n<main>\n");
    page.push_str(&walk.main);
    page.push_str("</main>\n</body>\n</html>\n");
    HtmlExport {
        html: page,
        notes: walk.notes,
    }
}

/// The contents list and the sections, written side by side in the Postman exporter's order.
struct Walk<'c> {
    collection: &'c Collection,
    nav: String,
    main: String,
    notes: Vec<ExportNote>,
}

impl Walk<'_> {
    /// Writes `folder`'s children: sub-folders by name then seq, then requests; opaque items
    /// take no position and only leave a note. `trail` holds positions, `path` names.
    fn folder(&mut self, folder: &Folder, trail: &[usize], path: &[&str]) {
        let folders =
            sort_by_name_then_seq(folder.folders().iter().collect(), |f| f.name(), |f| f.seq());
        let mut position = 0;
        for sub in folders {
            position += 1;
            let trail = [trail, &[position]].concat();
            let path = [path, &[sub.name()]].concat();
            let id = anchor("f", &trail);
            let name = escape(sub.name());
            self.nav
                .push_str(&format!("<li><a href=\"#{id}\">{name}</a>"));
            self.main
                .push_str(&format!("<section id=\"{id}\">\n<h2>{name}</h2>\n"));
            if let Some(file) = sub.file() {
                self.main.push_str(&docs(&file.docs(), &id, 3));
                let headers = file.headers();
                self.main.push_str(&kv_table("Headers", &headers));
                let raw = sub.document().and_then(|d| d.get(&["request", "auth"]));
                if !is_pass_through(raw) {
                    self.main.push_str(&auth_line(&file.auth()));
                }
                credential_notes(&headers, &path.join(" / "), &mut self.notes);
            }
            // Only rendered children open a nested list; a folder of opaque items has none.
            if sub.folders().is_empty() && sub.requests().is_empty() {
                self.folder(sub, &trail, &path);
                self.nav.push_str("</li>\n");
            } else {
                self.nav.push_str("\n<ul>\n");
                self.folder(sub, &trail, &path);
                self.nav.push_str("</ul>\n</li>\n");
            }
            self.main.push_str("</section>\n");
        }
        for req in folder.requests() {
            position += 1;
            let id = anchor("r", &[trail, &[position]].concat());
            let badge = method_badge(req.method());
            let name = escape(req.name());
            self.nav
                .push_str(&format!("<li><a href=\"#{id}\">{badge} {name}</a></li>\n"));
            let source = [path, &[req.name()]].concat().join(" / ");
            let article = request::article(self.collection, req, &id, &source, &mut self.notes);
            self.main.push_str(&article);
        }
        for opaque in folder.opaque_items() {
            let source = [path, &[opaque.name()]].concat().join(" / ");
            let detail = format!(
                "only HTTP requests are exported; this {} item was left out",
                opaque.kind()
            );
            self.notes.push(ExportNote::new(source, detail));
        }
    }
}

/// `prefix` and the positions joined by `-`, as in `f-1-2`.
fn anchor(prefix: &str, trail: &[usize]) -> String {
    let parts: Vec<String> = trail.iter().map(usize::to_string).collect();
    format!("{prefix}-{}", parts.join("-"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_stylesheet_stays_small() {
        assert!(super::STYLE.len() < 4096);
    }
}
