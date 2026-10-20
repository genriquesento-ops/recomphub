// The download button, the port list and the news come from the project itself, so the site
// never needs editing when a version or a port is added:
// - GitHub's "latest release" gives the installer;
// - catalog.json (copied here when the site is published) gives the ports;
// - the releases give the news.
const REPO = 'recomphub-app/recomphub';
const API = `https://api.github.com/repos/${REPO}`;

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const mb = (n) => `${(n / 1048576).toFixed(1).replace('.', ',')} MB`;
const date = (iso) => new Date(iso).toLocaleDateString('pt-BR', { day: '2-digit', month: 'long', year: 'numeric' });

async function download() {
  try {
    const r = await fetch(`${API}/releases/latest`);
    if (!r.ok) return;
    const rel = await r.json();
    const exe = (rel.assets || []).find((a) => a.name.toLowerCase().endsWith('.exe'));
    if (!exe) return;
    document.getElementById('download').href = exe.browser_download_url;
    document.getElementById('download-info').textContent = `versão ${rel.tag_name.replace(/^v/, '')} · ${mb(exe.size)}`;
  } catch { /* offline: the button still goes to the releases page */ }
}

async function ports() {
  const grid = document.getElementById('port-grid');
  try {
    const c = await (await fetch('catalog.json')).json();
    grid.innerHTML = c.ports.map((p) => `
      <div class="port">
        <span class="fam">${esc(p.family)}</span>
        <b>${esc(p.title)}</b>
        <span class="by">${esc(p.port_name)}, por ${esc(p.port_authors)}</span>
        <a href="${esc(p.project_url)}">Página do projeto →</a>
      </div>`).join('');
  } catch {
    grid.innerHTML = '<p class="sub">Veja a lista de ports dentro do programa, na aba Descobrir.</p>';
  }
}

// Release notes: the commit-style "- item" lines become a list.
function notes(body) {
  const items = String(body || '').split('\n').map((l) => l.trim()).filter((l) => l.startsWith('- ')).map((l) => l.slice(2));
  return items.length ? `<ul>${items.slice(0, 6).map((i) => `<li>${esc(i)}</li>`).join('')}</ul>` : '';
}

async function news() {
  const box = document.getElementById('releases');
  try {
    const r = await fetch(`${API}/releases?per_page=5`);
    if (!r.ok) throw new Error();
    const list = (await r.json()).filter((x) => !x.draft);
    if (!list.length) throw new Error();
    box.innerHTML = list.map((x) => `
      <div class="release">
        <h3>${esc(x.name || x.tag_name)}</h3>
        <span class="date">${date(x.published_at)}</span>
        ${notes(x.body)}
      </div>`).join('') + `<p><a href="https://github.com/${REPO}/releases">Todas as versões →</a></p>`;
  } catch {
    box.innerHTML = `<p class="sub">As novidades de cada versão ficam na <a href="https://github.com/${REPO}/releases">página de versões</a>.</p>`;
  }
}

// "Apoie o projeto": filled from doacoes.json; hidden while it has nothing.
async function donations() {
  let d;
  try {
    d = await (await fetch('doacoes.json')).json();
  } catch {
    return;
  }
  const links = (d.links || []).filter((l) => /^https:\/\//.test(l.url || ''));
  const pix = d.pix && d.pix.chave ? d.pix : null;
  if (!links.length && !pix) return;
  document.getElementById('apoie-msg').textContent = d.mensagem || '';
  const cards = [];
  if (pix) {
    cards.push(`
      <div class="donate-card pix">
        <b>Pix</b>
        <span class="muted">${esc(pix.nome || 'Chave Pix')}</span>
        <code id="pix-key">${esc(pix.chave)}</code>
        <button class="btn small primary" id="pix-copy">Copiar chave</button>
      </div>`);
  }
  for (const l of links) {
    cards.push(`
      <a class="donate-card" href="${esc(l.url)}" target="_blank" rel="noopener">
        <b>${esc(l.nome)}</b>
        <span class="muted">${esc(l.descricao || '')}</span>
        <span class="go">Abrir →</span>
      </a>`);
  }
  document.getElementById('apoie-opcoes').innerHTML = cards.join('');
  document.getElementById('apoie').hidden = false;
  document.getElementById('nav-apoie').hidden = false;
  const copy = document.getElementById('pix-copy');
  if (copy) {
    copy.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(pix.chave);
        copy.textContent = 'Copiada!';
      } catch {
        copy.textContent = 'Selecione e copie';
      }
      setTimeout(() => { copy.textContent = 'Copiar chave'; }, 2500);
    });
  }
}

download();
ports();
news();
donations();
