const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let games = [];
let openId = null;
let tab = 'all';          // 'all', 'port', 'pc_game' or 'discover'
let tagFilter = null;     // a tag (the player's, or the catalog's in Descobrir), or null
let search = '';
const progress = {};      // id -> { stage, done, total }
const filesResult = {};   // id -> last check after the player picked files

const STATUS_TEXT = {
  ready: 'Instalado',
  needs_files: 'Precisa de arquivos',
  not_installed: 'Não instalado',
  starting: 'Abrindo…',
  running: 'Jogando',
  installing: 'Instalando',
  missing: 'Programa não encontrado',
  needs_exe: 'Escolher programa',
};

const IN_GAME = ['starting', 'running'];

function sessionLength(g) {
  if (!g.session_started) return '';
  return playTime(Math.floor(Date.now() / 1000) - g.session_started).toLowerCase();
}

const $ = (sel) => document.querySelector(sel);

function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

function versionLabel(tag) {
  return tag ? tag.replace(/^windows-/, '') : null;
}

function playTime(seconds) {
  if (!seconds) return 'Nunca jogado';
  if (seconds < 60) return 'Menos de 1 min';
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return h ? `${h} h ${String(m).padStart(2, '0')} min` : `${m} min`;
}

function lastPlayed(unix) {
  if (!unix) return '—';
  return new Date(unix * 1000).toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' });
}

let toastTimer;
function toast(message, bad = false) {
  const t = $('#toast');
  t.textContent = message;
  t.className = bad ? 'toast bad' : 'toast';
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), bad ? 9000 : 4000);
}

async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    toast(String(e), true);
    throw e;
  }
}

function coverHtml(g) {
  if (!g.cover && g.icon) {
    return `<div class="cover"><div class="iconcover"><img src="${g.icon}" alt=""><span>${esc(g.title)}</span></div></div>`;
  }
  if (!g.cover) return `<div class="cover"><div class="placeholder">${esc(g.title)}</div></div>`;
  const contain = g.cover.startsWith('data:image/svg') ? ' contain' : '';
  return `<div class="cover${contain}"><img src="${g.cover}" alt=""></div>`;
}

let settings = { close_to_tray: true, show_hidden: false, version: '' };

function chips(g) {
  let html = g.favorite ? '<span class="chip fav" title="Favorito">★</span>' : '';
  html += `<span class="chip family">${esc(g.family)}</span>`;
  if (g.hidden) html += '<span class="chip family">Oculto</span>';
  html += `<span class="chip ${g.status}">${STATUS_TEXT[g.status]}</span>`;
  if (g.update_available) html += `<span class="chip update">Atualização</span>`;
  return html;
}

// Library tabs show what the player has; Descobrir shows the catalog's other ports.
function inTab(g) {
  if (tab === 'discover') return g.kind === 'port' && !g.in_library;
  if (!g.in_library) return false;
  if (g.hidden && !settings.show_hidden) return false;
  if (tab === 'port') return g.kind === 'port' || g.kind === 'community';
  return tab === 'all' || g.kind === tab;
}

// Favorites first, then the most recently played (as Ports Launcher), then by name.
function libraryOrder(a, b) {
  if (tab === 'discover') return a.title.localeCompare(b.title, 'pt-BR');
  return (b.favorite - a.favorite)
    || ((b.last_played || 0) - (a.last_played || 0))
    || a.title.localeCompare(b.title, 'pt-BR');
}

const tagsOf = (g) => (tab === 'discover' ? g.catalog_tags : g.tags);

function matchesSearch(g) {
  if (!search) return true;
  const hay = [g.title, g.port_name, g.family, ...g.tags, ...g.catalog_tags].join(' ').toLowerCase();
  return search.toLowerCase().split(/\s+/).every((w) => hay.includes(w));
}

function visibleGames() {
  return games
    .filter((g) => inTab(g) && matchesSearch(g) && (!tagFilter || tagsOf(g).includes(tagFilter)))
    .sort(libraryOrder);
}

function renderTabs() {
  document.querySelectorAll('#tabs button').forEach((b) => b.classList.toggle('on', b.dataset.tab === tab));
  const tags = [...new Set(games.filter(inTab).flatMap(tagsOf))].sort((a, b) => a.localeCompare(b, 'pt-BR'));
  if (tagFilter && !tags.includes(tagFilter)) tagFilter = null;
  $('#tagbar').innerHTML = tags.map((t) =>
    `<button class="tag${t === tagFilter ? ' on' : ''}" data-tag="${esc(t)}">${esc(t)}</button>`).join('');
  document.querySelectorAll('#tagbar .tag').forEach((b) => b.addEventListener('click', () => {
    tagFilter = tagFilter === b.dataset.tag ? null : b.dataset.tag;
    render();
  }));
}

function renderGrid() {
  if (tab === 'community') return renderCommunityGrid();
  $('#community-head').hidden = true;
  renderTabs();
  const list = visibleGames();
  const empty = $('#empty');
  empty.hidden = list.length > 0;
  if (!list.length) {
    const filtered = tagFilter || search;
    empty.textContent = filtered ? 'Nenhum jogo com esse filtro.'
      : tab === 'pc_game' ? 'Nenhum jogo de PC ainda. Use "Adicionar jogo de PC" e escolha o programa (.exe) de um jogo que você já tem instalado.'
      : tab === 'discover' ? 'Você já tem todos os ports do catálogo na biblioteca.'
      : 'Sua biblioteca está vazia. Veja os ports disponíveis na aba "Descobrir", ou use "Adicionar port" se já tiver um instalado.';
  }
  $('#grid').innerHTML = list.map((g) => `
    <div class="card" data-id="${g.id}">
      ${coverHtml(g)}
      <div class="body">
        <div class="title">${esc(g.title)}</div>
        <div>${chips(g)}</div>
        <div class="meta">${g.status === 'running' ? `Jogando há ${sessionLength(g)}`
          : g.in_library ? `${playTime(g.total_seconds)} · última vez ${lastPlayed(g.last_played)}` : `Port ${esc(g.port_name)}`}</div>
      </div>
    </div>`).join('');
  document.querySelectorAll('.card').forEach((el) => el.addEventListener('click', () => openGame(el.dataset.id)));
}

const downloadsFound = {};   // id -> 'searching' | 'none' | path found in Downloads

function dropZone(g) {
  const what = g.files_kind === 'folder' ? 'a pasta do dump' : 'o arquivo da ROM';
  if (checkingDrop === g.id) return '<div class="dropzone busy">Conferindo o arquivo…</div>';
  return `<div class="dropzone" id="dropzone">Arraste ${what} e solte aqui</div>`;
}

