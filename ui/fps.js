// The FPS counter over the game: numbers come from RecompHUB (fps.rs), about twice a second.
const box = document.getElementById('fps');
window.__TAURI__.event.listen('fps', (e) => {
  const n = Number(e.payload) || 0;
  box.firstChild.textContent = String(n);
  box.classList.toggle('low', n > 0 && n < 30);
});
