//! Cover art, in the style of PCSX2's cover downloader (and the xlenore/ps2-covers
//! collection it reads): each game's picture is fetched once from an address built from a
//! template such as `https://…/${serial}.jpg`, then kept in RecompHUB's data folder.
//!
//! Where a cover comes from, first match wins:
//! 1. a picture the player chose;
//! 2. the player's own game files (the PS4 dump's icon0.png) or the port's folder;
//! 3. a downloaded cover;
//! 4. a cover from SteamGridDB or IGDB (meta.rs), with the player's keys;
//! 5. for PC games, the program's own icon (shown small, not as a cover).

use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub(crate) const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// RecompHUB's own settings (settings.json).
#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    /// The player's own cover addresses, one template per line, with ${serial} and ${title}.
    #[serde(default)]
    pub cover_templates: Vec<String>,
    /// Closing the window keeps RecompHUB in the tray, timing games (as Ports Launcher).
    #[serde(default = "yes")]
    pub close_to_tray: bool,
    /// Library shows hidden games too.
    #[serde(default)]
    pub show_hidden: bool,
    /// The player's own SteamGridDB API key (covers and banners).
    #[serde(default)]
    pub steamgriddb_key: String,
    /// The player's own Twitch application for IGDB (description, cover, artwork).
    #[serde(default)]
    pub igdb_client_id: String,
    #[serde(default)]
    pub igdb_client_secret: String,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cover_templates: Vec::new(),
            close_to_tray: true,
            show_hidden: false,
            steamgriddb_key: String::new(),
            igdb_client_id: String::new(),
            igdb_client_secret: String::new(),
        }
    }
}

impl Settings {
    pub fn keys(&self) -> crate::meta::Keys {
        crate::meta::Keys {
            sgdb: self.steamgriddb_key.clone(),
            igdb_id: self.igdb_client_id.clone(),
            igdb_secret: self.igdb_client_secret.clone(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

/// Keeps only https addresses that name the game somehow.
pub fn clean_templates(lines: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if !line.starts_with("https://") {
            return Err(format!("O endereço precisa começar com https://: {line}"));
        }
        if !line.contains("${serial}") && !line.contains("${title}") {
            return Err(format!("O endereço precisa ter ${{serial}} ou ${{title}}: {line}"));
        }
        if !out.iter().any(|o: &String| o == line) {
            out.push(line.to_string());
        }
    }
    out.truncate(10);
    Ok(out)
}

/// Percent-encodes everything except unreserved URL characters.
pub(crate) fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn fill(template: &str, serial: Option<&str>, title: &str) -> Option<String> {
    if template.contains("${serial}") && serial.is_none() {
        return None;
    }
    Some(template.replace("${serial}", &encode(serial.unwrap_or(""))).replace("${title}", &encode(title)))
}

pub struct Dirs {
    pub custom: PathBuf,
    pub downloaded: PathBuf,
    pub icons: PathBuf,
}

impl Dirs {
    pub fn new(data_dir: &Path) -> Self {
        let base = data_dir.join("covers");
        Self { custom: base.join("custom"), downloaded: base.join("downloaded"), icons: base.join("icons") }
    }
}

/// `<dir>/<id>.<png|jpg|jpeg|webp>`, whichever exists.
pub fn find(dir: &Path, id: &str) -> Option<PathBuf> {
    IMAGE_EXTS.iter().map(|e| dir.join(format!("{id}.{e}"))).find(|p| p.is_file())
}

fn remove_all(dir: &Path, id: &str) {
    for e in IMAGE_EXTS {
        let _ = std::fs::remove_file(dir.join(format!("{id}.{e}")));
    }
}

/// The image format from its first bytes, so a web page or error text is never kept as a cover.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

pub(crate) fn store(dir: &Path, id: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let ext = sniff(bytes).ok_or("O arquivo não é uma imagem PNG, JPG ou WEBP.")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    remove_all(dir, id);
    let path = dir.join(format!("{id}.{ext}"));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// "Escolher capa": copies the player's picture into RecompHUB's folder.
pub fn set_custom(dirs: &Dirs, id: &str, picture: &Path) -> Result<(), String> {
    let size = std::fs::metadata(picture).map(|m| m.len()).map_err(|e| e.to_string())?;
    if size > MAX_IMAGE_BYTES {
        return Err("A imagem passa de 10 MB. Escolha uma menor.".into());
    }
    let bytes = std::fs::read(picture).map_err(|e| e.to_string())?;
    store(&dirs.custom, id, &bytes).map(|_| ())
}

/// "Restaurar capa": forgets the chosen and the downloaded picture.
pub fn clear(dirs: &Dirs, id: &str) {
    remove_all(&dirs.custom, id);
    remove_all(&dirs.downloaded, id);
}

/// Tries each address in order and keeps the first real image.
pub async fn download(dirs: &Dirs, id: &str, urls: &[String]) -> bool {
    let client = crate::github::client();
    for url in urls {
        let Ok(resp) = client.get(url).send().await else { continue };
        if !resp.status().is_success() || resp.content_length().unwrap_or(0) > MAX_IMAGE_BYTES {
            continue;
        }
        let Ok(bytes) = resp.bytes().await else { continue };
        if bytes.len() as u64 <= MAX_IMAGE_BYTES && store(&dirs.downloaded, id, &bytes).is_ok() {
            return true;
        }
    }
    false
}

/// Saves the icon Windows shows for `exe` as a PNG.
pub fn extract_icon(dirs: &Dirs, id: &str, exe: &Path) -> Result<(), String> {
    std::fs::create_dir_all(&dirs.icons).map_err(|e| e.to_string())?;
    let out = dirs.icons.join(format!("{id}.png"));
    const SCRIPT: &str = "Add-Type -AssemblyName System.Drawing; \
$i = [System.Drawing.Icon]::ExtractAssociatedIcon($env:RH_EXE); \
$i.ToBitmap().Save($env:RH_OUT, [System.Drawing.Imaging.ImageFormat]::Png)";
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("RH_EXE", exe)
        .env("RH_OUT", &out)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() && out.is_file() {
        Ok(())
    } else {
        Err("Não consegui ler o ícone do programa.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_fill_serial_and_title() {
        let t = "https://x.org/${serial}/${title}.png";
        assert_eq!(fill(t, Some("CUSA01715"), "God of War").unwrap(), "https://x.org/CUSA01715/God%20of%20War.png");
        assert_eq!(fill(t, None, "x"), None, "sem serial não monta o endereço");
        assert_eq!(fill("https://x.org/${title}.jpg", None, "Majora's").unwrap(), "https://x.org/Majora%27s.jpg");
    }

    #[test]
    fn only_https_templates_that_name_the_game() {
        assert!(clean_templates(&["http://x/${serial}".into()]).is_err());
        assert!(clean_templates(&["https://x/capa.png".into()]).is_err());
        let ok = clean_templates(&["  https://x/${serial}.jpg ".into(), "".into(), "https://x/${serial}.jpg".into()]).unwrap();
        assert_eq!(ok, vec!["https://x/${serial}.jpg".to_string()]);
    }

    #[test]
    fn images_are_recognised_by_their_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(sniff(b"\xFF\xD8\xFF\xE0...."), Some("jpg"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff(b"<!DOCTYPE html>"), None);
    }
}