function downloadsBox(g) {
  if (g.files_kind !== 'file') return '';
  const found = downloadsFound[g.id];
  if (found === 'searching') return '<p class="muted">Procurando na pasta Downloads…</p>';
  if (found === 'none') {
    return '<p class="muted">Nenhuma ROM que sirva para este port na pasta Downloads (nem nas pastas logo abaixo dela).</p>';
  }
  if (found) {
    return `
      <div class="found">
        <p>Encontrei a ROM certa: <b>${esc(found)}</b></p>
        <div class="row">
          <button class="primary" data-act="use-found">Usar este arquivo</button>
          <button class="ghost" data-act="dismiss-found">Não usar</button>
        </div>
      </div>`;
  }
  return '';
}

function filesBox(g) {
  const picked = filesResult[g.id];
  const files = picked && !picked.ok ? picked : g.files;
  const label = g.files_kind === 'folder' ? 'Escolher a pasta do dump' : 'Escolher a ROM';
  const searchBtn = g.files_kind === 'file' && !downloadsFound[g.id]
    ? '<button class="ghost" data-act="find-downloads">Procurar na pasta Downloads</button>' : '';
  if (files && files.ok) {
    return `
      <div class="box ok">
        <h3>Arquivos do jogo: tudo certo</h3>
        <ul>${files.details.map((d) => `<li>${esc(d)}</li>`).join('')}</ul>
        <p class="muted">${esc(files.path)}</p>
        <div class="row"><button data-act="files">${label.replace('Escolher', 'Trocar')}</button></div>
      </div>`;
  }
  const details = files ? files.details : [];
  return `
    <div class="box ${picked ? 'bad' : 'warn'}">
      <h3>${esc(files ? files.summary : 'Faltam os arquivos do jogo')}</h3>
      ${details.length ? `<ul>${details.map((d) => `<li>${esc(d)}</li>`).join('')}</ul>` : ''}
      <p><b>O que você precisa:</b> ${esc(g.needs)}</p>
      <p class="muted">${esc(g.files_help)} O RecompHUB nunca baixa arquivos de jogos: eles vêm da sua cópia.</p>
      ${dropZone(g)}
      <div class="row"><button class="primary" data-act="files">${label}</button>${searchBtn}</div>
      ${downloadsBox(g)}
    </div>
    ${guideBox(g)}`;
}

// The page whose files box takes a dropped file: an installed catalog port that is open.
function dropTarget() {
  const g = games.find((x) => x.id === openId);
  if (!g || g.kind !== 'port' || !g.files_kind) return null;
  if (['not_installed', 'installing'].includes(g.status)) return null;
  return g;
}

let checkingDrop = null;

async function provideFiles(g, path) {
  const result = await call('provide_files', { id: g.id, path: path || null });
  if (!result) return;
  filesResult[g.id] = result;
  if (result.ok) {
    delete filesResult[g.id];
    delete downloadsFound[g.id];
    toast('Arquivos conferidos. O jogo está pronto.');
  }
  setGame(await call('get_game', { id: g.id }));
}

function guideBox(g) {
  if (!g.guide.length) return '';
  return `
    <div class="box">
      <h3>Como obter seus arquivos</h3>
      <ol>${g.guide.map((s) => `<li>${esc(s)}</li>`).join('')}</ol>
      ${g.guide_links.length ? `<div class="row">${g.guide_links.map((l) =>
        `<button class="link" data-act="link" data-url="${esc(l.url)}">${esc(l.label)}</button>`).join('')}</div>` : ''}
    </div>`;
}

function tagsBox(g) {
  if (!g.in_library) return '';
  return `
    <div class="box">
      <h3>Etiquetas</h3>
      <div class="tags-edit">
        <input type="text" id="tags-input" placeholder="Ex.: favoritos, zerado, coop" value="${esc(g.tags.join(', '))}">
        <button data-act="tags">Salvar etiquetas</button>
      </div>
      <p class="muted">Separe com vírgulas. Use as etiquetas para filtrar a biblioteca.</p>
    </div>`;
}

function playRow(g, what) {
  if (IN_GAME.includes(g.status)) {
    return `
      <div class="row">
        <button class="primary play" disabled>${g.status === 'running' ? `Jogando há ${sessionLength(g)}` : 'Abrindo…'}</button>
        ${g.status === 'running' ? '<button class="danger" data-act="stop">Fechar jogo</button>' : ''}
      </div>
      <p class="muted">O tempo é contado enquanto ${what} estiver aberto e salvo a cada 30 segundos.</p>`;
  }
  return `
    <div class="row">
      <button class="primary play" data-act="play" ${g.status !== 'ready' ? 'disabled' : ''}>Jogar</button>
    </div>`;
}

function shortcutsRow(g) {
  return `
    <div class="row">
      <button class="ghost" data-act="shortcut-desktop">Atalho na área de trabalho</button>
      <button class="ghost" data-act="shortcut-start">Atalho no menu Iniciar</button>
      ${g.kind === 'port' ? '<button class="ghost" data-act="saves">Abrir pasta de saves</button>' : ''}
    </div>
    <p class="muted">Os atalhos abrem o jogo pelo RecompHUB, então o tempo de jogo continua sendo contado.</p>`;
}

function renderPcDetail(g) {
  const missing = g.status === 'missing' ? `
    <div class="box bad">
      <h3>Programa não encontrado</h3>
      <p>O RecompHUB não achou mais ${esc(g.install_dir)}. Se você moveu o jogo, remova-o da biblioteca e adicione de novo.</p>
    </div>` : '';
  $('#detail').innerHTML = `
    <button class="ghost back" data-act="back">← Biblioteca</button>
    <div class="detail">
      <div>${coverColumn(g)}</div>
      <div>
        <h2>${esc(g.title)}</h2>
        <div class="credit">Jogo de PC adicionado por você.</div>
        <div>${chips(g)}</div>
        <div class="stats">
          <div><div class="k">Tempo de jogo</div><div class="v">${playTime(g.total_seconds)}</div></div>
          <div><div class="k">Última sessão</div><div class="v">${lastPlayed(g.last_played)}</div></div>
        </div>
        ${missing}
        ${playRow(g, 'o jogo')}
        ${shortcutsRow(g)}
        <div class="box">
          <h3>Nome</h3>
          <div class="tags-edit">
            <input type="text" id="title-input" value="${esc(g.title)}">
            <button data-act="rename">Renomear</button>
          </div>
        </div>
        ${manageRow(g)}
        ${tagsBox(g)}
        ${launchBox(g)}
        <div class="row">
          <button class="ghost" data-act="folder">Abrir pasta do jogo</button>
          <button class="ghost" data-act="reset-time">Zerar tempo de jogo</button>
          <button class="ghost danger" data-act="remove">Remover da biblioteca</button>
        </div>
        <p class="muted">Remover da biblioteca não apaga nenhum arquivo. ${esc(g.install_dir || '')}</p>
      </div>
    </div>`;
}

