//! Contador de FPS: a small number over the game, measured by Intel's PresentMon (MIT,
//! github.com/GameTechDev/PresentMon), the tool FPS counters use underneath. PresentMon reads
//! the frames Windows presents (ETW), so nothing is injected into the game and it works with
//! DirectX and Vulkan alike.
//!
//! Reading those events needs administrator or the "Performance Log Users" group. The player
//! chose to grant it once: an elevated copy of RecompHUB adds their account to the group, which
//! counts from the next sign-in.

use serde::Serialize;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

const REPO: &str = "GameTechDev/PresentMon";
const DOWNLOAD_PREFIX: &str = "https://github.com/GameTechDev/PresentMon/releases/download/";
/// Well-known SID of the built-in "Performance Log Users" group (its name changes with the
/// Windows language).
const PERF_LOG_USERS: &str = "S-1-5-32-559";
pub const GRANT_FLAG: &str = "--liberar-contador-fps";
const SESSION: &str = "RecompHUB-FPS";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_TOOL_SIZE: u64 = 32 * 1024 * 1024;

pub fn tool_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_local_data_dir().ok().map(|d| d.join("ferramentas").join("PresentMon.exe"))
}

// ---- Permission ----

fn whoami(args: &[&str]) -> Option<String> {
    Command::new("whoami")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
}

/// The account is in "Performance Log Users" in this sign-in (a membership added now only
/// counts after signing out and in again).
pub fn has_permission() -> bool {
    whoami(&["/groups", "/fo", "csv", "/nh"]).map(|s| s.contains(PERF_LOG_USERS)).unwrap_or(false)
}

pub fn valid_sid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    s.len() <= 184
        && parts.len() >= 4
        && parts.len() <= 15
        && parts[0] == "S"
        && parts[1] == "1"
        && parts[2..].iter().all(|p| !p.is_empty() && p.len() <= 10 && p.chars().all(|c| c.is_ascii_digit()))
}

/// The SID in `whoami /user /fo csv /nh`: "pc\\nome","S-1-5-21-...".
pub fn parse_user_sid(csv: &str) -> Option<String> {
    let sid = csv.trim().rsplit(',').next()?.trim().trim_matches('"').to_string();
    valid_sid(&sid).then_some(sid)
}

/// Runs in the elevated copy of RecompHUB: adds `sid` to "Performance Log Users". Returns the
/// process exit code (0 = done or already a member).
pub fn grant(sid: &str) -> i32 {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::NetworkManagement::NetManagement::{NetLocalGroupAddMembers, LOCALGROUP_MEMBERS_INFO_0};
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{LookupAccountSidW, PSID, SID_NAME_USE};
    const ERROR_MEMBER_IN_ALIAS: u32 = 1378;
    if !valid_sid(sid) {
        return 2;
    }
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    // SAFETY: every buffer passed is sized as the call expects, and the SIDs made by
    // ConvertStringSidToSidW are freed with LocalFree.
    unsafe {
        let mut group_sid: PSID = std::ptr::null_mut();
        let mut user_sid: PSID = std::ptr::null_mut();
        if ConvertStringSidToSidW(wide(PERF_LOG_USERS).as_ptr(), &mut group_sid) == 0 {
            return 3;
        }
        if ConvertStringSidToSidW(wide(sid).as_ptr(), &mut user_sid) == 0 {
            LocalFree(group_sid as _);
            return 2;
        }
        // The group's name in this Windows language.
        let mut name = vec![0u16; 256];
        let mut domain = vec![0u16; 256];
        let (mut n, mut d) = (name.len() as u32, domain.len() as u32);
        let mut kind: SID_NAME_USE = 0;
        let found = LookupAccountSidW(std::ptr::null(), group_sid, name.as_mut_ptr(), &mut n, domain.as_mut_ptr(), &mut d, &mut kind) != 0;
        let code = if found {
            let member = LOCALGROUP_MEMBERS_INFO_0 { lgrmi0_sid: user_sid };
            let r = NetLocalGroupAddMembers(std::ptr::null(), name.as_ptr(), 0, (&member as *const LOCALGROUP_MEMBERS_INFO_0).cast(), 1);
            if r == 0 || r == ERROR_MEMBER_IN_ALIAS {
                0
            } else {
                r as i32
            }
        } else {
            3
        };
        LocalFree(group_sid as _);
        LocalFree(user_sid as _);
        code
    }
}

