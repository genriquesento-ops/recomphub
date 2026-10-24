//! "Antes de jogar": what can stop a game from opening, found before the player clicks Jogar,
//! each with a way out. Only reads; fixing is always a separate click.
//!
//! - DLLs the game's program imports (also through its own DLLs) that this PC lacks: the
//!   Visual C++ runtime and the old DirectX are installed with one click, from Microsoft only;
//! - files that live only in OneDrive's cloud (the game stalls while Windows downloads them);
//! - paths longer than Windows' old 260-character limit;
//! - a folder the game cannot write to, for games that keep saves next to the program.

use std::collections::BTreeSet;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::pcinfo::Finding;

// ---- Which DLLs a program imports (its PE import tables) ----

#[derive(Debug, PartialEq)]
pub struct Pe {
    /// A 32-bit program (its DLLs come from SysWOW64).
    pub x86: bool,
    /// Imported DLL names, lowercase, normal and delay-loaded.
    pub imports: Vec<String>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o.checked_add(2)?).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u32_at(b, o)? as u64 | (u32_at(b, o + 4)? as u64) << 32)
}

const IMPORT_DIR: usize = 1;
const DELAY_IMPORT_DIR: usize = 13;
/// Bounds for damaged or hostile files.
const MAX_SECTIONS: usize = 96;
const MAX_DESCRIPTORS: usize = 4096;

