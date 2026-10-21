//! "Adicionar à Steam": the game becomes a non-Steam shortcut that opens through RecompHUB
//! ("RecompHUB.exe --play <id>"), so Big Picture and the Steam controller settings work and the
//! session is still timed. As GithubLauncher's ShortcutHelper, it edits the account's
//! userdata/<id>/config/shortcuts.vdf, which Steam only reads at start and rewrites on exit, so
//! Steam must be closed.

use std::path::{Path, PathBuf};

// ---- Binary VDF (the format of shortcuts.vdf) ----

/// A value in a binary VDF file. Strings stay as bytes so names Steam wrote in another encoding
/// are written back unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Obj(Vec<(Vec<u8>, Val)>),
    Str(Vec<u8>),
    Int(u32),
    Float(u32),
    U64(u64),
}

const T_OBJ: u8 = 0x00;
const T_STR: u8 = 0x01;
const T_INT: u8 = 0x02;
const T_FLOAT: u8 = 0x03;
const T_U64: u8 = 0x07;
const T_END: u8 = 0x08;

fn cstr(b: &[u8], pos: &mut usize) -> Result<Vec<u8>, String> {
    let rest = b.get(*pos..).ok_or("fim inesperado")?;
    let end = rest.iter().position(|&c| c == 0).ok_or("texto sem fim")?;
    *pos += end + 1;
    Ok(rest[..end].to_vec())
}

fn bytes<const N: usize>(b: &[u8], pos: &mut usize) -> Result<[u8; N], String> {
    let end = pos.checked_add(N).ok_or("fim inesperado")?;
    let s = b.get(*pos..end).ok_or("fim inesperado")?;
    *pos += N;
    Ok(s.try_into().unwrap())
}

fn parse_obj(b: &[u8], pos: &mut usize, depth: u32) -> Result<Vec<(Vec<u8>, Val)>, String> {
    if depth > 16 {
        return Err("aninhamento demais".into());
    }
    let mut items = Vec::new();
    loop {
        let t = *b.get(*pos).ok_or("fim inesperado")?;
        *pos += 1;
        if t == T_END {
            return Ok(items);
        }
        let key = cstr(b, pos)?;
        let v = match t {
            T_OBJ => Val::Obj(parse_obj(b, pos, depth + 1)?),
            T_STR => Val::Str(cstr(b, pos)?),
            T_INT => Val::Int(u32::from_le_bytes(bytes(b, pos)?)),
            T_FLOAT => Val::Float(u32::from_le_bytes(bytes(b, pos)?)),
            T_U64 => Val::U64(u64::from_le_bytes(bytes(b, pos)?)),
            other => return Err(format!("tipo desconhecido {other:#04x}")),
        };
        items.push((key, v));
    }
}

/// The whole file. Anything not understood is an error: the file is then left alone.
pub fn parse(b: &[u8]) -> Result<Vec<(Vec<u8>, Val)>, String> {
    let mut pos = 0;
    let items = parse_obj(b, &mut pos, 0)?;
    // Bytes after the end would be lost on writing, so such a file is not touched.
    if pos != b.len() {
        return Err("dados depois do fim".into());
    }
    Ok(items)
}

fn write_obj(items: &[(Vec<u8>, Val)], out: &mut Vec<u8>) {
    for (k, v) in items {
        let t = match v {
            Val::Obj(_) => T_OBJ,
            Val::Str(_) => T_STR,
            Val::Int(_) => T_INT,
            Val::Float(_) => T_FLOAT,
            Val::U64(_) => T_U64,
        };
        out.push(t);
        out.extend_from_slice(k);
        out.push(0);
        match v {
            Val::Obj(o) => write_obj(o, out),
            Val::Str(s) => {
                out.extend_from_slice(s);
                out.push(0);
            }
            Val::Int(n) | Val::Float(n) => out.extend_from_slice(&n.to_le_bytes()),
            Val::U64(n) => out.extend_from_slice(&n.to_le_bytes()),
        }
    }
    out.push(T_END);
}

pub fn write(items: &[(Vec<u8>, Val)]) -> Vec<u8> {
    let mut out = Vec::new();
    write_obj(items, &mut out);
    out
}

// ---- The shortcut ----