/// Asks Windows for administrator once (UAC) and waits for the elevated copy to finish.
pub fn request_permission() -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let sid = whoami(&["/user", "/fo", "csv", "/nh"]).and_then(|s| parse_user_sid(&s)).ok_or("Não consegui identificar a sua conta do Windows.")?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let wide = |s: &std::ffi::OsStr| {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>()
    };
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(me.as_os_str());
    let params = wide(std::ffi::OsStr::new(&format!("{GRANT_FLAG} {sid}")));
    // SAFETY: the strings are NUL-terminated and live until the call returns; the process
    // handle is waited on and closed.
    let (ok, code) = unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 {
            (false, 0)
        } else {
            let mut code: u32 = 1;
            if !info.hProcess.is_null() {
                WaitForSingleObject(info.hProcess, INFINITE);
                GetExitCodeProcess(info.hProcess, &mut code);
                CloseHandle(info.hProcess);
            }
            (true, code)
        }
    };
    if !ok {
        let err = std::io::Error::last_os_error();
        return Err(if err.raw_os_error() == Some(1223) {
            "Você não deu a permissão de administrador, então nada mudou.".into()
        } else {
            format!("O Windows não pediu a permissão: {err}")
        });
    }
    if code != 0 {
        return Err(format!("O Windows não deixou colocar sua conta no grupo (código {code})."));
    }
    Ok(())
}

// ---- The tool ----