/// The import tables of a Windows program or DLL; None when it is not one.
pub fn parse_pe(b: &[u8]) -> Option<Pe> {
    if b.get(0..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(b, 0x3C)? as usize;
    if b.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let machine = u16_at(b, coff)?;
    let sections = u16_at(b, coff + 2)? as usize;
    let opt_size = u16_at(b, coff + 16)? as usize;
    let opt = coff + 20;
    // PE32 and PE32+ differ in where the image base and the data directories are.
    let (dirs, image_base) = match u16_at(b, opt)? {
        0x10b => (opt + 96, u32_at(b, opt + 28)? as u64),
        0x20b => (opt + 112, u64_at(b, opt + 24)?),
        _ => return None,
    };
    let dir_count = u32_at(b, dirs - 4)? as usize;
    let table = opt + opt_size;
    // (virtual address, size in memory, file offset, size in the file)
    let secs: Vec<(u64, u64, u64, u64)> = (0..sections.min(MAX_SECTIONS))
        .filter_map(|i| {
            let s = table + i * 40;
            Some((u32_at(b, s + 12)? as u64, u32_at(b, s + 8)? as u64, u32_at(b, s + 20)? as u64, u32_at(b, s + 16)? as u64))
        })
        .collect();
    let offset = |rva: u64| -> Option<usize> {
        secs.iter().find(|(va, vs, _, rs)| rva >= *va && rva < va + (*vs).max(*rs)).map(|(va, _, ptr, _)| (rva - va + ptr) as usize)
    };
    let name_at = |rva: u64| -> Option<String> {
        let o = offset(rva)?;
        let rest = b.get(o..)?;
        let end = rest.iter().take(260).position(|&c| c == 0)?;
        let s = std::str::from_utf8(&rest[..end]).ok()?;
        (!s.is_empty()).then(|| s.to_ascii_lowercase())
    };
    let dir = |i: usize| -> Option<u64> {
        if i >= dir_count {
            return None;
        }
        Some(u32_at(b, dirs + i * 8)? as u64).filter(|&rva| rva != 0)
    };

    let mut imports = Vec::new();
    if let Some(mut o) = dir(IMPORT_DIR).and_then(|rva| offset(rva)) {
        for _ in 0..MAX_DESCRIPTORS {
            let name = match u32_at(b, o + 12) {
                Some(0) | None => break,
                Some(n) => n as u64,
            };
            imports.extend(name_at(name));
            o += 20;
        }
    }
    if let Some(mut o) = dir(DELAY_IMPORT_DIR).and_then(|rva| offset(rva)) {
        for _ in 0..MAX_DESCRIPTORS {
            let (attrs, name) = match (u32_at(b, o), u32_at(b, o + 4)) {
                (Some(a), Some(n)) if n != 0 => (a, n as u64),
                _ => break,
            };
            // Old linkers wrote addresses instead of offsets (attribute bit 0 clear).
            let rva = if attrs & 1 == 0 && name >= image_base { name - image_base } else { name };
            imports.extend(name_at(rva));
            o += 32;
        }
    }
    imports.sort();
    imports.dedup();
    Some(Pe { x86: machine == 0x14c, imports })
}

/// Windows' API sets: names that map to system DLLs and never exist as files.
fn is_api_set(name: &str) -> bool {
    name.starts_with("api-ms-") || name.starts_with("ext-ms-")
}

/// Where Windows looks for a DLL the program's folder does not have.
fn system_dirs(x86: bool) -> Vec<PathBuf> {
    let win = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    let wow = win.join("SysWOW64");
    let sys = if x86 && wow.is_dir() { wow } else { win.join("System32") };
    let mut dirs = vec![sys, win];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

/// Programs larger than this are not read (no port comes close).
const MAX_PE_BYTES: u64 = 600 * 1024 * 1024;
/// The game's own DLLs read for their imports, at most.
const MAX_LOCAL_DLLS: usize = 64;

/// The DLLs `exe` needs that are neither in its folder nor in Windows. The game's own DLLs are
/// followed too (a port's SDL2.dll needs the Visual C++ runtime as much as the port does).
/// None when the program cannot be read.
pub fn missing_dlls(exe: &Path) -> Option<(bool, Vec<String>)> {
    let dir = exe.parent()?;
    let read = |p: &Path| -> Option<Pe> {
        if std::fs::metadata(p).ok()?.len() > MAX_PE_BYTES {
            return None;
        }
        parse_pe(&std::fs::read(p).ok()?)
    };
    let main = read(exe)?;
    let x86 = main.x86;
    let system = system_dirs(x86);
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut missing: BTreeSet<String> = BTreeSet::new();
    let mut queue = vec![main];
    let mut read_count = 0;
    while let Some(pe) = queue.pop() {
        for name in pe.imports {
            if is_api_set(&name) || !seen.insert(name.clone()) {
                continue;
            }
            let local = dir.join(&name);
            if local.is_file() {
                if read_count < MAX_LOCAL_DLLS {
                    read_count += 1;
                    queue.extend(read(&local));
                }
            } else if !system.iter().any(|d| d.join(&name).is_file()) {
                missing.insert(name);
            }
        }
    }
    Some((x86, missing.into_iter().collect()))
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DllKind {
    /// Visual C++ 2015-2022.
    Vc,
    /// An older Visual C++ (2005 to 2013).
    VcOld,
    /// The DirectX End-User Runtime (d3dx9, xinput1_3…), never part of Windows.
    DirectX,
    Vulkan,
    /// Anything else: a file that should be in the game's folder.
    Game,
}

pub fn classify(name: &str) -> DllKind {
    const VC: &[&str] = &["msvcp140", "vcruntime140", "concrt140", "vccorlib140", "vcomp140", "vcamp140", "mfc140"];
    const DX: &[&str] = &["d3dx", "d3dcsx", "x3daudio", "xapofx", "xactengine"];
    let n = name.to_ascii_lowercase();
    if VC.iter().any(|p| n.starts_with(p)) {
        DllKind::Vc
    } else if n.starts_with("msvcr") || n.starts_with("msvcp") || n.starts_with("mfc") {
        DllKind::VcOld
    } else if DX.iter().any(|p| n.starts_with(p))
        || ["xinput1_1.dll", "xinput1_2.dll", "xinput1_3.dll"].contains(&n.as_str())
        || (n.starts_with("xaudio2_") && n.as_bytes().get(8).is_some_and(|c| (b'0'..=b'7').contains(c)))
        || (n.starts_with("d3dcompiler_") && n != "d3dcompiler_47.dll")
    {
        DllKind::DirectX
    } else if n == "vulkan-1.dll" {
        DllKind::Vulkan
    } else {
        DllKind::Game
    }
}

fn name_list(names: &[&String]) -> String {
    let shown: Vec<&str> = names.iter().take(4).map(|s| s.as_str()).collect();
    match names.len().saturating_sub(4) {
        0 => shown.join(", "),
        more => format!("{} e mais {more}", shown.join(", ")),
    }
}

/// What the missing DLLs mean, with the one-click fix where Microsoft has an installer.
pub fn dll_findings(missing: &[String], x86: bool) -> Vec<Finding> {
    let of = |k: DllKind| -> Vec<&String> { missing.iter().filter(|n| classify(n) == k).collect() };
    let mut out = Vec::new();
    let vc = of(DllKind::Vc);
    if !vc.is_empty() {
        out.push(Finding {
            level: "bad",
            text: format!(
                "Falta o Visual C++ 2015-2022 ({}) da Microsoft, que este jogo usa ({}). Sem ele o jogo não abre.",
                if x86 { "x86" } else { "x64" },
                name_list(&vc)
            ),
            link: None,
            fix: Some(if x86 { "vc_x86" } else { "vc_x64" }),
        });
    }
    let dx = of(DllKind::DirectX);
    if !dx.is_empty() {
        out.push(Finding {
            level: "bad",
            text: format!(
                "Falta o DirectX antigo (DirectX End-User Runtime), que este jogo usa ({}). O Windows não traz esses arquivos, nem no Windows 11.",
                name_list(&dx)
            ),
            link: None,
            fix: Some("directx"),
        });
    }
    let old = of(DllKind::VcOld);
    if !old.is_empty() {
        out.push(Finding {
            level: "bad",
            text: format!(
                "Este jogo usa um Visual C++ antigo ({}). Instale o pacote do ano certo (2008, 2010, 2012 ou 2013, versão {}) na página da Microsoft.",
                name_list(&old),
                if x86 { "x86" } else { "x64" }
            ),
            link: Some(("Página da Microsoft".into(), crate::pcinfo::VC_PAGE.into())),
            fix: None,
        });
    }
    if !of(DllKind::Vulkan).is_empty() {
        out.push(Finding {
            level: "bad",
            text: "O jogo usa Vulkan, mas o driver da sua placa de vídeo não instalou o Vulkan (falta vulkan-1.dll). Atualize o driver da placa de vídeo.".into(),
            link: None,
            fix: None,
        });
    }
    let game = of(DllKind::Game);
    if !game.is_empty() {
        out.push(Finding {
            level: "bad",
            text: format!(
                "Faltam arquivos na pasta do jogo: {}. O antivírus pode ter apagado algum, ou a instalação ficou incompleta. Reinstale o jogo (os saves ficam guardados).",
                name_list(&game)
            ),
            link: None,
            fix: None,
        });
    }
    out
}

// ---- The game's folder: OneDrive, long paths, permission ----

/// Attributes of a file that is only in the cloud (OneDrive "Disponível online"): reading it
/// makes Windows download it first.
const CLOUD_ONLY: u32 = 0x0040_0000 | 0x0004_0000 | 0x1000;
/// Windows' old path limit, still the limit for programs that do not opt out of it.
pub const MAX_PATH: usize = 260;
const MAX_SCAN: usize = 20_000;

#[derive(Default, Debug)]
pub struct Scan {
    pub cloud: Vec<PathBuf>,
    pub long: usize,
}

pub fn is_cloud_only(attributes: u32) -> bool {
    attributes & CLOUD_ONLY != 0
}

/// Reads only names and attributes, so nothing is downloaded from the cloud. Links and
/// junctions are not followed.
pub fn scan(dir: &Path) -> Scan {
    let mut out = Scan::default();
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0;
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > MAX_SCAN {
                return out;
            }
            let Ok(ft) = e.file_type() else { continue };
            let path = e.path();
            if path.as_os_str().encode_wide().count() >= MAX_PATH {
                out.long += 1;
            }
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(path);
            } else if e.metadata().map(|m| is_cloud_only(m.file_attributes())).unwrap_or(false) {
                out.cloud.push(path);
            }
        }
    }
    out
}