pub struct Shortcut {
    pub name: String,
    /// RecompHUB.exe, written in quotes as Steam does.
    pub exe: PathBuf,
    pub icon: PathBuf,
    pub launch_options: String,
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// The id Steam itself gives a non-Steam shortcut, so artwork files named after it are used.
pub fn app_id(exe_field: &str, name: &str) -> u32 {
    crc32(format!("{exe_field}{name}").as_bytes()) | 0x8000_0000
}

fn quoted(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

fn key_is(k: &[u8], name: &str) -> bool {
    k.eq_ignore_ascii_case(name.as_bytes())
}

fn get<'a>(o: &'a [(Vec<u8>, Val)], name: &str) -> Option<&'a Val> {
    o.iter().find(|(k, _)| key_is(k, name)).map(|(_, v)| v)
}

fn set(o: &mut Vec<(Vec<u8>, Val)>, name: &str, v: Val) {
    match o.iter_mut().find(|(k, _)| key_is(k, name)) {
        Some(slot) => slot.1 = v,
        None => o.push((name.as_bytes().to_vec(), v)),
    }
}

fn str_val(s: &str) -> Val {
    Val::Str(s.as_bytes().to_vec())
}

/// Adds the shortcut, or updates RecompHUB's earlier one for the same game (same launch
/// options and program) instead of making a copy. Returns the shortcut's app id and whether it
/// was already there. Other shortcuts are kept exactly as they were; a file whose
/// "shortcuts" is not a list is refused rather than replaced.
pub fn upsert(root: &mut Vec<(Vec<u8>, Val)>, sc: &Shortcut) -> Result<(u32, bool), String> {
    match get(root, "shortcuts") {
        Some(Val::Obj(_)) => {}
        Some(_) => return Err("a lista de atalhos tem um formato que eu não conheço".into()),
        None => root.push((b"shortcuts".to_vec(), Val::Obj(Vec::new()))),
    }
    let Some(Val::Obj(list)) = root.iter_mut().find(|(k, _)| key_is(k, "shortcuts")).map(|(_, v)| v) else {
        unreachable!()
    };
    let exe_field = quoted(&sc.exe);
    let ours = |e: &[(Vec<u8>, Val)]| {
        let s = |n: &str| match get(e, n) {
            Some(Val::Str(b)) => String::from_utf8_lossy(b).to_string(),
            _ => String::new(),
        };
        s("LaunchOptions") == sc.launch_options && s("Exe").eq_ignore_ascii_case(&exe_field)
    };
    let id = app_id(&exe_field, &sc.name);
    let start_dir = sc.exe.parent().map(quoted).unwrap_or_default();
    let found = list.iter().position(|(_, v)| matches!(v, Val::Obj(e) if ours(e)));
    let existed = found.is_some();
    let pos = match found {
        Some(p) => p,
        None => {
            let index = list.len().to_string().into_bytes();
            let e = vec![
                (b"appid".to_vec(), Val::Int(id)),
                (b"AppName".to_vec(), str_val("")),
                (b"Exe".to_vec(), str_val("")),
                (b"StartDir".to_vec(), str_val("")),
                (b"icon".to_vec(), str_val("")),
                (b"ShortcutPath".to_vec(), str_val("")),
                (b"LaunchOptions".to_vec(), str_val("")),
                (b"IsHidden".to_vec(), Val::Int(0)),
                (b"AllowDesktopConfig".to_vec(), Val::Int(1)),
                (b"AllowOverlay".to_vec(), Val::Int(1)),
                (b"OpenVR".to_vec(), Val::Int(0)),
                (b"Devkit".to_vec(), Val::Int(0)),
                (b"DevkitGameID".to_vec(), str_val("")),
                (b"DevkitOverrideAppID".to_vec(), Val::Int(0)),
                (b"LastPlayTime".to_vec(), Val::Int(0)),
                (b"FlatpakAppID".to_vec(), str_val("")),
                (b"tags".to_vec(), Val::Obj(Vec::new())),
            ];
            list.push((index, Val::Obj(e)));
            list.len() - 1
        }
    };
    let Val::Obj(entry) = &mut list[pos].1 else { unreachable!() };
    // An existing shortcut keeps its id, so Steam keeps its play time and artwork.
    let id = match get(entry, "appid") {
        Some(Val::Int(n)) if existed => *n,
        _ => id,
    };
    set(entry, "appid", Val::Int(id));
    set(entry, "AppName", str_val(&sc.name));
    set(entry, "Exe", str_val(&exe_field));
    set(entry, "StartDir", str_val(&start_dir));
    set(entry, "icon", str_val(&sc.icon.display().to_string()));
    set(entry, "LaunchOptions", str_val(&sc.launch_options));
    Ok((id, existed))
}

