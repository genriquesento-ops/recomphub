//! "O jogo fechou com erro": after a session ends badly, RecompHUB looks for what Windows and the
//! port left behind and says, in plain words, what it means and what to try. Everything is read
//! on this PC; nothing is sent anywhere.
//!
//! Sources, from the ports' own issue trackers (most reports are one of these):
//! - the exit code of the program RecompHUB started (a missing DLL never reaches the event log);
//! - Windows' "Application Error" events (id 1000), which name the exception code and the module
//!   it happened in, also for games started as administrator or from outside RecompHUB;
//! - crash dumps in %LOCALAPPDATA%\CrashDumps;
//! - the port's own log files (catalog field `logs`), whose last lines go into the report.

use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A game that closes this soon after opening did not get to run.
pub const SHORT_SESSION_SECS: u64 = 15;
const LOG_TAIL_LINES: usize = 40;
const LOG_TAIL_BYTES: u64 = 64 * 1024;

/// One "Application Error" event: a program closed by an unhandled exception.
#[derive(Clone, Debug, PartialEq)]
pub struct CrashEvent {
    pub at: u64,
    pub app_name: String,
    pub app_path: String,
    pub module: String,
    pub code: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct Action {
    pub label: String,
    /// The page's data-act ("shader-clear", "gfx-api", "open-dumps", "open-logs", "link"…).
    pub act: &'static str,
    /// data-v / data-url for the action, when it needs one.
    pub value: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Finding {
    /// "bad" (this is the cause), "warn" (likely related) or "info".
    pub level: &'static str,
    pub text: String,
    pub action: Option<Action>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Diagnosis {
    pub id: String,
    /// When the game closed (Unix time).
    pub at: u64,
    /// How long it ran, when known.
    pub seconds: Option<u64>,
    /// "0xC000001D".
    pub code: Option<String>,
    pub module: Option<String>,
    pub findings: Vec<Finding>,
    pub dumps: Vec<String>,
    pub logs: Vec<String>,
    /// Plain text for "Copiar relatório", with the user's folder name hidden.
    pub report: String,
}

/// What the game page knows about the game and this PC, for `findings`.
#[derive(Default)]
pub struct Context {
    pub code: Option<u32>,
    pub module: Option<String>,
    pub seconds: Option<u64>,
    /// Instruction sets the port needs that this processor lacks.
    pub cpu_missing: Vec<String>,
    pub vc_runtime: bool,
    pub has_shader_cache: bool,
    pub driver_changed: bool,
    /// N64: Recompiled ports: the graphics API in use ("Auto", "D3D12", "Vulkan").
    pub n64_api: Option<String>,
    pub gpus: usize,
    pub mods_on: usize,
    pub dumps: usize,
    pub has_logs: bool,
    /// Pages for the video driver of each vendor present.
    pub driver_pages: Vec<(&'static str, &'static str)>,
}

pub fn code_text(code: u32) -> String {
    format!("0x{code:08X}")
}

const ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
const DLL_NOT_FOUND: u32 = 0xC000_0135;
const BAD_IMAGE: u32 = 0xC000_007B;
const DLL_INIT_FAILED: u32 = 0xC000_0142;
const ENTRY_POINT_NOT_FOUND: u32 = 0xC000_0139;
const ACCESS_DENIED: u32 = 0xC000_0022;
const NO_MEMORY: u32 = 0xC000_0017;
const COMMIT_LIMIT: u32 = 0xC000_012D;

/// Exit codes that mean the program died (an NTSTATUS error or a C++ exception), as opposed to
/// a program that chose to quit with a code of its own.
pub fn is_crash_code(code: u32) -> bool {
    code >= 0xC000_0000 || code == 0xE06D_7363 || code == 0x8000_0003
}

/// Modules of the video drivers, by vendor: a crash inside one points at the driver.
fn driver_vendor(module: &str) -> Option<&'static str> {
    let m = module.to_ascii_lowercase();
    const NVIDIA: &[&str] = &["nvoglv", "nvwgf2um", "nvd3dum", "nvldumd", "nvgpucomp", "nvvk", "nvcuda", "nvapi"];
    const AMD: &[&str] = &["amdvlk", "atiumd", "atio6axx", "amdxc", "amdxx", "amd_ags", "atidxx", "amdihk"];
    const INTEL: &[&str] = &["igvk", "igd10", "igd12", "igxelpicd", "igc64", "igdusc", "igdml", "igdgmm", "igdumdim", "intel_gfx"];
    if NVIDIA.iter().any(|p| m.starts_with(p)) {
        Some("NVIDIA")
    } else if AMD.iter().any(|p| m.starts_with(p)) {
        Some("AMD")
    } else if INTEL.iter().any(|p| m.starts_with(p)) {
        Some("Intel")
    } else if m.starts_with("vulkan-1") || m.starts_with("d3d12") || m.starts_with("dxgi") {
        Some("")
    } else {
        None
    }
}

/// Overlays that inject themselves into games and are a known cause of crashes.
fn overlay(module: &str) -> Option<&'static str> {
    let m = module.to_ascii_lowercase();
    if m.starts_with("rtsshooks") {
        Some("RivaTuner / MSI Afterburner")
    } else if m.starts_with("gameoverlayrenderer") {
        Some("a sobreposição da Steam")
    } else if m.starts_with("discord") {
        Some("a sobreposição do Discord")
    } else if m.starts_with("graphics-hook") || m.starts_with("obs-") {
        Some("a captura de jogo do OBS")
    } else if m.starts_with("reshade") {
        Some("o ReShade")
    } else {
        None
    }
}

/// What the evidence means, most likely cause first.
pub fn findings(c: &Context) -> Vec<Finding> {
    let mut out = Vec::new();
    let module = c.module.as_deref().unwrap_or("");
    let code = c.code.map(code_text).unwrap_or_default();
    let mut explained = false;

    match c.code {
        Some(ILLEGAL_INSTRUCTION) => {
            explained = true;
            out.push(Finding {
                level: "bad",
                text: if c.cpu_missing.is_empty() {
                    format!("O jogo fechou por \"instrução ilegal\" ({code}): o processador recebeu uma instrução que não conhece. Acontece com processadores antigos que não têm instruções que o port usa, e não há ajuste que resolva.")
                } else {
                    format!(
                        "O jogo fechou por \"instrução ilegal\" ({code}): seu processador não tem as instruções {} que este port usa. Não há ajuste que resolva: só outro processador.",
                        c.cpu_missing.iter().map(|m| m.to_uppercase()).collect::<Vec<_>>().join(", ")
                    )
                },
                action: None,
            });
        }
        Some(DLL_NOT_FOUND | BAD_IMAGE | DLL_INIT_FAILED | ENTRY_POINT_NOT_FOUND) => {
            explained = true;
            out.push(Finding {
                level: "bad",
                text: format!(
                    "O jogo não conseguiu carregar uma DLL ({code}). Quase sempre é o Visual C++ 2015-2022 (x64) da Microsoft{}. Instale ou repare pela página da Microsoft e abra o jogo de novo. Se continuar, o antivírus pode ter apagado uma DLL da pasta do port: reinstale o port.",
                    if c.vc_runtime { ", que pode estar danificado" } else { ", que não está instalado no seu PC" }
                ),
                action: Some(Action { label: "Página do Visual C++".into(), act: "link", value: Some(crate::pcinfo::VC_PAGE.into()) }),
            });
        }
        Some(ACCESS_DENIED) => {
            explained = true;
            out.push(Finding {
                level: "warn",
                text: format!("O Windows negou acesso a um arquivo ({code}). Costuma ser o antivírus bloqueando o port ou a pasta dele. Veja o Histórico de proteção na Segurança do Windows."),
                action: Some(Action { label: "Abrir Segurança do Windows".into(), act: "win-security", value: None }),
            });
        }
        Some(NO_MEMORY | COMMIT_LIMIT) => {
            explained = true;
            out.push(Finding {
                level: "warn",
                text: format!("Faltou memória ({code}). Feche outros programas pesados (navegador com muitas abas, por exemplo) e confira se a memória virtual do Windows não está desligada."),
                action: None,
            });
        }
        _ => {}
    }

    if !explained && !module.is_empty() {
        if let Some(vendor) = driver_vendor(module) {
            explained = true;
            let page = c.driver_pages.iter().find(|(v, _)| vendor.is_empty() || *v == vendor).copied();
            out.push(Finding {
                level: "warn",
                text: format!(
                    "O erro aconteceu dentro do driver de vídeo ({module}). Atualize o driver da placa de vídeo{}.",
                    if c.has_shader_cache { " e, se continuar, limpe o cache de shaders" } else { "" }
                ),
                action: page.map(|(v, u)| Action { label: format!("Baixar driver {v}"), act: "link", value: Some(u.into()) }),
            });
            if c.n64_api.as_deref().is_some_and(|a| a != "Vulkan") {
                out.push(Finding {
                    level: "info",
                    text: "Este port pode usar DirectX 12 ou Vulkan. Quando o driver dá problema com um, o outro costuma funcionar.".into(),
                    action: Some(Action { label: "Usar Vulkan".into(), act: "gfx-api", value: Some("Vulkan".into()) }),
                });
            }
        } else if let Some(name) = overlay(module) {
            explained = true;
            out.push(Finding {
                level: "warn",
                text: format!("O erro aconteceu dentro de {name} ({module}), que se coloca por cima dos jogos. Desligue essa sobreposição para este jogo e tente de novo."),
                action: None,
            });
        }
    }

    if c.driver_changed && c.has_shader_cache {
        out.push(Finding {
            level: "warn",
            text: "O driver de vídeo mudou desde a última vez que o jogo fechou bem, e o cache de shaders foi feito com o driver antigo. Limpar o cache resolve em muitos casos.".into(),
            action: Some(Action { label: "Limpar cache de shaders".into(), act: "shader-clear", value: None }),
        });
    }
    if c.n64_api.as_deref() == Some("Auto") && c.gpus >= 2 && !out.iter().any(|f| f.action.as_ref().is_some_and(|a| a.act == "gfx-api")) {
        out.push(Finding {
            level: "info",
            text: "Seu PC tem duas placas de vídeo e o jogo escolhe a API sozinho. Em notebooks, escolher Vulkan resolveu tela preta e fechamentos em vários casos.".into(),
            action: Some(Action { label: "Usar Vulkan".into(), act: "gfx-api", value: Some("Vulkan".into()) }),
        });
    }
    if c.mods_on > 0 {
        out.push(Finding {
            level: "info",
            text: format!(
                "Você tem {} ligado{}. Se o jogo funcionava antes, desligue os mods mais recentes na Central de Mods e teste de novo.",
                if c.mods_on == 1 { "1 mod".to_string() } else { format!("{} mods", c.mods_on) },
                if c.mods_on == 1 { "" } else { "s" }
            ),
            action: None,
        });
    }

    if !explained {
        let short = c.seconds.is_some_and(|s| s < SHORT_SESSION_SECS);
        let text = match (c.code, short) {
            (Some(_), _) if !module.is_empty() => format!("O jogo fechou com o erro {code} em {module}. Esse código sozinho não diz a causa: copie o relatório abaixo e mostre no GitHub do port."),
            (Some(_), _) => format!("O jogo fechou com o erro {code}. Esse código sozinho não diz a causa: copie o relatório abaixo e mostre no GitHub do port."),
            (None, true) => format!("O jogo fechou {} segundos depois de abrir, sem deixar código de erro no Windows. Confira a caixa \"Seu PC\" e o log do port.", c.seconds.unwrap_or(0)),
            (None, false) => "O Windows não registrou um erro deste jogo. Se ele fechou sozinho, o log do port pode dizer o motivo.".into(),
        };
        out.push(Finding { level: if c.code.is_some() || short { "warn" } else { "info" }, text, action: None });
    }
    if c.dumps > 0 {
        out.push(Finding {
            level: "info",
            text: "O Windows gravou um arquivo de despejo (crash dump) do fechamento. Os desenvolvedores do port podem pedir esse arquivo; ele pode conter dados da memória do jogo.".into(),
            action: Some(Action { label: "Abrir pasta do despejo".into(), act: "open-dumps", value: None }),
        });
    }
    if c.has_logs {
        out.push(Finding {
            level: "info",
            text: "O fim do log do port está no relatório.".into(),
            action: Some(Action { label: "Abrir pasta do log".into(), act: "open-logs", value: None }),
        });
    }
    out
}

/// The latest diagnosis of each game, shown on its page until dismissed (not kept on disk:
/// "Procurar o erro" finds it again from Windows' records).
static LAST: Mutex<Vec<Diagnosis>> = Mutex::new(Vec::new());

pub fn remember(d: Diagnosis) {
    let mut list = LAST.lock().unwrap();
    list.retain(|x| x.id != d.id);
    list.push(d);
}

pub fn last(id: &str) -> Option<Diagnosis> {
    LAST.lock().unwrap().iter().find(|d| d.id == id).cloned()
}

pub fn forget(id: &str) {
    LAST.lock().unwrap().retain(|d| d.id != id);
}

// ---- Reading what Windows left ----

#[derive(Deserialize)]
struct RawEvent {
    t: i64,
    p: Vec<Option<String>>,
}

/// Get-WinEvent's events as written by `EVENTS_SCRIPT`: one object or an array of them.
/// Properties of event 1000: 0 program, 3 module, 6 exception code (hex), 10 program path.
pub fn parse_events(json: &str) -> Vec<CrashEvent> {
    let json = json.trim();
    if json.is_empty() {
        return Vec::new();
    }
    let raw: Vec<RawEvent> = if json.starts_with('[') {
        serde_json::from_str(json).unwrap_or_default()
    } else {
        serde_json::from_str::<RawEvent>(json).map(|e| vec![e]).unwrap_or_default()
    };
    raw.into_iter()
        .filter_map(|e| {
            let prop = |i: usize| e.p.get(i).cloned().flatten().unwrap_or_default();
            let hex = prop(6);
            let code = u32::from_str_radix(hex.trim().trim_start_matches("0x").trim_start_matches("0X"), 16).ok()?;
            Some(CrashEvent { at: e.t.max(0) as u64, app_name: prop(0), app_path: prop(10), module: prop(3), code })
        })
        .collect()
}

/// Application Error events since `since` (Unix time). About a second; never on the UI thread.
pub fn events_since(since: u64) -> Vec<CrashEvent> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = format!(
        "$s=[DateTimeOffset]::FromUnixTimeSeconds({since}).LocalDateTime; \
         Get-WinEvent -FilterHashtable @{{LogName='Application';ProviderName='Application Error';Id=1000;StartTime=$s}} -MaxEvents 50 -ErrorAction SilentlyContinue | \
         ForEach-Object {{ [pscustomobject]@{{ t=([DateTimeOffset]$_.TimeCreated).ToUnixTimeSeconds(); p=@($_.Properties | ForEach-Object {{ [string]$_.Value }}) }} }} | \
         ConvertTo-Json -Compress -Depth 3"
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .map(|o| parse_events(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// The newest event of a program inside `install_dir`, or named like one of `exe_names` (the
/// path is sometimes missing).
pub fn newest_for<'a>(events: &'a [CrashEvent], install_dir: &Path, exe_names: &[String]) -> Option<&'a CrashEvent> {
    let prefix = folder_prefix(install_dir);
    events
        .iter()
        .filter(|e| {
            let path = e.app_path.replace('/', "\\").to_lowercase();
            (!path.is_empty() && path.starts_with(&prefix)) || (path.is_empty() && exe_names.iter().any(|n| n.eq_ignore_ascii_case(&e.app_name)))
        })
        .max_by_key(|e| e.at)
}

fn folder_prefix(dir: &Path) -> String {
    let mut s = dir.to_string_lossy().replace('/', "\\").to_lowercase();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

/// Programs in the port's folder and one level below (gow3 starts bin\gow3-probe.exe).
pub fn exe_names(install_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut look = |dir: &Path, deeper: bool| -> Vec<PathBuf> {
        let mut subdirs = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if deeper {
                        subdirs.push(p);
                    }
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
                    out.push(e.file_name().to_string_lossy().to_string());
                }
            }
        }
        subdirs
    };
    for sub in look(install_dir, true) {
        look(&sub, false);
    }
    out
}

