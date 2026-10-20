const { invoke } = window.__TAURI__.core;
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// A short chime made here (no sound file from anyone else): two or three rising notes.
function chime(complete) {
  try {
    const ctx = new AudioContext();
    ctx.resume().catch(() => {});
    const notes = complete ? [659.25, 783.99, 1046.5, 1318.5] : [783.99, 1174.66];
    notes.forEach((f, i) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.type = 'triangle';
      o.frequency.value = f;
      const t = ctx.currentTime + i * 0.11;
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(0.22, t + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, t + 0.5);
      o.connect(g).connect(ctx.destination);
      o.start(t);
      o.stop(t + 0.55);
    });
    setTimeout(() => ctx.close(), 1500);
  } catch { /* no audio device */ }
}

async function show(n) {
  const card = document.getElementById('card');
  const label = n.kind === 'complete' ? 'Jogo completo' : n.kind === 'test' ? 'Teste' : 'Conquista desbloqueada';
  card.className = 'card' + (n.kind === 'complete' ? ' complete' : '');
  card.innerHTML = `
    <div class="badge">${esc(n.badge || '🏆')}</div>
    <div>
      <div class="k">${label} · ${esc(n.game)}</div>
      <div class="t">${esc(n.title)}</div>
      <div class="d">${esc(n.description)}</div>
    </div>
    <div class="p">${n.points} pts</div>`;
  await sleep(30);
  card.classList.add('show');
  if (n.sound) chime(n.kind === 'complete');
  await sleep(5200);
  card.classList.remove('show');
  await sleep(450);
}

(async () => {
  let idle = 0;
  while (idle < 4) {
    const n = await invoke('take_overlay_notice').catch(() => null);
    if (n) { idle = 0; await show(n); } else { idle += 1; await sleep(400); }
  }
  window.__TAURI__.window.getCurrentWindow().close();
})();
