//! Pictures for the Comunidade tab. Each port gets one small square picture, in this order:
//! the game's cover from SteamGridDB or IGDB (with the player's own keys, searched by the game
//! name taken from the description), or the first picture of the project's README on GitHub
//! (usually a banner or a screenshot). Only GitHub's own servers and Imgur are accepted for
//! README pictures; badges and donation buttons are skipped. Pictures become 360×360 JPEG
//! thumbnails, so a list of 150 ports stays light, and a port without one is asked again after
//! a week.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

use crate::community::{id_for, Item};
use crate::meta::Keys;

const RETRY_AFTER: u64 = 7 * 86_400;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const SIZE: u32 = 360;
const PARALLEL: usize = 4;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Entry {
    pub checked_at: u64,
    /// "SteamGridDB", "IGDB" or "README"; None when nothing was found.
    pub source: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Index {
    entries: HashMap<String, Entry>,
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join("index.json")
}

fn load_index(dir: &Path) -> Index {
    std::fs::read(index_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn thumb_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.jpg"))
}

/// The pictures already made, as data URLs, with where each came from.
pub fn all(dir: &Path) -> HashMap<String, (String, String)> {
    use base64::Engine;
    let index = load_index(dir);
    index
        .entries
        .into_iter()
        .filter_map(|(id, e)| {
            let source = e.source?;
            let bytes = std::fs::read(thumb_path(dir, &id)).ok()?;
            let url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
            Some((id, (url, source)))
        })
        .collect()
}

// ---- README pictures ----

/// Pictures that are not the game: build badges, donation buttons, logos of services.
fn is_decoration(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    const SKIP: &[&str] = &[
        "shields.io", "badge", ".svg", "travis-ci", "/actions/workflows", "codecov", "discord", "ko-fi", "patreon", "paypal",
        "buymeacoffee", "liberapay", "opencollective", "sponsor", "star-history", "contrib.rocks", "githubusercontent.com/u/",
        "avatars.", "logo",
    ];
    SKIP.iter().any(|s| u.contains(s))
}

fn allowed_host(host: &str) -> bool {
    matches!(
        host,
        "raw.githubusercontent.com"
            | "github.com"
            | "user-images.githubusercontent.com"
            | "private-user-images.githubusercontent.com"
            | "objects.githubusercontent.com"
            | "camo.githubusercontent.com"
            | "i.imgur.com"
    )
}

fn host_of(url: &str) -> &str {
    url.strip_prefix("https://").and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or("")
}

/// Turns a picture address in `repo`'s README into one RecompHUB may download, or None.
pub fn resolve(repo: &str, src: &str) -> Option<String> {
    let src = src.trim().trim_matches(['<', '>']);
    if src.is_empty() || src.starts_with("data:") || src.starts_with('#') || src.contains("..") {
        return None;
    }
    let url = if src.starts_with("http://") {
        return None;
    } else if src.starts_with("https://") {
        // A file page on GitHub: its raw file.
        if let Some(rest) = src.strip_prefix(&format!("https://github.com/{repo}/blob/")) {
            format!("https://raw.githubusercontent.com/{repo}/{}", rest.split('?').next().unwrap_or(rest))
        } else {
            src.to_string()
        }
    } else {
        let path = src.trim_start_matches("./").trim_start_matches('/');
        format!("https://raw.githubusercontent.com/{repo}/HEAD/{path}")
    };
    (allowed_host(host_of(&url)) && !is_decoration(&url)).then_some(url)
}

/// Picture addresses in a README, in order: Markdown `![..](url)` and HTML `<img src="url">`.
pub fn images_in(markdown: &str) -> Vec<String> {
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while let Some(p) = markdown[i..].find("![") {
        let at = i + p;
        i = at + 2;
        let Some(close) = markdown[at..].find("](").map(|c| at + c + 2) else { continue };
        let Some(end) = markdown[close..].find(')').map(|e| close + e) else { continue };
        // "url "title"": the address is the first word.
        let url = markdown[close..end].split_whitespace().next().unwrap_or("");
        found.push((at, url.to_string()));
    }
    let lower = markdown.to_ascii_lowercase();
    let mut j = 0;
    while let Some(p) = lower[j..].find("<img") {
        let at = j + p;
        j = at + 4;
        let Some(tag_end) = lower[at..].find('>').map(|e| at + e) else { continue };
        let tag = &markdown[at..tag_end];
        let tag_lower = &lower[at..tag_end];
        let Some(s) = tag_lower.find("src=") else { continue };
        let rest = &tag[s + 4..];
        let url = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or(""),
            _ => rest.split_whitespace().next().unwrap_or(""),
        };
        found.push((at, url.to_string()));
    }
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, u)| u).collect()
}

const MAX_README: u64 = 1024 * 1024;

/// The body, read piece by piece and given up past `max` bytes (a missing or false
/// Content-Length does not make it read without end).
async fn read_capped(resp: reqwest::Response, max: u64) -> Option<Vec<u8>> {
    use futures_util::StreamExt;
    if resp.content_length().unwrap_or(0) > max {
        return None;
    }
    let mut out = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.ok()?);
        if out.len() as u64 > max {
            return None;
        }
    }
    Some(out)
}