// ---- Where Steam is ----

fn registry_steam_path() -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, value) = (wide("Software\\Valve\\Steam"), wide("SteamPath"));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its size in bytes are passed together; the strings end in 0.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if r != 0 {
        return None;
    }
    let len = (size as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..len]);
    let p = PathBuf::from(s.trim_end_matches('\0').replace('/', "\\"));
    p.is_dir().then_some(p)
}

pub fn steam_root() -> Option<PathBuf> {
    registry_steam_path().or_else(|| {
        ["ProgramFiles(x86)", "ProgramFiles"]
            .iter()
            .filter_map(|v| std::env::var_os(v))
            .map(|d| PathBuf::from(d).join("Steam"))
            .find(|p| p.join("userdata").is_dir())
    })
}

/// The config folder of the account that used Steam last on this PC.
pub fn account_config(root: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in std::fs::read_dir(root.join("userdata")).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == "0" || !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let config = e.path().join("config");
        let Ok(meta) = std::fs::metadata(config.join("localconfig.vdf")).or_else(|_| std::fs::metadata(&config)) else {
            continue;
        };
        let when = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map(|(t, _)| when > *t).unwrap_or(true) {
            best = Some((when, config));
        }
    }
    best.map(|(_, p)| p)
}

pub fn steam_running() -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
    sys.processes().values().any(|p| p.name().to_string_lossy().eq_ignore_ascii_case("steam.exe"))
}

pub struct Added {
    pub app_id: u32,
    pub updated: bool,
    pub art: bool,
}

