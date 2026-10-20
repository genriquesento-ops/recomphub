# Catálogo de ports (`src-tauri/catalog.json`)

O RecompHUB não tem jogos escritos no código: cada port é uma entrada deste arquivo. Para
incluir um port novo, basta adicionar uma entrada e aumentar o `revision`.

## Regras para entrar no catálogo

Estas regras vêm dos princípios do `SPEC.md`:

- O port é publicado no GitHub do próprio projeto e exige a cópia do usuário. Ports que
  embutem arquivos do jogo, ou que dependem de protótipos nunca vendidos, ficam de fora.
- Os dados de verificação (hash da ROM, códigos do jogo) são tirados **do código-fonte do
  port**, nunca de listas de terceiros.
- Todos os links são `https://` e apontam para o projeto, para guias de dump da própria cópia
  ou para capas.

## Como o catálogo chega ao programa

1. O RecompHUB já vem com uma cópia (`include_str!`).
2. Em "Verificar atualizações", ele baixa o `catalog.json` deste repositório. Se for válido e
   tiver `revision` maior, passa a usá-lo e guarda uma cópia na pasta de dados.
3. Cada entrada passa por `catalog::validate` antes de ser usada. Uma entrada inválida é
   descartada, nunca usada pela metade.

Enquanto o repositório for privado, o endereço remoto responde 404 e só a cópia embutida vale.

## Campos

| Campo | O que é |
| --- | --- |
| `id` | Identificador curto (`a-z`, `0-9`, `-`). Não pode começar com `pc-`. |
| `title`, `family`, `tags` | Nome do jogo, plataforma original e etiquetas da aba Descobrir. |
| `port_name`, `port_authors`, `project_url` | Crédito do port. |
| `repo`, `asset_prefix`, `asset_suffix` | Repositório no GitHub e como reconhecer o `.zip` de Windows nos releases. |
| `exe`, `icon` | Programa aberto pelo Jogar e ícone dos atalhos (caminhos dentro da pasta do port). |
| `markers` | Arquivos que identificam uma pasta como sendo deste port. |
| `keep_on_update` | Arquivos ou pastas (terminadas em `/`) que a atualização não sobrescreve. |
| `requirements`, `needs`, `files_help` | Textos da página do jogo. |
| `files` | Como conferir os arquivos da cópia do usuário (veja abaixo). |
| `saves` | Pastas de saves, com `%APPDATA%`, `%LOCALAPPDATA%`, `%USERPROFILE%` ou `{install}`. |
| `cover_files`, `cover_urls`, `serial` | Fontes de capa: imagem na pasta do port, endereços curados e o serial para `${serial}`. |
| `guide`, `guide_links` | Passo a passo "Como obter seus arquivos". |

## Verificações (`files.kind`)

### `n64_rom`

Para ports feitos com N64: Recompiled. O usuário escolhe a ROM; o RecompHUB a converte para a
ordem `.z64` (aceita `.n64` e `.v64`), confere o XXH3 e a copia para onde o port procura:
`%LOCALAPPDATA%\<program_id>\<game_id>.z64`, ou a pasta do port quando existe `portable.txt`.

| Campo | De onde tirar no código do port |
| --- | --- |
| `xxh3` | `GameEntry::rom_hash` em `src/main/main.cpp` |
| `game_id` | `GameEntry::game_id` |
| `program_id` | `programconfig::set_program_id` / `program_id` |
| `edition`, `region`, `revision`, `name_hint` | Para explicar uma ROM errada (região `E` = USA). |

### `ps4_dump`

Para ports de PS4 que leem um dump. O RecompHUB confere `eboot.bin`, `sce_module` e o
`sce_sys/param.sfo` (`TITLE_ID` e `APP_VER`) e grava a pasta na configuração do próprio port.

| Campo | O que é |
| --- | --- |
| `title_ids`, `tested_versions` | Códigos aceitos e versões testadas pelo port. |
| `settings_file` | Arquivo JSON de configuração do port. |
| `game_dir_key`, `update_dir_key`, `user_dir_key` | Chaves desse arquivo. |

### `none`

O jogo não precisa de arquivos (jogo livre).

## Conferir antes de enviar

`cargo test --manifest-path src-tauri/Cargo.toml --lib` valida o catálogo embutido. O mesmo
teste roda no GitHub Actions antes de gerar o instalador.

## Mods (opcional)

`"mods": { "thunderstore": "zelda-64-recompiled", "game_id": "mm" }` liga a Central de Mods do port:
`thunderstore` é a comunidade em `https://thunderstore.io/c/<comunidade>/` e `game_id` é o que os
`.nrm` desse jogo declaram no `mod.json`. Só vale para ports `n64_rom`.
