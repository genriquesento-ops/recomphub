# Situação do RecompHUB

## Marcos

- [x] **v0.1** Biblioteca, instalação pelo GitHub do projeto, conferência de arquivos, botão Jogar, tempo de jogo, aviso de versão nova
- [x] **v0.2** Monitor de processos (tempo de jogo por fora), Fechar jogo, atalhos, pasta de saves, espaço em disco
- [x] **v0.3** "Como obter seus arquivos", Outros jogos de PC, etiquetas, abas
- [x] **v0.4** Capas no estilo do PCSX2
- [x] **v0.5** Fase 1: catálogo aberto (`catalog.json`) com verificação genérica, aba Descobrir, 5 ports
- [x] **v0.6** Fase 2: escolher versão, aviso por port, desinstalar guardando saves, backup em .zip, zerar tempo
- [x] **v0.7** Fase 3: opções de inicialização, favoritos e ocultos, bandeja, Modo TV, aviso de versão do RecompHUB
- [x] **v0.8** Fase 4: decisões de arquitetura, este arquivo, mais testes automáticos
- [x] **v0.9** Aba Comunidade: ports da lista awesome-unofficial-pc-ports e dos tópicos do GitHub, instalação experimental com aviso, escolha do programa, atualização e desinstalação
- [x] **v0.10** Arrastar e soltar a ROM ou a pasta do dump na página do jogo, botão "Procurar na pasta Downloads"
- [x] **v0.11** Capa, banner e descrição da SteamGridDB e da IGDB com a chave do jogador, guardados no PC (busca uma vez por jogo)
- [x] **v0.12** Perfis e Conquistas, fase 1: histórico de sessões, estado do jogo, vezes jogado, anotações, página Estatísticas
- [x] **v0.13** Fase 2: backup automático ao fechar o jogo (só quando mudou), limite por jogo, travar, restaurar com segurança, vigia da pasta de saves, Ludusavi para jogos de PC
- [x] **v0.14** Fase 3: perfis (progresso, histórico e anotações por jogador, "Quem vai jogar?" com mouse, teclado ou controle, saves separados por perfil jogo a jogo)
- [x] **v0.15** Fase 4: conquistas do RecompHUB (gerais por tempo e sessões; Banjo-Kazooie pelo save), aviso por cima do jogo com som, 100%, aviso de teste
- [x] **v0.16** Fase 5: RetroAchievements só leitura, por perfil, jogo achado pelo MD5 da ROM
- [x] **v0.17** Jogos que exigem administrador abrem pelo pedido de permissão do Windows (UAC); opção "Abrir como administrador" por jogo
- [x] **v0.18** Central de Mods (Thunderstore) para Majora's Mask e Banjo: instalar com dependências, ligar e desligar, atualizar, desinstalar, mods por perfil, coleções
- [x] **v0.19** Token do GitHub (5.000 consultas por hora), Adicionar à Steam (jogo não Steam com capa), "Pular esta versão", limite de tamanho nos pacotes de mods
- [x] **v0.20** Visual novo do programa, no estilo de manual do site (tema escuro, um vermelho, linhas finas, Archivo nos títulos)
- [x] **v0.21** Seu PC: aviso de compatibilidade por port (processador, vídeo, driver, Windows, Visual C++), placa de vídeo e API gráfica por jogo, antivírus explicado; variáveis de ambiente também no modo administrador
- [x] Repositório público (MIT) na organização `recomphub-app`, com site no GitHub Pages; a situação atual está em `HANDOFF.md`
- [ ] Mais ports no catálogo (cada um com dados conferidos no código-fonte)

## Testes automáticos

Rodam no GitHub Actions antes de gerar o instalador (`cargo test --lib`):

| Área | O que garantem |
| --- | --- |
| Catálogo | O `catalog.json` embutido é válido; caminhos não saem da pasta do port; modelos de caminho só usam pastas conhecidas |
| ROM de N64 | `.v64` e `.n64` viram `.z64`; ROM de outra região é identificada na mensagem; a busca numa pasta só acha a ROM exata, até um nível abaixo |
| Dump de PS4 | `param.sfo` é lido (`TITLE_ID`, `APP_VER`) e arquivo estranho é recusado |
| Saves | Saves sobrevivem a desinstalar/reinstalar; o backup .zip tem uma pasta por jogo |
| Capas | `${serial}` e `${title}` são montados e codificados; só `https`; imagem reconhecida pelos bytes |
| SteamGridDB e IGDB | Nome igual tem preferência sobre o primeiro resultado; só imagens dos servidores dos dois sites; a busca da IGDB não deixa o nome escapar da consulta; jogo encontrado fica guardado e jogo não encontrado é tentado de novo depois de uma semana |
| Histórico | Sessões curtas não entram; a semana começa na segunda no fuso do jogador; totais por jogo, dia, semana, maior sessão e dias seguidos |
| Backups automáticos | Saves iguais não geram backup novo; só os N mais recentes ficam, os travados sempre; restaurar devolve a pasta inteira e recusa backup de outro jogo ou nome inválido; resposta JSON do Ludusavi |
| Perfis | Cada perfil guarda o próprio progresso e a instalação é a mesma; nomes e ids de perfil; histórico e estatísticas só do perfil em uso |
| Conquistas | `achievements.json` embutido é válido; contador inventado é recusado; o save do Banjo conta Jiggies e notas do melhor arquivo; cada conquista desbloqueia uma vez só e guarda a data |
| RetroAchievements | Jogo achado pelo hash da ROM; progresso lido na ordem de exibição; nome de ícone não sai da pasta |
| Central de Mods | Lista da Thunderstore só com downloads da Thunderstore; dependências antes e ciclos param; só .nrm/.rtz saem do pacote; id e jogo lidos do .nrm; mods.json preserva o que o port escreveu; dependências órfãs |
| Instalação | A pasta única no topo do .zip é removida; zip plano fica como está |
| Limites dos mods | Pacote no limite passa; arquivo ou pacote acima é recusado e a pasta temporária some |
| Token do GitHub | Formato do token; o token só vai para api.github.com, nunca para downloads ou endereços parecidos |
| Seu PC | Placas de vídeo lidas do PowerShell; processador sem MOVBE/BMI1/LZCNT bloqueia o gow3; driver antigo e adaptador básico avisados; valores do GpuPreference |
| Steam | CRC igual ao padrão; atalhos existentes regravados byte a byte; clicar de novo atualiza sem duplicar e mantém o id; arquivo quebrado ou com dados sobrando é recusado |
| Outros | Argumentos com aspas, nomes de variáveis, nomes de atalho, datas, comparação de versões |

## Regras de trabalho

- Toda mudança passa pelo ciclo: código → GitHub Actions (testes + instalador) → teste do
  Gustavo no Windows → correção.
- Uma decisão de `ARCHITECTURE_DECISIONS.md` só muda com o motivo escrito lá primeiro.
- Port novo no catálogo: dados tirados do código-fonte do port, `revision` aumentado, e
  `cargo test` passando. Veja `CATALOG.md`.
- Campos novos na biblioteca ou nas configurações sempre com valor padrão, para não quebrar
  bibliotecas antigas.
- Textos para o usuário em português do Brasil, explicando o que fazer, não só o erro.