async fn readme(client: &reqwest::Client, repo: &str) -> Option<String> {
    for name in ["README.md", "readme.md", "Readme.md", "README.MD"] {
        let url = format!("https://raw.githubusercontent.com/{repo}/HEAD/{name}");
        if let Ok(r) = client.get(&url).send().await {
            if r.status().is_success() {
                return read_capped(r, MAX_README).await.map(|b| String::from_utf8_lossy(&b).into_owned());
            }
        }
    }
    None
}

async fn download(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let resp = client.get(url).send().await.ok()?;
    // Redirects (github.com/user-attachments) must end on an accepted server too.
    if !resp.status().is_success() || !allowed_host(resp.url().host_str().unwrap_or("")) {
        return None;
    }
    read_capped(resp, MAX_BYTES).await
}

/// A square JPEG thumbnail, cut from the middle of the picture. Size and memory are capped
/// before decoding, so a small file that claims a huge picture is refused.
pub fn thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    // Tiny pictures are icons, not the game.
    if img.width() < 120 || img.height() < 80 {
        return None;
    }
    let small = img.resize_to_fill(SIZE, SIZE, image::imageops::FilterType::Triangle).to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82).encode_image(&small).ok()?;
    Some(out.into_inner())
}

/// One port's picture: (thumbnail, source).
async fn find(client: &reqwest::Client, item: &Item, keys: &Keys, meta_dir: &Path) -> Option<(Vec<u8>, String)> {
    let id = id_for(&item.repo);
    let game = if item.game.is_empty() { String::new() } else { item.game.clone() };
    if keys.any() && !game.is_empty() {
        let meta = crate::meta::fetch(meta_dir, &id, &game, keys).await;
        if let Some(path) = crate::meta::image(meta_dir, &id, "cover") {
            if let Some(t) = std::fs::read(&path).ok().and_then(|b| thumbnail(&b)) {
                return Some((t, meta.cover_from.unwrap_or_else(|| "SteamGridDB".into())));
            }
        }
    }
    let text = readme(client, &item.repo).await?;
    for src in images_in(&text).into_iter().take(8) {
        let Some(url) = resolve(&item.repo, &src) else { continue };
        if let Some(t) = download(client, &url).await.and_then(|b| thumbnail(&b)) {
            return Some((t, "README".into()));
        }
    }
    None
}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Looks for the pictures still missing, a few at a time, in the background. Each one found
/// goes to the page as "community-art" ({id, url, source}).
pub fn fill(app: AppHandle, dir: PathBuf, meta_dir: PathBuf, items: Vec<Item>, keys: Keys) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let now = crate::library::now();
        let mut index = load_index(&dir);
        let todo: Vec<Item> = items
            .into_iter()
            .filter(|i| match index.entries.get(&id_for(&i.repo)) {
                Some(e) if e.source.is_some() => false,
                Some(e) => now.saturating_sub(e.checked_at) > RETRY_AFTER,
                None => true,
            })
            .collect();
        let _ = std::fs::create_dir_all(&dir);
        let client = crate::github::client();
        for chunk in todo.chunks(PARALLEL) {
            let found = futures_util::future::join_all(chunk.iter().map(|i| find(&client, i, &keys, &meta_dir))).await;
            for (item, result) in chunk.iter().zip(found) {
                let id = id_for(&item.repo);
                let source = match result {
                    Some((thumb, source)) if std::fs::write(thumb_path(&dir, &id), &thumb).is_ok() => {
                        use base64::Engine;
                        let url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(&thumb));
                        let _ = app.emit("community-art", serde_json::json!({ "id": id, "url": url, "source": source }));
                        Some(source)
                    }
                    _ => None,
                };
                index.entries.insert(id, Entry { checked_at: now, source });
            }
            // Written beside it and swapped in, so a reader never sees half a file.
            if let Ok(text) = serde_json::to_string(&index) {
                let tmp = dir.join("index.json.tmp");
                if std::fs::write(&tmp, text).is_ok() {
                    let _ = std::fs::rename(&tmp, index_path(&dir));
                }
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_pictures_in_order_without_badges() {
        let md = r#"# Port
[![Build](https://github.com/a/b/actions/workflows/ci.yml/badge.svg)](x)
<p align="center"><img src="docs/banner.png" width="600"></p>
![screenshot](https://github.com/user-attachments/assets/abc "Title")
![remote](http://example.com/x.png)
![tracker](https://evil.example/x.png)
"#;
        let found: Vec<String> = images_in(md).iter().filter_map(|s| resolve("a/b", s)).collect();
        assert_eq!(
            found,
            vec![
                "https://raw.githubusercontent.com/a/b/HEAD/docs/banner.png".to_string(),
                "https://github.com/user-attachments/assets/abc".to_string(),
            ]
        );
    }

    #[test]
    fn github_file_pages_become_raw_files() {
        assert_eq!(
            resolve("a/b", "https://github.com/a/b/blob/main/img/shot.jpg?raw=true").as_deref(),
            Some("https://raw.githubusercontent.com/a/b/main/img/shot.jpg")
        );
        assert_eq!(resolve("a/b", "../../etc/passwd"), None);
        assert_eq!(resolve("a/b", "https://img.shields.io/badge/x"), None);
    }

    #[test]
    fn thumbnails_are_small_squares() {
        let img = image::RgbImage::from_pixel(800, 400, image::Rgb([200, 30, 30]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let t = thumbnail(png.get_ref()).unwrap();
        let back = image::load_from_memory(&t).unwrap();
        assert_eq!((back.width(), back.height()), (360, 360));
        assert!(thumbnail(b"not a picture").is_none());
    }
}