function progressBox(g) {
  const p = progress[g.id] || { stage: 'download', done: 0, total: 0 };
  const label = { download: 'Baixando do GitHub do projeto', verify: 'Conferindo o download', extract: 'Descompactando' }[p.stage];
  const pct = p.total ? Math.round((p.done / p.total) * 100) : 0;
  const mb = (n) => (n / 1048576).toFixed(1);
  const amount = p.stage === 'download' && p.total ? ` · ${mb(p.done)} de ${mb(p.total)} MB` : '';
  return `
    <div class="box">
      <h3>${label}${amount}</h3>
      <div class="progress"><div style="width:${pct}%"></div></div>
    </div>`;
}

const COVER_SOURCE = {
  custom: 'Capa escolhida por você',
  own_files: 'Capa dos seus arquivos do jogo',
  downloaded: 'Capa baixada',
};

function coverColumn(g) {
  if (!g.in_library) return coverHtml(g);
  return `
    ${coverHtml(g)}
    ${g.cover_source ? `<p class="muted">${COVER_SOURCE[g.cover_source]}</p>` : ''}
    <div class="cover-actions">
      <button data-act="cover-choose">Escolher capa</button>
      ${['custom', 'downloaded'].includes(g.cover_source) ? '<button class="ghost" data-act="cover-clear">Restaurar</button>' : ''}
    </div>`;
}

let coverTemplates = null;   // loaded when the covers page opens
let coverReport = null;      // last "Baixar capas" result

function renderCovers() {
  const report = coverReport ? `
    <div class="box ${coverReport.missing.length ? 'warn' : 'ok'}">
      <h3>${coverReport.downloaded} capa(s) baixada(s)</h3>
      ${coverReport.missing.length ? `<p>Sem capa encontrada: ${coverReport.missing.map(esc).join(', ')}.</p>
        <p class="muted">Para esses, use "Escolher capa" na página do jogo ou adicione um endereço abaixo.</p>` : ''}
    </div>` : '';
  $('#detail').innerHTML = `
    <button class="ghost back" data-act="back">← Biblioteca</button>
    <div class="covers-page">
      <h2>Capas</h2>
      <p>Funciona como o baixador de capas do PCSX2: para cada jogo da biblioteca sem capa, o RecompHUB
        tenta os endereços abaixo e guarda a primeira imagem que encontrar. Cada capa é baixada uma vez só.</p>
      <div class="box">
        <h3>Ordem das capas</h3>
        <ol>
          <li>A imagem que você escolheu na página do jogo.</li>
          <li>A imagem dos seus próprios arquivos do jogo (por exemplo, o ícone do dump de PS4).</li>
          <li>Uma capa baixada: primeiro das fontes do catálogo do RecompHUB, depois dos seus endereços.</li>
          <li>Para jogos de PC sem capa, o ícone do programa.</li>
        </ol>
      </div>
      <div class="row"><button class="primary" data-act="covers-download">Baixar capas que faltam</button></div>
      ${report}
      <div class="box">
        <h3>Seus endereços de capas (opcional)</h3>
        <p>Um endereço por linha, começando com <code>https://</code>. Use <code>\${serial}</code> para o código do jogo
          (como <code>CUSA01715</code>) e <code>\${title}</code> para o nome do jogo.</p>
        <textarea id="templates" spellcheck="false" placeholder="https://exemplo.com/capas/\${serial}.jpg">${esc((coverTemplates || []).join('\n'))}</textarea>
        <div class="row"><button data-act="covers-save">Salvar endereços</button></div>
        <p class="muted">As capas ficam só no seu PC. Os endereços são escolhidos por você; o RecompHUB só aceita imagens (PNG, JPG ou WEBP) de até 10 MB.</p>
      </div>
    </div>`;
  document.querySelectorAll('#detail [data-act]').forEach((el) =>
    el.addEventListener('click', () => act(el.dataset.act, null, el)));
}

async function openCovers() {
  openId = '__covers';
  coverReport = null;
  coverTemplates = await call('get_cover_templates');
  render();
}

function launchBox(g) {
  if (!g.in_library) return '';
  const env = g.env.map((v) => `${v.name}=${v.value}`).join('\n');
  return `
    <details class="box" ${g.launch_args || g.env.length ? 'open' : ''}>
      <summary><h3>Opções de inicialização</h3></summary>
      <p class="muted">Para usuários avançados. Valem quando o jogo é aberto pelo RecompHUB (botão Jogar, atalhos e bandeja).</p>
      <p><b>Argumentos</b></p>
      <input type="text" id="launch-args" class="wide" value="${esc(g.launch_args)}" placeholder='Ex.: --fullscreen  ou  --rom "C:\\Meus jogos\\rom.z64"'>
      <p><b>Variáveis de ambiente</b> (uma por linha, NOME=valor)</p>
      <textarea id="launch-env" spellcheck="false" placeholder="DXVK_HUD=fps">${esc(env)}</textarea>
      <div class="row"><button data-act="launch-save">Salvar opções</button></div>
    </details>`;
}

function manageRow(g) {
  if (!g.in_library) return '';
  return `
    <div class="row">
      <button class="ghost" data-act="favorite">${g.favorite ? '★ Tirar dos favoritos' : '☆ Favoritar'}</button>
      <button class="ghost" data-act="hide">${g.hidden ? 'Mostrar na biblioteca' : 'Ocultar da biblioteca'}</button>
    </div>`;
}

function renderSettings() {
  $('#detail').innerHTML = `
    <button class="ghost back" data-act="back">← Biblioteca</button>
    <div class="covers-page">
      <h2>Configurações</h2>
      <div class="box">
        <label class="check"><input type="checkbox" id="set-tray" ${settings.close_to_tray ? 'checked' : ''}>
          Fechar para a bandeja: ao fechar a janela, o RecompHUB continua perto do relógio e segue contando o tempo de jogo.
          Para sair de vez, use "Sair" no ícone da bandeja.</label>
        <label class="check"><input type="checkbox" id="set-hidden" ${settings.show_hidden ? 'checked' : ''}>
          Mostrar jogos ocultos na biblioteca.</label>
        <div class="row"><button data-act="settings-save">Salvar</button></div>
      </div>
      <p class="muted">RecompHUB versão ${esc(settings.version)}.</p>
    </div>`;
  document.querySelectorAll('#detail [data-act]').forEach((el) =>
    el.addEventListener('click', () => act(el.dataset.act, null, el)));
}

function renderDetail() {
  if (openId === '__covers') return renderCovers();
  if (openId === '__settings') return renderSettings();
  if (openId && openId.startsWith('c:')) return renderCommunityItem(openId.slice(2));
  const g = games.find((x) => x.id === openId);
  if (!g) {
    openId = null;
    render();
    return;
  }
  if (g.kind === 'pc_game') renderPcDetail(g);
  else if (g.kind === 'community') renderCommunityDetail(g);
  else renderPortDetail(g);
  document.querySelectorAll('#detail [data-act]').forEach((el) =>
    el.addEventListener('click', () => act(el.dataset.act, g, el)));
}

const versions = {};   // id -> 'loading' | [{tag, published, size}] while "Escolher versão" is open

