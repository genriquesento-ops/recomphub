//! Game information from SteamGridDB (covers and banners made by its community) and IGDB
//! (description, release date, genres, cover and artwork), with the player's own keys.
//!
//! Each game is looked up once and kept in RecompHUB's data folder, so the network is only
//! used again when the player asks ("Buscar de novo") or after a lookup that found nothing:
//! - `meta/<id>.json`: what was found and where it came from;
//! - `meta/<id>/cover.<ext>` and `meta/<id>/banner.<ext>`: the pictures.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::covers;

const SGDB: &str = "https://www.steamgriddb.com/api/v2";
const IGDB_GAMES: &str = "https://api.igdb.com/v4/games";
const TWITCH_TOKEN: &str = "https://id.twitch.tv/oauth2/token";
/// A lookup that found nothing is tried again after a week; a found game stays cached.
const RETRY_AFTER: u64 = 7 * 86_400;

/// Where the player gets their keys (the only extra pages "open_link" accepts from here).
pub const SGDB_KEY_PAGE: &str = "https://www.steamgriddb.com/profile/preferences/api";
pub const IGDB_KEY_PAGE: &str = "https://api-docs.igdb.com/#account-creation";

/// The player's keys, from settings.json.
#[derive(Clone, Default)]
pub struct Keys {
    pub sgdb: String,
    pub igdb_id: String,
    pub igdb_secret: String,
}

