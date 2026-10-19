//! The games RecompHUB knows. Each entry describes where the port is published, how to
//! recognise an installed copy and what the player has to provide from their own copy.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// God of War III Remastered (gow3_pc): the player provides a PS4 dump folder.
    Gow3,
    /// Banjo-Kazooie (Banjo: Recompiled): the player provides a ROM file.
    Banjo,
}

pub struct GameDef {
    pub id: &'static str,
    pub kind: Kind,
    pub title: &'static str,
    pub family: &'static str,
    pub port_name: &'static str,
    pub port_authors: &'static str,
    pub project_url: &'static str,
    /// GitHub "owner/name" of the official project.
    pub repo: &'static str,
    /// The Windows asset of a release: its name starts and ends with these.
    pub asset_prefix: &'static str,
    pub asset_suffix: &'static str,
    /// Started by the Play button (relative to the install folder).
    pub exe: &'static str,
    /// Icon for shortcuts (relative to the install folder; an .exe uses its own icon).
    pub icon: &'static str,
    /// Files whose presence identifies an installed copy of this port.
    pub markers: &'static [&'static str],
    /// Files kept from the old copy when updating (paths or folder prefixes, with '/').
    pub keep_on_update: &'static [&'static str],
    /// "folder" or "file": what the player picks when providing the game files.
    pub files_kind: &'static str,
    pub needs: &'static str,
    pub files_help: &'static str,
    pub requirements: &'static str,
}

pub const GAMES: &[GameDef] = &[
    GameDef {
        id: "gow3",
        kind: Kind::Gow3,
        title: "God of War III Remastered",
        family: "PS4",
        port_name: "gow3_pc",
        port_authors: "FelipePrado0 e colaboradores",
        project_url: "https://github.com/FelipePrado0/gow3_pc",
        repo: "FelipePrado0/gow3_pc",
        asset_prefix: "gow3-windows",
        asset_suffix: ".zip",
        exe: "Play God of War III.exe",
        icon: "launcher/gow3.ico",
        markers: &["Play God of War III.exe", "bin/gow3-probe.exe"],
        keep_on_update: &["gow3.ini", "user/"],
        files_kind: "folder",
        needs: "A pasta com o dump da sua própria cópia de PS4 (a que tem eboot.bin, sce_module e sce_sys).",
        files_help: "O dump é feito no seu PS4 com o jogo instalado. Versões testadas pelo port: CUSA01623 e CUSA01715, versões 01.00 e 01.02.",
        requirements: "Windows 10/11 64-bit, processador com AVX e placa de vídeo com Vulkan 1.3.",
    },
    GameDef {
        id: "banjo",
        kind: Kind::Banjo,
        title: "Banjo-Kazooie",
        family: "N64",
        port_name: "Banjo: Recompiled",
        port_authors: "equipe Banjo: Recompiled (N64: Recompiled)",
        project_url: "https://github.com/BanjoRecomp/BanjoRecomp",
        repo: "BanjoRecomp/BanjoRecomp",
        asset_prefix: "BanjoRecompiled-",
        asset_suffix: "-Windows.zip",
        exe: "BanjoRecompiled.exe",
        icon: "BanjoRecompiled.exe",
        markers: &["BanjoRecompiled.exe"],
        keep_on_update: &["portable.txt"],
        files_kind: "file",
        needs: "O arquivo da ROM da sua própria cópia: Banjo-Kazooie norte-americano (USA), versão 1.0.",
        files_help: "A ROM é a cópia do cartucho de N64, feita com um leitor de cartuchos. Formatos aceitos: .z64, .n64 e .v64 (não compactado).",
        requirements: "Windows 10/11 64-bit e placa de vídeo com Direct3D 12 ou Vulkan 1.2.",
    },
];

pub fn get(id: &str) -> Option<&'static GameDef> {
    GAMES.iter().find(|g| g.id == id)
}

impl GameDef {
    pub fn matches(&self, dir: &Path) -> bool {
        self.markers.iter().all(|m| dir.join(m).is_file())
    }
}

/// Which port is in `dir`: the folder itself, or one folder below it (the player often
/// picks the folder that contains the port's folder).
pub fn identify(dir: &Path) -> Option<(&'static GameDef, std::path::PathBuf)> {
    for g in GAMES {
        if g.matches(dir) {
            return Some((g, dir.to_path_buf()));
        }
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        for g in GAMES {
            if g.matches(&p) {
                return Some((g, p));
            }
        }
    }
    None
}