function versionBox(g) {
  if (!g.installed_by_hub) return '';
  const list = versions[g.id];
  let picker = '';
  if (list === 'loading') {
    picker = '<p class="muted">Buscando as versões no GitHub do projeto…</p>';
  } else if (Array.isArray(list)) {
    picker = `
      <ul class="versions">${list.map((v) => `
        <li>
          <span><b>${esc(versionLabel(v.tag))}</b> · ${new Date(v.published * 1000).toLocaleDateString('pt-BR')} · ${(v.size / 1048576).toFixed(0)} MB</span>
          ${v.tag === g.version ? '<span class="muted">instalada</span>'
            : `<button data-act="install" data-tag="${esc(v.tag)}" ${IN_GAME.includes(g.status) ? 'disabled' : ''}>Instalar esta</button>`}
        </li>`).join('')}
      </ul>
      <p class="muted">Escolher uma versão mais antiga desliga o aviso de versão nova deste port. Saves e configurações são mantidos.</p>
      <div class="row"><button class="ghost" data-act="versions-close">Fechar lista</button></div>`;
  }
  return `
    <div class="box">
      <h3>Versão do port</h3>
      <div class="row">
        ${list ? '' : '<button data-act="versions">Escolher versão</button>'}
        <button class="ghost" data-act="ignore-updates">Avisar sobre versões novas: ${g.ignore_updates ? 'desligado' : 'ligado'}</button>
      </div>
      ${picker}
    </div>`;
}

function renderPortDetail(g) {
  const latest = versionLabel(g.latest_version);
  const version = versionLabel(g.version) || (g.status !== 'not_installed' ? 'não identificada (adicionada de uma pasta)' : null);

  let main = '';
  if (g.status === 'installing') {
    main = progressBox(g);
  } else if (g.status === 'not_installed') {
    main = `
      <div class="box">
        <h3>Instalar</h3>
        <p>O RecompHUB baixa a versão mais recente do port na página oficial do projeto${latest ? ` (${esc(latest)})` : ''} e depois pede os arquivos da sua cópia.</p>
        <p class="muted">Requisitos: ${esc(g.requirements)}</p>
        ${g.pending_saves ? '<p><b>Seus saves da instalação anterior estão guardados</b> e voltam para o lugar quando você instalar.</p>' : ''}
        <div class="row">
          <button class="primary" data-act="install">Instalar</button>
          <button data-act="add">Já tenho instalado: escolher a pasta</button>
        </div>
      </div>
      ${guideBox(g)}`;
  } else {
    if (g.update_available) {
      main += `
        <div class="box warn">
          <h3>Versão nova disponível${latest ? `: ${esc(latest)}` : ''}</h3>
          <p>Seus saves e configurações são mantidos.</p>
          <div class="row"><button data-act="install" ${IN_GAME.includes(g.status) ? 'disabled' : ''}>Atualizar</button></div>
        </div>`;
    }
    main += playRow(g, 'o port');
    main += filesBox(g);
    main += shortcutsRow(g);
    main += versionBox(g);
    main += manageRow(g);
    main += tagsBox(g);
    main += launchBox(g);
  }

  $('#detail').innerHTML = `
    <button class="ghost back" data-act="back">← Biblioteca</button>
    <div class="detail">
      <div>${coverColumn(g)}</div>
      <div>
        <h2>${esc(g.title)}</h2>
        <div class="credit">Port <b>${esc(g.port_name)}</b>, feito por ${esc(g.port_authors)} ·
          <button class="link" data-act="project">página do projeto</button></div>
        <div>${chips(g)}</div>
        <div class="stats">
          <div><div class="k">Tempo de jogo</div><div class="v">${playTime(g.total_seconds)}</div></div>
          <div><div class="k">Última sessão</div><div class="v">${lastPlayed(g.last_played)}</div></div>
          <div><div class="k">Versão do port</div><div class="v">${esc(version || '—')}</div></div>
        </div>
        ${main}
        ${g.in_library ? `
          <div class="row">
            <button class="ghost" data-act="folder">Abrir pasta do port</button>
            <button class="ghost" data-act="reset-time">Zerar tempo de jogo</button>
            ${g.installed_by_hub ? '<button class="ghost danger" data-act="uninstall">Desinstalar</button>' : ''}
            <button class="ghost danger" data-act="remove">Remover da biblioteca</button>
          </div>
          <p class="muted">${g.installed_by_hub
            ? 'Desinstalar apaga a pasta do port, mas guarda seus saves e configurações para a próxima instalação. Remover da biblioteca não apaga nada.'
            : 'Este port foi adicionado de uma pasta sua: o RecompHUB nunca apaga esses arquivos. Remover da biblioteca não apaga nada.'}
            ${esc(g.install_dir || '')}</p>` : ''}
      </div>
    </div>`;
}

function render() {
  const inDetail = openId !== null;
  $('#library').hidden = inDetail;
  $('#detail').hidden = !inDetail;
  if (inDetail) renderDetail();
  else renderGrid();
  if (tvOn) renderTv();
}

function setGame(g) {
  const i = games.findIndex((x) => x.id === g.id);
  if (i >= 0) games[i] = g;
  else games.push(g);
  render();
}

async function refresh() {
  games = await call('get_library');
  render();
}

function openGame(id) {
  openId = id;
  render();
}

