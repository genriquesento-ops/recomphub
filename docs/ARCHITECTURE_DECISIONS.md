# Decisões de arquitetura

Decisões já tomadas, no estilo do `ARCHITECTURE_DECISIONS.md` do goldenballoon: cada uma diz
o que foi escolhido e por quê. Mudar uma delas pede um motivo novo, escrito aqui antes do código.

## Princípios que viram código

1. **Nenhum arquivo de jogo passa pelo RecompHUB.** O launcher baixa só o *port* (código do
   projeto, do GitHub oficial). ROMs e dumps vêm da cópia do usuário e só são lidos para
   conferência; a ROM conferida é copiada para onde o port espera (`files::n64`), nunca enviada.
2. **Catálogo fechado e validado.** Ports são dados (`src-tauri/catalog.json`), não código.
   Cada entrada passa por `catalog::validate` antes de ser usada: links `https`, caminhos
   relativos que não saem da pasta do port, `repo` no formato `dono/nome`, modelos de caminho
   só com `%APPDATA%`, `%LOCALAPPDATA%`, `%USERPROFILE%` e `{install}`. Não há "fontes de
   terceiros" configuráveis. Um catálogo baixado só substitui o embutido com `revision` maior.
3. **Links abertos só do catálogo.** `open_link` recusa qualquer endereço que não esteja numa
   entrada do catálogo; o aviso de versão do RecompHUB só abre a página de releases do projeto.

## Plataforma

4. **Tauri 2 (Rust + página web), só Windows por enquanto.** Instalador NSIS por usuário,
   sem administrador. O build acontece no GitHub Actions: o PC de desenvolvimento não precisa
   de Rust nem de Node. Antes do instalador, `cargo test --lib` precisa passar.
5. **Interface sem framework.** `ui/` é HTML, CSS e um `app.js` sem dependências, carregado
   com `withGlobalTauri`. A página nunca recebe caminhos para abrir por conta própria: tudo
   que mexe em arquivos ou abre programas é um comando Rust.
6. **Diálogos de arquivo bloqueiam** (`blocking_pick_*`) e por isso só rodam dentro de comandos
   `async`, nunca na thread principal.

## Dados do jogador

7. **Biblioteca em um JSON** (`%APPDATA%\io.github.recomphub\library.json`), gravada por
   arquivo temporário + `rename`. Campos novos sempre com `#[serde(default)]`, para que uma
   biblioteca antiga continue abrindo. Remover um jogo guarda o tempo de jogo em `removed`.
8. **Tempo de jogo por pasta, não por processo filho.** Um monitor (como o do Hydra) olha a cada
   3 s os programas que rodam de dentro da pasta de cada jogo; conta mesmo quando o jogo é
   aberto por fora. Salva a cada 30 s; intervalos maiores que 30 s (PC dormindo) não contam.
   Por isso pastas amplas demais (raiz do disco, Área de Trabalho, Downloads) são recusadas para
   "Outros jogos de PC".
9. **O RecompHUB só apaga o que instalou.** "Desinstalar" exige que o port tenha sido instalado
   pelo RecompHUB *e* que a pasta esteja dentro de `%LOCALAPPDATA%\io.github.recomphub\games`.
   Antes de apagar, os arquivos do jogador (`keep_on_update` e saves dentro do port) vão para
   `saves-pendentes\<id>` e voltam na próxima instalação.
10. **Atualizar nunca sobrescreve o que é do jogador.** A extração pula os caminhos de
    `keep_on_update` que já existem. Downloads com `.sha256` publicado são conferidos.

## Verificação de arquivos

11. **Dados de verificação vêm do código-fonte do port**, não de listas de terceiros: o XXH3, o
    `game_id` e o `program_id` dos ports N64: Recompiled saem do `src/main/main.cpp` de cada
    projeto. Assim o RecompHUB aceita exatamente o que o port aceita.
12. **Explicar, não só recusar.** Uma ROM errada é identificada pelo cabeçalho (jogo, região,
    revisão) e a mensagem diz o que é e o que falta.

