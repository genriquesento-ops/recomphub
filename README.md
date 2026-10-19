# RecompHUB

Launcher para Windows que reúne ports nativos de jogos de console num só lugar: instala o port da
página oficial do projeto, confere os arquivos da sua cópia, abre o jogo e conta o tempo de jogo.

O RecompHUB nunca baixa, hospeda ou aponta para arquivos de jogos. Os ports vêm só das páginas
oficiais de cada projeto, e cada jogo mostra o crédito de quem fez o port.

## Jogos da v1

| Jogo | Port | Você fornece |
| --- | --- | --- |
| God of War III Remastered | [gow3_pc](https://github.com/FelipePrado0/gow3_pc) | Pasta do dump da sua cópia de PS4 (CUSA01623 ou CUSA01715) |
| Banjo-Kazooie | [Banjo: Recompiled](https://github.com/BanjoRecomp/BanjoRecomp) | ROM da sua cópia, versão USA 1.0 |

## Como baixar

A cada envio para `main`, o GitHub Actions gera o instalador. Abra a aba **Actions**, entre no
build mais recente e baixe o artefato **RecompHUB-instalador**. Uma tag `v*` também publica o
instalador em **Releases**.

## Onde ficam os dados

- Biblioteca e tempo de jogo: `%APPDATA%\io.github.recomphub\library.json`
- Ports instalados pelo RecompHUB: `%LOCALAPPDATA%\io.github.recomphub\games\`
- God of War III: a pasta do dump é gravada na configuração do próprio port
  (`%APPDATA%\gow3-launcher\settings.json`, campo `game_dir`).
- Banjo-Kazooie: a ROM conferida é copiada para onde o port a procura
  (`%LOCALAPPDATA%\BanjoRecompiled\bk.n64.us.1.0.z64`).

## Desenvolvimento

Requer Node 22 e Rust estável.

```
npm install
npx tauri icon app-icon.png
npx tauri dev
```
