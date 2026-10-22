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

## Histórico e estado

23. **Uma linha por sessão em `history.json`, separado da biblioteca.** Cada sessão guarda início,
    fim e o tempo que contou (sem o tempo com o PC dormindo). Sessões com menos de 10 segundos
    não entram; o arquivo guarda no máximo 20 mil sessões. Ao fechar o RecompHUB com um jogo
    aberto, a sessão é registrada até ali.
24. **Estado do jogo como no Playnite.** Não jogado, Na fila, Jogando, Pausado, Zerado, Completo
    e Abandonado. Enquanto o jogador não escolhe, o estado vem do tempo de jogo; a primeira sessão
    marca "Jogando". Dias e semanas das estatísticas começam à meia-noite do fuso do PC; a
    semana começa na segunda.

## Saves automáticos

25. **Backup ao fechar o jogo, como o `wrap` do Ludusavi, mas só quando algo mudou.** Cada
    backup é um .zip em `Backups de saves/automaticos/<id>/` com `recomphub-backup.json`
    (jogo, pastas de origem, número de arquivos e uma impressão SHA-256 dos arquivos). Se a
    impressão for igual à do último backup, nenhum backup novo é feito. Ficam os N mais recentes
    (10 por padrão); um backup travado (`<arquivo>.lock`) nunca é apagado nem conta no limite.
26. **Restaurar nunca deixa uma pasta pela metade.** Antes, os saves atuais viram um backup
    "antes de restaurar" (sem apagar nenhum antigo até o fim). O .zip inteiro é lido e conferido
    contra a lista dele; a pasta nova é escrita ao lado da atual e as duas são trocadas por
    renomeação. Com o jogo aberto, restaurar é recusado.
27. **Vigia de saves por consulta, não por eventos do Windows.** As pastas de saves são pequenas
    e o RecompHUB já olha os processos a cada 3 segundos; a cada 15 segundos ele olha o arquivo
    mais novo da pasta do jogo aberto e avisa a página ("save-changed"). As conquistas vão usar
    esse mesmo aviso.
28. **Ludusavi para os Outros jogos de PC, só se o jogador tiver.** O RecompHUB procura o
    `ludusavi.exe` (escolhido pelo jogador, winget, scoop ou PATH), nunca baixa nem instala. Usa
    a linha de comando JSON dele (`find --api` e `backup --force --api`), e o backup fica na
    pasta configurada no próprio Ludusavi. Só um arquivo chamado `ludusavi.exe` é executado.

## Perfis

29. **O perfil ativo mora na biblioteca.** O que é de cada jogador (tempo, última sessão,
    favoritos, ocultos, estado, vezes jogado, anotações) fica no `library.json` para o perfil em
    uso; trocar de perfil guarda isso em `profiles/<id>.json` e carrega o do outro. Assim o resto
    do RecompHUB não precisa saber de perfis. O que está instalado e onde é o mesmo para todos.
    As sessões do histórico levam o perfil; as antigas são do perfil "Principal".
30. **Trocar de perfil só com tudo fechado.** Com um jogo aberto ou uma instalação em andamento,
    a troca é recusada, para o tempo de jogo nunca ir para o perfil errado.
31. **Saves separados por perfil, ligados jogo a jogo.** Ao trocar de perfil: backup dos saves
    ("antes de trocar para o perfil X"), o conteúdo da pasta de saves vai para
    `profiles/saves/<perfil>/<jogo>/` e o do outro perfil volta. A pasta em si fica no lugar,
    para o port continuar achando. Ao ligar, os saves atuais ficam com o perfil em uso.

## Conquistas

32. **Conquistas próprias, nunca do RetroAchievements.** Os ports não avisam conquistas, e o
    RetroAchievements só aceita desbloqueios dos emuladores aprovados por ele; ler a memória do
    jogo para isso iria contra as regras deles. As conquistas do RecompHUB têm o formato das do
    RetroAchievements (título, descrição, pontos, tipo, ícone, oculta) e ficam em
    `achievements.json`, conferido como o catálogo (ids, pontos de 1 a 100, contadores conhecidos).
33. **Gatilhos só com o que o RecompHUB já lê.** Gerais (todo jogo): sessões, tempo total, maior
    sessão e dias jogados, do perfil em uso. Por jogo: o progresso lido do save, só leitura, com o
    formato tirado da decompilação pública do jogo (Banjo-Kazooie: `n64decomp/banjo-kazooie`, CC0;
    vale o melhor dos três saves). Um jogo novo ganha leitor só com o formato conferido e testado.
34. **Conferidas ao fechar o jogo e quando o save muda.** O vigia de saves (decisão 27) e o fim da
    sessão disparam a conferência; abrir a página do jogo também, sem aviso por cima. Os
    desbloqueios ficam em `achievements/<perfil>.json`.
