// SPDX-License-Identifier: GPL-2.0-or-later
//! The manual (`docs/manual`) is whole in every language and its links lead somewhere: each page
//! of `SUMMARY.md` is written in English and Bahasa, in the same order, every link between pages
//! points to a page and a heading that exist, and the pages are served.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use linkscope_web::LiveState;
use linkscope_web::docs::{LANGUAGES, manual};
use tower::ServiceExt;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/manual");

fn listed(lang: &str) -> Vec<String> {
    let text = std::fs::read_to_string(format!("{ROOT}/{lang}/SUMMARY.md")).unwrap();
    text.lines().filter_map(|l| l.trim().strip_prefix("- [")).filter_map(|l| l.split_once("](")).map(|(_, f)| f.trim_end_matches(')').trim_end_matches(".md").to_owned()).collect()
}

#[test]
fn every_page_is_written_in_every_language() {
    let en = listed("en");
    assert!(en.len() >= 10, "{en:?}");
    for (code, _) in LANGUAGES {
        assert_eq!(listed(code), en, "{code}'s SUMMARY.md lists other pages, or in another order");
        for slug in &en {
            let path = format!("{ROOT}/{code}/{slug}.md");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert!(text.starts_with("# "), "{path} does not start with its title");
            assert!(!text.contains('\u{2014}'), "{path} has an em dash");
        }
        let served = manual().index.languages.iter().find(|l| l.code == code).unwrap();
        assert_eq!(served.pages.iter().map(|p| p.slug.clone()).collect::<Vec<_>>(), en, "{code}: a page is missing from the binary");
    }
}

#[test]
fn every_link_between_pages_leads_to_a_page_and_a_heading() {
    let m = manual();
    let link = regex::Regex::new(r#"href="/docs/([a-z]+)/([a-z0-9-]+)(?:#([^"]+))?""#).unwrap();
    let heading = regex::Regex::new(r#"<h[2-4] id="([^"]+)""#).unwrap();
    for (code, _) in LANGUAGES {
        let lang = m.index.languages.iter().find(|l| l.code == code).unwrap();
        let ids: HashMap<&str, HashSet<String>> = lang
            .pages
            .iter()
            .map(|p| {
                let page = m.page(code, &p.slug).unwrap();
                let found = heading.captures_iter(&page.html).map(|c| c[1].to_owned()).collect();
                (p.slug.as_str(), found)
            })
            .collect();
        for p in &lang.pages {
            let page = m.page(code, &p.slug).unwrap();
            for c in link.captures_iter(&page.html) {
                assert_eq!(&c[1], code, "{code}/{}: a link into another language", p.slug);
                let to = ids.get(&c[2]).unwrap_or_else(|| panic!("{code}/{}: a link to a page that is not in the manual: {}", p.slug, &c[0]));
                if let Some(h) = c.get(3) {
                    assert!(to.contains(h.as_str()), "{code}/{}: a link to no heading: {} (headings of {}: {to:?})", p.slug, &c[0], &c[2]);
                }
            }
            // A page link written so the renderer did not recognize it would stay `x.md`.
            assert!(!page.html.contains(".md\"") && !page.html.contains(".md#"), "{code}/{}: a link left as a file name", p.slug);
        }
    }
}

#[tokio::test]
async fn the_manual_is_served() {
    let app = linkscope_web::router(Arc::new(RwLock::new(LiveState::default())));
    let get = |uri: &'static str| {
        let app = app.clone();
        async move {
            let res = app.oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
            let status = res.status();
            let body = axum::body::to_bytes(res.into_body(), 1 << 22).await.unwrap();
            (status, serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default())
        }
    };
    let handler = regex::Regex::new(r"<[^>]*\son[a-z]+\s*=").unwrap();
    let (status, index) = get("/api/docs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(index["languages"].as_array().map(|l| l.iter().map(|x| x["code"].as_str().unwrap_or("")).collect::<Vec<_>>()), Some(vec!["en", "id"]));
    for uri in ["/api/docs/en/alerts", "/api/docs/id/alerts"] {
        let (status, page) = get(uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        let html = page["html"].as_str().unwrap_or("");
        assert!(html.contains(r#"<h2 id="webhook">"#), "{uri}: no Webhook section");
        assert!(!html.contains("<script") && !handler.is_match(html), "{uri}");
        assert_eq!(page["prev"]["slug"], "metrics", "{uri}");
    }
    assert_eq!(get("/api/docs/en/nothing").await.0, StatusCode::NOT_FOUND);
    assert_eq!(get("/api/docs/xx/alerts").await.0, StatusCode::NOT_FOUND);
}
