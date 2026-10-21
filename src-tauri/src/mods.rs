//! Central de Mods: mods for the Recompiled ports from Thunderstore, the community's own mod
//! site (the same public API r2modman uses). Only Thunderstore packages of the game's
//! community are listed and downloaded; authors and pages are always shown.
//!
//! How the ports load mods (N64ModernRuntime): every `.nrm` (and container types the game
//! registers, such as Majora's Mask's `.rtz` texture packs) in `<data>/mods` is scanned, and
//! `<data>/mods.json` lists the ids of the enabled ones (`enabled_mods`). A `.nrm` is a zip
//! whose `mod.json` holds that id and the `game_id` it is for. So RecompHUB installs a
//! package's mod files there, reads their ids, and turns mods on or off in `mods.json`, only
//! while the game is closed (the game rewrites that file).
//!
//! What RecompHUB installed is kept in `mods/instalados/<game>.json`; what each profile has
//! turned on in `mods/perfis/<profile>/<game>.json`; named sets in `mods/colecoes/<game>.json`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

const API: &str = "https://thunderstore.io/c/";
pub const LIST_MAX_AGE: u64 = 6 * 3600;
/// File types RecompHUB copies into the mods folder.
const MOD_EXTS: &[&str] = &["nrm", "rtz"];
/// Biggest mod file and biggest package taken from a download, so a broken or hostile .zip
/// cannot fill the disk. The largest texture packs on Thunderstore are well below this.
pub const MAX_MOD_FILE: u64 = 512 * 1024 * 1024;
pub const MAX_MOD_PACKAGE: u64 = 2 * 1024 * 1024 * 1024;