35. **Aviso por cima do jogo, como o Steam Achievement Notifier.** Uma janela pequena, sem bordas,
    transparente, sempre no topo, que não rouba o foco e deixa os cliques passarem; some sozinha.
    O som é gerado na hora (Web Audio), sem arquivo de terceiros. Só aparece sobre jogos em janela
    ou tela cheia sem bordas; em tela cheia exclusiva a conquista fica registrada sem aviso. O
    Steam Achievement Notifier não tem licença: dele vieram só ideias, nenhum código.

## RetroAchievements

36. **Só leitura, por perfil, com a conta do próprio jogador.** Cada perfil pode ter usuário e
    Web API Key do RetroAchievements (a chave nunca volta para a página). A página de um port de
    N64 mostra o que o jogador ganhou no jogo ORIGINAL em emuladores; nada é enviado nem
    desbloqueado lá (decisão 32).
37. **O jogo é achado pela ROM, não por um id escrito à mão.** O RetroAchievements identifica
    jogos de N64 pelo MD5 da ROM em .z64, que é como os ports guardam a ROM do jogador. A lista de
    jogos do console (com os hashes) fica em cache por uma semana, o progresso de cada jogo por 6
    horas (ou "Atualizar agora") e os ícones das conquistas para sempre. Uma ROM que o
    RetroAchievements não conhece mostra só um aviso.

## Jogos que pedem administrador

38. **Pedir a permissão do Windows em vez de falhar.** Um programa cujo manifesto exige
    administrador não abre por CreateProcess (erro 740) e o Windows não mostra o UAC sozinho.
    Nesse caso o RecompHUB abre o jogo com ShellExecuteEx "runas", que mostra o pedido de
    permissão, e marca o jogo para abrir assim das próximas vezes (o jogador pode desligar). O
    RecompHUB em si continua sem administrador. Como um jogo com administrador pode esconder o
    caminho do programa, o vigia de sessões também o reconhece pelo nome do programa; e "Fechar
    jogo" avisa que o Windows não deixa fechá-lo de fora.
    Um programa aberto por "runas" não herda as variáveis de ambiente do RecompHUB. Quando o
    jogo tem variáveis (Opções de inicialização), o pedido de permissão é para uma cópia do
    RecompHUB com `--abrir-jogo-como-admin` e o pedido (programa, pasta, argumentos e
    variáveis) em base64 na linha de comando, não num arquivo, para nada mudar entre o UAC e a
    abertura. Essa cópia abre o jogo já como administrador e termina antes de abrir janela. Sem
    variáveis, o jogo é aberto direto, como antes (v0.20.1).

## Central de Mods

39. **Só a Thunderstore, só a comunidade do jogo.** O catálogo diz, por port, a comunidade da
    Thunderstore e o `game_id` dos mods (`"mods": { "thunderstore": ..., "game_id": ... }`). A
    lista vem da API pública (a mesma do r2modman), em cache por 6 horas; pacotes marcados como
    adultos não aparecem e os descontinuados só aparecem se já estiverem instalados. Os downloads
    só são aceitos de `thunderstore.io` e seus subdomínios, em https. Autor e página sempre à vista.
40. **Instalar do jeito que o port lê.** O N64ModernRuntime carrega os `.nrm` (e `.rtz`) de
    `<dados>/mods` e liga os mods listados em `<dados>/mods.json`. O RecompHUB copia só esses
    arquivos (achatados, nomes conferidos, extração em streaming para pacotes de vários GB), lê o
    `mod.json` de cada `.nrm` para saber o id e recusa mod de outro jogo. Um arquivo de outro
    pacote nunca é sobrescrito; dependências vêm antes, e as que ninguém mais usa saem junto ao
    desinstalar. Mods que o jogador pôs à mão não são tocados.
41. **Mods por perfil e coleções.** Cada perfil guarda os pacotes que ligou
    (`mods/perfis/<perfil>/<jogo>.json`); antes de o jogo abrir, o `mods.json` fica igual à escolha
    do perfil em uso. Coleções são conjuntos com nome. Tudo isso só com o jogo fechado, porque o
    jogo reescreve o `mods.json`. Antes de instalar ou ligar um mod, backup dos saves.
42. **Limite de tamanho nos pacotes.** Um arquivo de mod pode ter até 512 MB e um pacote até
    2 GB. Os bytes copiados são contados (o tamanho escrito no .zip pode mentir), e um pacote
    acima do limite é recusado e o que já foi copiado é apagado, para nada pela metade parecer
    um mod inteiro. Ideia da revisão pelo Codex.