async function act(action, g, el) {
  switch (action) {
    case 'link':
      await call('open_link', { url: el.dataset.url });
      break;
    case 'cover-choose': {
      const updated = await call('choose_cover', { id: g.id });
      if (updated) setGame(updated);
      break;
    }
    case 'cover-clear':
      setGame(await call('clear_cover', { id: g.id }));
      break;
    case 'covers-save': {
      const lines = $('#templates').value.split('\n');
      coverTemplates = await call('set_cover_templates', { lines });
      toast(coverTemplates.length ? 'Endereços salvos.' : 'Nenhum endereço salvo.');
      render();
      break;
    }
    case 'covers-download': {
      el.disabled = true;
      el.textContent = 'Baixando capas…';
      try {
        coverReport = await call('download_covers');
        games = coverReport.library;
      } finally {
        render();
      }
      break;
    }
    case 'tags': {
      const tags = $('#tags-input').value.split(',');
      setGame(await call('set_tags', { id: g.id, tags }));
      toast('Etiquetas salvas.');
      break;
    }
    case 'rename':
      setGame(await call('rename_game', { id: g.id, title: $('#title-input').value }));
      toast('Nome salvo.');
      break;
    case 'back':
      openId = null;
      render();
      break;
    case 'project':
      if (g.kind === 'community') await call('open_link', { url: g.project_url });
      else await call('open_project', { id: g.id });
      break;
    case 'exe-list':
      exeChoices[g.id] = await call('list_executables', { id: g.id });
      render();
      break;
    case 'exe-set':
      setGame(await call('set_executable', { id: g.id, exe: el.dataset.exe }));
      delete exeChoices[g.id];
      render();
      toast('Programa escolhido. Agora é só jogar.');
      break;
    case 'community-update':
      progress[g.id] = { id: g.id, stage: 'download', done: 0, total: 0 };
      g.status = 'installing';
      render();
      try {
        setGame(await invoke('install_community', { repo: g.repo, asset: null, title: g.title }));
        toast(`${g.title} atualizado.`);
      } catch (e) {
        toast(String(e), true);
        await refresh();
      }
      break;
    case 'folder':
      await call('open_folder', { id: g.id });
      break;
    case 'add':
      await addExisting();
      break;
    case 'install': {
      const tag = el && el.dataset.tag ? el.dataset.tag : null;
      progress[g.id] = { stage: 'download', done: 0, total: 0 };
      g.status = 'installing';
      delete versions[g.id];
      render();
      try {
        setGame(await invoke('install_game', { id: g.id, tag }));
        toast(tag ? `${g.title}: versão ${versionLabel(tag)} instalada.` : `${g.title} instalado.`);
      } catch (e) {
        toast(String(e), true);
        await refresh();
      }
      break;
    }
    case 'files':
      await provideFiles(g, null);
      break;
    case 'find-downloads': {
      downloadsFound[g.id] = 'searching';
      render();
      let found = null;
      try {
        found = await invoke('find_rom_in_downloads', { id: g.id });
      } catch (e) {
        toast(String(e), true);
      }
      downloadsFound[g.id] = found || 'none';
      render();
      break;
    }
    case 'use-found': {
      const path = downloadsFound[g.id];
      delete downloadsFound[g.id];
      await provideFiles(g, path);
      break;
    }
    case 'dismiss-found':
      delete downloadsFound[g.id];
      render();
      break;
    case 'play':
      await call('play', { id: g.id });
      setGame(await call('get_game', { id: g.id }));
      break;
    case 'stop':
      if (!confirm(`Fechar ${g.title}? O progresso que não foi salvo no jogo será perdido.`)) break;
      await call('stop_game', { id: g.id });
      break;
    case 'saves':
      await call('open_saves', { id: g.id });
      break;
    case 'shortcut-desktop':
    case 'shortcut-start': {
      const location = action === 'shortcut-desktop' ? 'desktop' : 'start_menu';
      await call('create_shortcut', { id: g.id, location });
      toast(location === 'desktop' ? 'Atalho criado na área de trabalho.' : 'Atalho criado no menu Iniciar (pasta RecompHUB).');
      break;
    }
    case 'remove':
      await call('remove_game', { id: g.id });
      await refresh();
      break;
    case 'favorite':
      setGame(await call('set_favorite', { id: g.id, favorite: !g.favorite }));
      break;
    case 'hide':
      setGame(await call('set_hidden', { id: g.id, hidden: !g.hidden }));
      if (!g.hidden && !settings.show_hidden) toast(`${g.title} foi ocultado. Para ver ocultos, use Configurações.`);
      break;
    case 'launch-save': {
      const env = $('#launch-env').value.split('\n').map((l) => l.trim()).filter(Boolean).map((l) => {
        const i = l.indexOf('=');
        return i < 0 ? { name: l, value: '' } : { name: l.slice(0, i), value: l.slice(i + 1) };
      });
      setGame(await call('set_launch_options', { id: g.id, args: $('#launch-args').value, env }));
      toast('Opções de inicialização salvas.');
      break;
    }
    case 'settings-save':
      settings = await call('set_settings', {
        closeToTray: $('#set-tray').checked,
        showHidden: $('#set-hidden').checked,
      });
      toast('Configurações salvas.');
      break;
    case 'versions':
      versions[g.id] = 'loading';
      render();
      try {
        versions[g.id] = await call('list_versions', { id: g.id });
      } catch {
        delete versions[g.id];
      }
      render();
      break;
    case 'versions-close':
      delete versions[g.id];
      render();
      break;
    case 'ignore-updates':
      setGame(await call('set_ignore_updates', { id: g.id, ignore: !g.ignore_updates }));
      break;
    case 'reset-time':
      if (!confirm(`Zerar o tempo de jogo de ${g.title}? Isso não pode ser desfeito.`)) break;
      setGame(await call('reset_playtime', { id: g.id }));
      toast('Tempo de jogo zerado.');
      break;
    case 'uninstall': {
      if (!confirm(`Desinstalar ${g.title}? A pasta do port será apagada. Seus saves e configurações ficam guardados e voltam quando você instalar de novo.`)) break;
      const kept = await call('uninstall_game', { id: g.id });
      await refresh();
      toast(kept.length ? `${g.title} desinstalado. Guardado para depois: ${kept.join(', ')}.` : `${g.title} desinstalado.`);
      break;
    }
  }
}

async function backupSaves() {
  const btn = $('#btn-backup');
  btn.disabled = true;
  btn.textContent = 'Fazendo backup…';
  try {
    const r = await call('backup_saves');
    toast(`Backup feito: ${r.files} arquivo(s) de ${r.games} jogo(s) em ${r.path}`);
    await call('open_backups');
  } catch {
    // the toast already shows why
  } finally {
    btn.disabled = false;
    btn.textContent = 'Backup dos saves';
  }
}

async function addExisting() {
  const g = await call('add_existing');
  if (!g) return;
  await refresh();
  openGame(g.id);
  toast(`${g.title} adicionado à biblioteca.`);
}

async function addPcGame() {
  const g = await call('add_pc_game');
  if (!g) return;
  await refresh();
  openGame(g.id);
  toast(g.kind === 'port'
    ? `Esse programa é do port ${g.port_name}: ${g.title} foi adicionado como port.`
    : `${g.title} adicionado em "Outros jogos de PC". Você pode renomear na página do jogo.`);
}

$('#btn-add').addEventListener('click', addExisting);
$('#btn-add-pc').addEventListener('click', addPcGame);
$('#btn-covers').addEventListener('click', openCovers);
$('#btn-backup').addEventListener('click', backupSaves);
document.querySelectorAll('#tabs button').forEach((b) => b.addEventListener('click', () => {
  tab = b.dataset.tab;
  tagFilter = null;
  render();
}));
$('#search').addEventListener('input', (e) => {
  search = e.target.value.trim();
  renderGrid();
});
$('#btn-updates').addEventListener('click', async () => {
  games = await call('check_updates');
  render();
  const n = games.filter((g) => g.update_available).length;
  toast(n ? `${n} jogo(s) com versão nova.` : 'Tudo atualizado.');
});

listen('install-progress', (e) => {
  progress[e.payload.id] = e.payload;
  const onItem = openId && openId.startsWith('c:') && communityId(openId.slice(2)) === e.payload.id;
  if (openId === e.payload.id || onItem) renderDetail();
});

