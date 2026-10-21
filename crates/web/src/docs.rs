// SPDX-License-Identifier: GPL-2.0-or-later
//! The manual, served with the UI so it matches the installed version and works on isolated
//! networks. Its pages are Markdown in `docs/manual/<lang>/`, in mdBook's layout (`SUMMARY.md`
//! lists the pages in order), embedded in the binary and rendered here. The HTML is sanitized like
//! every other HTML the API gives, so the UI renders no HTML of its own.
//!
//! * `GET /api/docs`                  the languages, each with its title and pages in order
//! * `GET /api/docs/{lang}/{page}`    one page: its title, HTML, headings and neighbors
//!
//! A link to another page is written as on GitHub (`alerts.md#webhook`) and served as the UI's
//! address for it (`/docs/en/alerts#webhook`).

use std::collections::HashSet;
use std::sync::OnceLock;

use axum::Json;
use axum::extract::Path;
use axum::http::StatusCode;
use include_dir::{Dir, include_dir};
use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Serialize;

static MANUAL: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../docs/manual");

/// The manual's languages, in the order the UI offers them: code and the name shown.
pub const LANGUAGES: [(&str, &str); 2] = [("en", "English"), ("id", "Bahasa")];

#[derive(Debug, Clone, Serialize)]
pub struct PageRef {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    /// The manual's title in this language (`SUMMARY.md`'s first heading).
    pub title: String,
    pub pages: Vec<PageRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocsIndex {
    pub languages: Vec<Language>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Heading {
    pub level: u8,
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocPage {
    pub lang: &'static str,
    pub slug: String,
    pub title: String,
    /// Sanitized: structure, text, tables, code and links only.
    pub html: String,
    /// The page's `##` and `###` headings, for its contents list.
    pub headings: Vec<Heading>,
    pub prev: Option<PageRef>,
    pub next: Option<PageRef>,
}

/// The manual as read once from the binary.
pub struct Manual {
    pub index: DocsIndex,
    pages: Vec<DocPage>,
}

impl Manual {
    pub fn page(&self, lang: &str, slug: &str) -> Option<&DocPage> {
        self.pages.iter().find(|p| p.lang == lang && p.slug == slug)
    }
}

/// `SUMMARY.md`: its first `# ` heading is the manual's title, each `- [Title](page.md)` a page.
fn summary(text: &str) -> (String, Vec<PageRef>) {
    let title = text.lines().find_map(|l| l.strip_prefix("# ")).unwrap_or("LinkScope").trim().to_owned();
    let item = regex::Regex::new(r"^\s*[-*]\s*\[([^\]]+)\]\(([^)#\s]+)\.md\)\s*$").expect("a valid pattern");
    let pages = text
        .lines()
        .filter_map(|l| item.captures(l))
        .map(|c| PageRef { slug: c[2].trim_start_matches("./").to_owned(), title: c[1].trim().to_owned() })
        .collect();
    (title, pages)
}

/// A heading's anchor as GitHub and mdBook make it: lowercase, letters, digits, `-` and `_` kept,
/// spaces as `-`, everything else left out; a repeat gets `-1`, `-2`.
fn anchor(text: &str, seen: &mut HashSet<String>) -> String {
    let base: String = text
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect();
    let mut id = base.clone();
    let mut n = 1;
    while !seen.insert(id.clone()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// A link as the UI serves it: `page.md#part` (or `./page.md`) to `/docs/<lang>/page#part`; a link
/// to a page not in the manual, or anything else, is left as written (the sanitizer keeps only
/// http, https and relative links).
fn link(dest: &str, lang: &str, slugs: &HashSet<&str>) -> Option<String> {
    if dest.contains("://") || dest.starts_with('/') || dest.starts_with('#') {
        return None;
    }
    let (file, frag) = dest.split_once('#').map_or((dest, None), |(f, h)| (f, Some(h)));
    let slug = file.trim_start_matches("./").strip_suffix(".md")?;
    slugs.contains(slug).then(|| format!("/docs/{lang}/{slug}{}", frag.map(|h| format!("#{h}")).unwrap_or_default()))
}

/// The HTML the manual may contain.
fn clean(html: &str) -> String {
    let tags: HashSet<&str> = [
        "h1", "h2", "h3", "h4", "p", "br", "hr", "b", "i", "em", "strong", "del", "code", "pre", "blockquote", "table", "thead", "tbody", "tr", "td", "th", "ul", "ol", "li", "a",
    ]
    .into_iter()
    .collect();
    let ids = HashSet::from(["id"]);
    ammonia::Builder::default()
        .tags(tags)
        .generic_attributes(HashSet::new())
        .tag_attributes([("a", HashSet::from(["href"])), ("h2", ids.clone()), ("h3", ids.clone()), ("h4", ids), ("ol", HashSet::from(["start"]))].into_iter().collect())
        .url_schemes(HashSet::from(["http", "https"]))
        .link_rel(Some("noopener noreferrer"))
        .clean(html)
        .to_string()
}

/// One page, rendered: the `# ` heading is its title (the page shows it apart), the others get
/// anchors, links between pages point into the UI.
fn render(lang: &'static str, slug: &str, markdown: &str, slugs: &HashSet<&str>) -> (String, String, Vec<Heading>) {
    let mut events: Vec<Event> = Parser::new_ext(markdown, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).collect();
    let mut title = String::new();
    let mut headings = Vec::new();
    let mut seen = HashSet::new();
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Heading { level, .. }) = &events[i] {
            let level = *level;
            let end = (i..events.len()).find(|j| matches!(events[*j], Event::End(TagEnd::Heading(_)))).unwrap_or(events.len() - 1);
            let text: String = events[i + 1..end]
                .iter()
                .filter_map(|e| match e {
                    Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
                    _ => None,
                })
                .collect();
            if level == HeadingLevel::H1 && title.is_empty() {
                title = text;
                events.drain(i..=end);
                continue;
            }
            let id = anchor(&text, &mut seen);
            if matches!(level, HeadingLevel::H2 | HeadingLevel::H3) {
                headings.push(Heading { level: level as u8, id: id.clone(), text });
            }
            if let Event::Start(Tag::Heading { id: slot, .. }) = &mut events[i] {
                *slot = Some(CowStr::from(id));
            }
            i = end + 1;
            continue;
        }
        if let Event::Start(Tag::Link { dest_url, .. }) = &mut events[i]
            && let Some(to) = link(dest_url, lang, slugs) {
                *dest_url = CowStr::from(to);
            }
        i += 1;
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    if title.is_empty() {
        title = slug.to_owned();
    }
    (title, clean(&html), headings)
}

fn build() -> Manual {
    let mut languages = Vec::new();
    let mut pages = Vec::new();
    for (code, name) in LANGUAGES {
        let file = |f: &str| MANUAL.get_file(format!("{code}/{f}")).and_then(|f| f.contents_utf8());
        let Some(list) = file("SUMMARY.md") else { continue };
        let (title, refs) = summary(list);
        let slugs: HashSet<&str> = refs.iter().map(|r| r.slug.as_str()).collect();
        let present: Vec<PageRef> = refs.iter().filter(|r| file(&format!("{}.md", r.slug)).is_some()).cloned().collect();
        for (n, r) in present.iter().enumerate() {
            let markdown = file(&format!("{}.md", r.slug)).unwrap_or_default();
            let (heading, html, headings) = render(code, &r.slug, markdown, &slugs);
            pages.push(DocPage {
                lang: code,
                slug: r.slug.clone(),
                // The list's title wins: it is what the contents show.
                title: if r.title.is_empty() { heading } else { r.title.clone() },
                html,
                headings,
                prev: n.checked_sub(1).map(|p| present[p].clone()),
                next: present.get(n + 1).cloned(),
            });
        }
        languages.push(Language { code, name, title, pages: present });
    }
    Manual { index: DocsIndex { languages }, pages }
}

/// The manual, rendered at first use.
pub fn manual() -> &'static Manual {
    static M: OnceLock<Manual> = OnceLock::new();
    M.get_or_init(build)
}

/// `GET /api/docs`
pub async fn index() -> Json<DocsIndex> {
    Json(manual().index.clone())
}

/// `GET /api/docs/{lang}/{page}`
pub async fn page(Path((lang, slug)): Path<(String, String)>) -> Result<Json<DocPage>, (StatusCode, String)> {
    manual().page(&lang, &slug).cloned().map(Json).ok_or_else(|| (StatusCode::NOT_FOUND, format!("no page {slug} in the manual ({lang})")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_and_anchors_read_as_mdbook_writes_them() {
        let (title, pages) = summary("# LinkScope manual\n\n- [Introduction](introduction.md)\n- [Alerts](./alerts.md)\n- not a page\n");
        assert_eq!(title, "LinkScope manual");
        assert_eq!(pages.iter().map(|p| (p.slug.as_str(), p.title.as_str())).collect::<Vec<_>>(), [("introduction", "Introduction"), ("alerts", "Alerts")]);
        let mut seen = HashSet::new();
        assert_eq!(anchor("Webhook", &mut seen), "webhook");
        assert_eq!(anchor("The `ThreshFor[]` keyword", &mut seen), "the-threshfor-keyword");
        assert_eq!(anchor("Webhook", &mut seen), "webhook-1");
    }

    #[test]
    fn a_page_renders_with_anchors_inner_links_and_nothing_unsafe() {
        let slugs = HashSet::from(["alerts", "install"]);
        let md = "# Alerts\n\nSee [install](install.md) and [the webhook](alerts.md#webhook), [MRTG](https://oss.oetiker.ch/mrtg/), [other](other.md).\n\n## Webhook\n\n### The `body`\n\n<script>alert(1)</script>\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n[x](javascript:alert(1))\n";
        let (title, html, headings) = render("en", "alerts", md, &slugs);
        assert_eq!(title, "Alerts");
        assert!(!html.contains("<h1"), "{html}");
        assert!(html.contains(r#"<h2 id="webhook">Webhook</h2>"#), "{html}");
        assert!(html.contains(r#"<h3 id="the-body">The <code>body</code></h3>"#), "{html}");
        assert!(html.contains(r#"href="/docs/en/install""#) && html.contains(r#"href="/docs/en/alerts#webhook""#), "{html}");
        assert!(html.contains(r#"href="https://oss.oetiker.ch/mrtg/""#) && html.contains(r#"href="other.md""#), "{html}");
        assert!(html.contains("<table>") && !html.contains("<script") && !html.contains("javascript:"), "{html}");
        assert_eq!(headings.iter().map(|h| (h.level, h.id.as_str())).collect::<Vec<_>>(), [(2, "webhook"), (3, "the-body")]);
    }
}