/// Whether a file can be created in `dir`; only a refusal from Windows counts as "no".
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".recomphub-teste-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(f) => {
            drop(f);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(e) => e.kind() != std::io::ErrorKind::PermissionDenied,
    }
}

pub fn folder_findings(scan: &Scan, exe_in_cloud: bool, can_write: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    if !scan.cloud.is_empty() {
        out.push(Finding {
            level: if exe_in_cloud { "bad" } else { "warn" },
            text: format!(
                "{} do jogo {} só na nuvem do OneDrive. O jogo pode travar ou demorar para abrir enquanto o Windows baixa. Baixe tudo antes de jogar (precisa de internet).",
                if scan.cloud.len() == 1 { "1 arquivo".to_string() } else { format!("{} arquivos", scan.cloud.len()) },
                if scan.cloud.len() == 1 { "está" } else { "estão" }
            ),
            link: None,
            fix: Some("hydrate"),
        });
    }
    if scan.long > 0 {
        out.push(Finding {
            level: "warn",
            text: format!(
                "{} na pasta do jogo {} o caminho com mais de {MAX_PATH} letras, e muitos programas não conseguem abrir arquivos assim. Mova o jogo para uma pasta de nome curto, como C:\\Jogos.",
                if scan.long == 1 { "1 arquivo".to_string() } else { format!("{} arquivos", scan.long) },
                if scan.long == 1 { "tem" } else { "têm" }
            ),
            link: None,
            fix: None,
        });
    }
    if !can_write {
        out.push(Finding {
            level: "warn",
            text: "O Windows não deixa gravar na pasta do jogo, e este jogo guarda saves e configurações nela: eles podem não ser salvos. Mova o jogo para fora de \"Arquivos de Programas\", por exemplo para C:\\Jogos.".into(),
            link: None,
            fix: None,
        });
    }
    out
}