listen('session', async (e) => {
  const { id, state, seconds } = e.payload;
  try {
    setGame(await invoke('get_game', { id }));
  } catch {
    return; // removed from the library meanwhile
  }
  // "Abrindo…" ends after a minute if the port never showed up.
  if (state === 'starting') setTimeout(async () => setGame(await invoke('get_game', { id })), 61000);
  if (state === 'ended' && seconds > 0) {
    const g = games.find((x) => x.id === id);
    toast(`Sessão de ${playTime(seconds).toLowerCase()} somada a ${g ? g.title : id}.`);
  }
});

listen('open-game', (e) => openGame(e.payload));

// Dropping the ROM (or the dump folder) on the window; the zone lights up while dragging.
const dropHighlight = (on) => {
  const zone = document.getElementById('dropzone');
  if (zone) zone.classList.toggle('over', on);
};
listen('tauri://drag-enter', () => dropHighlight(!!dropTarget()));
listen('tauri://drag-leave', () => dropHighlight(false));
listen('tauri://drag-drop', async (e) => {
  dropHighlight(false);
  const g = dropTarget();
  const paths = (e.payload && e.payload.paths) || [];
  if (!g) {
    if (paths.length) toast('Abra a página do jogo e solte o arquivo na caixa "Arquivos do jogo".', true);
    return;
  }
  if (!paths.length || checkingDrop) return;
  if (paths.length > 1) toast('Solte um arquivo de cada vez. Conferindo só o primeiro.');
  checkingDrop = g.id;
  render();
  try {
    await provideFiles(g, paths[0]);
  } finally {
    checkingDrop = null;
    render();
  }
});
listen('notify', (e) => toast(String(e.payload), true));

// ---- Aba Comunidade: ports from the awesome list and GitHub topics, not verified ----

let communityData = null;      // { items, fetched_at, errors, list_page }
let communityLoading = false;
const communityRelease = {};   // repo -> 'loading' | null | { tag, published, prerelease, assets }

// Same as community::id_for in Rust.
const communityId = (repo) => 'gh-' + repo.toLowerCase().replace(/[^a-z0-9]/g, '-').slice(0, 60);

async function loadCommunity(force) {
  communityLoading = true;
  render();
  try {
    communityData = await call('get_community', { force });
  } finally {
    communityLoading = false;
    render();
  }
}

function communityMatches(i) {
  if (tagFilter && i.console !== tagFilter) return false;
  if (!search) return true;
  const hay = [i.name, i.game, i.console, i.description, i.repo].join(' ').toLowerCase();
  return search.toLowerCase().split(/\s+/).every((w) => hay.includes(w));
}

function renderCommunityGrid() {
  document.querySelectorAll('#tabs button').forEach((b) => b.classList.toggle('on', b.dataset.tab === tab));
  const head = $('#community-head');
  head.hidden = false;
  if (!communityData && !communityLoading) loadCommunity(false);
  const data = communityData || { items: [], errors: [], fetched_at: 0 };
  head.innerHTML = `
    <p>Ports feitos pela comunidade, puxados direto do GitHub: a lista
      <button class="link" data-clink="${esc(data.list_page || '')}">awesome-unofficial-pc-ports</button>
      e os tópicos <code>pc-port</code> e <code>n64recomp</code>.
      <b>Eles não foram verificados pelo RecompHUB.</b> Os marcados "No catálogo" têm a versão verificada na aba Descobrir.</p>
    <div class="row">
      <button data-cact="refresh" ${communityLoading ? 'disabled' : ''}>${communityLoading ? 'Buscando no GitHub…' : 'Atualizar lista'}</button>
      ${data.fetched_at ? `<span class="muted">Atualizada em ${lastPlayed(data.fetched_at)} · ${data.items.length} ports</span>` : ''}
    </div>
    ${data.errors.length ? `<p class="muted">Não consegui ler: ${data.errors.map(esc).join('; ')}.</p>` : ''}`;
  head.querySelector('[data-cact=refresh]').addEventListener('click', () => loadCommunity(true));
  const lp = head.querySelector('[data-clink]');
  if (lp && lp.dataset.clink) lp.addEventListener('click', () => call('open_link', { url: lp.dataset.clink }));

  const consoles = [...new Set(data.items.map((i) => i.console).filter(Boolean))].sort((a, b) => a.localeCompare(b, 'pt-BR'));
  if (tagFilter && !consoles.includes(tagFilter)) tagFilter = null;
  $('#tagbar').innerHTML = consoles.map((t) =>
    `<button class="tag${t === tagFilter ? ' on' : ''}" data-tag="${esc(t)}">${esc(t)}</button>`).join('');
  document.querySelectorAll('#tagbar .tag').forEach((b) => b.addEventListener('click', () => {
    tagFilter = tagFilter === b.dataset.tag ? null : b.dataset.tag;
    render();
  }));

  const list = data.items.filter(communityMatches)
    .sort((a, b) => (b.recommended - a.recommended) || ((b.stars || 0) - (a.stars || 0)) || a.name.localeCompare(b.name));
  const empty = $('#empty');
  empty.hidden = list.length > 0 || communityLoading;
  empty.textContent = data.items.length ? 'Nenhum port com esse filtro.' : 'Nenhum port carregado. Verifique a internet e clique em "Atualizar lista".';
  $('#grid').innerHTML = list.map((i) => `
    <div class="card" data-repo="${esc(i.repo)}">
      <div class="cover"><div class="placeholder">${esc(i.game || i.name)}</div></div>
      <div class="body">
        <div class="title">${esc(i.name)}</div>
        <div>
          ${i.console ? `<span class="chip family">${esc(i.console)}</span>` : ''}
          ${i.recommended ? '<span class="chip fav">⭐ Recomendado</span>' : ''}
          ${i.catalog_id ? '<span class="chip ready">No catálogo</span>' : ''}
          ${i.installed_id ? '<span class="chip running">Na biblioteca</span>' : ''}
        </div>
        <div class="meta">${[i.game && i.game !== i.name ? esc(i.game) : '', i.stars != null ? `★ ${i.stars}` : '', i.pushed ? `atualizado em ${new Date(i.pushed * 1000).toLocaleDateString('pt-BR')}` : ''].filter(Boolean).join(' · ')}</div>
      </div>
    </div>`).join('');
  document.querySelectorAll('#grid .card').forEach((el) => el.addEventListener('click', () => openGame('c:' + el.dataset.repo)));
}

function communityWarning() {
  return `
    <div class="box warn">
      <h3>Instalação experimental: port não verificado</h3>
      <ul>
        <li>O RecompHUB baixa o .zip para Windows da página de releases deste projeto, mas não conferiu o port nem os arquivos que ele pede.</li>
        <li>É um programa feito pela comunidade: só instale se confiar no projeto.</li>
        <li>Siga o README do projeto para colocar os arquivos da sua própria cópia do jogo. O RecompHUB nunca baixa arquivos de jogos.</li>
      </ul>
    </div>`;
}

