'use strict';
// The playground on the phone: record a phrase, read the draft, mark what is wrong.
// Everything is kept on the computer; this page only shows it and sends taps.

// ---- Helpers without a page (tests/phone.test.cjs loads these alone) ----

const REASONS = [
  ['wrong', 'Wrong word'],
  ['weird', 'Weird'],
  ['fluency', 'Not fluent / my fault'],
  ['accent', 'Wrong accent'],
  ['loshn-koydesh', 'Loshn koydesh'],
  ['name', 'Name'],
  ['english', 'English word'],
  ['custom', 'Custom…'],
];
const LABEL = Object.fromEntries([...REASONS, ['dropped', 'Dropped word'], ['custom', 'Custom']]);
const STATUS = {pending: 'To review', correct: 'Correct', fixed: 'Fixed', tossed: 'Tossed'};

function sortMarks(marks) {
  return [...marks].sort((a, b) => a.start - b.start || a.end - b.end);
}
// The draft with the marks applied: the same rule as the computer's (src/freebench.rs).
function applyMarks(words, marks) {
  const out = [];
  let at = 0;
  for (const m of sortMarks(marks)) {
    out.push(...words.slice(at, m.start));
    if (m.right == null) out.push(...words.slice(m.start, m.end));
    else out.push(...m.right.split(/\s+/).filter(Boolean));
    at = m.end;
  }
  out.push(...words.slice(at));
  return out.join(' ');
}
// Would a mark over words start..end (end not included; start == end is a place between two
// words) run into another mark? `skip` is the mark being edited.
function clashes(marks, start, end, skip) {
  return marks.some((m, k) => {
    if (k === skip) return false;
    const place = start === end, other = m.start === m.end;
    if (place && other) return m.start === start;
    if (place) return m.start < start && start < m.end;
    if (other) return start < m.start && m.start < end;
    return m.start < end && start < m.end;
  });
}
function clock(seconds) {
  const s = Math.max(0, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
// Samples (-1..1) as a 16-bit mono WAV file. The computer brings it to 16 kHz.
function wavBytes(pcm, rate) {
  const view = new DataView(new ArrayBuffer(44 + pcm.length * 2));
  const text = (at, s) => { for (let i = 0; i < s.length; i++) view.setUint8(at + i, s.charCodeAt(i)); };
  text(0, 'RIFF'); view.setUint32(4, 36 + pcm.length * 2, true); text(8, 'WAVEfmt ');
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, rate, true); view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  text(36, 'data'); view.setUint32(40, pcm.length * 2, true);
  for (let i = 0; i < pcm.length; i++) {
    const v = Math.max(-1, Math.min(1, pcm[i]));
    view.setInt16(44 + i * 2, v < 0 ? v * 0x8000 : v * 0x7fff, true);
  }
  return view.buffer;
}

// ---- The page ----

function startPage() {
  const $ = id => document.getElementById(id);
  const el = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  };
  const icon = (use, name) => use.setAttribute('href', '#i-' + name);
  const player = $('player');
  // Browsers give the microphone to secure pages only. On a plain link the phone's own
  // recorder is used instead, through a file field.
  const live = window.isSecureContext && !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
  const MAX_SECONDS = 180;

  let items = [], goal = 100, screen = '';
  let take = null, marks = [], typed = null, fromList = false;
  let sel = null, editing = -1, reason = 'wrong', touched = false;
  let rec = null, starting = false, busy = false, lastBlob = null, frame = 0;
  let loaded = null, audioURL = null, alertTimer = 0;

  async function api(path, options = {}) {
    let response;
    try { response = await fetch('api/' + path, {cache: 'no-store', ...options}); }
    catch { throw new Error('Cannot reach the computer. Is 3Yiddish open there?'); }
    let data = null;
    try { data = await response.json(); } catch {}
    if (!response.ok) {
      throw new Error((data && data.error && data.error.message)
        || (response.status === 404 ? 'This link no longer works. Pair the phone again from the app.' : `The computer answered ${response.status}.`));
    }
    return data;
  }
  function say(text) {
    const a = $('alert');
    a.textContent = text || '';
    a.hidden = !text;
    clearTimeout(alertTimer);
    if (text) { alertTimer = setTimeout(() => { a.hidden = true; }, 8000); a.scrollIntoView({block: 'nearest'}); }
  }
  const done = () => items.filter(i => i.status === 'correct' || i.status === 'fixed').length;

  function renderTop() {
    const n = done();
    $('count').replaceChildren(String(n), el('small', '', goal ? ` / ${goal}` : ''));
    $('progress').hidden = !goal;
    $('progress-fill').style.width = (goal ? Math.min(100, n / goal * 100) : 0) + '%';
    $('progress').classList.toggle('complete', goal > 0 && n >= goal);
    $('progress').setAttribute('aria-valuemax', goal);
    $('progress').setAttribute('aria-valuenow', Math.min(n, goal));
    $('nav-back').hidden = screen === 'record';
    $('nav-list').hidden = screen === 'list';
    $('take-no').textContent = `Take ${n + 1}`;
    $('take-of').textContent = !goal ? '' : n >= goal ? `The set of ${goal} is complete. More is fine.` : `of ${goal}`;
  }
  function show(name) {
    if (screen === 'review' && name !== 'review') { closeSheet(true); player.pause(); }
    screen = name;
    for (const id of ['record', 'review', 'list']) $(id).hidden = id !== name;
    renderTop();
    if (name === 'list') renderList();
    window.scrollTo(0, 0);
  }
  // The takes and the idea as the computer has them (the app there may have changed them).
  // `quiet`: only ask, so the computer knows the phone is here; mid-review nothing changes.
  async function refresh(quiet) {
    const data = await api('state');
    if (quiet) return;
    items = data.items;
    goal = data.goal;
    showIdea(data.idea);
    renderTop();
    if (screen === 'list') renderList();
  }

  // ---- Record ----

  // One idea for the phone and the app, kept on the computer: "another idea" on either
  // changes it on both.
  function showIdea(text) {
    if (typeof text === 'string') $('idea-text').textContent = text;
  }
  async function nextIdea() {
    try { showIdea((await api('idea', {method: 'POST'})).idea); } catch (error) { say(error.message); }
  }
  function recordingLook(on) {
    $('rec').classList.toggle('on', on);
    $('rec').setAttribute('aria-label', on ? 'Stop' : 'Record');
    icon($('rec-icon'), on ? 'stop' : 'mic');
    $('wave').hidden = !on;
    $('timer').hidden = !on;
    $('hint').hidden = on;
    $('nav-list').disabled = on;
    $('idea-next').disabled = on;
  }
  function drawWave() {
    if (!rec) return;
    const canvas = $('wave'), ratio = window.devicePixelRatio || 1;
    const w = canvas.clientWidth, h = canvas.clientHeight;
    if (canvas.width !== Math.round(w * ratio)) { canvas.width = Math.round(w * ratio); canvas.height = Math.round(h * ratio); }
    const c = canvas.getContext('2d');
    c.setTransform(ratio, 0, 0, ratio, 0, 0);
    c.clearRect(0, 0, w, h);
    c.fillStyle = '#ff5a6a';
    const step = 7, bars = Math.floor(w / step), levels = rec.levels.slice(-bars);
    levels.forEach((level, i) => {
      const height = Math.max(4, Math.min(1, level) * h), x = w - (levels.length - i) * step;
      c.beginPath();
      c.roundRect ? c.roundRect(x, (h - height) / 2, 4, height, 2) : c.rect(x, (h - height) / 2, 4, height);
      c.fill();
    });
    const seconds = (performance.now() - rec.started) / 1000;
    $('timer').textContent = clock(Math.floor(seconds));
    if (seconds >= MAX_SECONDS) { stopRecording(); return; }
    frame = requestAnimationFrame(drawWave);
  }
  async function startRecording() {
    if (busy || starting || rec) return;
    starting = true;
    say('');
    $('retry').hidden = true;
    // Made inside the tap, before anything is waited for: Safari starts sound only then.
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    ctx.resume().catch(() => {});
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({audio: {channelCount: 1}});
    } catch (error) {
      ctx.close();
      starting = false;
      say(error && error.name === 'NotAllowedError'
        ? 'The microphone is blocked for this page. Allow it in the browser, then tap again.'
        : 'The microphone could not be opened.');
      return;
    }
    const source = ctx.createMediaStreamSource(stream);
    // Nothing is played back: the sound ends in a silent output so the browser keeps it flowing.
    const quiet = ctx.createGain();
    quiet.gain.value = 0;
    quiet.connect(ctx.destination);
    const state = {ctx, stream, chunks: [], samples: 0, levels: [], started: performance.now(), nodes: [source, quiet], wake: null};
    const keep = block => {
      state.chunks.push(block);
      state.samples += block.length;
      let peak = 0;
      for (let i = 0; i < block.length; i += 4) peak = Math.max(peak, Math.abs(block[i]));
      state.levels.push(Math.sqrt(peak) * 1.15);
    };
    let node;
    try {
      await ctx.audioWorklet.addModule('recorder.js');
      node = new AudioWorkletNode(ctx, 'take');
      node.port.onmessage = event => keep(event.data);
    } catch {
      // A browser without audio worklets.
      node = ctx.createScriptProcessor(4096, 1, 1);
      node.onaudioprocess = event => keep(new Float32Array(event.inputBuffer.getChannelData(0)));
    }
    source.connect(node);
    node.connect(quiet);
    state.nodes.push(node);
    state.started = performance.now();
    rec = state;
    starting = false;
    recordingLook(true);
    frame = requestAnimationFrame(drawWave);
    // The screen stays on while recording.
    if (navigator.wakeLock) navigator.wakeLock.request('screen').then(lock => { if (rec === state) state.wake = lock; else lock.release(); }).catch(() => {});
  }
  function stopRecording() {
    const r = rec;
    if (!r) return;
    rec = null;
    cancelAnimationFrame(frame);
    for (const node of r.nodes) { try { node.disconnect(); } catch {} }
    r.stream.getTracks().forEach(track => track.stop());
    const rate = r.ctx.sampleRate;
    r.ctx.close();
    if (r.wake) r.wake.release().catch(() => {});
    recordingLook(false);
    if (r.samples < rate * 0.5) { say('That was too short. Tap, speak, then tap again.'); return; }
    const pcm = new Float32Array(r.samples);
    let at = 0;
    for (const chunk of r.chunks) { pcm.set(chunk, at); at += chunk.length; }
    send(new Blob([wavBytes(pcm, rate)], {type: 'audio/wav'}));
  }
  // The phone's own recorder gave a file: turn it into plain sound here when the browser can
  // read it, else send it as it is and let the computer try.
  async function picked(file) {
    if (!file || busy) return;
    let blob = file;
    try {
      const ctx = new (window.AudioContext || window.webkitAudioContext)();
      const audio = await ctx.decodeAudioData(await file.arrayBuffer());
      ctx.close();
      const pcm = new Float32Array(audio.length);
      for (let channel = 0; channel < audio.numberOfChannels; channel++) {
        const data = audio.getChannelData(channel);
        for (let i = 0; i < data.length; i++) pcm[i] += data[i] / audio.numberOfChannels;
      }
      blob = new Blob([wavBytes(pcm, audio.sampleRate)], {type: 'audio/wav'});
    } catch {}
    send(blob);
  }
  async function send(blob) {
    busy = true;
    lastBlob = blob;
    say('');
    $('retry').hidden = true;
    $('hint').hidden = true;
    $('thinking').hidden = false;
    $('rec').disabled = true;
    try {
      const item = await api('takes', {method: 'POST', body: blob, headers: {'Content-Type': blob.type || 'application/octet-stream'}});
      lastBlob = null;
      items.push(item);
      openReview(item, false);
    } catch (error) {
      // The answer was lost (the screen locked, the network dropped), but the computer may
      // have kept the take: look before offering to send it a second time.
      const known = new Set(items.map(i => i.n));
      let kept = null;
      try { await refresh(); kept = items.find(i => !known.has(i.n) && i.status === 'pending'); } catch {}
      if (kept) { lastBlob = null; openReview(kept, false); }
      else { say(error.message); $('retry').hidden = false; }
    } finally {
      busy = false;
      $('thinking').hidden = true;
      $('hint').hidden = !$('retry').hidden;
      $('rec').disabled = false;
    }
  }

  // ---- Listen ----

  async function listen(n, play) {
    if (loaded !== n) {
      player.pause();
      loaded = n;
      if (audioURL) URL.revokeObjectURL(audioURL);
      audioURL = null;
      player.removeAttribute('src');
      syncPlayer();
      try {
        // Fetched whole and played from memory, so seeking works in every browser.
        const response = await fetch(`api/audio/${n}`, {cache: 'no-store'});
        if (!response.ok) throw new Error();
        const blob = await response.blob();
        if (loaded !== n) return;
        audioURL = URL.createObjectURL(blob);
        player.src = audioURL;
      } catch {
        if (loaded === n) loaded = null;
        say('The recording could not be loaded.');
        return;
      }
    }
    if (play) player.play().catch(() => {});
  }
  function length() {
    if (Number.isFinite(player.duration) && player.duration > 0) return player.duration;
    const item = items.find(i => i.n === loaded);
    return item ? item.seconds : 0;
  }
  function syncPlayer() {
    const playing = !player.paused && !player.ended;
    icon($('play-icon'), playing ? 'pause' : 'play');
    $('play').setAttribute('aria-label', playing ? 'Pause' : 'Play');
    const total = length(), at = player.src ? player.currentTime : 0;
    $('seek').value = total ? Math.round(at / total * 1000) : 0;
    $('time').textContent = `${clock(at)} / ${clock(total)}`;
    for (const use of document.querySelectorAll('.mini use')) {
      icon(use, playing && Number(use.dataset.n) === loaded ? 'pause' : 'play');
    }
  }

  // ---- Review ----

  function openReview(item, list) {
    take = item;
    marks = item.marks.map(m => ({...m}));
    typed = item.typed ? item.final : null;
    fromList = list;
    sel = null;
    editing = -1;
    $('sheet').hidden = true;
    $('app').classList.remove('sheet-open');
    show('review');
    renderReview();
    listen(item.n, false);
  }
  function finalText() {
    return typed != null ? typed.trim().replace(/\s+/g, ' ') : applyMarks(take.words, marks);
  }
  function renderFinal() {
    const text = finalText(), changed = marks.length > 0 || typed != null;
    $('final').textContent = text || '(nothing)';
    $('final-card').classList.toggle('same', !changed);
    const save = $('save');
    save.textContent = changed ? (fromList ? 'Save' : 'Save & next') : 'All correct';
    save.classList.toggle('good', !changed);
  }
  function renderReview() {
    $('take-title').textContent = `Take ${take.n}`;
    const status = $('take-status');
    status.textContent = STATUS[take.status] || take.status;
    status.className = 'status ' + take.status;
    const box = $('words');
    box.replaceChildren();
    box.hidden = typed != null;
    $('typed').hidden = typed == null;
    // The field shows the text being kept (a reopened take's too), never an older one.
    if (typed != null && $('typed').value !== typed) $('typed').value = typed;
    $('type-out').textContent = typed != null ? 'Back to marking words' : 'Type it out instead';
    $('help').textContent = typed != null ? 'Type exactly what you said.'
      : take.words.length ? 'Tap a word that is wrong. Tap a second word to mark everything between them.'
        : 'Nothing was heard. Type what you said, or toss the take.';
    renderFinal();
    if (typed != null) return;
    const word = k => {
      const b = el('button', 'w', take.words[k]);
      b.dataset.i = k;
      if (sel && sel.start <= k && k < sel.end && editing < 0) b.classList.add('sel');
      return b;
    };
    for (let p = 0; p <= take.words.length;) {
      // A place between two words: a missing word, saved or being added.
      const place = marks.findIndex(m => m.start === p && m.end === p);
      const adding = sel && sel.start === p && sel.end === p;
      if (place >= 0 || adding) {
        const pill = el('button', 'ins r-dropped' + (adding ? ' sel' : ''));
        pill.dataset.k = place;
        pill.append(el('span', '', place >= 0 ? marks[place].right : '+'), el('small', '', 'missing'));
        box.append(pill);
      }
      if (p === take.words.length) break;
      const k = marks.findIndex(m => m.start === p && m.end > p);
      if (k < 0) { box.append(word(p)); p++; continue; }
      const m = marks[k], heard = take.words.slice(m.start, m.end).join(' ');
      const changed = m.right != null && m.right !== heard;
      const group = el('span', `m r-${m.reason}` + (changed ? ' changed' : '') + (editing === k ? ' sel' : ''));
      const top = el('span', 'heard');
      for (let i = m.start; i < m.end; i++) top.append(word(i));
      group.append(top);
      if (changed && m.right) group.append(el('span', 'fix', m.right));
      const tag = el('span', 'tag', changed && !m.right ? 'not said' : m.reason === 'custom' ? m.note : LABEL[m.reason]);
      group.append(tag);
      box.append(group);
      p = m.end;
    }
  }
  function tapWord(i) {
    const k = marks.findIndex(m => m.start <= i && i < m.end);
    if (k >= 0) return openSheet({start: marks[k].start, end: marks[k].end}, k);
    if (sel && editing < 0 && sel.end > sel.start) {
      if (sel.end - sel.start === 1 && sel.start === i) return closeSheet();
      const next = {start: Math.min(sel.start, i), end: Math.max(sel.end, i + 1)};
      if (clashes(marks, next.start, next.end, -1)) return hint('That would run into another mark. Mark up to it, or remove it first.');
      return openSheet(next, -1, true);
    }
    openSheet({start: i, end: i + 1}, -1);
  }
  function heardNow() {
    return take.words.slice(sel.start, sel.end).join(' ');
  }
  function grow(field) {
    field.style.height = 'auto';
    field.style.height = Math.min(field.scrollHeight + 3, 180) + 'px';
  }
  function hint(text) {
    const help = $('right-help');
    help.textContent = text;
    help.classList.add('bad');
  }
  // The panel for one mark. `k` is the saved mark being edited (-1: a new one); `keep` keeps
  // what is already chosen in the panel (the selection only grew).
  function openSheet(span, k, keep) {
    sel = span;
    editing = k;
    const m = k >= 0 ? marks[k] : null, place = span.start === span.end;
    if (!keep) {
      reason = m ? m.reason : place ? 'dropped' : 'wrong';
      touched = !!m;
      $('right').value = m ? (m.right == null ? '' : m.right) : place ? '' : heardNow();
      $('note').value = m ? m.note : '';
    } else if (!touched) {
      $('right').value = reason === 'weird' ? '' : heardNow();
    }
    $('sheet').hidden = false;
    $('app').classList.add('sheet-open');
    renderSheet();
    renderReview();
    const at = document.querySelector('#words .sel');
    if (at) at.scrollIntoView({block: 'center', behavior: 'smooth'});
  }
  function renderSheet() {
    const place = sel.start === sel.end;
    $('sel-label').textContent = place ? 'A word is missing here' : 'You marked';
    $('sel').textContent = place
      ? [take.words[sel.start - 1], '[ + ]', take.words[sel.start]].filter(Boolean).join(' ')
      : heardNow();
    const chips = $('reasons');
    chips.hidden = place;
    chips.replaceChildren();
    for (const [key, label] of REASONS) {
      const chip = el('button', `chip r-${key}` + (key === reason ? ' on' : ''), label);
      chip.dataset.reason = key;
      chips.append(chip);
    }
    $('note').hidden = place || reason !== 'custom';
    $('right-label').textContent = place ? 'The missing word or words' : 'What it should be';
    const help = $('right-help');
    help.classList.remove('bad');
    help.textContent = place ? ''
      : reason === 'weird' ? 'Leave it empty if nothing was said there.'
        : reason === 'fluency' ? 'Optional: leave it empty to only flag the spot.' : '';
    $('insert-row').hidden = place || editing >= 0;
    $('mark-remove').hidden = editing < 0;
    $('mark-save').textContent = editing >= 0 ? 'Update mark' : 'Save mark';
    grow($('right'));
  }
  function closeSheet(quiet) {
    sel = null;
    editing = -1;
    $('sheet').hidden = true;
    $('app').classList.remove('sheet-open');
    if (document.activeElement && document.activeElement.blur) document.activeElement.blur();
    if (!quiet && take) renderReview();
  }
  function saveMark() {
    const place = sel.start === sel.end, heard = heardNow();
    let right = $('right').value.trim().replace(/\s+/g, ' ');
    const note = $('note').value.trim();
    if (place) {
      if (!right) return hint('Type the missing word.');
    } else if (reason === 'fluency') {
      if (!right) right = null;
    } else if (reason !== 'weird') {
      if (!right) return hint('Type what it should be.');
      if (reason === 'wrong' && right === heard) return hint('That is what was written. Change it, or pick another reason.');
    }
    if (!place && reason === 'custom' && !note) return hint('Say the reason in a few words.');
    const mark = {start: sel.start, end: sel.end, heard, right, reason: place ? 'dropped' : reason, note: !place && reason === 'custom' ? note : ''};
    if (editing >= 0) marks[editing] = mark;
    else marks.push(mark);
    marks = sortMarks(marks);
    closeSheet();
  }
  function addPlace(p) {
    const existing = marks.findIndex(m => m.start === p && m.end === p);
    if (existing >= 0) return openSheet({start: p, end: p}, existing);
    if (clashes(marks, p, p, -1)) return hint('That spot is inside another mark.');
    openSheet({start: p, end: p}, -1);
    $('right').focus();
  }
  async function save(toss) {
    if (busy || !take) return;
    if (!toss && typed != null && !typed.trim()) return say('Type what you said, or toss the take.');
    busy = true;
    try {
      const body = {
        marks: marks.map(({start, end, right, reason, note}) => ({start, end, right, reason, note})),
        typed: typed != null ? typed : null,
        toss,
      };
      const item = await api(`takes/${take.n}`, {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(body)});
      items = items.map(i => i.n === item.n ? item : i);
      say('');
      leaveReview();
    } catch (error) {
      say(error.message);
    } finally {
      busy = false;
    }
  }
  function leaveReview() {
    const list = fromList;
    take = null;
    if (!list) nextIdea();
    show(list ? 'list' : 'record');
  }
  async function redo() {
    if (busy || !take) return;
    busy = true;
    try {
      await api(`takes/${take.n}`, {method: 'DELETE'});
      items = items.filter(i => i.n !== take.n);
      if (loaded === take.n) { player.pause(); loaded = null; }
      take = null;
      show('record');
    } catch (error) {
      say(error.message);
    } finally {
      busy = false;
    }
  }
  function ask(text, yes, then) {
    $('ask-text').textContent = text;
    $('ask-yes').textContent = yes;
    $('ask').hidden = false;
    $('ask-yes').onclick = () => { $('ask').hidden = true; then(); };
  }

  // ---- All takes ----

  function renderList() {
    const n = done(), waiting = items.filter(i => i.status === 'pending').length, tossed = items.filter(i => i.status === 'tossed').length;
    const title = $('list-title');
    title.replaceChildren();
    if (!goal) title.append(el('b', '', String(n)), ' done');
    else if (n >= goal) title.append(el('b', '', `The set of ${goal} is complete.`), ` ${n} done; more is fine.`);
    else title.append(el('b', '', String(n)), ` of ${goal} done`);
    if (waiting) title.append(`, ${waiting} to review`);
    if (tossed) title.append(`, ${tossed} tossed`);
    const box = $('items');
    box.replaceChildren();
    if (!items.length) { box.append(el('p', 'empty', 'No takes yet. Record the first one.')); return; }
    for (const item of [...items].reverse()) {
      const row = el('div', 'item ' + item.status);
      row.setAttribute('role', 'button');
      row.tabIndex = 0;
      row.dataset.n = item.n;
      const text = el('span', 'text', item.final || item.draft || '(nothing heard)');
      text.dir = 'rtl';
      text.lang = 'yi';
      const mini = el('button', 'mini');
      mini.setAttribute('aria-label', `Play take ${item.n}`);
      mini.dataset.play = item.n;
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
      use.dataset.n = item.n;
      svg.append(use);
      mini.append(svg);
      const body = el('div', 'body');
      body.append(text, el('span', 'status ' + item.status, STATUS[item.status] || item.status));
      row.append(el('span', 'no', String(item.n)), body, mini);
      box.append(row);
    }
    syncPlayer();
  }

  // ---- Taps ----

  $('rec').onclick = () => {
    if (rec) return stopRecording();
    if (live) return startRecording();
    if (!busy) $('pick').click();
  };
  $('pick').onchange = event => { const file = event.target.files[0]; event.target.value = ''; picked(file); };
  $('retry').onclick = () => { if (lastBlob && !busy) send(lastBlob); };
  $('idea-next').onclick = nextIdea;
  $('nav-list').onclick = () => show('list');
  $('nav-back').onclick = () => {
    if (screen === 'review') { take = null; show(fromList ? 'list' : 'record'); }
    else show('record');
  };
  $('list-record').onclick = () => show('record');
  $('items').onclick = event => {
    const play = event.target.closest('[data-play]');
    if (play) {
      const n = Number(play.dataset.play);
      if (loaded === n && !player.paused) player.pause();
      else listen(n, true);
      return;
    }
    const row = event.target.closest('.item');
    const item = row && items.find(i => i.n === Number(row.dataset.n));
    if (item) openReview(item, true);
  };
  $('play').onclick = () => {
    if (!take) return;
    if (loaded === take.n && player.src && !player.paused) player.pause();
    else listen(take.n, true);
  };
  $('seek').oninput = event => {
    const total = length();
    if (player.src && total) player.currentTime = event.target.value / 1000 * total;
    syncPlayer();
  };
  for (const name of ['play', 'pause', 'ended', 'timeupdate', 'loadedmetadata', 'emptied']) player.addEventListener(name, syncPlayer);
  $('words').onclick = event => {
    const pill = event.target.closest('.ins');
    if (pill) {
      const k = Number(pill.dataset.k);
      if (k >= 0) openSheet({start: marks[k].start, end: marks[k].end}, k);
      return;
    }
    const word = event.target.closest('.w');
    if (word) tapWord(Number(word.dataset.i));
  };
  $('reasons').onclick = event => {
    const chip = event.target.closest('.chip');
    if (!chip) return;
    reason = chip.dataset.reason;
    if (!touched) $('right').value = reason === 'weird' ? '' : heardNow();
    renderSheet();
    if (reason === 'custom') $('note').focus();
  };
  $('right').oninput = event => { touched = true; grow(event.target); $('right-help').classList.remove('bad'); };
  $('sheet-close').onclick = () => closeSheet();
  $('mark-cancel').onclick = () => closeSheet();
  $('mark-save').onclick = saveMark;
  $('mark-remove').onclick = () => { marks.splice(editing, 1); closeSheet(); };
  $('ins-before').onclick = () => addPlace(sel.start);
  $('ins-after').onclick = () => addPlace(sel.end);
  $('type-out').onclick = () => {
    typed = typed == null ? applyMarks(take.words, marks) : null;
    closeSheet(true);
    $('typed').value = typed == null ? '' : typed;
    renderReview();
    if (typed != null) { grow($('typed')); $('typed').focus(); }
  };
  $('typed').oninput = event => { typed = event.target.value; grow(event.target); renderFinal(); };
  $('save').onclick = () => save(false);
  $('toss').onclick = () => save(true);
  $('redo').onclick = () => {
    if (!take) return;
    if (take.status === 'pending') redo();
    else ask(`Record take ${take.n} again? The recording you have now is deleted.`, 'Record again', redo);
  };
  $('ask-no').onclick = () => { $('ask').hidden = true; };

  // The screen locked or the page went to the background while recording: what was said so
  // far is kept and sent, like a tap on Stop.
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) { if (rec) stopRecording(); }
    else if (!busy && !rec) refresh(screen === 'review').catch(() => {});
  });
  // The phone's keyboard: the mark panel sits on top of it.
  if (window.visualViewport) {
    const fit = () => {
      const v = window.visualViewport;
      document.documentElement.style.setProperty('--kb', Math.max(0, window.innerHeight - v.height - v.offsetTop) + 'px');
    };
    window.visualViewport.addEventListener('resize', fit);
    window.visualViewport.addEventListener('scroll', fit);
  }
  // Tells the computer the phone is still here, and picks up changes made there (takes
  // recorded in the app, another idea) except in the middle of a review.
  setInterval(() => { if (!document.hidden && !busy && !rec && !starting) refresh(screen === 'review').catch(() => {}); }, 5000);

  $('rec-note').textContent = live ? '' : "This link is not a secure one, so the phone's own recorder opens: record there, then confirm.";
  (async () => {
    try { await refresh(); } catch (error) { say(error.message); }
    // A take that was recorded but not reviewed (the screen locked, the page closed) comes back.
    const waiting = [...items].reverse().find(i => i.status === 'pending');
    if (waiting) openReview(waiting, false);
    else show('record');
  })();
}

if (typeof document !== 'undefined') startPage();