## Aba Comunidade

14. **Duas camadas: verificado e comunidade.** O catálogo (`catalog.json`) continua sendo a
    única fonte com conferência de arquivos. A aba Comunidade mostra ports puxados ao vivo da
    lista `Sebastrion/awesome-unofficial-pc-ports` (MIT) e dos tópicos `pc-port` e `n64recomp`
    do GitHub, sempre com o aviso "não verificado". Um port que já está no catálogo aparece
    marcado e leva à versão verificada.
15. **Cache de 12 horas.** Sem login, o GitHub permite 10 buscas por minuto e 60 chamadas por
    hora; a lista fica em `community.json` e só as páginas de release são consultadas na hora,
    uma por port aberto.
16. **Instalação experimental só da página de releases do projeto.** O endereço do .zip é
    buscado de novo no GitHub (nunca vem da página) e precisa começar com
    `https://github.com/<repo>/releases/download/`. Só repositórios que estão na lista da
    Comunidade (ou já instalados por ela) são aceitos. Só `.zip` de Windows; se o projeto não
    tem um, a página manda para o README. O RecompHUB nunca procura nem baixa arquivos de jogos.
17. **Programa escolhido pelo jogador.** Depois de instalar, os `.exe` da pasta são listados; com
    um só, ele vira o botão Jogar; com vários, o jogador escolhe ("Escolher programa").

## Arquivos do jogador

18. **Arrastar e soltar e procurar nos Downloads usam a mesma conferência.** O arquivo solto na
    janela ou achado na pasta Downloads passa pelo mesmo caminho do botão "Escolher a ROM"
    (XXH3-64 tirado do código-fonte do port). Não usamos MD5: seria um segundo valor, sem
    garantia de bater com o que o port aceita.
19. **Procurar nos Downloads só quando o jogador clica.** Só arquivos .z64/.n64/.v64 de até
    64 MB, na pasta e nas pastas logo abaixo dela, no máximo 100 arquivos; só o que bater
    exatamente é mostrado, e o jogador confirma antes de ser entregue ao port. Nada sai do PC.
20. **Sem extrator ou patcher em segundo plano.** Os ports do catálogo leem os arquivos do
    jogador quando abrem; não há passo de extração. Se um port precisar disso, o passo entra no
    `catalog.json` como dado conferido e roda com progresso visível, nunca escondido. Programas
    de ports da Comunidade nunca são rodados pelo RecompHUB além do botão Jogar.

## Capas

13. **Ordem fixa:** capa escolhida pelo jogador → imagem dos arquivos do próprio jogo → capa
    baixada (fontes curadas do catálogo, depois modelos do jogador com `${serial}`/`${title}`)
    → capa da SteamGridDB ou da IGDB (decisão 21) → ícone do programa (jogos de PC). Imagens só
    são guardadas se os primeiros bytes forem de PNG, JPG ou WEBP, até 10 MB.
21. **SteamGridDB e IGDB com a chave do próprio jogador.** As duas exigem chave (a IGDB, um
    Client ID e um Client Secret de um aplicativo da Twitch), e um segredo embutido no
    instalador deixaria de ser segredo. As chaves ficam no `settings.json` deste PC, nunca
    voltam para a página e só vão para os próprios sites (o segredo da IGDB no corpo do pedido,
    não no endereço). Sem chave, nada muda.
22. **Cada jogo é buscado uma vez.** O resultado fica em `meta/<id>.json` e as imagens em
    `meta/<id>/`. Um jogo encontrado só é buscado de novo quando o jogador pede ("Buscar de
    novo" ou "Buscar com outro nome"); um jogo não encontrado é tentado de novo depois de uma
    semana. O token da Twitch é reaproveitado até vencer. Só imagens dos servidores das duas
    (`*.steamgriddb.com`, `images.igdb.com`) são baixadas. Com as duas chaves, capa e banner
    vêm primeiro da SteamGridDB; descrição, ano e gêneros, da IGDB (em inglês). As fontes
    aparecem na página do jogo.
