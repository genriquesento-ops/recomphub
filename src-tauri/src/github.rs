//! Releases from the official GitHub page of each port.

use serde::Deserialize;

use crate::catalog::GameDef;

#[derive(Clone)]
pub struct Release {
    pub tag: String,
    /// Unix time the release was published.
    pub published: u64,
    pub asset_url: String,
    pub asset_size: u64,
    /// URL of "<asset>.sha256", when the project publishes one.
    pub sha256_url: Option<String>,
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

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("RecompHUB/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

/// The newest stable release that has a Windows build.
pub async fn latest(game: &GameDef) -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{}/releases?per_page=10", game.repo);
    let resp = client()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Sem conexão com o GitHub ({e})"))?;
    if !resp.status().is_success() {
        return Err(format!("O GitHub respondeu {}", resp.status()));
    }
    let releases: Vec<ApiRelease> = resp.json().await.map_err(|e| e.to_string())?;
    for r in releases.into_iter().filter(|r| !r.draft && !r.prerelease) {
        let Some(asset) = r
            .assets
            .iter()
            .find(|a| a.name.starts_with(game.asset_prefix) && a.name.ends_with(game.asset_suffix))
        else {
            continue;
        };
        let sha_name = format!("{}.sha256", asset.name);
        let sha256_url = r.assets.iter().find(|a| a.name == sha_name).map(|a| a.browser_download_url.clone());
        return Ok(Release {
            tag: r.tag_name.clone(),
            published: r.published_at.as_deref().and_then(parse_time).unwrap_or(0),
            asset_url: asset.browser_download_url.clone(),
            asset_size: asset.size,
            sha256_url,
        });
    }
    Err("Nenhuma versão para Windows encontrada na página do projeto".into())
}

/// "2026-10-18T06:10:15Z" to Unix time.
fn parse_time(s: &str) -> Option<u64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    u64::try_from(days * 86400 + hh * 3600 + mm * 60 + ss).ok()
}
