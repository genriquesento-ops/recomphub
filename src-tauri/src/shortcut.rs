//! Desktop and Start Menu shortcuts. As in Hydra, a shortcut opens the game through the
//! launcher ("RecompHUB.exe --play <id>"), so the session is still timed.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// The values travel in environment variables, so no path needs quoting inside the script.
const SCRIPT: &str = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:RH_LNK); \
$s.TargetPath = $env:RH_TARGET; $s.Arguments = $env:RH_ARGS; \
$s.WorkingDirectory = $env:RH_WORKDIR; $s.IconLocation = $env:RH_ICON; \
$s.Description = $env:RH_DESC; $s.Save()";

/// Characters Windows does not allow in file names.
fn file_name(title: &str) -> String {
    title.chars().filter(|c| !r#"<>:"/\|?*"#.contains(*c)).collect::<String>().trim().to_string()
}

pub fn create(folder: &Path, title: &str, id: &str, icon: &Path) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let lnk = folder.join(format!("{}.lnk", file_name(title)));
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("RH_LNK", &lnk)
        .env("RH_TARGET", &exe)
        .env("RH_ARGS", format!("--play {id}"))
        .env("RH_WORKDIR", exe.parent().unwrap_or(Path::new(".")))
        .env("RH_ICON", format!("{},0", icon.display()))
        .env("RH_DESC", format!("{title} (RecompHUB)"))
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("Não consegui criar o atalho: {e}"))?;
    if !status.success() || !lnk.is_file() {
        return Err("O Windows não criou o atalho.".into());
    }
    Ok(lnk)
}

#[cfg(test)]
mod tests {
    #[test]
    fn titles_become_valid_file_names() {
        assert_eq!(super::file_name("The Legend of Zelda: Majora's Mask"), "The Legend of Zelda Majora's Mask");
        assert_eq!(super::file_name(r" a/b\c?*<>| "), "abc");
    }
}
