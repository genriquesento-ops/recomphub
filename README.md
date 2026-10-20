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

## O que ele faz

- Biblioteca com capa, status (instalado, precisa de arquivos, atualização disponível, jogando).
- Instala e atualiza o port pela página oficial do projeto, conferindo o espaço livre em disco e,
  quando o projeto publica, a soma SHA-256 do download.
- Confere os arquivos da sua cópia e explica em linguagem simples o que falta.
- Conta o tempo de jogo mesmo quando o jogo é aberto por fora do RecompHUB, salvando a cada
  30 segundos.
- Página "Como obter seus arquivos" em cada port, com o passo a passo para copiar o jogo da sua
  própria cópia.
- "Outros jogos de PC": adicione qualquer jogo instalado pelo .exe para ter tempo de jogo e
  atalhos no mesmo lugar. Abas e etiquetas para organizar a biblioteca.
- Capas no estilo do baixador de capas do PCSX2 (e da coleção
  [xlenore/ps2-covers](https://github.com/xlenore/ps2-covers)): imagem escolhida por você,
  imagem dos seus próprios arquivos, capa baixada de fontes do catálogo ou de endereços seus com
  `${serial}` e `${title}`, e o ícone do programa para jogos de PC.
- Botão para fechar o jogo, atalhos na área de trabalho e no menu Iniciar (que abrem o jogo pelo
  RecompHUB) e acesso à pasta de saves.

Algumas ideias vieram do [Hydra Launcher](https://github.com/hydralauncher/hydra) (MIT): o
monitor de processos para o tempo de jogo, os atalhos que passam pelo launcher, fechar o jogo e
conferir o espaço em disco. Fontes de download de jogos, como as do Hydra, nunca farão parte do
RecompHUB.

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

## Licença

Código sob a licença MIT (veja `LICENSE`). Os ports pertencem aos seus autores, cada um com a própria licença; o RecompHUB não distribui arquivos de jogos.