function renderCommunityItem(repo) {
  const i = communityData && communityData.items.find((x) => x.repo === repo);
  if (!i) { openId = null; render(); return; }
  const id = communityId(repo);
  const rel = communityRelease[repo];
  let install = '';
  if (i.catalog_id) {
    install = `
      <div class="box ok">
        <h3>Este port está no catálogo verificado</h3>
        <p>Use a versão do catálogo: o RecompHUB confere os arquivos do jogo e explica o que falta.</p>
        <div class="row"><button class="primary" data-cact="catalog">Abrir no catálogo</button></div>
      </div>`;
  } else if (i.installed_id) {
    install = `<div class="row"><button class="primary" data-cact="library">Abrir na biblioteca</button></div>`;
  } else if (progress[id] && games.every((g) => g.id !== id)) {
    install = progressBox({ id });
  } else {
    let files = '';
    if (rel === 'loading') files = '<p class="muted">Procurando a versão para Windows no GitHub…</p>';
    else if (rel === null) files = '<p>Este projeto não tem um .zip para Windows nos releases. Talvez precise ser compilado: veja a página do projeto.</p>';
    else if (rel) {
      files = `
        <p>Versão <b>${esc(rel.tag)}</b>${rel.prerelease ? ' (pré-lançamento)' : ''} · ${new Date(rel.published * 1000).toLocaleDateString('pt-BR')}</p>
        <ul class="versions">${rel.assets.map((a) => `
          <li><span>${esc(a.name)} · ${(a.size / 1048576).toFixed(0)} MB</span>
            <button data-cact="install" data-asset="${esc(a.name)}">Instalar (experimental)</button></li>`).join('')}
        </ul>`;
    }
    install = `
      ${communityWarning()}
      <div class="box">
        <h3>Versão para Windows</h3>
        ${files || '<div class="row"><button data-cact="release">Ver versão para Windows</button></div>'}
      </div>`;
  }
  $('#detail').innerHTML = `
    <button class="ghost back" data-act="back">← Comunidade</button>
    <div class="covers-page">
      <h2>${esc(i.name)}</h2>
      <div class="credit">${i.game ? `${esc(i.game)} · ` : ''}${i.console ? `${esc(i.console)} · ` : ''}
        <button class="link" data-cact="page">${esc(i.repo)} no GitHub</button></div>
      <div>
        ${i.recommended ? '<span class="chip fav">⭐ Recomendado pela lista</span>' : ''}
        <span class="chip family">Fonte: ${i.sources.map(esc).join(' e ')}</span>
        ${i.stars != null ? `<span class="chip family">★ ${i.stars}</span>` : ''}
      </div>
      ${i.description ? `<p>${esc(i.description)}</p>` : ''}
      ${install}
    </div>`;
  const on = (sel, fn) => document.querySelectorAll(`#detail [data-cact=${sel}]`).forEach((el) => el.addEventListener('click', () => fn(el)));
  document.querySelector('#detail [data-act=back]').addEventListener('click', () => { openId = null; render(); });
  on('page', () => call('open_link', { url: `https://github.com/${repo}` }));
  on('catalog', () => { tab = 'discover'; openGame(i.catalog_id); });
  on('library', () => openGame(i.installed_id));
  on('release', async () => {
    communityRelease[repo] = 'loading';
    render();
    try {
      communityRelease[repo] = await call('community_release', { repo });
    } catch {
      delete communityRelease[repo];
    }
    render();
  });
  on('install', async (el) => {
    const ok = confirm(`Instalar ${i.name} em modo experimental?\n\nEste port não foi verificado pelo RecompHUB. Ele será baixado da página de releases de ${repo} no GitHub. Só continue se confiar no projeto.`);
    if (!ok) return;
    progress[id] = { id, stage: 'download', done: 0, total: 0 };
    render();
    try {
      const g = await invoke('install_community', { repo, asset: el.dataset.asset, title: i.game || i.name });
      setGame(g);
      delete progress[id];
      communityData = await invoke('get_community', { force: false });
      openGame(g.id);
      toast(g.status === 'needs_exe'
        ? `${g.title} instalado. Escolha qual programa o botão Jogar deve abrir.`
        : `${g.title} instalado. Leia o README do projeto para colocar os arquivos do jogo.`);
    } catch (e) {
      delete progress[id];
      toast(String(e), true);
      render();
    }
  });
}

const exeChoices = {};   // id -> list of .exe paths while choosing

function renderCommunityDetail(g) {
  const choices = exeChoices[g.id];
  const exeBox = g.status === 'needs_exe' || choices ? `
    <div class="box ${g.status === 'needs_exe' ? 'warn' : ''}">
      <h3>Qual programa o botão Jogar abre?</h3>
      ${Array.isArray(choices) ? `<ul class="versions">${choices.map((x) => `
        <li><span>${esc(x)}</span><button data-act="exe-set" data-exe="${esc(x)}">Usar este</button></li>`).join('')}</ul>`
        : '<div class="row"><button data-act="exe-list">Mostrar programas da pasta</button></div>'}
    </div>` : '';
  const update = g.update_available ? `
    <div class="box warn">
      <h3>Versão nova no GitHub: ${esc(g.latest_version)}</h3>
      <p>Arquivos em saves, save, savedata, user e config são mantidos.</p>
      <div class="row"><button data-act="community-update">Atualizar</button></div>
    </div>` : '';
  $('#detail').innerHTML = `
    <button class="ghost back" data-act="back">← Biblioteca</button>
    <div class="detail">
      <div>${coverColumn(g)}</div>
      <div>
        <h2>${esc(g.title)}</h2>
        <div class="credit">Port da comunidade <b>${esc(g.port_name)}</b>, de ${esc(g.port_authors)} ·
          <button class="link" data-act="project">página do projeto</button></div>
        <div>${chips(g)}<span class="chip needs_files">Não verificado</span></div>
        <div class="stats">
          <div><div class="k">Tempo de jogo</div><div class="v">${playTime(g.total_seconds)}</div></div>
          <div><div class="k">Última sessão</div><div class="v">${lastPlayed(g.last_played)}</div></div>
          <div><div class="k">Versão</div><div class="v">${esc(g.version || '—')}</div></div>
        </div>
        ${g.status === 'installing' ? progressBox(g) : ''}
        ${update}
        ${playRow(g, 'o port')}
        ${exeBox}
        <p class="muted">Port não verificado pelo RecompHUB: os arquivos do jogo seguem o README do projeto.</p>
        ${shortcutsRow(g)}
        ${g.status !== 'needs_exe' && !choices ? '<div class="row"><button class="ghost" data-act="exe-list">Trocar o programa do botão Jogar</button></div>' : ''}
        ${manageRow(g)}
        ${tagsBox(g)}
        ${launchBox(g)}
        <div class="row">
          <button class="ghost" data-act="folder">Abrir pasta do port</button>
          <button class="ghost" data-act="reset-time">Zerar tempo de jogo</button>
          ${g.installed_by_hub ? '<button class="ghost danger" data-act="uninstall">Desinstalar</button>' : ''}
          <button class="ghost danger" data-act="remove">Remover da biblioteca</button>
        </div>
        <p class="muted">Desinstalar apaga a pasta do port, mas guarda saves, save, savedata, user e config para a próxima instalação. ${esc(g.install_dir || '')}</p>
      </div>
    </div>`;
}