pub fn dumps_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("CrashDumps"))
}

/// Crash dumps ("<program>.<pid>.dmp") of the port's programs written since `since`.
pub fn dumps_since(dir: &Path, exe_names: &[String], since: u64) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name.ends_with(".dmp") && exe_names.iter().any(|x| name.starts_with(&format!("{}.", x.to_lowercase())))
        })
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|d| d.as_secs() + 5 >= since)
        })
        .map(|e| e.path())
        .collect()
}

/// The last lines of a log, read from its end so a huge log costs nothing.
pub fn tail(path: &Path, lines: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(LOG_TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.take(LOG_TAIL_BYTES).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let all: Vec<&str> = text.lines().collect();
    // A cut in the middle of the file leaves a partial first line.
    let skip = usize::from(start > 0);
    let from = all.len().saturating_sub(lines).max(skip.min(all.len()));
    Some(all[from..].join("\n"))
}

pub fn log_tail(path: &Path) -> Option<String> {
    tail(path, LOG_TAIL_LINES)
}

/// Hides the Windows user name in paths before the text leaves this PC.
pub fn anonymize(text: &str, profile: Option<&str>, user: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(p) = profile.filter(|p| p.len() > 3) {
        out = replace_ignore_case(&out, p, "%USERPROFILE%");
        out = replace_ignore_case(&out, &p.replace('\\', "/"), "%USERPROFILE%");
    }
    if let Some(u) = user.filter(|u| u.len() >= 3) {
        out = replace_ignore_case(&out, &format!("\\Users\\{u}"), "\\Users\\<usuário>");
    }
    out
}

fn replace_ignore_case(text: &str, from: &str, to: &str) -> String {
    let lower = text.to_lowercase();
    let needle = from.to_lowercase();
    // Lowercasing may change lengths outside ASCII; then only exact matches are replaced.
    if lower.len() != text.len() || needle.len() != from.len() {
        return text.replace(from, to);
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&needle) {
        out.push_str(&text[i..i + pos]);
        out.push_str(to);
        i += pos + needle.len();
    }
    out.push_str(&text[i..]);
    out
}

/// The text people paste into an issue.
pub fn report(game: &str, version: &str, d: &Diagnosis, pc: &str, logs: &[(String, String)]) -> String {
    let mut s = format!("Jogo: {game}\nVersão do port: {version}\n");
    if let Some(code) = &d.code {
        s += &format!("Erro: {code}{}\n", d.module.as_ref().map(|m| format!(" em {m}")).unwrap_or_default());
    }
    if let Some(sec) = d.seconds {
        s += &format!("Tempo aberto: {sec} s\n");
    }
    s += &format!("\n--- PC ---\n{pc}\n\n--- O que o RecompHUB achou ---\n");
    for f in &d.findings {
        s += &format!("- {}\n", f.text);
    }
    for (name, text) in logs {
        s += &format!("\n--- Fim de {name} ---\n{text}\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_from_powershell() {
        let one = r#"{"t":1792600000,"p":["SpartanPort.exe","2.6.0.0","00000000","nvoglv64.dll","31.0.15","00000000","c0000005","0000000000123abc","1a2b","01db","C:\\Games\\SpartanPort\\SpartanPort.exe","C:\\WINDOWS\\System32\\nvoglv64.dll","x",null]}"#;
        let ev = parse_events(one);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].code, 0xC000_0005);
        assert_eq!(ev[0].module, "nvoglv64.dll");
        let many = format!("[{one},{one}]");
        assert_eq!(parse_events(&many).len(), 2);
        assert!(parse_events("").is_empty());
        assert!(parse_events("not json").is_empty());
    }

    #[test]
    fn event_belongs_to_the_game() {
        let ev = vec![
            CrashEvent { at: 10, app_name: "other.exe".into(), app_path: "C:\\Other\\other.exe".into(), module: "x".into(), code: 1 },
            CrashEvent { at: 20, app_name: "Zelda64Recompiled.exe".into(), app_path: "c:\\games\\mm\\Zelda64Recompiled.exe".into(), module: "x".into(), code: 2 },
            CrashEvent { at: 30, app_name: "Zelda64Recompiled.exe".into(), app_path: "".into(), module: "x".into(), code: 3 },
        ];
        let names = vec!["Zelda64Recompiled.exe".to_string()];
        assert_eq!(newest_for(&ev, Path::new("C:\\Games\\MM"), &names).unwrap().code, 3);
        assert_eq!(newest_for(&ev, Path::new("C:\\Games\\MM"), &[]).unwrap().code, 2);
        assert!(newest_for(&ev, Path::new("C:\\Games\\MMX"), &[]).is_none());
    }

    #[test]
    fn crash_codes_are_explained() {
        let cpu = findings(&Context { code: Some(0xC000_001D), cpu_missing: vec!["avx".into()], ..Default::default() });
        assert_eq!(cpu[0].level, "bad");
        assert!(cpu[0].text.contains("AVX"));

        let dll = findings(&Context { code: Some(0xC000_0135), ..Default::default() });
        assert_eq!(dll[0].action.as_ref().unwrap().act, "link");

        let driver = findings(&Context {
            code: Some(0xC000_0005),
            module: Some("nvoglv64.dll".into()),
            n64_api: Some("D3D12".into()),
            driver_pages: vec![("NVIDIA", "https://n")],
            ..Default::default()
        });
        assert!(driver[0].text.contains("driver de vídeo"));
        assert_eq!(driver[0].action.as_ref().unwrap().value.as_deref(), Some("https://n"));
        assert_eq!(driver[1].action.as_ref().unwrap().act, "gfx-api");

        let overlay = findings(&Context { code: Some(0xC000_0005), module: Some("RTSSHooks64.dll".into()), ..Default::default() });
        assert!(overlay[0].text.contains("RivaTuner"));

        let unknown = findings(&Context { code: Some(0xC000_0409), module: Some("game.exe".into()), ..Default::default() });
        assert!(unknown[0].text.contains("0xC0000409"));

        let short = findings(&Context { seconds: Some(4), ..Default::default() });
        assert!(short[0].text.contains("4 segundos"));
    }

    #[test]
    fn context_findings() {
        let f = findings(&Context {
            code: Some(0xC000_0005),
            driver_changed: true,
            has_shader_cache: true,
            n64_api: Some("Auto".into()),
            gpus: 2,
            mods_on: 3,
            dumps: 1,
            has_logs: true,
            ..Default::default()
        });
        let acts: Vec<&str> = f.iter().filter_map(|x| x.action.as_ref().map(|a| a.act)).collect();
        assert_eq!(acts, ["shader-clear", "gfx-api", "open-dumps", "open-logs"]);
        assert!(f.iter().any(|x| x.text.contains("3 mods ligados")));
    }

    #[test]
    fn crash_or_quit() {
        assert!(is_crash_code(0xC000_0005));
        assert!(is_crash_code(0xE06D_7363));
        assert!(!is_crash_code(0));
        assert!(!is_crash_code(1));
    }

    #[test]
    fn user_name_is_hidden() {
        let t = "C:\\Users\\Gustavo\\AppData\\Local\\x.log and c:/users/gustavo/y";
        let out = anonymize(t, Some("C:\\Users\\Gustavo"), Some("Gustavo"));
        assert!(!out.to_lowercase().contains("gustavo"), "{out}");
        assert!(out.contains("%USERPROFILE%\\AppData"));
    }

    #[test]
    fn tail_reads_the_end() {
        let dir = std::env::temp_dir().join(format!("recomphub-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.log");
        let text: String = (0..100).map(|i| format!("linha {i}\n")).collect();
        std::fs::write(&path, text).unwrap();
        let t = tail(&path, 3).unwrap();
        assert_eq!(t, "linha 97\nlinha 98\nlinha 99");
        let big: String = (0..20000).map(|i| format!("linha {i}\n")).collect();
        std::fs::write(&path, big).unwrap();
        assert!(tail(&path, 2).unwrap().ends_with("linha 19999"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