pub fn valid_community(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// "Owner-Name" with Thunderstore's characters only.
pub fn valid_package(s: &str) -> bool {
    let mut parts = s.splitn(2, '-');
    let (Some(a), Some(b)) = (parts.next(), parts.next()) else { return false };
    let ok = |x: &str| !x.is_empty() && x.len() <= 128 && x.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok(a) && ok(b)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Package {
    /// "Owner-Name".
    pub full_name: String,
    pub name: String,
    pub owner: String,
    pub description: String,
    pub icon: String,
    pub version: String,
    pub download_url: String,
    pub file_size: u64,
    pub downloads: u64,
    pub rating: u64,
    pub updated: String,
    pub page: String,
    pub website: String,
    pub categories: Vec<String>,
    /// "Owner-Name" of each dependency (Thunderstore gives "Owner-Name-1.2.3").
    pub dependencies: Vec<String>,
    pub deprecated: bool,
    pub nsfw: bool,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ListCache {
    pub fetched_at: u64,
    pub packages: Vec<Package>,
}

/// "Owner-Name-1.2.3" -> "Owner-Name".
pub fn strip_version(dep: &str) -> String {
    match dep.rsplit_once('-') {
        Some((head, v)) if v.chars().all(|c| c.is_ascii_digit() || c == '.') => head.to_string(),
        _ => dep.to_string(),
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// https on `base` itself or one of its subdomains (never "xbase").
fn https_on(url: &str, base: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|r| r.split(['/', '?', '#']).next())
        .map(|h| h == base || h.ends_with(&format!(".{base}")))
        .unwrap_or(false)
}

/// Thunderstore's package list, newest version of each, checked field by field.
pub fn parse_list(v: &Value) -> Vec<Package> {
    let Some(items) = v.as_array() else { return vec![] };
    items
        .iter()
        .filter_map(|p| {
            let full_name = s(&p["full_name"]);
            let latest = p["versions"].as_array()?.first()?;
            let download_url = s(&latest["download_url"]);
            if !valid_package(&full_name) || !https_on(&download_url, "thunderstore.io") {
                return None;
            }
            let icon = s(&latest["icon"]);
            Some(Package {
                name: s(&p["name"]),
                owner: s(&p["owner"]),
                description: s(&latest["description"]),
                icon: if https_on(&icon, "thunderstore.io") { icon } else { String::new() },
                version: s(&latest["version_number"]),
                file_size: latest["file_size"].as_u64().unwrap_or(0),
                downloads: p["versions"].as_array().map(|vs| vs.iter().filter_map(|x| x["downloads"].as_u64()).sum()).unwrap_or(0),
                rating: p["rating_score"].as_u64().unwrap_or(0),
                updated: s(&p["date_updated"]),
                page: s(&p["package_url"]),
                website: s(&latest["website_url"]),
                categories: p["categories"].as_array().map(|c| c.iter().map(s).collect()).unwrap_or_default(),
                dependencies: latest["dependencies"]
                    .as_array()
                    .map(|d| d.iter().map(|x| strip_version(&s(x))).filter(|x| valid_package(x)).collect())
                    .unwrap_or_default(),
                deprecated: p["is_deprecated"].as_bool().unwrap_or(false),
                nsfw: p["has_nsfw_content"].as_bool().unwrap_or(false),
                download_url,
                full_name,
            })
        })
        .collect()
}

pub fn load_list(dir: &Path, community: &str) -> Option<ListCache> {
    std::fs::read(dir.join(format!("{community}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// The community's packages, from the cache when it is under 6 hours old (or `force`).
pub async fn list(dir: &Path, community: &str, force: bool, now: u64) -> Result<ListCache, String> {
    if !valid_community(community) {
        return Err("Comunidade inválida.".into());
    }
    if let Some(c) = load_list(dir, community) {
        if !force && now < c.fetched_at + LIST_MAX_AGE {
            return Ok(c);
        }
    }
    let fetched = async {
        let resp = crate::github::client()
            .get(format!("{API}{community}/api/v1/package/"))
            .send()
            .await
            .map_err(|_| "sem conexão com a Thunderstore.".to_string())?;
        if !resp.status().is_success() {
            return Err(format!("a Thunderstore respondeu com erro {}.", resp.status().as_u16()));
        }
        resp.json::<Value>().await.map_err(|_| "resposta ilegível da Thunderstore.".to_string())
    }
    .await;
    match fetched {
        Ok(v) => {
            let cache = ListCache { fetched_at: now, packages: parse_list(&v) };
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let _ = std::fs::write(dir.join(format!("{community}.json")), serde_json::to_vec(&cache).unwrap_or_default());
            Ok(cache)
        }
        // Offline: the old list is better than nothing.
        Err(e) => load_list(dir, community).ok_or(e),
    }
}

/// `wanted` and every dependency under it, dependencies first. Unknown names are returned
/// in the second list.
pub fn resolve(packages: &[Package], wanted: &str) -> (Vec<Package>, Vec<String>) {
    let mut order = Vec::new();
    let mut missing = Vec::new();
    let mut seen = BTreeSet::new();
    fn visit(name: &str, packages: &[Package], seen: &mut BTreeSet<String>, order: &mut Vec<Package>, missing: &mut Vec<String>) {
        if !seen.insert(name.to_string()) {
            return;
        }
        match packages.iter().find(|p| p.full_name == name) {
            Some(p) => {
                for d in &p.dependencies {
                    visit(d, packages, seen, order, missing);
                }
                order.push(p.clone());
            }
            None => missing.push(name.to_string()),
        }
    }
    visit(wanted, packages, &mut seen, &mut order, &mut missing);
    (order, missing)
}

// ---- What is installed ----

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Installed {
    pub version: String,
    /// File names inside the mods folder.
    pub files: Vec<String>,
    /// Ids from each .nrm's mod.json, as mods.json lists them.
    pub mod_ids: Vec<String>,
    pub installed_at: u64,
    /// Installed only because another package needs it.
    #[serde(default)]
    pub as_dependency: bool,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct InstalledSet {
    #[serde(default)]
    pub packages: BTreeMap<String, Installed>,
}

impl InstalledSet {
    pub fn load(dir: &Path, game: &str) -> Self {
        std::fs::read(dir.join("instalados").join(format!("{game}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path, game: &str) -> Result<(), String> {
        write_json(&dir.join("instalados").join(format!("{game}.json")), self)
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// The id and game of a .nrm, from its mod.json.
pub fn nrm_manifest(bytes: &[u8]) -> Option<(String, String)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut text = String::new();
    z.by_name("mod.json").ok()?.read_to_string(&mut text).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some((v["id"].as_str()?.to_string(), v["game_id"].as_str().unwrap_or("").to_string()))
}

/// Copies the mod files of a Thunderstore package zip into `staging` (streamed: texture
/// packs can be several GB) and returns their names. Folders inside the zip are flattened;
/// names are checked so nothing lands outside the folder; manifest, icon and README stay out.
pub fn extract<R: std::io::Read + std::io::Seek>(reader: R, staging: &Path) -> Result<Vec<String>, String> {
    extract_limited(reader, staging, MAX_MOD_FILE, MAX_MOD_PACKAGE)
}

/// As `extract`, with the limits as arguments. A package over a limit is refused and what was
/// already copied is removed: nothing half-written stays behind looking like a whole mod.
pub fn extract_limited<R: std::io::Read + std::io::Seek>(reader: R, staging: &Path, per_file: u64, total: u64) -> Result<Vec<String>, String> {
    let result = extract_into(reader, staging, per_file, total);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

fn extract_into<R: std::io::Read + std::io::Seek>(reader: R, staging: &Path, per_file: u64, total: u64) -> Result<Vec<String>, String> {
    let mb = |n: u64| n / (1024 * 1024);
    let mut used: u64 = 0;
    let mut z = zip::ZipArchive::new(reader).map_err(|_| "O pacote baixado não é um .zip válido.".to_string())?;
    std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
    let mut out: Vec<String> = Vec::new();
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        if e.is_dir() {
            continue;
        }
        let Some(path) = e.enclosed_name() else { continue };
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
        let ext = Path::new(&name).extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !MOD_EXTS.contains(&ext.as_str()) {
            continue;
        }
        if out.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            return Err(format!("O pacote tem dois arquivos chamados {name}."));
        }
        // The size in the .zip header can lie, so the bytes actually copied are counted. One
        // byte more than the limit is read to tell "exactly the limit" from "over it".
        let room = per_file.min(total - used);
        let mut file = std::fs::File::create(staging.join(&name)).map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut (&mut e).take(room + 1), &mut file).map_err(|err| format!("O pacote está danificado: {err}"))?;
        if copied > room {
            return Err(if room == per_file {
                format!("{name} passa de {} MB, o limite do RecompHUB para um arquivo de mod. O pacote não foi instalado.", mb(per_file))
            } else {
                format!("O pacote passa de {} MB no total, o limite do RecompHUB. Ele não foi instalado.", mb(total))
            });
        }
        used += copied;
        out.push(name);
    }
    Ok(out)
}

/// The id and game of a .nrm file on disk.
pub fn nrm_manifest_file(path: &Path) -> Option<(String, String)> {
    let file = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(file).ok()?;
    let mut text = String::new();
    z.by_name("mod.json").ok()?.read_to_string(&mut text).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some((v["id"].as_str()?.to_string(), v["game_id"].as_str().unwrap_or("").to_string()))
}

// ---- mods.json (the port's own list of enabled mods) ----

pub fn enabled_in(mods_json: &Path) -> BTreeSet<String> {
    std::fs::read(mods_json)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["enabled_mods"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()))
        .unwrap_or_default()
}

/// Turns ids on and off in mods.json, keeping everything else the port wrote there.
pub fn set_enabled(mods_json: &Path, on: &[String], off: &[String]) -> Result<(), String> {
    let mut v: Value = std::fs::read(mods_json).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({}));
    if !v.is_object() {
        v = serde_json::json!({});
    }
    let mut set: Vec<String> = v["enabled_mods"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    set.retain(|x| !off.contains(x));
    for id in on {
        if !set.contains(id) {
            set.push(id.clone());
        }
    }
    v["enabled_mods"] = serde_json::json!(set);
    write_json(mods_json, &v)
}

// ---- Per profile and collections ----

/// Packages a profile has turned on for a game (None: never chosen, the port's state stays).
pub fn profile_set(dir: &Path, profile: &str, game: &str) -> Option<BTreeSet<String>> {
    std::fs::read(dir.join("perfis").join(profile).join(format!("{game}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn save_profile_set(dir: &Path, profile: &str, game: &str, set: &BTreeSet<String>) -> Result<(), String> {
    write_json(&dir.join("perfis").join(profile).join(format!("{game}.json")), set)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Collection {
    pub name: String,
    pub packages: Vec<String>,
}

pub fn collections(dir: &Path, game: &str) -> Vec<Collection> {
    std::fs::read(dir.join("colecoes").join(format!("{game}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_collections(dir: &Path, game: &str, list: &[Collection]) -> Result<(), String> {
    write_json(&dir.join("colecoes").join(format!("{game}.json")), &list)
}

/// The packages a set needs on: the chosen ones and everything they depend on.
pub fn with_dependencies(installed: &InstalledSet, chosen: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack: Vec<String> = chosen.iter().cloned().collect();
    while let Some(p) = stack.pop() {
        if let Some(i) = installed.packages.get(&p) {
            if out.insert(p.clone()) {
                stack.extend(i.dependencies.iter().cloned());
            }
        }
    }
    out
}

/// Dependencies nobody needs any more (installed only as a dependency).
pub fn orphans(installed: &InstalledSet) -> Vec<String> {
    let needed: BTreeSet<String> = with_dependencies(
        installed,
        &installed.packages.iter().filter(|(_, i)| !i.as_dependency).map(|(k, _)| k.clone()).collect(),
    );
    installed.packages.keys().filter(|k| !needed.contains(*k)).cloned().collect()
}

pub fn mods_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("mods")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn pkg(name: &str, deps: &[&str]) -> Package {
        Package {
            full_name: name.into(),
            name: name.into(),
            owner: "x".into(),
            description: String::new(),
            icon: String::new(),
            version: "1.0.0".into(),
            download_url: "https://thunderstore.io/package/download/x/y/1.0.0/".into(),
            file_size: 1,
            downloads: 0,
            rating: 0,
            updated: String::new(),
            page: String::new(),
            website: String::new(),
            categories: vec![],
            dependencies: deps.iter().map(|d| d.to_string()).collect(),
            deprecated: false,
            nsfw: false,
        }
    }

    #[test]
    fn reads_thunderstore_list() {
        let v = json!([{
            "name": "Disable_Draw_Distance", "full_name": "tomtee-Disable_Draw_Distance", "owner": "tomtee",
            "package_url": "https://thunderstore.io/c/zelda-64-recompiled/p/tomtee/Disable_Draw_Distance/",
            "rating_score": 46, "is_deprecated": false, "has_nsfw_content": false, "categories": ["Mods"],
            "versions": [{"description": "d", "icon": "https://ccdn.thunderstore.io/x.png", "version_number": "1.3.0",
                          "dependencies": ["ProxyMM-ObjDepLoader-0.0.3"], "downloads": 10,
                          "download_url": "https://thunderstore.io/package/download/tomtee/Disable_Draw_Distance/1.3.0/", "file_size": 416617}]
        }, {
            "name": "Bad", "full_name": "evil-Bad", "versions": [{"download_url": "https://evil.example/x.zip"}]
        }]);
        let list = parse_list(&v);
        assert_eq!(list.len(), 1, "endereço fora da Thunderstore é recusado");
        assert_eq!(list[0].dependencies, vec!["ProxyMM-ObjDepLoader".to_string()]);
        assert_eq!(list[0].version, "1.3.0");
    }

    #[test]
    fn dependencies_come_first_and_cycles_stop() {
        let list = vec![pkg("a-A", &["b-B", "c-C"]), pkg("b-B", &["c-C"]), pkg("c-C", &["a-A"])];
        let (order, missing) = resolve(&list, "a-A");
        assert_eq!(order.iter().map(|p| p.full_name.as_str()).collect::<Vec<_>>(), vec!["c-C", "b-B", "a-A"]);
        assert!(missing.is_empty());
        let (_, missing) = resolve(&[pkg("a-A", &["z-Z"])], "a-A");
        assert_eq!(missing, vec!["z-Z".to_string()]);
    }

    #[test]
    fn package_names_and_versions() {
        assert_eq!(strip_version("ProxyMM-ObjDepLoader-0.0.3"), "ProxyMM-ObjDepLoader");
        assert!(valid_package("LT_Schmiddy-EZ_Text_Replacer_API"));
        assert!(!valid_package("../x"));
        assert!(!valid_package("semdono"));
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, b) in files {
            w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn only_mod_files_are_taken_and_ids_are_read() {
        let nrm = zip_of(&[("mod.json", br#"{"id": "mm_recomp_draw_distance", "game_id": "mm"}"#)]);
        let pkg = zip_of(&[("manifest.json", b"{}"), ("icon.png", b"x"), ("sub/mm_recomp_draw_distance.nrm", &nrm), ("../evil.nrm", b"x")]);
        let staging = std::env::temp_dir().join(format!("recomphub-modx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        let files = extract(std::io::Cursor::new(pkg), &staging).unwrap();
        assert_eq!(files, vec!["mm_recomp_draw_distance.nrm".to_string()]);
        assert_eq!(nrm_manifest(&nrm), Some(("mm_recomp_draw_distance".into(), "mm".into())));
        assert_eq!(nrm_manifest_file(&staging.join(&files[0])), Some(("mm_recomp_draw_distance".into(), "mm".into())));
        let _ = std::fs::remove_dir_all(&staging);
    }

    #[test]
    fn oversized_packages_are_refused_and_cleaned() {
        let pkg = zip_of(&[("a.nrm", &[1u8; 100][..]), ("b.nrm", &[2u8; 100][..])]);
        let staging = std::env::temp_dir().join(format!("recomphub-modlimit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        // Exactly at the limits: accepted.
        assert_eq!(extract_limited(std::io::Cursor::new(pkg.clone()), &staging, 100, 200).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&staging);
        // One file over the per-file limit.
        assert!(extract_limited(std::io::Cursor::new(pkg.clone()), &staging, 99, 1000).unwrap_err().contains("a.nrm"));
        assert!(!staging.exists());
        // Second file pushes the package over the total.
        assert!(extract_limited(std::io::Cursor::new(pkg), &staging, 100, 150).unwrap_err().contains("no total"));
        assert!(!staging.exists());
    }

    #[test]
    fn mods_json_keeps_what_the_port_wrote() {
        let dir = std::env::temp_dir().join(format!("recomphub-modsjson-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mods.json");
        std::fs::write(&path, r#"{"enabled_mods": ["a", "b"], "mod_order": ["a", "b"], "latest_game_mode": "mm"}"#).unwrap();
        set_enabled(&path, &["c".into()], &["a".into()]).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["enabled_mods"], json!(["b", "c"]));
        assert_eq!(v["latest_game_mode"], "mm");
        assert_eq!(enabled_in(&path).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unused_dependencies_are_orphans() {
        let mut set = InstalledSet::default();
        set.packages.insert("a-A".into(), Installed { dependencies: vec!["b-B".into()], ..Default::default() });
        set.packages.insert("b-B".into(), Installed { as_dependency: true, ..Default::default() });
        set.packages.insert("c-C".into(), Installed { as_dependency: true, ..Default::default() });
        assert_eq!(orphans(&set), vec!["c-C".to_string()]);
    }
}