#[derive(serde::Deserialize)]
struct Rel {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// "PresentMon-2.6.0-x64.exe", the console application of a release.
pub fn is_console_asset(name: &str) -> bool {
    name.strip_prefix("PresentMon-")
        .and_then(|r| r.strip_suffix("-x64.exe"))
        .map(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .unwrap_or(false)
}

/// The file must carry Intel's valid digital signature, so a changed file is never run.
fn verify_signature(path: &Path) -> Result<(), String> {
    const SCRIPT: &str = "$s = Get-AuthenticodeSignature -LiteralPath $env:RH_FILE; \"$($s.Status)|$($s.SignerCertificate.Subject)\"";
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("RH_FILE", path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().starts_with("Valid|CN=Intel Corporation,") {
        Ok(())
    } else {
        Err("O arquivo baixado não tem a assinatura digital da Intel, então não foi usado.".into())
    }
}

/// Downloads the newest PresentMon console application from Intel's GitHub releases.
pub async fn download(app: &AppHandle) -> Result<String, String> {
    let path = tool_path(app).ok_or("Pasta de dados indisponível.")?;
    let resp = crate::github::api(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .send()
        .await
        .map_err(|e| format!("Sem conexão com o GitHub ({e})"))?;
    if !resp.status().is_success() {
        return Err(crate::github::status_error(resp.status()));
    }
    let rel: Rel = resp.json().await.map_err(|e| e.to_string())?;
    let asset = rel
        .assets
        .iter()
        .find(|a| is_console_asset(&a.name) && a.browser_download_url.starts_with(DOWNLOAD_PREFIX) && a.size < MAX_TOOL_SIZE)
        .ok_or("A versão mais nova do PresentMon não tem o programa para Windows 64 bits.")?;
    let bytes = crate::github::client()
        .get(&asset.browser_download_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("O download falhou: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("O download foi interrompido: {e}"))?;
    if bytes.len() as u64 > MAX_TOOL_SIZE {
        return Err("O arquivo baixado é grande demais.".into());
    }
    let dir = path.parent().ok_or("Pasta inválida.")?.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join("PresentMon.baixando.exe");
    std::fs::write(&tmp, &bytes).map_err(|e| crate::play::antivirus_error(&e).unwrap_or_else(|| e.to_string()))?;
    let tmp2 = tmp.clone();
    let checked = tauri::async_runtime::spawn_blocking(move || verify_signature(&tmp2)).await.map_err(|e| e.to_string())?;
    if let Err(e) = checked {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(rel.tag_name)
}

// ---- Measuring ----

/// Turns PresentMon's CSV lines into frames per second, about twice a second.
#[derive(Default)]
pub struct Meter {
    column: Option<usize>,
    frames: u32,
    ms: f64,
}

impl Meter {
    pub fn feed(&mut self, line: &str) -> Option<u32> {
        let fields: Vec<&str> = line.trim().split(',').collect();
        let Some(col) = self.column else {
            self.column = fields.iter().position(|f| f.trim_matches('"').eq_ignore_ascii_case("msBetweenPresents"));
            return None;
        };
        let ms: f64 = fields.get(col)?.trim().parse().ok()?;
        if !(ms > 0.0 && ms < 1000.0) {
            return None;
        }
        self.frames += 1;
        self.ms += ms;
        if self.ms < 500.0 {
            return None;
        }
        let fps = (self.frames as f64 * 1000.0 / self.ms).round() as u32;
        self.frames = 0;
        self.ms = 0.0;
        Some(fps)
    }
}

struct Capture {
    id: String,
    child: Child,
}

static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);
/// One start at a time (two games can start in the same watcher look).
static STARTING: Mutex<()> = Mutex::new(());
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().unwrap().clone()
}

fn close_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("fps") {
        let _ = w.close();
    }
}

fn open_window(app: &AppHandle) {
    if app.get_webview_window("fps").is_some() {
        return;
    }
    let built = tauri::WebviewWindowBuilder::new(app, "fps", tauri::WebviewUrl::App("fps.html".into()))
        .title("FPS")
        .inner_size(120.0, 52.0)
        .position(14.0, 14.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        // Same as the main window (tauri.conf.json): WebView2 needs the same arguments for all.
        .additional_browser_args("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required")
        .build();
    if let Ok(w) = built {
        // Clicks go through to the game.
        let _ = w.set_ignore_cursor_events(true);
    }
}

/// A game started: measure it, when the counter is on and ready. `exe_name` is its program.
pub fn start(app: &AppHandle, id: &str, exe_name: &str) {
    let enabled = app.state::<crate::AppState>().settings.lock().unwrap().fps_overlay;
    let Some(tool) = tool_path(app).filter(|p| p.is_file()) else { return };
    if !enabled || exe_name.is_empty() || !has_permission() {
        return;
    }
    let _one = STARTING.lock().unwrap();
    stop(app, None);
    let spawned = Command::new(&tool)
        .args([
            "--process_name",
            exe_name,
            "--output_stdout",
            "--no_console_stats",
            "--v1_metrics",
            "--session_name",
            SESSION,
            "--stop_existing_session",
            "--terminate_on_proc_exit",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            *LAST_ERROR.lock().unwrap() = Some(crate::play::antivirus_error(&e).unwrap_or_else(|| format!("O PresentMon não abriu: {e}")));
            return;
        }
    };
    *LAST_ERROR.lock().unwrap() = None;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // Anything still in the slot is ended rather than left running.
    if let Some(mut old) = CAPTURE.lock().unwrap().replace(Capture { id: id.to_string(), child }) {
        let _ = old.child.kill();
        let _ = old.child.wait();
    }

    if let Some(mut err) = stderr {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            let lower = text.to_lowercase();
            if lower.contains("access denied") || lower.contains("acesso negado") {
                *LAST_ERROR.lock().unwrap() =
                    Some("O Windows negou a medição. Saia da sua conta do Windows e entre de novo para a permissão valer.".into());
            } else if lower.contains("error") || lower.contains("failed") {
                let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(200).collect::<String>();
                *LAST_ERROR.lock().unwrap() = Some(format!("O PresentMon parou com um erro: {first}"));
            }
        });
    }
    let Some(out) = stdout else { return };
    let app = app.clone();
    let id = id.to_string();
    std::thread::spawn(move || {
        let mut meter = Meter::default();
        let mut shown = false;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(fps) = meter.feed(&line) {
                if !shown {
                    open_window(&app);
                    shown = true;
                }
                let _ = app.emit_to("fps", "fps", fps);
            }
        }
        // PresentMon ended (the game closed or it was stopped) or its output broke. Only this
        // capture is cleaned up, and its process is ended in case it is still running.
        let taken = {
            let mut cap = CAPTURE.lock().unwrap();
            if cap.as_ref().map(|c| c.id == id).unwrap_or(false) { cap.take() } else { None }
        };
        if let Some(mut c) = taken {
            let _ = c.child.kill();
            let _ = c.child.wait();
            close_window(&app);
        }
    });
}

/// Stops measuring: `id` = only that game's capture, None = any.
pub fn stop(app: &AppHandle, id: Option<&str>) {
    let taken = {
        let mut cap = CAPTURE.lock().unwrap();
        if cap.as_ref().map(|c| id.map(|i| c.id == i).unwrap_or(true)).unwrap_or(false) {
            cap.take()
        } else {
            None
        }
    };
    if let Some(mut c) = taken {
        let _ = c.child.kill();
        let _ = c.child.wait();
        close_window(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_from_presentmon_lines() {
        let mut m = Meter::default();
        assert_eq!(m.feed("Application,ProcessID,SwapChainAddress,Runtime,SyncInterval,PresentFlags,Dropped,TimeInSeconds,msInPresentAPI,msBetweenPresents"), None);
        let row = |ms: &str| format!("Zelda64Recompiled.exe,1234,0x1,DXGI,1,0,0,1.5,0.2,{ms}");
        let mut got = None;
        for _ in 0..30 {
            got = got.or(m.feed(&row("16.667")));
        }
        assert_eq!(got, Some(60));
        // Garbage and absurd values are skipped.
        assert_eq!(m.feed(&row("NA")), None);
        assert_eq!(m.feed(&row("5000")), None);
    }

    #[test]
    fn sids_and_assets() {
        assert_eq!(parse_user_sid("\"pc\\gustavo\",\"S-1-5-21-1004336348-1177238915-682003330-1001\"\r\n").as_deref(), Some("S-1-5-21-1004336348-1177238915-682003330-1001"));
        assert!(!valid_sid("S-1-5-21-1 & calc"));
        assert!(!valid_sid("S-1-5-21--1"));
        assert!(is_console_asset("PresentMon-2.6.0-x64.exe"));
        assert!(!is_console_asset("PresentMon-2.6.0.msi"));
        assert!(!is_console_asset("PresentMon--x64.exe"));
    }
}