/// Adds or updates the shortcut in the last-used account. `cover` and `banner` become the
/// shortcut's artwork in Big Picture when they are PNG or JPEG.
pub fn add(sc: &Shortcut, cover: Option<&Path>, banner: Option<&Path>) -> Result<Added, String> {
    if steam_running() {
        return Err("Feche a Steam primeiro (menu Steam > Sair, não só a janela) e tente de novo. A Steam regrava a lista de jogos ao fechar e apagaria o atalho novo.".into());
    }
    let root = steam_root().ok_or("Não achei a Steam neste PC.")?;
    let config = account_config(&root).ok_or("Não achei sua conta na Steam. Abra a Steam e entre na sua conta uma vez, depois feche e tente de novo.")?;
    let path = config.join("shortcuts.vdf");
    let mut items = match std::fs::read(&path) {
        Ok(b) => parse(&b).map_err(|e| format!("Não entendi o arquivo de atalhos da Steam ({e}), então não mexi nele."))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("Não consegui ler o arquivo de atalhos da Steam: {e}")),
    };
    let (app_id, updated) = upsert(&mut items, sc).map_err(|e| format!("Não entendi o arquivo de atalhos da Steam ({e}), então não mexi nele."))?;
    // The player's list as it was before RecompHUB ever touched it.
    let backup = config.join("shortcuts.vdf.recomphub-backup");
    if path.is_file() && !backup.exists() {
        std::fs::copy(&path, &backup).map_err(|e| format!("Não consegui guardar uma cópia dos atalhos da Steam: {e}"))?;
    }
    let tmp = config.join("shortcuts.vdf.recomphub-tmp");
    // Written beside it and then swapped in, so a failure never leaves a half-written list.
    if let Err(e) = std::fs::write(&tmp, write(&items)).and_then(|_| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Não consegui gravar os atalhos da Steam: {e}"));
    }

    // Artwork: <id>p is the cover (portrait), <id>_hero the banner.
    let grid = config.join("grid");
    let mut art = false;
    for (src, suffix) in [(cover, "p"), (banner, "_hero")] {
        let Some(src) = src else { continue };
        let Some(ext) = src.extension().map(|e| e.to_string_lossy().to_lowercase()).filter(|e| ["png", "jpg", "jpeg"].contains(&e.as_str())) else {
            continue;
        };
        if std::fs::create_dir_all(&grid).is_err() {
            break;
        }
        for old in ["png", "jpg", "jpeg"] {
            let _ = std::fs::remove_file(grid.join(format!("{app_id}{suffix}.{old}")));
        }
        art |= std::fs::copy(src, grid.join(format!("{app_id}{suffix}.{ext}"))).is_ok();
    }
    Ok(Added { app_id, updated, art })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(name: &str, id: &str) -> Shortcut {
        Shortcut {
            name: name.into(),
            exe: PathBuf::from(r"C:\Programas\RecompHUB\RecompHUB.exe"),
            icon: PathBuf::from(r"C:\Jogos\banjo\BanjoRecompiled.exe"),
            launch_options: format!("--play {id}"),
        }
    }

    #[test]
    fn crc_matches_the_standard() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert!(app_id("\"x.exe\"", "Jogo") & 0x8000_0000 != 0);
    }

    #[test]
    fn other_shortcuts_survive_byte_for_byte() {
        // A shortcut written by Steam, with a tag, a 64-bit field and a name that is not UTF-8.
        let mut original = Vec::new();
        original.extend_from_slice(b"\x00shortcuts\x00\x000\x00");
        original.extend_from_slice(b"\x02appid\x00\x11\x22\x33\xA4");
        original.extend_from_slice(b"\x01AppName\x00Jogo \xE9\x00");
        original.extend_from_slice(b"\x01Exe\x00\"C:\\outro.exe\"\x00");
        original.extend_from_slice(b"\x07Tempo\x00\x01\x02\x03\x04\x05\x06\x07\x08");
        original.extend_from_slice(b"\x00tags\x00\x010\x00favorite\x00\x08");
        original.extend_from_slice(b"\x08\x08\x08");
        let parsed = parse(&original).unwrap();
        assert_eq!(write(&parsed), original);

        let mut items = parsed;
        let (id, existed) = upsert(&mut items, &sc("Banjo-Kazooie", "banjo")).unwrap();
        assert!(!existed);
        let bytes = write(&items);
        // The old shortcut is still there, unchanged, and the new one comes after it.
        assert!(bytes.starts_with(&original[..original.len() - 2]));
        let Some(Val::Obj(list)) = get(&parse(&bytes).unwrap(), "shortcuts").cloned() else { panic!() };
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].0, b"1".to_vec());
        let Val::Obj(e) = &list[1].1 else { panic!() };
        assert_eq!(get(e, "appid"), Some(&Val::Int(id)));
        assert_eq!(get(e, "LaunchOptions"), Some(&str_val("--play banjo")));
        assert_eq!(get(e, "Exe"), Some(&str_val(r#""C:\Programas\RecompHUB\RecompHUB.exe""#)));
        assert_eq!(get(e, "StartDir"), Some(&str_val(r#""C:\Programas\RecompHUB""#)));
    }

    #[test]
    fn adding_again_updates_instead_of_copying() {
        let mut items = Vec::new();
        let (id, existed) = upsert(&mut items, &sc("Banjo", "banjo")).unwrap();
        assert!(!existed);
        // Renamed in RecompHUB: same shortcut, same id (Steam keeps its play time).
        let (id2, existed2) = upsert(&mut items, &sc("Banjo-Kazooie", "banjo")).unwrap();
        assert!(existed2);
        assert_eq!(id, id2);
        upsert(&mut items, &sc("Majora's Mask", "mm")).unwrap();
        let Some(Val::Obj(list)) = get(&items, "shortcuts") else { panic!() };
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn broken_files_are_refused() {
        assert!(parse(b"\x00shortcuts\x00\x09x\x00").is_err());
        assert!(parse(b"\x00shortcuts\x00\x01AppName\x00sem fim").is_err());
        assert!(parse(b"").is_err());
        assert!(parse(b"\x00shortcuts\x00\x08\x08lixo").is_err());
        let mut odd = parse(b"\x01shortcuts\x00texto\x00\x08").unwrap();
        assert!(upsert(&mut odd, &sc("Banjo", "banjo")).is_err());
    }
}