impl Keys {
    pub fn any(&self) -> bool {
        !self.sgdb.is_empty() || self.igdb()
    }
    pub fn igdb(&self) -> bool {
        !self.igdb_id.is_empty() && !self.igdb_secret.is_empty()
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Meta {
    pub fetched_at: u64,
    /// The name that was searched.
    pub term: String,
    /// The game that was found, as the sites call it.
    pub name: Option<String>,
    /// IGDB's description (in English).
    pub summary: Option<String>,
    /// Unix time of the first release.
    pub released: Option<i64>,
    #[serde(default)]
    pub genres: Vec<String>,
    pub igdb_url: Option<String>,
    pub sgdb_id: Option<u64>,
    /// "SteamGridDB" or "IGDB".
    pub cover_from: Option<String>,
    pub banner_from: Option<String>,
    #[serde(default)]
    pub errors: Vec<String>,
}

impl Meta {
    pub fn found(&self) -> bool {
        self.name.is_some()
    }
}

fn json_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

pub fn load(dir: &Path, id: &str) -> Option<Meta> {
    std::fs::read(json_path(dir, id)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// Nothing cached yet, or a lookup that found nothing a week ago.
pub fn stale(meta: Option<&Meta>, now: u64) -> bool {
    match meta {
        None => true,
        Some(m) => !m.found() && now >= m.fetched_at + RETRY_AFTER,
    }
}

/// `kind` is "cover" or "banner".
pub fn image(dir: &Path, id: &str, kind: &str) -> Option<PathBuf> {
    covers::find(&dir.join(id), kind)
}

pub fn clear(dir: &Path, id: &str) {
    let _ = std::fs::remove_file(json_path(dir, id));
    let _ = std::fs::remove_dir_all(dir.join(id));
}

/// Looks `term` up on every site the player has a key for, keeps the pictures and saves the
/// result (also when nothing was found, so it is not asked again right away).
pub async fn fetch(dir: &Path, id: &str, term: &str, keys: &Keys) -> Meta {
    let client = crate::github::client();
    let mut meta = Meta { fetched_at: crate::library::now(), term: term.to_string(), ..Default::default() };
    let mut covers_found: Vec<(String, &str)> = Vec::new();
    let mut banners_found: Vec<(String, &str)> = Vec::new();

    if !keys.sgdb.is_empty() {
        match sgdb(&client, &keys.sgdb, term).await {
            Ok(Some(g)) => {
                meta.sgdb_id = Some(g.id);
                meta.name = Some(g.name);
                covers_found.extend(g.grids.into_iter().map(|u| (u, "SteamGridDB")));
                banners_found.extend(g.heroes.into_iter().map(|u| (u, "SteamGridDB")));
            }
            Ok(None) => meta.errors.push("SteamGridDB: nenhum jogo com esse nome.".into()),
            Err(e) => meta.errors.push(format!("SteamGridDB: {e}")),
        }
    }
    if keys.igdb() {
        match igdb(&client, keys, term).await {
            Ok(Some(g)) => {
                meta.name = Some(g.name);
                meta.summary = g.summary;
                meta.released = g.released;
                meta.genres = g.genres;
                meta.igdb_url = g.url;
                covers_found.extend(g.cover.map(|u| (u, "IGDB")));
                banners_found.extend(g.banner.map(|u| (u, "IGDB")));
            }
            Ok(None) => meta.errors.push("IGDB: nenhum jogo com esse nome.".into()),
            Err(e) => meta.errors.push(format!("IGDB: {e}")),
        }
    }

    let images = dir.join(id);
    let _ = std::fs::remove_dir_all(&images);
    for (url, from) in covers_found {
        if download(&client, &images, "cover", &url).await {
            meta.cover_from = Some(from.to_string());
            break;
        }
    }
    for (url, from) in banners_found {
        if download(&client, &images, "banner", &url).await {
            meta.banner_from = Some(from.to_string());
            break;
        }
    }
    let _ = std::fs::create_dir_all(dir);
    if let Ok(text) = serde_json::to_string_pretty(&meta) {
        let _ = std::fs::write(json_path(dir, id), text);
    }
    meta
}

/// Only pictures from the two sites' own image servers are kept.
fn allowed_image(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split('/').next().unwrap_or("");
    host == "images.igdb.com" || host == "steamgriddb.com" || host.ends_with(".steamgriddb.com")
}

async fn download(client: &reqwest::Client, dir: &Path, kind: &str, url: &str) -> bool {
    if !allowed_image(url) {
        return false;
    }
    let Ok(resp) = client.get(url).send().await else { return false };
    if !resp.status().is_success() || resp.content_length().unwrap_or(0) > covers::MAX_IMAGE_BYTES {
        return false;
    }
    let Ok(bytes) = resp.bytes().await else { return false };
    bytes.len() as u64 <= covers::MAX_IMAGE_BYTES && covers::store(dir, kind, &bytes).is_ok()
}

/// Lower case, letters and digits only, so "Banjo-Kazooie" finds "Banjo Kazooie".
fn simple(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// The entry whose name is the term itself, else the site's first (most relevant) one.
fn best<'a>(items: &'a [Value], term: &str) -> Option<&'a Value> {
    let want = simple(term);
    items
        .iter()
        .find(|i| i["name"].as_str().map(simple).as_deref() == Some(want.as_str()))
        .or_else(|| items.first())
}

// ---- SteamGridDB ----

struct SgdbGame {
    id: u64,
    name: String,
    grids: Vec<String>,
    heroes: Vec<String>,
}

async fn sgdb_get(client: &reqwest::Client, key: &str, path: &str) -> Result<Value, String> {
    let resp = client
        .get(format!("{SGDB}{path}"))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| "sem conexão com o site.".to_string())?;
    match resp.status().as_u16() {
        200..=299 => resp.json().await.map_err(|_| "resposta ilegível.".to_string()),
        404 => Ok(Value::Null),
        401 | 403 => Err("a chave foi recusada. Confira a chave em Configurações.".into()),
        429 => Err("muitas consultas seguidas; tente daqui a pouco.".into()),
        s => Err(format!("o site respondeu com erro {s}.")),
    }
}

fn sgdb_pick(search: &Value, term: &str) -> Option<(u64, String)> {
    let items = search["data"].as_array()?;
    let g = best(items, term)?;
    Some((g["id"].as_u64()?, g["name"].as_str()?.to_string()))
}

fn sgdb_urls(list: &Value) -> Vec<String> {
    list["data"]
        .as_array()
        .map(|a| a.iter().filter_map(|i| i["url"].as_str()).filter(|u| allowed_image(u)).take(3).map(String::from).collect())
        .unwrap_or_default()
}

async fn sgdb(client: &reqwest::Client, key: &str, term: &str) -> Result<Option<SgdbGame>, String> {
    let search = sgdb_get(client, key, &format!("/search/autocomplete/{}", covers::encode(term))).await?;
    let Some((id, name)) = sgdb_pick(&search, term) else { return Ok(None) };
    let filters = "types=static&nsfw=false&humor=false";
    let grids = sgdb_get(client, key, &format!("/grids/game/{id}?dimensions=600x900,342x482,660x930&{filters}")).await?;
    let heroes = sgdb_get(client, key, &format!("/heroes/game/{id}?{filters}")).await?;
    Ok(Some(SgdbGame { id, name, grids: sgdb_urls(&grids), heroes: sgdb_urls(&heroes) }))
}

// ---- IGDB (keys from a Twitch developer application) ----

/// (client id, token, valid until), so a token is asked for once, not once per game.
static TOKEN: Mutex<Option<(String, String, u64)>> = Mutex::new(None);

async fn igdb_token(client: &reqwest::Client, keys: &Keys) -> Result<String, String> {
    let now = crate::library::now();
    if let Some((id, token, until)) = TOKEN.lock().unwrap().clone() {
        if id == keys.igdb_id && until > now + 60 {
            return Ok(token);
        }
    }
    // The secret goes in the body, never in the address.
    let body = format!(
        "client_id={}&client_secret={}&grant_type=client_credentials",
        covers::encode(&keys.igdb_id),
        covers::encode(&keys.igdb_secret)
    );
    let resp = client
        .post(TWITCH_TOKEN)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|_| "sem conexão com a Twitch.".to_string())?;
    if !resp.status().is_success() {
        return Err("o Client ID ou o Client Secret foi recusado. Confira em Configurações.".into());
    }
    let v: Value = resp.json().await.map_err(|_| "resposta ilegível da Twitch.".to_string())?;
    let token = v["access_token"].as_str().ok_or("a Twitch não devolveu o acesso.")?.to_string();
    let until = now + v["expires_in"].as_u64().unwrap_or(3600);
    *TOKEN.lock().unwrap() = Some((keys.igdb_id.clone(), token.clone(), until));
    Ok(token)
}

/// Apicalypse query; quotes and backslashes are dropped from the name so it stays one string.
fn igdb_query(term: &str) -> String {
    let t: String = term.chars().filter(|c| *c != '"' && *c != '\\' && !c.is_control()).collect();
    format!(
        "search \"{t}\"; fields name,summary,first_release_date,url,genres.name,cover.image_id,\
artworks.image_id,screenshots.image_id; limit 10;"
    )
}

struct IgdbGame {
    name: String,
    summary: Option<String>,
    released: Option<i64>,
    genres: Vec<String>,
    url: Option<String>,
    cover: Option<String>,
    banner: Option<String>,
}

fn igdb_image(v: &Value, size: &str) -> Option<String> {
    let id = v["image_id"].as_str()?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(format!("https://images.igdb.com/igdb/image/upload/{size}/{id}.jpg"))
}

fn igdb_pick(list: &Value, term: &str) -> Option<IgdbGame> {
    let g = best(list.as_array()?, term)?;
    let first_image = |field: &str| g[field].as_array().and_then(|a| a.first()).and_then(|i| igdb_image(i, "t_1080p"));
    Some(IgdbGame {
        name: g["name"].as_str()?.to_string(),
        summary: g["summary"].as_str().map(String::from),
        released: g["first_release_date"].as_i64(),
        genres: g["genres"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect())
            .unwrap_or_default(),
        url: g["url"].as_str().filter(|u| u.starts_with("https://www.igdb.com/games/")).map(String::from),
        cover: igdb_image(&g["cover"], "t_cover_big_2x"),
        banner: first_image("artworks").or_else(|| first_image("screenshots")),
    })
}

async fn igdb(client: &reqwest::Client, keys: &Keys, term: &str) -> Result<Option<IgdbGame>, String> {
    let token = igdb_token(client, keys).await?;
    let resp = client
        .post(IGDB_GAMES)
        .header("Client-ID", &keys.igdb_id)
        .bearer_auth(&token)
        .header(reqwest::header::CONTENT_TYPE, "text/plain")
        .body(igdb_query(term))
        .send()
        .await
        .map_err(|_| "sem conexão com o site.".to_string())?;
    match resp.status().as_u16() {
        200..=299 => {}
        401 | 403 => {
            *TOKEN.lock().unwrap() = None;
            return Err("o acesso foi recusado. Confira o Client ID e o Client Secret em Configurações.".into());
        }
        429 => return Err("muitas consultas seguidas; tente daqui a pouco.".into()),
        s => return Err(format!("o site respondeu com erro {s}.")),
    }
    let list: Value = resp.json().await.map_err(|_| "resposta ilegível.".to_string())?;
    Ok(igdb_pick(&list, term))
}

/// "Salvar e testar" in Configurações: one cheap request per site, with plain-language results.
pub async fn test(keys: &Keys) -> Vec<(bool, String)> {
    let client = crate::github::client();
    let mut out = Vec::new();
    if !keys.sgdb.is_empty() {
        out.push(match sgdb_get(&client, &keys.sgdb, "/search/autocomplete/zelda").await {
            Ok(_) => (true, "SteamGridDB: chave aceita.".to_string()),
            Err(e) => (false, format!("SteamGridDB: {e}")),
        });
    }
    if keys.igdb() {
        *TOKEN.lock().unwrap() = None;
        out.push(match igdb_token(&client, keys).await {
            Ok(_) => (true, "IGDB: Client ID e Client Secret aceitos.".to_string()),
            Err(e) => (false, format!("IGDB: {e}")),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_name_wins_over_first_result() {
        let search = json!({"success": true, "data": [
            {"id": 1, "name": "Banjo-Tooie"},
            {"id": 2, "name": "Banjo Kazooie"}
        ]});
        assert_eq!(sgdb_pick(&search, "Banjo-Kazooie"), Some((2, "Banjo Kazooie".to_string())));
        assert_eq!(sgdb_pick(&search, "Banjo").map(|g| g.0), Some(1), "sem nome igual, o primeiro");
        assert_eq!(sgdb_pick(&json!({"data": []}), "x"), None);
    }

    #[test]
    fn only_the_sites_own_image_servers() {
        assert!(allowed_image("https://cdn2.steamgriddb.com/grid/abc.png"));
        assert!(allowed_image("https://images.igdb.com/igdb/image/upload/t_1080p/x.jpg"));
        assert!(!allowed_image("http://cdn2.steamgriddb.com/grid/abc.png"));
        assert!(!allowed_image("https://steamgriddb.com.evil.net/a.png"));
        assert!(!allowed_image("https://example.com/?u=images.igdb.com"));
    }

    #[test]
    fn igdb_query_keeps_the_name_as_one_string() {
        let q = igdb_query("Majora's \"Mask\"\\; fields *");
        assert!(q.starts_with("search \"Majora's Mask; fields *\";"), "{q}");
    }

    #[test]
    fn igdb_result_gives_cover_banner_and_text() {
        let list = json!([{
            "name": "The Legend of Zelda: Majora's Mask",
            "summary": "Link must save Termina.",
            "first_release_date": 956275200,
            "url": "https://www.igdb.com/games/the-legend-of-zelda-majoras-mask",
            "genres": [{"name": "Adventure"}],
            "cover": {"image_id": "co3pah"},
            "screenshots": [{"image_id": "sc1abc"}],
            "artworks": [{"image_id": "../x"}]
        }]);
        let g = igdb_pick(&list, "Majora's Mask").unwrap();
        assert_eq!(g.cover.unwrap(), "https://images.igdb.com/igdb/image/upload/t_cover_big_2x/co3pah.jpg");
        assert_eq!(g.banner.unwrap(), "https://images.igdb.com/igdb/image/upload/t_1080p/sc1abc.jpg", "arte inválida é pulada");
        assert_eq!(g.genres, vec!["Adventure".to_string()]);
        assert!(g.url.is_some() && g.summary.is_some());
    }

    #[test]
    fn found_games_stay_cached_and_misses_retry_after_a_week() {
        let found = Meta { fetched_at: 100, name: Some("x".into()), ..Default::default() };
        let missed = Meta { fetched_at: 100, ..Default::default() };
        assert!(stale(None, 100));
        assert!(!stale(Some(&found), 100 + 365 * 86_400));
        assert!(!stale(Some(&missed), 100 + 86_400));
        assert!(stale(Some(&missed), 100 + RETRY_AFTER));
    }
}
