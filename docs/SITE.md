# Site do RecompHUB

O site fica em `site/` (HTML, CSS e JS puros, sem build) e vai para o GitHub Pages pelo
workflow `.github/workflows/site.yml`.

## O que se atualiza sozinho

- **Botão Baixar:** pega o `.exe` do último Release do GitHub (`releases/latest`). Sem Release,
  leva para a página de versões.
- **Ports:** vêm do `src-tauri/catalog.json`, copiado para o site na hora de publicar.
- **Novidades:** as 5 últimas versões, com as linhas "- …" das notas de cada Release.

## Para lançar uma versão

1. Subir a versão em `package.json`, `src-tauri/Cargo.toml` e `src-tauri/tauri.conf.json`.
2. Criar a tag: `git tag v0.19.0 && git push origin v0.19.0`.
3. O workflow Build gera o instalador e publica o Release com ele; o site e o aviso de versão
   nova dentro do RecompHUB passam a apontar para ele.

## Doações

A seção "Apoie o projeto" lê `site/doacoes.json`: com `pix.chave` preenchida, ela aparece com o botão
de copiar; vazia, some. Mudar o arquivo e enviar ao GitHub já atualiza o site.

## Regras do conteúdo

- Prints só do próprio RecompHUB, sem capas de jogos nem ícones de mods com personagens.
- Nenhum logo ou arte da Nintendo, Sony ou de outras empresas; créditos a cada port.
- Nada que indique onde baixar jogos.
