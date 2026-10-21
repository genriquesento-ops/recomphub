# Passagem de bastão (handoff)

Atualizado em 20/10/2026. Para quem pegar o projeto agora (pessoa ou Claude): onde estamos, o que já foi
feito, o que deu errado e o que falta.

## Onde estamos

- **Versão publicada:** v0.20.0 (21/10/2026): visual novo do programa, igual ao site. A v0.19.0 trouxe Steam, token do GitHub, pular versão e limite nos mods.
- **Repositório:** público, licença MIT, na organização `recomphub-app`:
  https://github.com/recomphub-app/recomphub (o Gustavo é administrador).
- **Site:** https://recomphub-app.github.io/recomphub/ (GitHub Pages; fica no ar com o PC desligado).
- **Árvore local:** sincronizada com o GitHub (nada pendente para enviar).
- **Etapa:** tudo de v0.10 a v0.18 está pronto e passa nos testes automáticos (50 testes), mas
  **ainda não foi testado pelo Gustavo no Windows**. O próximo passo é esse teste.

## Como o trabalho funciona

1. O Claude escreve o código neste PC (que não tem Rust nem Node; só git e gh).
2. O GitHub Actions (`windows-latest`) roda `cargo test --lib` e gera o instalador.
3. Uma tag `v*` faz o workflow publicar a Release; o site lê a Release mais recente sozinho.
4. O Gustavo testa no Windows e conta o que achou; o Claude corrige.

Erros de compilação só aparecem no CI: `gh run view --log-failed`.

## O que foi feito

| Versão | O que entrou |
| --- | --- |
| v0.1 a v0.9 | Biblioteca, instalação pelo GitHub de cada port, conferência de arquivos, tempo de jogo, saves, atalhos, capas, catálogo aberto (`catalog.json`), Modo TV, aba Comunidade |
| v0.10 | Arrastar e soltar a ROM ou o dump; botão "Procurar na pasta Downloads" |
| v0.11 | Capa, banner e descrição da SteamGridDB e da IGDB (com a chave do jogador), guardados no PC |
| v0.12 | Histórico de sessões, estado do jogo (jogando, zerado, 100%), anotações, página Estatísticas |
| v0.13 | Backup automático dos saves ao fechar o jogo (só se mudou), travar, restaurar com segurança; Ludusavi para jogos de PC |
| v0.14 | Perfis: progresso, histórico e saves separados por jogador; "Quem vai jogar?" também pelo controle |
| v0.15 | Conquistas próprias (por tempo de jogo e pelo save do Banjo-Kazooie), aviso por cima do jogo com som |
| v0.16 | RetroAchievements só leitura, por perfil, jogo achado pelo MD5 da ROM |
| v0.17 | Jogos que exigem administrador abrem pelo pedido de permissão do Windows (UAC); opção por jogo |
| v0.18.0 | Central de Mods (Thunderstore) para Majora's Mask e Banjo: dependências, ligar/desligar, mods por perfil, coleções |
| v0.18.1 | Projeto movido para a organização `recomphub-app`; aviso de versão e catálogo remoto apontam para lá |
| v0.20.0 | Visual novo do programa no estilo de manual do site (tema escuro, um vermelho, linhas finas, fonte Archivo embutida) |
| v0.19.0 | Token do GitHub (5.000 consultas por hora), Adicionar à Steam (jogo não Steam com capa), "Pular esta versão", limite de tamanho nos mods. Ideias do GithubLauncher; revisão pelo Codex |

Fora do programa:

- **Site** em `site/` (página única, baixa o instalador da Release mais recente, lista os ports do
  `catalog.json`, mostra as novidades das Releases). Detalhes em `SITE.md`.
- **Licença MIT** e **seção de doações (Pix)** no site, escondida enquanto a chave estiver vazia.
- **Histórico do git reescrito** para tirar o e-mail pessoal: os commits usam
  `240055676+genriquesento-ops@users.noreply.github.com`.
- **Post no X** sobre o programa: rascunho com imagem preparado; publicar é um clique do Gustavo.

## O que deu errado (e como foi resolvido)

| Problema | Causa | Solução |
| --- | --- | --- |
| Patches grandes de JS quebravam no terminal | Aspas e `$` do shell em heredoc / `node -e` | Escrever o script de patch num arquivo e rodar com `node arquivo.js` |
| Build da 0.15 falhou (E0106) | `for_game` devolvia referência sem lifetime | `for_game<'a>(defs: &'a Defs, ...) -> (..., Option<&'a GameSet>)` |
| Build da 0.17 falhou | `SHELLEXECUTEINFOW` só existe com a feature `Win32_System_Registry` do windows-sys | Feature adicionada no `Cargo.toml` |
| Build da 0.18.0 falhou (E0597) | Guarda temporária do Mutex dentro da tupla no fim do bloco | Guardar `keep` e `sources` em `let` antes |
| Empréstimos `&mut` duplos nas configurações | Dois `&mut` pela mesma guarda | `let s: &mut Settings = &mut guard;` |
| Patch do catálogo não achava os jogos | O campo é `ports`, não `games` | Corrigido no script |
| Checagem de domínio dos mods muito frouxa | `https_on` aceitava nomes parecidos | Exige o domínio exato ou um subdomínio `.dominio` |
| Jogo com administrador não abria | `CreateProcess` dá erro 740 | Cai para `ShellExecuteExW` com "runas"; erro 1223 = o jogador cancelou |
| Jogo elevado sem tempo contado | O Windows esconde o caminho do processo elevado | O jogo é achado pelo nome do programa |
| E-mail pessoal aparecia no GitHub | Commits antigos com o Gmail | Histórico reescrito; nunca commitar com o Gmail |
| Endereço do site com o nome da conta pessoal | Pages fica em `<conta>.github.io` | Organização grátis `recomphub-app`; o endereço antigo agora dá 404 (esperado) |
| Doações não apareciam no site | `site/doacoes.json` está com a chave Pix vazia | De propósito: a seção aparece quando a chave for preenchida |
| Limite da API de busca do GitHub | Muitas consultas seguidas | Esperar; a aba Comunidade guarda cache de 12 h |

## O que falta

1. **Ouvir o retorno de quem usa** a 0.19 e a 0.20 e corrigir o que aparecer.
   perfis, backup automático, conquistas do Banjo (aviso por cima do jogo), Central de Mods e um jogo
   que exige administrador.
2. **Chave Pix** em `site/doacoes.json` (`pix.chave` e `pix.nome`). A chave fica pública: usar chave
   aleatória ou e-mail, não CPF nem telefone. Pode editar direto no GitHub ou mandar para o Claude.
3. **Post no X**, se ainda não foi publicado.
4. Mais ports no catálogo (dados sempre conferidos no código-fonte do port; veja `CATALOG.md`).
5. Mais tarde, se quiser:
   - trocar o nome, porque existe outro projeto parecido em github.com/recomphub/RecompHub (não é nosso);
   - assinatura digital do instalador, para tirar o aviso do Windows.

## Regras que continuam valendo

- Textos e respostas em português do Brasil.
- Nunca baixar, procurar ou indicar ROMs ou repacks.
- RetroAchievements só leitura: nunca desbloquear nada lá.
- Commits com o e-mail noreply e terminando com
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- O Claude não faz login nem digita senhas pelo Gustavo; publicar no X é clique dele.
- Decisões de arquitetura: `ARCHITECTURE_DECISIONS.md` (até a 41). Marcos e testes: `STATUS.md`.
