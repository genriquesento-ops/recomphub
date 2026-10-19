const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let games = [];
let openId = null;
let tab = 'all';          // 'all', 'port' or 'pc_game'
let tagFilter = null;     // one of the player's tags, or null
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

function chips(g) {
  let html = `<span class="chip family">${esc(g.family)}</span>`;
  html += `<span class="chip ${g.status}">${STATUS_TEXT[g.status]}</span>`;
  if (g.update_available) html += `<span class="chip update">Atualização</span>`;
  return html;
}

function visibleGames() {
  return games.filter((g) =>
    (tab === 'all' || g.kind === tab) && (!tagFilter || g.tags.includes(tagFilter)));
}

function renderTabs() {
  document.querySelectorAll('#tabs button').forEach((b) => b.classList.toggle('on', b.dataset.tab === tab));
  const tags = [...new Set(games.flatMap((g) => g.tags))].sort((a, b) => a.localeCompare(b, 'pt-BR'));
  if (tagFilter && !tags.includes(tagFilter)) tagFilter = null;
  $('#tagbar').innerHTML = tags.map((t) =>
    `<button class="tag${t === tagFilter ? ' on' : ''}" data-tag="${esc(t)}">${esc(t)}</button>`).join('');
  document.querySelectorAll('#tagbar .tag').forEach((b) => b.addEventListener('click', () => {
    tagFilter = tagFilter === b.dataset.tag ? null : b.dataset.tag;
    render();
  }));
}

function renderGrid() {
  renderTabs();
  const list = visibleGames();
  const empty = $('#empty');
  empty.hidden = list.length > 0;
  if (!list.length) {
    empty.textContent = tab === 'pc_game' && !tagFilter
      ? 'Nenhum jogo de PC ainda. Use "Adicionar jogo de PC" e escolha o programa (.exe) de um jogo que você já tem instalado.'
      : 'Nenhum jogo com esse filtro.';
  }
  $('#grid').innerHTML = list.map((g) => `
    <div class="card" data-id="${g.id}">
      ${coverHtml(g)}
      <div class="body">
        <div class="title">${esc(g.title)}</div>
        <div>${chips(g)}</div>
        <div class="meta">${g.status === 'running' ? `Jogando há ${sessionLength(g)}`
          : g.in_library ? `${playTime(g.total_seconds)} · última vez ${lastPlayed(g.last_played)}` : 'Fora da biblioteca'}</div>
      </div>
    </div>`).join('');
  document.querySelectorAll('.card').forEach((el) => el.addEventListener('click', () => openGame(el.dataset.id)));
}

function filesBox(g) {
  const picked = filesResult[g.id];
  const files = picked && !picked.ok ? picked : g.files;
  const label = g.files_kind === 'folder' ? 'Escolher a pasta do dump' : 'Escolher a ROM';
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
      <div class="row"><button class="primary" data-act="files">${label}</button></div>
    </div>
    ${guideBox(g)}`;
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
        ${tagsBox(g)}
        <div class="row">
          <button class="ghost" data-act="folder">Abrir pasta do jogo</button>
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

function renderDetail() {
  if (openId === '__covers') return renderCovers();
  const g = games.find((x) => x.id === openId);
  if (!g) {
    openId = null;
    render();
    return;
  }
  if (g.kind === 'pc_game') renderPcDetail(g);
  else renderPortDetail(g);
  document.querySelectorAll('#detail [data-act]').forEach((el) =>
    el.addEventListener('click', () => act(el.dataset.act, g, el)));
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
    main += tagsBox(g);
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
            <button class="ghost danger" data-act="remove">Remover da biblioteca</button>
          </div>
          <p class="muted">Remover da biblioteca não apaga nenhum arquivo. ${esc(g.install_dir || '')}</p>` : ''}
      </div>
    </div>`;
}

function render() {
  const inDetail = openId !== null;
  $('#library').hidden = inDetail;
  $('#detail').hidden = !inDetail;
  if (inDetail) renderDetail();
  else renderGrid();
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
      await call('open_project', { id: g.id });
      break;
    case 'folder':
      await call('open_folder', { id: g.id });
      break;
    case 'add':
      await addExisting();
      break;
    case 'install': {
      progress[g.id] = { stage: 'download', done: 0, total: 0 };
      g.status = 'installing';
      render();
      try {
        setGame(await invoke('install_game', { id: g.id }));
        toast(`${g.title} instalado.`);
      } catch (e) {
        toast(String(e), true);
        await refresh();
      }
      break;
    }
    case 'files': {
      const result = await call('provide_files', { id: g.id });
      if (!result) break;
      filesResult[g.id] = result;
      if (result.ok) {
        delete filesResult[g.id];
        toast('Arquivos conferidos. O jogo está pronto.');
      }
      setGame(await call('get_game', { id: g.id }));
      break;
    }
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
document.querySelectorAll('#tabs button').forEach((b) => b.addEventListener('click', () => {
  tab = b.dataset.tab;
  render();
}));
$('#btn-updates').addEventListener('click', async () => {
  games = await call('check_updates');
  render();
  const n = games.filter((g) => g.update_available).length;
  toast(n ? `${n} jogo(s) com versão nova.` : 'Tudo atualizado.');
});

listen('install-progress', (e) => {
  progress[e.payload.id] = e.payload;
  if (openId === e.payload.id) renderDetail();
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

// Keeps "Jogando há …" current.
setInterval(() => {
  // Not on the covers page: it would erase addresses being typed.
  if (openId !== '__covers' && games.some((g) => g.status === 'running')) render();
}, 20000);

// Show the library at once, then ask GitHub for new versions in the background.
refresh()
  .then(async () => {
    const pending = await invoke('take_pending_open');
    if (pending) openGame(pending);
  })
  .then(() => invoke('check_updates').then((list) => { games = list; render(); }).catch(() => {}));
