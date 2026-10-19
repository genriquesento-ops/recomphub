const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let games = [];
let openId = null;
const progress = {};      // id -> { stage, done, total }
const filesResult = {};   // id -> last check after the player picked files

const STATUS_TEXT = {
  ready: 'Instalado',
  needs_files: 'Precisa de arquivos',
  not_installed: 'Não instalado',
  running: 'Jogando',
  installing: 'Instalando',
};

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

function renderGrid() {
  $('#grid').innerHTML = games.map((g) => `
    <div class="card" data-id="${g.id}">
      ${coverHtml(g)}
      <div class="body">
        <div class="title">${esc(g.title)}</div>
        <div>${chips(g)}</div>
        <div class="meta">${g.in_library ? `${playTime(g.total_seconds)} · última vez ${lastPlayed(g.last_played)}` : 'Fora da biblioteca'}</div>
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

function renderDetail() {
  const g = games.find((x) => x.id === openId);
  if (!g) return;
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
      </div>`;
  } else {
    if (g.update_available) {
      main += `
        <div class="box warn">
          <h3>Versão nova disponível${latest ? `: ${esc(latest)}` : ''}</h3>
          <p>Seus saves e configurações são mantidos.</p>
          <div class="row"><button data-act="install" ${g.status === 'running' ? 'disabled' : ''}>Atualizar</button></div>
        </div>`;
    }
    main += `
      <div class="row">
        <button class="primary play" data-act="play" ${g.status !== 'ready' ? 'disabled' : ''}>${g.status === 'running' ? 'Jogando…' : 'Jogar'}</button>
      </div>`;
    main += filesBox(g);
  }

  $('#detail').innerHTML = `
    <button class="ghost back" data-act="back">← Biblioteca</button>
    <div class="detail">
      <div>${coverHtml(g)}</div>
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

  document.querySelectorAll('#detail [data-act]').forEach((el) =>
    el.addEventListener('click', () => act(el.dataset.act, g)));
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

async function act(action, g) {
  switch (action) {
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

$('#btn-add').addEventListener('click', addExisting);
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
  setGame(await invoke('get_game', { id }));
  if (state === 'ended' && seconds > 0) {
    const g = games.find((x) => x.id === id);
    toast(`Sessão de ${playTime(seconds).toLowerCase()} somada a ${g ? g.title : id}.`);
  }
});

// Show the library at once, then ask GitHub for new versions in the background.
refresh().then(() => invoke('check_updates').then((list) => { games = list; render(); }).catch(() => {}));
