//! "Outros jogos de PC": RecompHUB does not know where they keep their saves, but Ludusavi
//! (mtkennerly/ludusavi, MIT) knows more than 19 000 games. When the player has Ludusavi,
//! RecompHUB asks it, through its JSON command line (`--api`), to find the game and back it
//! up into Ludusavi's own backup folder. RecompHUB never downloads or installs Ludusavi.

use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const PROJECT_PAGE: &str = "https://github.com/mtkennerly/ludusavi";

/// The player's choice first, then where winget and scoop put it, then PATH.
pub fn locate(chosen: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !chosen.trim().is_empty() {
        candidates.push(PathBuf::from(chosen.trim()));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(&local).join("Microsoft").join("WinGet").join("Links").join("ludusavi.exe"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        candidates.push(PathBuf::from(home).join("scoop").join("shims").join("ludusavi.exe"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("ludusavi.exe")));
    }
    candidates.into_iter().find(|p| p.is_file() && valid_exe(p))
}

fn valid_exe(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("ludusavi.exe")).unwrap_or(false)
}

fn run(exe: &Path, args: &[&str]) -> Result<Value, String> {
    if !valid_exe(exe) {
        return Err("O programa escolhido não é o ludusavi.exe.".into());
    }
    let out = std::process::Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Não consegui abrir o Ludusavi: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("sem resposta");
        return Err(format!("O Ludusavi não respondeu: {first}"));
    }
    serde_json::from_str(&text).map_err(|_| "Resposta do Ludusavi ilegível.".to_string())
}

/// The first title in a `find --api` answer.
fn first_title(v: &Value) -> Option<String> {
    v["games"].as_object()?.keys().next().cloned()
}

/// The name Ludusavi knows the game by: the exact title first, then an approximate one.
pub fn find(exe: &Path, title: &str) -> Result<Option<String>, String> {
    if let Ok(v) = run(exe, &["find", "--api", "--backup", title]) {
        if let Some(t) = first_title(&v) {
            return Ok(Some(t));
        }
    }
    match run(exe, &["find", "--api", "--backup", "--normalized", title]) {
        Ok(v) => Ok(first_title(&v)),
        // `find` ends with an error when nothing matches.
        Err(_) => Ok(None),
    }
}

#[derive(serde::Serialize, Debug, PartialEq)]
pub struct Outcome {
    pub name: String,
    pub processed: bool,
    pub files: usize,
    pub bytes: u64,
    pub failed: bool,
}

fn outcome(v: &Value, name: &str) -> Outcome {
    let g = &v["games"][name];
    let files = g["files"].as_object();
    Outcome {
        name: name.to_string(),
        processed: g["decision"].as_str() == Some("Processed"),
        files: files.map(|f| f.len()).unwrap_or(0),
        bytes: files.map(|f| f.values().filter_map(|x| x["bytes"].as_u64()).sum()).unwrap_or(0),
        failed: v["errors"]["someGamesFailed"].as_bool().unwrap_or(false)
            || files.map(|f| f.values().any(|x| x["failed"].as_bool() == Some(true))).unwrap_or(false),
    }
}

/// `backup --force --api <name>`: into the backup folder set in Ludusavi.
pub fn backup(exe: &Path, name: &str) -> Result<Outcome, String> {
    let v = run(exe, &["backup", "--force", "--api", name])?;
    Ok(outcome(&v, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_backup_answer() {
        let v = json!({
            "errors": {},
            "overall": {"totalGames": 1, "processedGames": 1},
            "games": {"Hollow Knight": {
                "decision": "Processed",
                "files": {"C:/Users/x/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat": {"bytes": 100},
                          "C:/Users/x/AppData/LocalLow/Team Cherry/Hollow Knight/user2.dat": {"bytes": 50}},
                "registry": {}
            }}
        });
        assert_eq!(outcome(&v, "Hollow Knight"), Outcome { name: "Hollow Knight".into(), processed: true, files: 2, bytes: 150, failed: false });
        assert_eq!(first_title(&json!({"games": {"Celeste": {}}})).as_deref(), Some("Celeste"));
        assert_eq!(first_title(&json!({"errors": {}})), None);
    }

    #[test]
    fn only_ludusavi_itself_is_run() {
        assert!(valid_exe(Path::new(r"C:\Tools\ludusavi.exe")));
        assert!(valid_exe(Path::new(r"C:\Tools\Ludusavi.EXE")));
        assert!(!valid_exe(Path::new(r"C:\Tools\outro.exe")));
    }
}
