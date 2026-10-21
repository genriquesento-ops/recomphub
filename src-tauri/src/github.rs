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

/// The player's personal access token (Configurações), empty for none. Without it GitHub allows
/// 60 API requests an hour per PC; with it, 5,000 (as GithubLauncher's GitHubApiToken).
static TOKEN: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

pub fn set_token(token: &str) {
    *TOKEN.write().unwrap() = token.to_string();
}

/// Classic tokens are "ghp_" plus 36 characters, fine-grained ones "github_pat_" plus about 80.
pub fn valid_token(token: &str) -> bool {
    (20..=255).contains(&token.len()) && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

const API: &str = "https://api.github.com/";

/// A GET to the GitHub API. The token goes only to api.github.com, never to download hosts
/// or other sites.
pub fn api(url: &str) -> reqwest::RequestBuilder {
    let token = TOKEN.read().unwrap().clone();
    api_with(url, &token)
}

fn api_with(url: &str, token: &str) -> reqwest::RequestBuilder {
    let req = client()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if !token.is_empty() && url.starts_with(API) {
        req.bearer_auth(token)
    } else {
        req
    }
}

/// The message for an API answer that is not 200.
pub fn status_error(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 => "O GitHub recusou o token guardado. Confira ou apague o token em Configurações.".into(),
        403 | 429 if TOKEN.read().unwrap().is_empty() => {
            "O GitHub limitou as consultas por agora (60 por hora sem token). Tente mais tarde ou coloque um token do GitHub em Configurações.".into()
        }
        403 | 429 => "O GitHub limitou as consultas por agora. Tente de novo mais tarde.".into(),
        404 => "O repositório não existe mais ou ficou privado.".into(),
        s => format!("O GitHub respondeu {s}"),
    }
}

/// Requests left this hour and the hourly limit, for "Salvar e testar". `token` is the one
/// being tested, before it is saved.
pub async fn rate_limit(token: &str) -> Result<(u64, u64), String> {
    #[derive(Deserialize)]
    struct Core {
        limit: u64,
        remaining: u64,
    }
    #[derive(Deserialize)]
    struct Resources {
        core: Core,
    }
    #[derive(Deserialize)]
    struct Answer {
        resources: Resources,
    }
    let resp = api_with(&format!("{API}rate_limit"), token)
        .send()
        .await
        .map_err(|e| format!("Sem conexão com o GitHub ({e})"))?;
    match resp.status().as_u16() {
        200 => {
            let a: Answer = resp.json().await.map_err(|e| e.to_string())?;
            Ok((a.resources.core.remaining, a.resources.core.limit))
        }
        401 => Err("O GitHub não aceitou esse token. Ele pode ter vencido ou sido copiado pela metade.".into()),
        s => Err(format!("O GitHub respondeu {s}")),
    }
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("RecompHUB/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

/// The newest stable release that has a Windows build.
pub async fn latest(game: &GameDef) -> Result<Release, String> {
    releases(game, 1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| "Nenhuma versão para Windows encontrada na página do projeto".into())
}

/// Up to `max` stable releases with a Windows build, newest first ("Escolher versão").
pub async fn releases(game: &GameDef, max: usize) -> Result<Vec<Release>, String> {
    let url = format!("{API}repos/{}/releases?per_page=20", game.repo);
    let resp = api(&url).send().await.map_err(|e| format!("Sem conexão com o GitHub ({e})"))?;
    if !resp.status().is_success() {
        return Err(status_error(resp.status()));
    }
    let api: Vec<ApiRelease> = resp.json().await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in api.into_iter().filter(|r| !r.draft && !r.prerelease) {
        if out.len() >= max {
            break;
        }
        let Some(asset) = r
            .assets
            .iter()
            .find(|a| a.name.starts_with(game.asset_prefix.as_str()) && a.name.ends_with(game.asset_suffix.as_str()))
        else {
            continue;
        };
        let sha_name = format!("{}.sha256", asset.name);
        let sha256_url = r.assets.iter().find(|a| a.name == sha_name).map(|a| a.browser_download_url.clone());
        out.push(Release {
            tag: r.tag_name.clone(),
            published: r.published_at.as_deref().and_then(parse_time).unwrap_or(0),
            asset_url: asset.browser_download_url.clone(),
            asset_size: asset.size,
            sha256_url,
        });
    }
    Ok(out)
}

/// "2026-10-18T06:10:15Z" to Unix time.
pub fn parse_time(s: &str) -> Option<u64> {
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

#[cfg(test)]
mod tests {
    #[test]
    fn release_dates() {
        assert_eq!(super::parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(super::parse_time("2026-10-18T06:10:15Z"), Some(1_792_303_815));
        assert_eq!(super::parse_time("ontem"), None);
    }

    #[test]
    fn tokens() {
        assert!(super::valid_token("ghp_0123456789abcdefABCDEF0123456789abcd"));
        assert!(super::valid_token(&format!("github_pat_{}", "A1_b".repeat(20))));
        assert!(!super::valid_token("ghp_curto"));
        assert!(!super::valid_token("ghp_0123456789 abcdef0123456789abcdef"));
        assert!(!super::valid_token("ghp_0123456789abcdef0123456789abcdef\nX-Evil: 1"));
    }

    #[test]
    fn token_goes_only_to_the_api() {
        let with = |url: &str| super::api_with(url, "ghp_teste").build().unwrap().headers().contains_key("authorization");
        assert!(with("https://api.github.com/repos/a/b/releases"));
        assert!(!with("https://github.com/a/b/releases/download/v1/x.zip"));
        assert!(!with("https://api.github.com.evil.example/x"));
        assert!(!with("https://thunderstore.io/api/v1/package/"));
    }
}
