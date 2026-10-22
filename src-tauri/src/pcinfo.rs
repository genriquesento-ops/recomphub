//! "Seu PC": what RecompHUB can tell about this computer without administrator, to warn before
//! a port that cannot run here (the most common reasons a port "does not open", from the
//! ports' own issue trackers) and to fill problem reports.

use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::sync::Mutex;

use crate::reg::{self, Root};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// "NVIDIA", "AMD", "Intel" or "".
    pub vendor: &'static str,
    pub driver_version: String,
    /// Unix time of the driver's date, when Windows gives one.
    pub driver_date: Option<u64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PcInfo {
    /// "Windows 11 24H2 (build 26100)".
    pub windows: String,
    pub build: u32,
    pub cpu: String,
    /// Instruction sets found, from `CPU_FEATURES`.
    pub cpu_features: Vec<&'static str>,
    pub ram_gb: f64,
    pub gpus: Vec<Gpu>,
    /// The Visual C++ 2015-2022 runtime (x64) many ports need.
    pub vc_runtime: bool,
}

/// Instruction sets a catalog port may require (catalog field `cpu_features`).
pub const CPU_FEATURES: &[&str] = &["sse4.1", "sse4.2", "popcnt", "avx", "avx2", "fma", "f16c", "bmi1", "bmi2", "lzcnt", "movbe"];

#[allow(unused_unsafe)]
pub fn cpu_features() -> Vec<&'static str> {
    use std::arch::x86_64::{CpuidResult, __cpuid, __cpuid_count};
    const NONE: CpuidResult = CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 };
    let bit = |reg: u32, n: u32| reg & (1 << n) != 0;
    // SAFETY: CPUID exists on every x86-64 processor; leaves are checked before use.
    let (max, ext_max) = unsafe { (__cpuid(0).eax, __cpuid(0x8000_0000).eax) };
    let l1 = unsafe { __cpuid(1) };
    let l7 = if max >= 7 { unsafe { __cpuid_count(7, 0) } } else { NONE };
    let e1 = if ext_max >= 0x8000_0001 { unsafe { __cpuid(0x8000_0001) } } else { NONE };
    // AVX-family instructions also need the operating system to save the AVX registers
    // (OSXSAVE set and XCR0 bits 1 and 2), or using them crashes like a missing instruction.
    let os_avx = bit(l1.ecx, 27) && unsafe { std::arch::x86_64::_xgetbv(0) } & 0b110 == 0b110;
    let found = [
        ("sse4.1", bit(l1.ecx, 19)),
        ("sse4.2", bit(l1.ecx, 20)),
        ("popcnt", bit(l1.ecx, 23)),
        ("avx", os_avx && bit(l1.ecx, 28)),
        ("avx2", os_avx && bit(l7.ebx, 5)),
        ("fma", os_avx && bit(l1.ecx, 12)),
        ("f16c", os_avx && bit(l1.ecx, 29)),
        ("bmi1", bit(l7.ebx, 3)),
        ("bmi2", bit(l7.ebx, 8)),
        ("lzcnt", bit(e1.ecx, 5)),
        ("movbe", bit(l1.ecx, 22)),
    ];
    found.iter().filter(|(_, on)| *on).map(|(n, _)| *n).collect()
}

fn windows() -> (String, u32) {
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build: u32 = reg::get_string(Root::Machine, KEY, "CurrentBuild").and_then(|b| b.parse().ok()).unwrap_or(0);
    // ProductName still says "Windows 10" on Windows 11; the build tells them apart.
    let name = if build >= 22000 { "Windows 11" } else { "Windows 10" };
    let version = reg::get_string(Root::Machine, KEY, "DisplayVersion").unwrap_or_default();
    (format!("{name} {version} (build {build})").replace("  ", " "), build)
}

fn cpu_name() -> String {
    reg::get_string(Root::Machine, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString")
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| "Processador desconhecido".into())
}

fn vc_runtime() -> bool {
    let sys = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into()).join("System32");
    ["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"].iter().all(|f| sys.join(f).is_file())
}

pub fn vendor(pnp_id: &str, name: &str) -> &'static str {
    let id = pnp_id.to_uppercase();
    let n = name.to_uppercase();
    if id.contains("VEN_10DE") || n.contains("NVIDIA") {
        "NVIDIA"
    } else if id.contains("VEN_1002") || n.contains("AMD") || n.contains("RADEON") {
        "AMD"
    } else if id.contains("VEN_8086") || n.contains("INTEL") {
        "Intel"
    } else {
        ""
    }
}