// ---- Modo TV: full-screen library for a controller (Gamepad API) or the keyboard ----

let tvOn = false;
let tvSel = 0;
let tvPad = { held: {}, nextAt: 0 };

function tvGames() {
  return games
    .filter((g) => g.in_library && !g.hidden && !['not_installed', 'missing'].includes(g.status))
    .sort((a, b) => (b.favorite - a.favorite) || ((b.last_played || 0) - (a.last_played || 0)) || a.title.localeCompare(b.title, 'pt-BR'));
}

function renderTv() {
  const list = tvGames();
  tvSel = Math.max(0, Math.min(tvSel, list.length - 1));
  $('#tv').innerHTML = `
    <h1>Recomp<span style="color:var(--accent)">HUB</span></h1>
    ${list.length ? '' : '<p class="muted">Nenhum jogo instalado ainda. Saia do Modo TV para instalar.</p>'}
    <div class="tv-grid">${list.map((g, i) => `
      <div class="card${i === tvSel ? ' sel' : ''}" data-i="${i}">
        ${coverHtml(g)}
        <div class="body">
          <div class="title">${esc(g.title)}</div>
          <div>${chips(g)}</div>
          <div class="meta">${g.status === 'running' ? `Jogando há ${sessionLength(g)}` : playTime(g.total_seconds)}</div>
        </div>
      </div>`).join('')}</div>
    <div class="hints"><span><b>A</b> / <b>Enter</b> Jogar</span><span><b>B</b> / <b>Esc</b> Sair do Modo TV</span><span><b>Direcional</b> / <b>setas</b> Escolher</span></div>`;
  document.querySelectorAll('#tv .card').forEach((el) => {
    el.addEventListener('mouseenter', () => { tvSel = Number(el.dataset.i); tvHighlight(); });
    el.addEventListener('click', () => { tvSel = Number(el.dataset.i); tvPlay(); });
  });
  tvHighlight();
}

function tvHighlight() {
  document.querySelectorAll('#tv .card').forEach((el) => el.classList.toggle('sel', Number(el.dataset.i) === tvSel));
  const sel = document.querySelector('#tv .card.sel');
  if (sel) sel.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

function tvColumns() {
  const cards = [...document.querySelectorAll('#tv .card')];
  if (cards.length < 2) return 1;
  const top = cards[0].offsetTop;
  const n = cards.findIndex((c) => c.offsetTop !== top);
  return n < 0 ? cards.length : n;
}

function tvMove(dx, dy) {
  const n = tvGames().length;
  if (!n) return;
  tvSel = Math.max(0, Math.min(n - 1, tvSel + dx + dy * tvColumns()));
  tvHighlight();
}

async function tvPlay() {
  const g = tvGames()[tvSel];
  if (!g) return;
  if (g.status !== 'ready') {
    toast(g.status === 'running' ? `${g.title} já está aberto.` : `${g.title}: ${STATUS_TEXT[g.status]}. Saia do Modo TV para resolver.`, true);
    return;
  }
  try {
    await invoke('play', { id: g.id });
    setGame(await invoke('get_game', { id: g.id }));
    renderTv();
  } catch (e) {
    toast(String(e), true);
  }
}

async function tvOpen() {
  tvOn = true;
  tvSel = 0;
  $('#tv').hidden = false;
  renderTv();
  try { await invoke('set_fullscreen', { on: true }); } catch { /* windowed is fine */ }
  requestAnimationFrame(tvPoll);
}

async function tvClose() {
  tvOn = false;
  $('#tv').hidden = true;
  try { await invoke('set_fullscreen', { on: false }); } catch { /* ignore */ }
}

// Standard gamepad mapping: 0 A, 1 B, 12-15 d-pad; left stick on axes 0/1.
function tvPoll(t) {
  if (!tvOn) return;
  const pad = [...(navigator.getGamepads ? navigator.getGamepads() : [])].find(Boolean);
  if (pad) {
    const b = (i) => pad.buttons[i] && pad.buttons[i].pressed;
    const ax = pad.axes[0] || 0;
    const ay = pad.axes[1] || 0;
    const dir = {
      left: b(14) || ax < -0.5, right: b(15) || ax > 0.5,
      up: b(12) || ay < -0.5, down: b(13) || ay > 0.5,
    };
    const press = (name, now) => {
      const was = tvPad.held[name];
      tvPad.held[name] = now;
      return now && !was;
    };
    if (press('a', b(0))) tvPlay();
    if (press('b', b(1))) tvClose();
    const any = dir.left || dir.right || dir.up || dir.down;
    // First move at once, then repeat while held.
    if (any && (press('dir', true) || t >= tvPad.nextAt)) {
      tvMove(dir.right - dir.left, dir.down - dir.up);
      tvPad.nextAt = t + (tvPad.repeating ? 120 : 380);
      tvPad.repeating = true;
    } else if (!any) {
      tvPad.held.dir = false;
      tvPad.repeating = false;
    }
  }
  requestAnimationFrame(tvPoll);
}

document.addEventListener('keydown', (e) => {
  if (!tvOn) return;
  const keys = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
  if (keys[e.key]) { tvMove(...keys[e.key]); e.preventDefault(); }
  else if (e.key === 'Enter') { tvPlay(); e.preventDefault(); }
  else if (e.key === 'Escape') { tvClose(); e.preventDefault(); }
});

$('#btn-tv').addEventListener('click', tvOpen);
$('#btn-settings').addEventListener('click', async () => {
  settings = await call('get_settings');
  openId = '__settings';
  render();
});

async function checkAppUpdate() {
  try {
    const u = await invoke('check_app_update');
    if (!u) return;
    const bar = $('#app-update');
    bar.innerHTML = `<span>Versão nova do RecompHUB: <b>${esc(u.version)}</b></span><button class="primary">Baixar</button>`;
    bar.hidden = false;
    bar.querySelector('button').addEventListener('click', () => call('open_app_update', { url: u.url }));
  } catch { /* offline */ }
}

// Keeps "Jogando há …" current.
setInterval(() => {
  // Not on the covers page: it would erase addresses being typed.
  if (openId !== '__covers' && games.some((g) => g.status === 'running')) render();
}, 20000);

// Show the library at once, then ask GitHub for new versions in the background.
invoke('get_settings').then((s) => { settings = s; render(); }).catch(() => {});
checkAppUpdate();

refresh()
  .then(async () => {
    const pending = await invoke('take_pending_open');
    if (pending) openGame(pending);
  })
  .then(() => invoke('check_updates').then((list) => { games = list; render(); }).catch(() => {}));