/// Everything about one installed game. `keeps_saves_inside`: the game writes next to its
/// program (ports whose saves are under `{install}`, community ports).
pub fn check_game(exe: &Path, keeps_saves_inside: bool) -> Vec<Finding> {
    let Some(dir) = exe.parent().filter(|_| exe.is_file()) else { return Vec::new() };
    let scan = scan(dir);
    let exe_in_cloud = std::fs::metadata(exe).map(|m| is_cloud_only(m.file_attributes())).unwrap_or(false);
    let mut out = Vec::new();
    // Reading a cloud-only program would download it; that waits for "Baixar agora".
    if !exe_in_cloud {
        if let Some((x86, missing)) = missing_dlls(exe) {
            out.extend(dll_findings(&missing, x86));
        }
    }
    out.extend(folder_findings(&scan, exe_in_cloud, !keeps_saves_inside || writable(dir)));
    out
}

/// "Baixar agora": reading each cloud-only file makes OneDrive bring it to this PC.
pub fn hydrate(dir: &Path) -> Result<usize, String> {
    let files = scan(dir).cloud;
    for f in &files {
        let mut file = std::fs::File::open(f).map_err(|e| download_error(f, &e))?;
        std::io::copy(&mut file, &mut std::io::sink()).map_err(|e| download_error(f, &e))?;
    }
    Ok(files.len())
}

fn download_error(f: &Path, e: &std::io::Error) -> String {
    format!(
        "Não consegui baixar {} do OneDrive: {e}. Confira se o OneDrive está conectado e com internet.",
        f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    )
}

// ---- Which program has a file open (Restart Manager) ----

