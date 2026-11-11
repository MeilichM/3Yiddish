'use strict';
const $ = id => document.getElementById(id), player = $('player');
let session = sessionStorage.getItem('3yiddish-phone-session');
let armed = false, recorder, stream, timer, chunks = [], discard = false, restarting = false;
let polling = false, starting = false, uploading = false, revision = 0, objectURL, wake = null;
// No sleep while the game is open: screen wake lock, re-taken whenever Android drops it.
async function keepAwake() {
  if (wake || !armed || document.hidden || !navigator.wakeLock) return;
  try { wake = await navigator.wakeLock.request('screen'); wake.addEventListener('release', () => { wake = null; }); } catch {}
}
async function tap(action) {
  try { await api('/api/game', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({action})}); }
  catch (error) { status('Not sent', error.message); }
}
['record','skip','back'].forEach(id => { $(id).onclick = () => { void keepAwake(); void tap(id); }; });
const shown = (id, on) => { $(id).hidden = !on; };
// Playground: tap the version that is what you actually said. Unsure-and-unanswered first.
let pgMode = false, pgN = null, pgKey = '';
const PG = [['gold', "Gold: judge's guess"], ['line', 'Line on screen'], ['yl', 'Reference heard'], ['ours', 'Ours heard']];
$('pg-open').onclick = () => { pgMode = true; listMode = false; pgKey = ''; };
$('pg-close').onclick = () => { pgMode = false; };
function pgOrder(items) { return [...items].sort((a, b) => ((a.unsure && !a.choice) ? 0 : 1) - ((b.unsure && !b.choice) ? 0 : 1) || a.n - b.n); }
function pgStep(items, by) { const o = pgOrder(items); const i = Math.max(0, o.findIndex(x => x.n === pgN)); pgN = o[(i + by + o.length) % o.length].n; pgKey = ''; }
function renderPg(items) {
  if (!items.length) { $('pg-title').textContent = 'No playground yet'; return; }
  const order = pgOrder(items);
  if (pgN == null || !items.some(x => x.n === pgN)) pgN = order[0].n;
  const it = items.find(x => x.n === pgN), left = items.filter(x => x.unsure && !x.choice).length;
  $('pg-title').textContent = `${left} to check · #${it.n}${it.unsure && !it.choice ? '  ⚠ judge unsure' : ''}`;
  const key = JSON.stringify(it); if (key === pgKey) return; pgKey = key;
  const box = $('pg-cards'); box.replaceChildren();
  for (const [k, title] of PG) {
    const c = el('div', `card choice${k === 'gold' ? ' gold' : ''}${it.choice === k ? ' picked' : ''}`);
    const p = el('p', '', it[k] || ''); p.dir = 'rtl'; p.lang = 'yi';
    c.append(el('small', '', title), p);
    c.onclick = () => { void tap(`pick:${it.n}-${k}`); pgStep(items, 1); };
    box.append(c);
  }
  $('pg-prev').onclick = () => pgStep(items, -1); $('pg-next').onclick = () => pgStep(items, 1);
  $('pg-play').onclick = () => it.id && tap(`play:${it.id}`);
  $('pg-toss').onclick = () => { void tap(`pick:${it.n}-toss`); pgStep(items, 1); };
}
// "My readings": every reading, newest first, with Play / Toss / Redo. Rebuilt only on change.
let listMode = false, listKey = '';
$('list-open').onclick = () => { listMode = true; pgMode = false; listKey = ''; };
$('list-close').onclick = () => { listMode = false; };
function el(tag, cls, text) { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; }
function renderList(items) {
  const key = JSON.stringify(items); if (key === listKey) return; listKey = key;
  $('list-title').textContent = `${items.length} readings`;
  const box = $('items'); box.replaceChildren();
  if (!items.length) { box.append(el('p', 'heard', 'Nothing read yet.')); return; }
  for (const r of items) {
    const card = el('div', 'card item'), top = el('div', 'top');
    top.append(el('span', 'score', r.score == null ? '' : `${r.score}%`));
    const line = el('p', '', r.line || ''); line.dir = 'rtl'; line.lang = 'yi';
    const heard = el('p', 'heard', r.heard || ''); heard.dir = 'rtl'; heard.lang = 'yi';
    const actions = el('div', 'actions');
    const play = el('button', '', '▶ Play'), toss = el('button', 'toss', '✗ Toss'), redo = el('button', '', '↺ Redo');
    play.onclick = () => tap(`play:${r.id}`);
    // Two taps to toss, so a slip of the thumb does not throw a reading away.
    toss.onclick = () => {
      if (!toss.classList.contains('armed')) { toss.classList.add('armed'); toss.textContent = 'Sure?'; setTimeout(() => { toss.classList.remove('armed'); toss.textContent = '✗ Toss'; }, 3000); return; }
      void tap(`toss:${r.id}`);
    };
    redo.onclick = () => { listMode = false; void tap(`redo:${r.id}`); };
    actions.append(play, toss, redo); card.append(top, line, heard, actions); box.append(card);
  }
}
function render(data) {
  const g = data.game || {}, r = g.result, rec = !!(data.recording || data.record_requested);
  shown('setup', !armed); shown('game', armed && !listMode && !pgMode);
  shown('list', armed && listMode); shown('pg', armed && pgMode);
  shown('list-open', armed && !listMode && !pgMode && !rec);
  shown('pg-open', armed && !listMode && !pgMode && !rec && (g.playground || []).length > 0);
  if (listMode) renderList(g.readings || []);
  if (pgMode) renderPg(g.playground || []);
  document.body.dataset.recording = rec ? 'true' : 'false';
  $('progress').textContent = g.progress || '';
  $('headline').textContent = !r ? 'Tap Record, read the line, tap Stop.'
    : r.missed_audio ? "🤷 Didn't catch that. Record it again."
    : `${r.badge} ${r.score}%   ·   avg ${r.average}%`;
  shown('result', !!r && !r.missed_audio);
  if (r && !r.missed_audio) {
    $('read').textContent = r.line || ''; $('heard').textContent = r.heard || '';
    $('missed').textContent = (r.missed || []).length ? '✗ ' + r.missed.join('  ') : '';
  }
  $('line').textContent = g.line || 'No unread lines left.';
  $('next-title').textContent = rec ? '● Recording: read this' : g.listening ? '👂 Listening back…' : 'Next';
  $('record').textContent = rec ? '■ Stop' : '● Record';
  $('record').disabled = !!g.listening && !rec;
  // Locked until the score is back, so Back cannot skip past the reading being transcribed.
  $('skip').disabled = rec || !!g.listening; $('back').disabled = rec || !!g.listening;
  shown('back', !!g.back && !rec); shown('skip', !rec); shown('restart', rec);
}
function status(text, message) {
  $('connection').textContent = text;
  if (message !== undefined) $('message').textContent = message;
}
async function api(path, options = {}) {
  const response = await fetch(path, { ...options, signal: AbortSignal.timeout(path === '/api/audio' ? 600000 : 4000), headers: { ...(session ? {Authorization: `Bearer ${session}`} : {}), ...(options.headers || {}) }, cache: 'no-store' });
  if (!response.ok) {
    let error; try { error = await response.json(); } catch {}
    throw new Error(error?.error?.message || `Connection failed (${response.status})`);
  }
  return response;
}
// The PC decodes WAV natively, so recordings go up as 16 kHz mono 16-bit WAV.
async function toWav(blob) {
  const ctx = new AudioContext(); const audio = await ctx.decodeAudioData(await blob.arrayBuffer()); void ctx.close();
  const off = new OfflineAudioContext(1, Math.max(1, Math.ceil(audio.duration * 16000)), 16000);
  const src = off.createBufferSource(); src.buffer = audio; src.connect(off.destination); src.start();
  const pcm = (await off.startRendering()).getChannelData(0);
  const out = new DataView(new ArrayBuffer(44 + pcm.length * 2)), text = (o, s) => [...s].forEach((c, i) => out.setUint8(o + i, c.charCodeAt(0)));
  text(0, 'RIFF'); out.setUint32(4, 36 + pcm.length * 2, true); text(8, 'WAVEfmt '); out.setUint32(16, 16, true);
  out.setUint16(20, 1, true); out.setUint16(22, 1, true); out.setUint32(24, 16000, true); out.setUint32(28, 32000, true);
  out.setUint16(32, 2, true); out.setUint16(34, 16, true); text(36, 'data'); out.setUint32(40, pcm.length * 2, true);
  pcm.forEach((v, i) => out.setInt16(44 + i * 2, Math.max(-1, Math.min(1, v)) * 0x7fff, true));
  return new Blob([out.buffer], {type: 'audio/wav'});
}
async function report(error) {
  await api('/api/mic', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({ready:armed, recording:recorder?.state === 'recording', error:error || null})});
}
function stop(cancel = false) {
  discard = cancel; clearTimeout(timer);
  if (recorder?.state === 'recording') recorder.stop();
  stream?.getTracks().forEach(track => track.stop());
  document.body.dataset.recording = 'false';
}
async function start() {
  if (!armed || starting || uploading || recorder?.state === 'recording') return;
  starting = true;
  try {
    stream = await navigator.mediaDevices.getUserMedia({audio:true});
    // A stop/disconnect can arrive while Android's permission request is open.
    const current = await (await api('/api/status')).json();
    if (!armed || !current.record_requested || document.hidden) { stream.getTracks().forEach(t => t.stop()); return; }
    const mime = ['audio/webm;codecs=opus','audio/mp4','audio/ogg;codecs=opus'].find(m => MediaRecorder.isTypeSupported(m));
    recorder = new MediaRecorder(stream, mime ? {mimeType:mime} : {});
    chunks = []; discard = false;
    recorder.ondataavailable = event => { if (event.data.size) chunks.push(event.data); };
    recorder.onstop = async () => {
      const blob = new Blob(chunks, {type:recorder.mimeType}); chunks = [];
      stream?.getTracks().forEach(track => track.stop());
      // Restart: drop this take; the PC still wants a recording, so just record again.
      if (restarting) { restarting = false; void start(); return; }
      try {
        if (!discard) uploading = true;
        await report();
        if (!discard && blob.size) {
          if (blob.size > 25*1024*1024) throw new Error('Recording exceeds 25 MiB.');
          uploading = true; status('Sending to PC…');
          await api('/api/audio', {method:'POST', headers:{'Content-Type':'application/octet-stream'}, body: await toWav(blob)});
        }
        status(armed ? 'Paired' : 'Paused', armed ? undefined : 'Tap Start to turn the microphone back on.');
      } catch (error) { status('Audio error', error.message); }
      finally { uploading = false; }
    };
    recorder.start(1000); document.body.dataset.recording = 'true';
    status('Recording'); await report();
    timer = setTimeout(() => stop(), 120000);
  } catch (error) { stop(true); armed = false; $('enable').hidden = false; status('Microphone unavailable', error.message); await report(error.message).catch(() => {}); }
  finally { starting = false; }
}
$('enable').onclick = async () => {
  $('enable').disabled = true;
  try {
    const permission = await navigator.mediaDevices.getUserMedia({audio:true});
    permission.getTracks().forEach(track => track.stop());
    // Unlock this audio element under the phone's explicit tap.
    player.src = 'data:audio/wav;base64,UklGRiYAAABXQVZFZm10IBAAAAABAAEAgD4AAAB9AAACABAAZGF0YQIAAAAAAA==';
    await player.play().catch(() => {}); player.pause();
    armed = true; await report(); $('enable').hidden = true;
    status('Paired');
    document.documentElement.requestFullscreen?.({navigationUI:'hide'}).catch(() => {});
    await keepAwake();
  } catch (error) { status('Microphone unavailable', error.message); }
  finally { $('enable').disabled = false; }
};
$('restart').onclick = () => {
  if (recorder?.state !== 'recording') return;
  restarting = true; discard = true; clearTimeout(timer); recorder.stop();
};
$('play').onclick = () => player.play().then(() => { $('play').hidden = true; }).catch(error => status('Playback blocked', error.message));
player.onplay = () => { if (revision) api('/api/played', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({revision})}).catch(() => {}); };
async function poll() {
  if (!session || polling) return;
  polling = true;
  try {
    const data = await (await api('/api/status')).json();
    render(data); void keepAwake();
    if (data.record_requested && armed && !document.hidden) { void start(); }
    else if (!data.record_requested && recorder?.state === 'recording') stop();
    if (data.playback_revision && data.playback_revision !== revision) {
      const blob = await (await api(`/api/playback/${data.playback_revision}`)).blob();
      if (objectURL) URL.revokeObjectURL(objectURL);
      objectURL = URL.createObjectURL(blob); player.src = objectURL; revision = data.playback_revision;
      try { await player.play(); } catch { $('play').hidden = false; }
    }
  } catch (error) {
    stop(true); armed = false; $('enable').hidden = !window.isSecureContext;
    status('Disconnected', error.message);
  } finally { polling = false; }
}
document.addEventListener('visibilitychange', () => {
  if (document.hidden) { armed = false; stop(true); report('Phone page hidden; enable audio again.').catch(() => {}); }
  else { $('enable').hidden = !window.isSecureContext; shown('setup', true); shown('game', false); status('Paused', 'Tap Start to keep reading.'); }
});
window.addEventListener('pagehide', () => { armed = false; stop(true); });
(async () => {
  const invite = location.hash.slice(1); history.replaceState(null,'',location.pathname);
  if (invite) {
    try {
      const data = await (await api('/api/connect', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({invite})})).json();
      session = data.session; sessionStorage.setItem('3yiddish-phone-session', session);
    } catch (error) { session = null; sessionStorage.removeItem('3yiddish-phone-session'); status('Not paired', error.message); return; }
  }
  if (!session) { status('Not paired', 'Scan the QR in the PC terminal.'); return; }
  if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia || !window.MediaRecorder) {
    status('Mic unavailable', 'Open the trusted HTTPS pairing link from the PC terminal. HTTP cannot use your microphone.');
  } else { $('enable').hidden = false; status('Connected', 'Tap Start to play. The screen stays on while you read.'); }
  await poll(); setInterval(poll, 500);
})();
