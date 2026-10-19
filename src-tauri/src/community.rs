//! Aba Comunidade: ports listed live from GitHub, outside RecompHUB's verified catalog.
//!
//! Two sources, merged by repository:
//! - the curated list Sebastrion/awesome-unofficial-pc-ports (MIT), read from its README;
//! - GitHub topic searches (`pc-port`, `n64recomp`).
//!
//! The result is kept for 12 hours (GitHub allows 10 searches a minute and 60 API calls an
//! hour without login). Nothing here is checked by RecompHUB: installing one of these ports
//! is experimental, and the page says so.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::github;

const LIST_URL: &str = "https://raw.githubusercontent.com/Sebastrion/awesome-unofficial-pc-ports/main/README.md";
pub const LIST_PAGE: &str = "https://github.com/Sebastrion/awesome-unofficial-pc-ports";
const TOPICS: &[&str] = &["pc-port", "n64recomp"];
const MAX_AGE_SECS: u64 = 12 * 3600;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Item {
    /// GitHub "owner/name".
    pub repo: String,
    pub name: String,
    /// The game, when the list says it.
    #[serde(default)]
    pub game: String,
    /// Console or family ("Nintendo", "PlayStation", "Nintendo 64"...).
    #[serde(default)]
    pub console: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub stars: Option<u64>,
    /// Unix time of the last push, when known.
    #[serde(default)]
    pub pushed: Option<u64>,
    /// Marked ⭐ ("most user-friendly version") in the awesome list.
    #[serde(default)]
    pub recommended: bool,
    /// "lista" and/or "tópico".
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Cache {
    pub fetched_at: u64,
    pub items: Vec<Item>,
    /// Sources that could not be read on the last refresh.
    #[serde(default)]
    pub errors: Vec<String>,
}

pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: Option<&str>| {
        p.map(|s| !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
            .unwrap_or(false)
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Library id for a community port: "gh-owner-name".
pub fn id_for(repo: &str) -> String {
    let s: String = repo.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    format!("gh-{}", s.chars().take(60).collect::<String>())
}

fn github_repo(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/").or_else(|| url.strip_prefix("http://github.com/"))?;
    let mut parts = rest.split(|c| c == '/' || c == '#' || c == '?');
    let repo = format!("{}/{}", parts.next()?, parts.next()?.trim_end_matches(".git"));
    valid_repo(&repo).then_some(repo)
}

/// The awesome list's README: "## 🎮 Nintendo", "<summary><strong>Game</strong>", "### ⭐ [Port](url)".
pub fn parse_awesome(md: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let (mut section, mut game) = (String::new(), String::new());
    for line in md.lines() {
        let line = line.trim();
        if let Some(h) = line.strip_prefix("## ") {
            section = h.trim_start_matches(|c: char| !c.is_alphanumeric()).trim().to_string();
        } else if let Some(rest) = line.strip_prefix("<summary><strong>") {
            game = rest.split("</strong>").next().unwrap_or("").trim().to_string();
        } else if let Some(rest) = line.strip_prefix("### ") {
            let recommended = rest.starts_with('⭐');
            let rest = rest.trim_start_matches('⭐').trim();
            let Some(rest) = rest.strip_prefix('[') else { continue };
            let Some((name, tail)) = rest.split_once("](") else { continue };
            let url = tail.split(')').next().unwrap_or("");
            let Some(repo) = github_repo(url) else { continue };
            if section.eq_ignore_ascii_case("Utilities") {
                continue; // launchers and tools, not games
            }
            items.push(Item {
                repo,
                name: name.to_string(),
                game: game.clone(),
                console: section.clone(),
                recommended,
                sources: vec!["lista".into()],
                ..Default::default()
            });
        }
    }
    items
}

fn console_from_topics(topics: &[String]) -> String {
    const MAP: &[(&str, &str)] = &[
        ("n64", "Nintendo 64"),
        ("nintendo-64", "Nintendo 64"),
        ("gamecube", "GameCube"),
        ("wii", "Wii"),
        ("snes", "Super Nintendo"),
        ("gba", "Game Boy Advance"),
        ("nintendo-ds", "Nintendo DS"),
        ("ps1", "PlayStation"),
        ("psx", "PlayStation"),
        ("playstation", "PlayStation"),
        ("ps2", "PlayStation 2"),
        ("ps3", "PlayStation 3"),
        ("ps4", "PlayStation 4"),
        ("psp", "PSP"),
        ("xbox-360", "Xbox 360"),
        ("xbox360", "Xbox 360"),
        ("xenonrecomp", "Xbox 360"),
        ("rexglue", "Xbox 360"),
        ("xbox", "Xbox"),
        ("dreamcast", "Dreamcast"),
        ("saturn", "Saturn"),
        ("sega-genesis", "Mega Drive"),
    ];
    MAP.iter().find(|(t, _)| topics.iter().any(|x| x == t)).map(|(_, c)| c.to_string()).unwrap_or_default()
}

#[derive(Deserialize)]
struct SearchResult {
    items: Vec<SearchRepo>,
}

#[derive(Deserialize)]
struct SearchRepo {
    full_name: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    stargazers_count: u64,
    #[serde(default)]
    pushed_at: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    fork: bool,
}

async fn search_topic(topic: &str) -> Result<Vec<Item>, String> {
    let url = format!(
        "https://api.github.com/search/repositories?q=topic:{topic}+fork:false+archived:false&sort=stars&order=desc&per_page=60"
    );
    let resp = github::client()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("o GitHub respondeu {}", resp.status()));
    }
    let r: SearchResult = resp.json().await.map_err(|e| e.to_string())?;
    Ok(r.items
        .into_iter()
        .filter(|i| !i.archived && !i.fork && valid_repo(&i.full_name))
        // Lists and toolkits are not games.
        .filter(|i| !i.topics.iter().any(|t| t.starts_with("awesome")) && !i.name.to_lowercase().contains("awesome"))
        .map(|i| Item {
            console: console_from_topics(&i.topics),
            repo: i.full_name,
            name: i.name,
            description: i.description.unwrap_or_default(),
            stars: Some(i.stargazers_count),
            pushed: i.pushed_at.as_deref().and_then(github::parse_time),
            sources: vec!["tópico".into()],
            ..Default::default()
        })
        .collect())
}

/// Joins the list and the searches: one item per repository, the list's game names kept.
pub fn merge(list: Vec<Item>, found: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for item in list.into_iter().chain(found) {
        match out.iter_mut().find(|o| o.repo.eq_ignore_ascii_case(&item.repo)) {
            Some(o) => {
                for s in item.sources {
                    if !o.sources.contains(&s) {
                        o.sources.push(s);
                    }
                }
                o.stars = o.stars.or(item.stars);
                o.pushed = o.pushed.or(item.pushed);
                o.recommended |= item.recommended;
                if o.description.is_empty() {
                    o.description = item.description;
                }
                if o.console.is_empty() {
                    o.console = item.console;
                }
            }
            None => out.push(item),
        }
    }
    out
}

pub fn load(path: &Path) -> Cache {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The cached list, refreshed from GitHub when older than 12 hours (or `force`).
/// A source that fails keeps its items from the previous refresh.
pub async fn refresh(path: &Path, force: bool) -> Cache {
    let old = load(path);
    let now = crate::library::now();
    if !force && !old.items.is_empty() && now.saturating_sub(old.fetched_at) < MAX_AGE_SECS {
        return old;
    }
    let mut errors = Vec::new();
    let list = match github::client().get(LIST_URL).send().await {
        Ok(r) if r.status().is_success() => parse_awesome(&r.text().await.unwrap_or_default()),
        Ok(r) => {
            errors.push(format!("lista awesome-unofficial-pc-ports: {}", r.status()));
            Vec::new()
        }
        Err(e) => {
            errors.push(format!("lista awesome-unofficial-pc-ports: {e}"));
            Vec::new()
        }
    };
    let mut found = Vec::new();
    for t in TOPICS {
        match search_topic(t).await {
            Ok(items) => found.extend(items),
            Err(e) => errors.push(format!("tópico {t}: {e}")),
        }
    }
    let mut items = merge(list, found);
    if items.is_empty() {
        // Offline or rate-limited: keep what we had.
        return Cache { errors, ..old };
    }
    // Sources that failed now keep their old entries.
    for o in old.items {
        if !items.iter().any(|i| i.repo.eq_ignore_ascii_case(&o.repo)) && !errors.is_empty() {
            items.push(o);
        }
    }
    let cache = Cache { fetched_at: now, items, errors };
    if let Ok(text) = serde_json::to_string(&cache) {
        let _ = std::fs::write(path, text);
    }
    cache
}

// ---- Releases of a community port ----

#[derive(Serialize, Clone)]
pub struct Asset {
    pub name: String,
    pub size: u64,
}

#[derive(Serialize, Clone)]
pub struct CommunityRelease {
    pub tag: String,
    pub published: u64,
    pub prerelease: bool,
    /// .zip files that look like a Windows build, the most likely first.
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// 0 = clearly Windows, 1 = no platform named, None = another platform or not a .zip.
pub fn windows_rank(name: &str) -> Option<u8> {
    let n = name.to_lowercase();
    if !n.ends_with(".zip") {
        return None;
    }
    const OTHER: &[&str] = &[
        "linux", "macos", "mac-", "-mac", "_mac", "osx", "darwin", "android", "apk", "appimage", "flatpak", "-ios", "_ios",
        "arm64", "aarch64", "source", "src.zip", "symbols", "pdb", "debuginfo", "switch", "vita", "web", "wasm",
    ];
    if OTHER.iter().any(|o| n.contains(o)) {
        return None;
    }
    let win = ["win", "windows", "x64", "x86_64", "amd64"].iter().any(|w| n.contains(w));
    Some(if win { 0 } else { 1 })
}

async fn api_releases(repo: &str) -> Result<Vec<ApiRelease>, String> {
    if !valid_repo(repo) {
        return Err("Repositório inválido.".into());
    }
    let resp = github::client()
        .get(format!("https://api.github.com/repos/{repo}/releases?per_page=10"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Sem conexão com o GitHub ({e})"))?;
    match resp.status().as_u16() {
        200 => resp.json().await.map_err(|e| e.to_string()),
        403 | 429 => Err("O GitHub limitou as consultas por agora (60 por hora sem login). Tente de novo mais tarde.".into()),
        404 => Err("O repositório não existe mais ou ficou privado.".into()),
        s => Err(format!("O GitHub respondeu {s}")),
    }
}

/// The newest release with a Windows .zip (a stable one when there is any).
fn pick(releases: Vec<ApiRelease>) -> Option<ApiRelease> {
    let usable = |r: &ApiRelease| !r.draft && r.assets.iter().any(|a| windows_rank(&a.name).is_some());
    let mut list: Vec<ApiRelease> = releases.into_iter().filter(usable).collect();
    let i = list.iter().position(|r| !r.prerelease).unwrap_or(0);
    (!list.is_empty()).then(|| list.swap_remove(i))
}

pub async fn latest_release(repo: &str) -> Result<Option<CommunityRelease>, String> {
    let Some(r) = pick(api_releases(repo).await?) else { return Ok(None) };
    let mut assets: Vec<(u8, Asset)> = r
        .assets
        .iter()
        .filter_map(|a| windows_rank(&a.name).map(|rank| (rank, Asset { name: a.name.clone(), size: a.size })))
        .collect();
    assets.sort_by_key(|(rank, a)| (*rank, a.name.clone()));
    Ok(Some(CommunityRelease {
        tag: r.tag_name,
        published: r.published_at.as_deref().and_then(github::parse_time).unwrap_or(0),
        prerelease: r.prerelease,
        assets: assets.into_iter().map(|(_, a)| a).collect(),
    }))
}

/// The download for `asset` of the newest release, looked up again on GitHub (never a
/// link from the page) and only from that repository's release files.
pub async fn release_for(repo: &str, asset: &str) -> Result<github::Release, String> {
    let r = pick(api_releases(repo).await?).ok_or("Este port não tem mais uma versão para Windows.")?;
    let a = r.assets.iter().find(|a| a.name == asset).ok_or("Esse arquivo não está mais na versão atual.")?;
    let prefix = format!("https://github.com/{repo}/releases/download/");
    if !a.browser_download_url.to_lowercase().starts_with(&prefix.to_lowercase()) || windows_rank(&a.name).is_none() {
        return Err("Endereço de download inesperado; a instalação foi cancelada.".into());
    }
    let sha_name = format!("{}.sha256", a.name);
    Ok(github::Release {
        tag: r.tag_name.clone(),
        published: r.published_at.as_deref().and_then(github::parse_time).unwrap_or(0),
        asset_url: a.browser_download_url.clone(),
        asset_size: a.size,
        sha256_url: r.assets.iter().find(|x| x.name == sha_name).map(|x| x.browser_download_url.clone()),
    })
}

/// Programs in an unpacked port that the Play button could start, most likely first.
pub fn find_executables(dir: &Path) -> Vec<String> {
    const SKIP: &[&str] = &[
        "unins", "setup", "vc_redist", "vcredist", "dxsetup", "crash", "updater", "update.exe", "uninstall", "dotnet",
        "redist", "7z", "launcher_helper",
    ];
    let mut found: Vec<(usize, String)> = Vec::new();
    fn walk(dir: &Path, rel: &str, depth: usize, found: &mut Vec<(usize, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() && depth < 2 {
                walk(&e.path(), &path, depth + 1, found);
            } else if kind.is_file() && name.to_lowercase().ends_with(".exe") {
                let lower = name.to_lowercase();
                if !SKIP.iter().any(|s| lower.contains(s)) {
                    found.push((depth, path));
                }
            }
        }
    }
    walk(dir, "", 0, &mut found);
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

/// A relative program path chosen in the page: inside the folder and an .exe.
pub fn valid_exe(dir: &Path, rel: &str) -> bool {
    let p = Path::new(rel);
    rel.to_lowercase().ends_with(".exe")
        && p.components().all(|c| matches!(c, std::path::Component::Normal(_)))
        && dir.join(p).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "## 📚 Table of Contents\n\n## 🎮 Nintendo\n\n<details>\n<summary><strong>Banjo-Kazooie</strong></summary>\n\n### ⭐ [BanjoRecomp](https://github.com/BanjoRecomp/BanjoRecomp)\n\ntext\n\n### [Lighthouse](https://github.com/HarbourMasters/Lighthouse#readme)\n\n### [sm64plus](https://www.mfgg.net/x)\n\n## 🔧 Utilities\n\n<summary><strong>N64 Recomp Launcher</strong></summary>\n\n### [N64RecompLauncher](https://github.com/SirDiabo/N64RecompLauncher)\n";

    #[test]
    fn awesome_list_is_read() {
        let items = parse_awesome(SAMPLE);
        assert_eq!(items.len(), 2, "só repositórios do GitHub, sem utilitários");
        assert_eq!(items[0].repo, "BanjoRecomp/BanjoRecomp");
        assert_eq!(items[0].game, "Banjo-Kazooie");
        assert_eq!(items[0].console, "Nintendo");
        assert!(items[0].recommended);
        assert_eq!(items[1].repo, "HarbourMasters/Lighthouse");
        assert!(!items[1].recommended);
    }

    #[test]
    fn list_and_search_merge_by_repo() {
        let list = parse_awesome(SAMPLE);
        let found = vec![Item {
            repo: "banjorecomp/banjorecomp".into(),
            stars: Some(2000),
            description: "PC port".into(),
            sources: vec!["tópico".into()],
            ..Default::default()
        }];
        let merged = merge(list, found);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].stars, Some(2000));
        assert_eq!(merged[0].sources, vec!["lista", "tópico"]);
    }

    #[test]
    fn windows_zips_are_recognised() {
        assert_eq!(windows_rank("BanjoRecompiled-v1.0.2-Windows.zip"), Some(0));
        assert_eq!(windows_rank("game-x64.zip"), Some(0));
        assert_eq!(windows_rank("release.zip"), Some(1));
        assert_eq!(windows_rank("BanjoRecompiled-v1.0.2-Linux-X64.tar.gz"), None);
        assert_eq!(windows_rank("game-macos.zip"), None);
        assert_eq!(windows_rank("game-android.zip"), None);
        assert_eq!(windows_rank("Setup.exe"), None);
    }

    #[test]
    fn repos_and_ids() {
        assert!(valid_repo("HarbourMasters/Shipwright"));
        assert!(!valid_repo("../etc"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("a/.."));
        assert_eq!(id_for("HarbourMasters/2ship2harkinian"), "gh-harbourmasters-2ship2harkinian");
        assert_eq!(github_repo("https://github.com/a/b.git#x").as_deref(), Some("a/b"));
        assert_eq!(github_repo("https://gitlab.com/a/b"), None);
    }
}