/// The programs holding `path` open, by name ("Explorador de Arquivos", "steam.exe").
pub fn who_uses(path: &Path) -> Vec<String> {
    use windows_sys::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY, RM_PROCESS_INFO,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let files = [wide.as_ptr()];
    let mut session: u32 = 0;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    let mut names = Vec::new();
    // SAFETY: every buffer outlives the calls; the session is always ended.
    unsafe {
        if RmStartSession(&mut session, 0, key.as_mut_ptr()) != 0 {
            return names;
        }
        if RmRegisterResources(session, 1, files.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null()) == 0 {
            let mut infos = vec![std::mem::zeroed::<RM_PROCESS_INFO>(); 16];
            let mut needed: u32 = 0;
            let mut count = infos.len() as u32;
            let mut reasons: u32 = 0;
            if RmGetList(session, &mut needed, &mut count, infos.as_mut_ptr(), &mut reasons) == 0 {
                for info in infos.iter().take(count as usize) {
                    let len = info.strAppName.iter().position(|&c| c == 0).unwrap_or(info.strAppName.len());
                    let name = String::from_utf16_lossy(&info.strAppName[..len]);
                    if !name.is_empty() && !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
        RmEndSession(session);
    }
    names
}

/// "The process cannot access the file because it is being used by another process" and
/// "access denied" (also what a file open in another program gives when replacing it).
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;

/// The message for a file that could not be written while installing or updating: the
/// antivirus, or the program holding it open, by name.
pub fn write_error(e: &std::io::Error, path: &Path) -> String {
    if let Some(av) = crate::play::antivirus_error(e) {
        return av;
    }
    if matches!(e.raw_os_error(), Some(ERROR_SHARING_VIOLATION) | Some(ERROR_ACCESS_DENIED)) {
        let users = who_uses(path);
        if !users.is_empty() {
            return format!("Não consegui gravar {}: o arquivo está aberto em {}. Feche e tente de novo.", path.display(), users.join(", "));
        }
    }
    format!("Não consegui gravar {} (o jogo está aberto?): {e}", path.display())
}

// ---- One-click runtimes from Microsoft ----

pub struct Runtime {
    pub id: &'static str,
    pub name: &'static str,
    /// Microsoft's permanent link; it redirects to the current file.
    pub url: &'static str,
    pub file: &'static str,
    /// Silent install, no restart.
    pub args: &'static str,
    pub max_bytes: u64,
}

pub const RUNTIMES: &[Runtime] = &[
    Runtime {
        id: "vc_x64",
        name: "Visual C++ 2015-2022 (x64)",
        url: "https://aka.ms/vs/17/release/vc_redist.x64.exe",
        file: "vc_redist.x64.exe",
        args: "/install /quiet /norestart",
        max_bytes: 80 * 1024 * 1024,
    },
    Runtime {
        id: "vc_x86",
        name: "Visual C++ 2015-2022 (x86)",
        url: "https://aka.ms/vs/17/release/vc_redist.x86.exe",
        file: "vc_redist.x86.exe",
        args: "/install /quiet /norestart",
        max_bytes: 80 * 1024 * 1024,
    },
    Runtime {
        id: "directx",
        name: "DirectX End-User Runtime",
        url: "https://download.microsoft.com/download/1/7/1/1718CCC4-6315-4D8E-9543-8E28A4E18C4C/dxwebsetup.exe",
        file: "dxwebsetup.exe",
        args: "/Q",
        max_bytes: 20 * 1024 * 1024,
    },
];

pub fn runtime(id: &str) -> Option<&'static Runtime> {
    RUNTIMES.iter().find(|r| r.id == id)
}

/// Downloads only from Microsoft (the permanent links redirect to download.*.microsoft.com).
pub fn microsoft_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    if host.contains('@') || host.contains(':') {
        return false;
    }
    host == "aka.ms" || host == "microsoft.com" || host.ends_with(".microsoft.com")
}

/// Microsoft puts the file's SHA-256 in the Visual C++ download address; checked when present.
pub fn sha256_in_url(url: &str) -> Option<String> {
    url.split(['/', '?']).find(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).map(|s| s.to_ascii_lowercase())
}

/// The output of the signature check: "Valid|CN=Microsoft Corporation, O=Microsoft Corporation, ...".
pub fn signature_ok(out: &str) -> bool {
    let Some((status, subject)) = out.trim().split_once('|') else { return false };
    status.trim() == "Valid" && subject.split(',').any(|part| part.trim() == "O=Microsoft Corporation")
}

/// What the installer's exit code means for the player.
pub fn exit_text(code: u32, name: &str) -> Result<String, String> {
    match code {
        0 => Ok(format!("{name} instalado.")),
        3010 | 1641 => Ok(format!("{name} instalado. Reinicie o PC antes de jogar.")),
        1638 => Ok(format!("{name}: já havia uma versão igual ou mais nova instalada.")),
        1602 => Err("A instalação foi cancelada.".into()),
        1618 => Err("Outra instalação está em andamento no Windows. Espere ela terminar e tente de novo.".into()),
        c => Err(format!("O instalador do {name} terminou com o código {c}. Tente pela página da Microsoft.")),
    }
}

fn check_signature(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let quoted = path.display().to_string().replace('\'', "''");
    let script = format!(
        "$s = Get-AuthenticodeSignature -LiteralPath '{quoted}'; [string]$s.Status + '|' + $s.SignerCertificate.Subject"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Não consegui conferir a assinatura do instalador: {e}"))?;
    if signature_ok(&String::from_utf8_lossy(&out.stdout)) {
        Ok(())
    } else {
        Err("O instalador baixado não tem a assinatura válida da Microsoft, então não foi aberto.".into())
    }
}

/// Downloads the runtime from Microsoft, checks its hash (when the address has one) and the
/// Microsoft signature, and runs it silently with one administrator prompt. Blocking.
pub fn install(rt: &Runtime) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("RecompHUB/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 8 || !microsoft_url(attempt.url().as_str()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(rt.url).send().map_err(|e| format!("O download do {} falhou: {e}", rt.name))?;
    let final_url = resp.url().to_string();
    if !resp.status().is_success() || !microsoft_url(&final_url) {
        return Err(format!("O site da Microsoft não entregou o {} (resposta {}).", rt.name, resp.status()));
    }
    if resp.content_length().is_some_and(|n| n > rt.max_bytes) {
        return Err("O arquivo da Microsoft é maior do que o esperado; nada foi instalado.".into());
    }
    let mut bytes = Vec::new();
    resp.take(rt.max_bytes + 1).read_to_end(&mut bytes).map_err(|e| format!("O download do {} foi interrompido: {e}", rt.name))?;
    if bytes.len() as u64 > rt.max_bytes {
        return Err("O arquivo da Microsoft é maior do que o esperado; nada foi instalado.".into());
    }
    if let Some(expected) = sha256_in_url(&final_url) {
        let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if got != expected {
            return Err("O arquivo baixado não confere com o da Microsoft (download corrompido). Tente de novo.".into());
        }
    }
    let dir = std::env::temp_dir().join("RecompHUB");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(rt.file);
    std::fs::write(&path, &bytes).map_err(|e| crate::play::antivirus_error(&e).unwrap_or_else(|| e.to_string()))?;
    let result = check_signature(&path).and_then(|_| crate::play::runas_and_wait(&path, rt.args)).and_then(|code| exit_text(code, rt.name));
    let _ = std::fs::remove_file(&path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PE32+ program: one section at RVA 0x1000 (file offset 0x200) holding an
    /// import table, a delay-import table and the DLL names.
    fn fake_pe(x86: bool, imports: &[&str], delay: &[&str]) -> Vec<u8> {
        let mut b = vec![0u8; 0x200];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        b[coff..coff + 2].copy_from_slice(&(if x86 { 0x14cu16 } else { 0x8664u16 }).to_le_bytes());
        b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        let opt = coff + 20;
        let (opt_size, dirs) = if x86 { (224u16, opt + 96) } else { (240u16, opt + 112) };
        b[coff + 16..coff + 18].copy_from_slice(&opt_size.to_le_bytes());
        b[opt..opt + 2].copy_from_slice(&(if x86 { 0x10bu16 } else { 0x20bu16 }).to_le_bytes());
        b[dirs - 4..dirs].copy_from_slice(&16u32.to_le_bytes());
        // Section layout (RVA = 0x1000 + offset in the section).
        let mut sec = vec![0u8; 0x400];
        let imp_at = 0x0;
        let delay_at = 0x100;
        let mut names_at = 0x200;
        for (i, n) in imports.iter().enumerate() {
            let o = imp_at + i * 20;
            sec[o + 12..o + 16].copy_from_slice(&(0x1000 + names_at as u32).to_le_bytes());
            sec[names_at..names_at + n.len()].copy_from_slice(n.as_bytes());
            names_at += n.len() + 1;
        }
        for (i, n) in delay.iter().enumerate() {
            let o = delay_at + i * 32;
            sec[o..o + 4].copy_from_slice(&1u32.to_le_bytes());
            sec[o + 4..o + 8].copy_from_slice(&(0x1000 + names_at as u32).to_le_bytes());
            sec[names_at..names_at + n.len()].copy_from_slice(n.as_bytes());
            names_at += n.len() + 1;
        }
        b[dirs + 8..dirs + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        b[dirs + 12..dirs + 16].copy_from_slice(&0x100u32.to_le_bytes());
        b[dirs + 13 * 8..dirs + 13 * 8 + 4].copy_from_slice(&0x1100u32.to_le_bytes());
        b[dirs + 13 * 8 + 4..dirs + 13 * 8 + 8].copy_from_slice(&0x100u32.to_le_bytes());
        let table = opt + opt_size as usize;
        b[table..table + 5].copy_from_slice(b".data");
        b[table + 8..table + 12].copy_from_slice(&0x400u32.to_le_bytes());
        b[table + 12..table + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        b[table + 16..table + 20].copy_from_slice(&0x400u32.to_le_bytes());
        b[table + 20..table + 24].copy_from_slice(&0x200u32.to_le_bytes());
        b.extend(sec);
        b
    }

    #[test]
    fn imports_are_read_from_both_tables() {
        let pe = parse_pe(&fake_pe(false, &["KERNEL32.dll", "VCRUNTIME140.dll"], &["XINPUT1_3.dll"])).unwrap();
        assert!(!pe.x86);
        assert_eq!(pe.imports, vec!["kernel32.dll", "vcruntime140.dll", "xinput1_3.dll"]);
        let pe32 = parse_pe(&fake_pe(true, &["d3dx9_43.dll"], &[])).unwrap();
        assert!(pe32.x86);
        assert_eq!(pe32.imports, vec!["d3dx9_43.dll"]);
    }

    #[test]
    fn damaged_files_are_not_programs() {
        assert_eq!(parse_pe(b""), None);
        assert_eq!(parse_pe(b"MZ"), None);
        let mut cut = fake_pe(false, &["a.dll"], &[]);
        cut.truncate(0x90);
        assert_eq!(parse_pe(&cut), None);
        // A name pointing outside every section is skipped, not followed.
        let mut bad = fake_pe(false, &["a.dll"], &[]);
        bad[0x200 + 12..0x200 + 16].copy_from_slice(&0x9999_0000u32.to_le_bytes());
        assert_eq!(parse_pe(&bad).unwrap().imports, Vec::<String>::new());
    }

    #[test]
    fn dlls_are_told_apart() {
        assert_eq!(classify("MSVCP140.dll"), DllKind::Vc);
        assert_eq!(classify("vcruntime140_1.dll"), DllKind::Vc);
        assert_eq!(classify("msvcr120.dll"), DllKind::VcOld);
        assert_eq!(classify("d3dx9_43.dll"), DllKind::DirectX);
        assert_eq!(classify("xinput1_3.dll"), DllKind::DirectX);
        assert_eq!(classify("xaudio2_7.dll"), DllKind::DirectX);
        assert_eq!(classify("xaudio2_9.dll"), DllKind::Game);
        assert_eq!(classify("d3dcompiler_43.dll"), DllKind::DirectX);
        assert_eq!(classify("d3dcompiler_47.dll"), DllKind::Game);
        assert_eq!(classify("vulkan-1.dll"), DllKind::Vulkan);
        assert_eq!(classify("SDL2.dll"), DllKind::Game);
        assert!(is_api_set("api-ms-win-crt-runtime-l1-1-0.dll"));
    }

    #[test]
    fn missing_dlls_become_fixes() {
        let missing: Vec<String> = ["msvcp140.dll", "vcruntime140_1.dll", "d3dx9_43.dll", "sdl2.dll"].iter().map(|s| s.to_string()).collect();
        let f = dll_findings(&missing, false);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].fix, Some("vc_x64"));
        assert!(f[0].text.contains("msvcp140.dll, vcruntime140_1.dll"));
        assert_eq!(f[1].fix, Some("directx"));
        assert!(f[2].text.contains("sdl2.dll"));
        assert_eq!(dll_findings(&missing[..1], true)[0].fix, Some("vc_x86"));
        assert!(dll_findings(&[], false).is_empty());
        let many: Vec<String> = (0..6).map(|i| format!("a{i}.dll")).collect();
        assert!(dll_findings(&many, false)[0].text.contains("e mais 2"));
    }

    #[test]
    fn folder_problems() {
        let scan = Scan { cloud: vec![PathBuf::from("a"), PathBuf::from("b")], long: 1 };
        let f = folder_findings(&scan, false, false);
        assert_eq!(f.len(), 3);
        assert_eq!((f[0].level, f[0].fix), ("warn", Some("hydrate")));
        assert!(f[0].text.starts_with("2 arquivos do jogo estão"));
        assert!(f[1].text.starts_with("1 arquivo na pasta do jogo tem"));
        assert_eq!(folder_findings(&scan, true, true)[0].level, "bad");
        assert!(folder_findings(&Scan::default(), false, true).is_empty());
        assert!(is_cloud_only(0x0040_0020));
        assert!(!is_cloud_only(0x20));
    }

    #[test]
    fn a_real_folder_is_scanned_and_written() {
        let dir = std::env::temp_dir().join(format!("recomphub-precheck-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("x.txt"), b"x").unwrap();
        let s = scan(&dir);
        assert!(s.cloud.is_empty());
        assert_eq!(s.long, 0);
        assert!(writable(&dir));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "the probe file is removed");
        assert!(who_uses(&dir.join("sub").join("x.txt")).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn this_program_is_read() {
        // The test binary itself is a PE32+ program; Windows' own DLLs are always found.
        let exe = std::env::current_exe().unwrap();
        let (x86, missing) = missing_dlls(&exe).unwrap();
        assert!(!x86);
        assert!(!missing.iter().any(|m| m == "kernel32.dll" || m == "ntdll.dll"), "{missing:?}");
        assert!(parse_pe(&std::fs::read(&exe).unwrap()).unwrap().imports.contains(&"kernel32.dll".to_string()));
    }

    #[test]
    fn downloads_only_from_microsoft() {
        assert!(microsoft_url("https://aka.ms/vs/17/release/vc_redist.x64.exe"));
        assert!(microsoft_url("https://download.visualstudio.microsoft.com/download/pr/x/VC_redist.x64.exe"));
        assert!(!microsoft_url("http://download.microsoft.com/x.exe"));
        assert!(!microsoft_url("https://microsoft.com.evil.example/x.exe"));
        assert!(!microsoft_url("https://evilmicrosoft.com/x.exe"));
        assert!(!microsoft_url("https://aka.ms@evil.example/x.exe"));
        assert!(RUNTIMES.iter().all(|r| microsoft_url(r.url)));
        let url = "https://download.visualstudio.microsoft.com/download/pr/990c720a/CC0FF0EB1DC3F5188AE6300FAEF32BF5BEEBA4BDD6E8E445A9184072096B713B/VC_redist.x64.exe";
        assert_eq!(sha256_in_url(url).unwrap(), "cc0ff0eb1dc3f5188ae6300faef32bf5beeba4bdd6e8e445a9184072096b713b");
        assert_eq!(sha256_in_url(RUNTIMES[2].url), None);
    }

    #[test]
    fn microsoft_signature_and_exit_codes() {
        assert!(signature_ok("Valid|CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US\r\n"));
        assert!(!signature_ok("NotSigned|"));
        assert!(!signature_ok("Valid|CN=Evil, O=Microsoft Corporation Fake, C=US"));
        assert!(!signature_ok("HashMismatch|CN=Microsoft Corporation, O=Microsoft Corporation"));
        assert!(exit_text(0, "X").is_ok());
        assert!(exit_text(3010, "X").unwrap().contains("Reinicie"));
        assert!(exit_text(1638, "X").is_ok());
        assert!(exit_text(1602, "X").is_err());
        assert!(exit_text(5, "X").unwrap_err().contains("código 5"));
    }
}