## GitHub e Steam (v0.19, ideias do GithubLauncher)

43. **Token do GitHub opcional, só para a API.** Sem login o GitHub responde 60 consultas por
    hora por PC; com um token pessoal sem permissões, 5.000. O token é testado antes de ser
    guardado, fica no `settings.json` deste PC, nunca volta para a página e só é enviado para
    `https://api.github.com/`, nunca para os endereços de download nem para outros sites.
44. **Pular uma versão, não todas.** "Pular esta versão" guarda a tag da versão nova; o aviso
    volta quando sair outra. Instalar limpa a escolha. "Avisar sobre versões novas: desligado"
    (decisão antiga) continua para quem não quer aviso nenhum.
45. **Adicionar à Steam como jogo não Steam.** O RecompHUB edita o `shortcuts.vdf` da conta que
    usou a Steam por último. O atalho abre `RecompHUB.exe --play <id>`, então o tempo continua
    sendo contado, e a Steam dá Big Picture e a configuração de controle. Regras: só com a Steam
    fechada (ela regrava o arquivo ao sair); um arquivo que o RecompHUB não entende por inteiro
    não é tocado; os outros atalhos são regravados byte a byte; clicar de novo atualiza o mesmo
    atalho (mesmo programa e mesmo `--play <id>`) e mantém o id dele; uma cópia do arquivo
    original fica em `shortcuts.vdf.recomphub-backup`. O id é o que a própria Steam calcularia,
    e capa e banner viram a arte do atalho na pasta `grid`.

## Seu PC (v0.21, dos issues dos ports)

46. **Avisar antes, com o que o próprio PC diz.** As causas mais comuns de "o port não abre"
    nos issues dos projetos são do PC: processador sem as instruções que o código do jogo usa
    (gow3_pc com Sandy/Ivy Bridge e Xeons antigos, erro 0xc000001d), driver de vídeo antigo ou
    ausente, Windows desatualizado e Visual C++ faltando. O RecompHUB lê isso sem administrador
    (CPUID, com o XGETBV para as instruções AVX; Win32_VideoController pelo PowerShell, uma vez
    por execução; registro do Windows; DLLs do System32) e mostra na página do jogo. Instruções
    exigidas vêm do catálogo (`cpu_features`) e só entram com confirmação do projeto. O aviso
    não impede o jogador de tentar.
47. **Placa de vídeo e API gráfica por jogo.** A placa usa a mesma opção das Configurações do
    Windows (`HKCUSoftwareMicrosoftDirectXUserGpuPreferences`, por caminho do programa). Nos
    ports N64, só o `api_option` do `graphics.json` do port muda (Auto, D3D12, Vulkan), com o jogo
    fechado, porque trocar para Vulkan resolveu notebooks que renderizavam na placa integrada.
48. **Antivírus explicado, não contornado.** Os erros 225/226 do Windows e o programa do port
    sumido de uma pasta que ainda existe viram uma explicação com o caminho até o Histórico de
    proteção. O RecompHUB nunca cria exclusões nem desliga o antivírus, e diz para só restaurar
    arquivos de ports vindos da página oficial.

## Contador de FPS (v0.22)

49. **PresentMon, não engenharia reversa.** O FPS vem do PresentMon da Intel (MIT), que lê os
    quadros pelo ETW do Windows, sem injetar nada no jogo, em DirectX e Vulkan. O HWMonitor foi
    descartado: não mede FPS e é um programa fechado. O PresentMon não vem no instalador: é
    baixado só quando o jogador pede, da release oficial da Intel no GitHub, e só é usado com
    assinatura digital válida da Intel. A leitura exige o grupo "Performance Log Users" (SID
    S-1-5-32-559): por escolha do jogador, uma cópia elevada do RecompHUB coloca a conta nele uma
    única vez (NetLocalGroupAddMembers, pelo SID, para funcionar em qualquer idioma do Windows),
    e vale a partir do próximo login. O número aparece numa janela transparente, sem foco e que
    ignora cliques, no canto de cima; em tela cheia exclusiva o Windows a esconde. A medição
    começa quando o vigia vê o jogo aberto e termina com ele.

## Ports que mudam de nome e cache de shaders (v0.23)

50. **Mudança de nome sem perder ninguém.** O gow3_pc virou SpartanPort (repositório, arquivo do
    release, programa e lugar das configurações). O catálogo usa os nomes novos e guarda o
    programa antigo (`legacy_exe`), para uma instalação antiga continuar jogável até a atualização.
    Os marcadores são `run.py` e `scripts/prepare.py`, presentes em todas as versões de Windows,
    da gow3_pc 2.0 à SpartanPort 2.6.0, então até versões antigas do RecompHUB reconhecem as duas
    (`legacy_markers` existe para quando não houver arquivos em comum). O caminho antigo das configurações fica em `settings_file`, que as
    versões antigas do RecompHUB entendem, e o novo em `portable_settings_file`; assim quem ainda
    não atualizou o RecompHUB continua vendo o jogo e já recebe o download certo. Os dados vêm do
    código do projeto (`legacy.py`, `USER_FILES` do launcher), não de suposição.