/// "/Date(1696118400000)/" (how Windows PowerShell writes dates in JSON) to Unix time.
pub fn json_date(s: &str) -> Option<u64> {
    let digits: String = s.trim_start_matches("/Date(").chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u64>().ok().map(|ms| ms / 1000).filter(|&t| t > 0)
}

/// The output of `Get-CimInstance Win32_VideoController | ConvertTo-Json` (one object or a
/// list). The "Microsoft Basic Display Adapter" means no real driver and is kept, so the
/// check can say so.
pub fn parse_gpus(json: &str) -> Vec<Gpu> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).unwrap_or(serde_json::Value::Null);
    let list = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(_) => vec![v],
        _ => Vec::new(),
    };
    let s = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    list.iter()
        .filter(|o| !s(o, "Name").is_empty())
        .map(|o| Gpu {
            name: s(o, "Name"),
            vendor: vendor(&s(o, "PNPDeviceID"), &s(o, "Name")),
            driver_version: s(o, "DriverVersion"),
            driver_date: json_date(&s(o, "DriverDate")),
        })
        .collect()
}

fn gpus() -> Vec<Gpu> {
    const SCRIPT: &str = "Get-CimInstance Win32_VideoController | Select-Object Name,PNPDeviceID,DriverVersion,DriverDate | ConvertTo-Json -Compress";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .map(|o| parse_gpus(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

static CACHE: Mutex<Option<PcInfo>> = Mutex::new(None);

/// Read once per run (the video card query takes about a second); `refresh` reads again.
pub fn get(refresh: bool) -> PcInfo {
    if !refresh {
        if let Some(info) = CACHE.lock().unwrap().clone() {
            return info;
        }
    }
    let (windows, build) = windows();
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let info = PcInfo {
        windows,
        build,
        cpu: cpu_name(),
        cpu_features: cpu_features(),
        ram_gb: (sys.total_memory() as f64 / 1_073_741_824.0 * 10.0).round() / 10.0,
        gpus: gpus(),
        vc_runtime: vc_runtime(),
    };
    *CACHE.lock().unwrap() = Some(info.clone());
    info
}

// ---- What the PC means for a port ----

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Finding {
    /// "bad": the port will not run; "warn": a common cause of trouble; "ok".
    pub level: &'static str,
    pub text: String,
    /// A page that solves it (driver or runtime download page).
    pub link: Option<(String, String)>,
}

const DRIVER_OLD_DAYS: u64 = 730;
pub const VC_PAGE: &str = "https://learn.microsoft.com/pt-br/cpp/windows/latest-supported-vc-redist";

pub fn driver_page(vendor: &str) -> Option<&'static str> {
    match vendor {
        "NVIDIA" => Some("https://www.nvidia.com/pt-br/drivers/"),
        "AMD" => Some("https://www.amd.com/pt/support/download/drivers.html"),
        "Intel" => Some("https://www.intel.com.br/content/www/br/pt/support/detect.html"),
        _ => None,
    }
}

/// Screens made by software (remote access, streaming, extra monitors): their "driver" says
/// nothing about the games, and they are never the card a game runs on.
fn is_virtual(g: &Gpu) -> bool {
    const NAMES: &[&str] = &[
        "parsec", "virtual display", "remote display", "meta virtual", "spacedesk", "iddsample", "indirect display",
        "vmware svga", "virtualbox", "hyper-v video", "citrix",
    ];
    let n = g.name.to_lowercase();
    NAMES.iter().any(|v| n.contains(v))
}

/// A real video card with its driver (not Windows' basic adapter nor a virtual display).
pub fn is_card(g: &Gpu) -> bool {
    !is_basic(g) && !is_virtual(g)
}

fn is_basic(g: &Gpu) -> bool {
    let n = g.name.to_lowercase();
    n.contains("basic display") || n.contains("basic render") || n.contains("vídeo básico") || n.contains("video basico")
}

/// What stands between this PC and the port. `required` are the port's instruction sets
/// (empty for ports without a known requirement).
pub fn check(info: &PcInfo, required: &[String], now: u64) -> Vec<Finding> {
    let mut out = Vec::new();
    let missing: Vec<&str> = required.iter().map(String::as_str).filter(|f| !info.cpu_features.contains(f)).collect();
    if !missing.is_empty() {
        out.push(Finding {
            level: "bad",
            text: format!(
                "Seu processador ({}) não tem as instruções {} que este port usa. O jogo fecha logo ao abrir (erro 0xc000001d, \"instrução ilegal\"), e não existe ajuste que resolva: só outro processador.",
                info.cpu,
                missing.iter().map(|m| m.to_uppercase()).collect::<Vec<_>>().join(", ")
            ),
            link: None,
        });
    }
    let real: Vec<&Gpu> = info.gpus.iter().filter(|g| is_card(g)).collect();
    if real.is_empty() {
        out.push(Finding {
            level: "bad",
            text: if info.gpus.iter().all(is_virtual) {
                "Não encontrei nenhuma placa de vídeo. Os ports precisam de uma placa com DirectX 12 ou Vulkan e o driver dela instalado.".into()
            } else {
                "O Windows está usando o \"Adaptador de vídeo básico\": o driver da sua placa de vídeo não está instalado. Instale o driver no site do fabricante da placa ou do notebook.".into()
            },
            link: None,
        });
    }
    for g in &real {
        let Some(date) = g.driver_date else { continue };
        let days = now.saturating_sub(date) / 86_400;
        if days > DRIVER_OLD_DAYS {
            out.push(Finding {
                level: "warn",
                text: format!(
                    "O driver da {} tem {} anos. Driver antigo é a causa mais comum de port que não abre ou mostra tela preta: vale atualizar.",
                    g.name,
                    days / 365
                ),
                link: driver_page(g.vendor).map(|u| (format!("Baixar driver {}", g.vendor), u.to_string())),
            });
        }
    }
    if !info.vc_runtime {
        out.push(Finding {
            level: "warn",
            text: "O Visual C++ 2015-2022 (x64) da Microsoft não está instalado. Muitos ports precisam dele para abrir.".into(),
            link: Some(("Página da Microsoft".into(), VC_PAGE.into())),
        });
    }
    if info.build > 0 && info.build < 19045 {
        out.push(Finding {
            level: "warn",
            text: format!("Seu Windows ({}) está desatualizado. Os ports são testados no Windows 10 22H2 e no Windows 11; atualize pelo Windows Update.", info.windows),
            link: None,
        });
    }
    if out.is_empty() {
        out.push(Finding { level: "ok", text: "Nada no seu PC impede este port de rodar.".into(), link: None });
    }
    out
}

// ---- Graphics card per game (Windows' own "Configurações de gráficos") ----

const GPU_PREF_KEY: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";

/// 0 = Windows decides, 1 = power saving, 2 = high performance; the same setting as
/// Configurações > Sistema > Tela > Gráficos, stored per program path.
pub fn gpu_preference(exe: &std::path::Path) -> u8 {
    reg::get_string(Root::User, GPU_PREF_KEY, &exe.display().to_string())
        .and_then(|v| parse_gpu_preference(&v))
        .unwrap_or(0)
}

pub fn parse_gpu_preference(v: &str) -> Option<u8> {
    v.split(';').find_map(|part| part.trim().strip_prefix("GpuPreference=")).and_then(|n| n.parse().ok()).filter(|n| *n <= 2)
}

pub fn set_gpu_preference(exe: &std::path::Path, pref: u8) -> Result<(), String> {
    let name = exe.display().to_string();
    match pref {
        0 => reg::delete_user_value(GPU_PREF_KEY, &name),
        1 | 2 => reg::set_user_string(GPU_PREF_KEY, &name, &format!("GpuPreference={pref};")),
        _ => Err("Opção inválida.".into()),
    }
}

/// The video cards and their driver versions, to notice a driver update.
pub fn driver_signature(info: &PcInfo) -> String {
    info.gpus.iter().filter(|g| is_card(g)).map(|g| format!("{}={}", g.name, g.driver_version)).collect::<Vec<_>>().join(";")
}

/// Plain text for "Copiar informações do PC" (problem reports).
pub fn report(info: &PcInfo) -> String {
    let gpus = info
        .gpus
        .iter()
        .map(|g| format!("{} (driver {})", g.name, if g.driver_version.is_empty() { "?" } else { &g.driver_version }))
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "Sistema: {}\nProcessador: {}\nInstruções: {}\nMemória: {} GB\nPlaca de vídeo: {}\nVisual C++: {}\nRecompHUB: {}",
        info.windows,
        info.cpu,
        info.cpu_features.join(", "),
        info.ram_gb,
        if gpus.is_empty() { "não encontrada".into() } else { gpus },
        if info.vc_runtime { "instalado" } else { "não encontrado" },
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc(features: &[&'static str], gpus: Vec<Gpu>) -> PcInfo {
        PcInfo {
            windows: "Windows 11 24H2 (build 26100)".into(),
            build: 26100,
            cpu: "Intel Xeon E3-1230 v2".into(),
            cpu_features: features.to_vec(),
            ram_gb: 16.0,
            gpus,
            vc_runtime: true,
        }
    }

    fn gpu(name: &str, date: Option<u64>) -> Gpu {
        Gpu { name: name.into(), vendor: vendor("", name), driver_version: "1".into(), driver_date: date }
    }

    #[test]
    fn video_cards_from_powershell() {
        let one = r#"{"Name":"NVIDIA GeForce RTX 3060","PNPDeviceID":"PCI\\VEN_10DE&DEV_2503","DriverVersion":"32.0.15.6094","DriverDate":"\/Date(1727740800000)\/"}"#;
        let g = parse_gpus(one);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].vendor, "NVIDIA");
        assert_eq!(g[0].driver_date, Some(1_727_740_800));
        let two = r#"[{"Name":"Intel(R) UHD Graphics 620","PNPDeviceID":"PCI\\VEN_8086&DEV_5917"},{"Name":"AMD Radeon RX 6600","PNPDeviceID":"PCI\\VEN_1002&DEV_73FF"}]"#;
        let g = parse_gpus(two);
        assert_eq!((g[0].vendor, g[1].vendor), ("Intel", "AMD"));
        assert!(parse_gpus("").is_empty());
    }

    #[test]
    fn missing_instructions_block_the_port() {
        let need: Vec<String> = ["avx", "movbe", "bmi1", "lzcnt", "f16c"].iter().map(|s| s.to_string()).collect();
        let now = 1_800_000_000;
        let ivy = pc(&["sse4.1", "sse4.2", "popcnt", "avx", "f16c"], vec![gpu("NVIDIA GeForce GTX 1060", Some(now - 86_400))]);
        let f = check(&ivy, &need, now);
        assert_eq!(f[0].level, "bad");
        assert!(f[0].text.contains("MOVBE, BMI1, LZCNT"));
        let modern = pc(&["avx", "avx2", "movbe", "bmi1", "lzcnt", "f16c"], vec![gpu("NVIDIA GeForce RTX 4060", Some(now - 86_400))]);
        assert_eq!(check(&modern, &need, now)[0].level, "ok");
    }

    #[test]
    fn old_driver_and_basic_adapter_are_warned() {
        let now = 1_800_000_000;
        let old = pc(&[], vec![gpu("AMD Radeon RX 580", Some(now - 3 * 365 * 86_400))]);
        let f = check(&old, &[], now);
        assert_eq!(f[0].level, "warn");
        assert_eq!(f[0].link.as_ref().unwrap().1, "https://www.amd.com/pt/support/download/drivers.html");
        let basic = pc(&[], vec![gpu("Microsoft Basic Display Adapter", None)]);
        assert_eq!(check(&basic, &[], now)[0].level, "bad");
    }

    #[test]
    fn virtual_displays_are_not_cards() {
        let now = 1_800_000_000;
        let old = Some(now - 900 * 86_400);
        let pc = pc(&["avx"], vec![gpu("Parsec Virtual Display Adapter", old), gpu("NVIDIA GeForce RTX 3060", Some(now))]);
        assert!(check(&pc, &[], now).iter().all(|f| f.level == "ok"));
        assert_eq!(driver_signature(&pc).matches('=').count(), 1);
    }

    #[test]
    fn gpu_preference_values() {
        assert_eq!(parse_gpu_preference("GpuPreference=2;"), Some(2));
        assert_eq!(parse_gpu_preference("SwapEffectUpgradeEnable=1;GpuPreference=1;"), Some(1));
        assert_eq!(parse_gpu_preference("GpuPreference=9;"), None);
        assert_eq!(parse_gpu_preference(""), None);
    }

    #[test]
    fn this_cpu_is_read() {
        // Every x86-64 runner has at least SSE4.2 and POPCNT nowadays; the list must be a
        // subset of the known names.
        let f = cpu_features();
        assert!(f.iter().all(|x| CPU_FEATURES.contains(x)));
    }
}