51. **Shaders: o port compila, o RecompHUB cuida do cache.** Um launcher não consegue compilar
    os shaders pelo port (só o jogo sabe quais vai usar), e os ports já compilam em segundo plano.
    O RecompHUB mostra o tamanho do cache, avisa na primeira vez que travadinhas são normais,
    lembra o driver de vídeo com que o cache foi feito e avisa quando ele mudou (cache de outro
    driver pode fechar o jogo ao abrir). "Limpar" move a pasta para `.recomphub-anterior` e
    "Desfazer" a devolve; nada é apagado sem uma cópia. Compartilhar cache entre jogadores fica de
    fora: depende da placa e do driver, e contém shaders do próprio jogo.

## Comunidade maior (v0.24)

52. **Mais formatos, nenhum instalador.** Ports da Comunidade podem vir em .zip, .7z ou como um
    .exe portátil. No .7z, cada nome é conferido antes de gravar (nada fora da pasta, nenhum nome
    de dispositivo do Windows, links ignorados) e o pacote é recusado inteiro se um falhar.
    Instaladores (Setup .exe, .msi) nunca são rodados pelo RecompHUB: a página explica e leva à
    página de versões do projeto; depois o jogador adiciona o jogo como jogo de PC. A página
    também separa "o projeto ainda não publicou versão" de "só há arquivos para outros sistemas".
53. **O nome do jogo, não o do repositório.** Resultados por tópico do GitHub não dizem o jogo, e
    um cartão "iss_pc" não é reconhecido como inFAMOUS Second Son. O nome sai da descrição
    ("port of X", "X native PC port"), com regras testadas nas descrições reais da lista; quando
    a descrição não diz, fica o nome do repositório. Entraram os tópicos `native-port`,
    `xenonrecomp` e `ps3recomp`, e ports para outras máquinas (3DS, Vita, Switch) ficam de fora.

54. **Fotos na Comunidade sem servidor nosso.** Cada port ganha uma miniatura: a capa da
    SteamGridDB ou da IGDB (com a chave do jogador, pelo nome tirado da descrição) ou a primeira
    imagem do README do projeto. Do README só valem servidores do GitHub e o Imgur, sem selos nem
    botões de doação, até 8 MB, com limite de pixels e de memória antes de abrir a imagem. As
    miniaturas (360×360, JPEG) ficam em `comunidade-imagens/` e chegam aos poucos, em segundo
    plano; um port sem imagem é tentado de novo depois de uma semana.
55. **Diagnóstico de erro só com o que o PC já guarda.** Quando uma sessão termina, o RecompHUB
    junta quatro fontes: o código de saída do programa que ele abriu (uma DLL faltando nunca
    chega ao registro do Windows), o evento 1000 "Application Error" do Windows (código e
    módulo, vale também para jogos abertos como administrador ou por fora), os crash dumps de
    `%LOCALAPPDATA%\CrashDumps` e o fim dos logs do port (campo `logs` do catálogo). Só aparece
    aviso com sinal concreto: código de falha (NTSTATUS ou exceção C++), evento, dump ou sessão
    de menos de 15 segundos; "Fechar jogo" nunca conta como erro, e o código de um launcher que
    sai normalmente é ignorado. A explicação vem de regras fixas (instrução ilegal → processador,
    DLL → Visual C++, módulo do driver → driver e Vulkan, sobreposições conhecidas), nunca de
    IA. O relatório esconde o nome da pasta do usuário e só sai do PC quando o jogador cola. O
    driver só é lembrado depois de uma sessão que terminou bem, para "o driver mudou desde a
    última vez que funcionou" continuar valendo depois de um fechamento com erro.
56. **Página do jogo em abas.** No alto ficam só Jogar e o que exige ação agora (versão nova,
    erro ao fechar, PC que não roda o port, arquivos faltando). O resto vai para abas: Visão geral,
    Mods, Saves, Conquistas e Ajustes; as Configurações seguem o mesmo modelo. O topo tem dois
    menus (Mais e Adicionar) em vez de oito botões. Na primeira abertura, um tutorial de seis
    passos explica o programa e ajuda a colar a chave da SteamGridDB; ele volta em Mais > Tutorial.
    Telas virtuais (Parsec, Spacedesk, monitores IDD) não contam como placa de vídeo.

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
