// Kaizen Desktop: the lamp.
//
// Almost no logic lives here. Kaizen's own App\Support\Ledger decides what a
// hole is, which state the lamp is in and when the question turns from 灯 to
// 印; this file renders the answer and asks Rust to place the window. Two
// implementations of those rules would drift within a week.

const { invoke } = window.__TAURI__.core;

const card = document.getElementById('card');
const lamp = document.getElementById('lamp');
const delta = document.getElementById('delta');
const deltaLabel = document.getElementById('deltaLabel');
const sub = document.getElementById('sub');
const track = document.getElementById('track');
const slab = document.getElementById('slab');
const slabTitle = document.getElementById('slabTitle');
const slabHint = document.getElementById('slabHint');
const entriesEl = document.getElementById('entries');
const actions = document.getElementById('actions');
const setup = document.getElementById('setup');
const serverInput = document.getElementById('server');
const connectBtn = document.getElementById('connectBtn');
const setupNote = document.getElementById('setupNote');
const discussBtn = document.getElementById('discuss');

const STATE_CLASSES = ['lit-wait', 'lit-ok', 'lit-warm', 'lit-call', 'lit-off'];
const LAMP_FOR = { waiting: 'lit-wait', running: 'lit-ok', attention: 'lit-warm', call: 'lit-call', quiet: 'lit-off' };

// Under the threshold the lamp is barely saying anything, so asking often is
// waste. Once it has something to say, look more often.
const CALM_MS = 5 * 60 * 1000;
const LOUD_MS = 60 * 1000;

let expanded = false;
let ledger = null;
let timer = null;

const hhmm = (m) => `${Math.floor(Math.max(0, m) / 60)}:${String(Math.max(0, m) % 60).padStart(2, '0')}`;
const toMinutes = (t) => {
  const [h, m] = String(t ?? '0:00').split(':').map(Number);
  return (h || 0) * 60 + (m || 0);
};
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

function windowBounds(l) {
  const match = /(\d{2}:\d{2})\D+(\d{2}:\d{2})/.exec(l.window ?? '');
  const open = match ? toMinutes(match[1]) : 8 * 60;
  const close = match ? toMinutes(match[2]) : 18 * 60;
  return { open, span: Math.max(60, close - open) };
}

function place(from, to, open, span) {
  const left = Math.max(0, Math.min(100, ((from - open) / span) * 100));
  const width = Math.max(0.6, Math.min(100 - left, ((to - from) / span) * 100));
  return `left:${left}%;width:${width}%`;
}

function renderTrack(l) {
  const { open, span } = windowBounds(l);
  const parts = [];

  // Before the start is dead ground: not work, not rest, not a hole. An empty
  // track would read exactly like time somebody forgot to account for.
  if (l.started_at) {
    const started = toMinutes(l.started_at);
    if (started > open) parts.push(`<span class="seg dead" style="${place(open, started, open, span)}"></span>`);
  }

  // The alternation is positional only: it flips on consecutive work and
  // resets after a rest or a hole, where the boundary is already obvious.
  let alt = false;
  for (const e of l.entries ?? []) {
    const from = toMinutes(e.from);
    const to = toMinutes(e.to);
    const title = esc(`${e.from}–${e.to} · ${e.description ?? (e.kind === 'work' ? 'Work' : 'Rest')}`);

    if (e.kind === 'work') {
      alt = !alt;
      const unref = l.logs_externally && !e.referenced ? ' unref' : '';
      parts.push(`<span class="seg work${alt ? ' alt' : ''}${unref}" style="${place(from, to, open, span)}" title="${title}"></span>`);
    } else {
      alt = false;
      parts.push(`<span class="seg rest" style="${place(from, to, open, span)}" title="${title}"></span>`);
    }
  }

  for (const g of l.gaps ?? []) {
    parts.push(`<span class="seg gap" style="${place(toMinutes(g.from), toMinutes(g.to), open, span)}" title="${g.minutes} minutes unaccounted"></span>`);
  }

  const now = new Date();
  const nowMin = now.getHours() * 60 + now.getMinutes();
  if (nowMin > open && nowMin < open + span) {
    parts.push(`<span class="now" style="left:${((nowMin - open) / span) * 100}%"></span>`);
  }

  track.innerHTML = parts.join('');
}

function renderEntries(l) {
  const rows = [];

  for (const e of l.entries ?? []) {
    const pill = !l.logs_externally || e.kind !== 'work'
      ? (e.kind === 'rest' ? '<span class="pill">rest</span>' : '')
      : e.referenced
        ? `<span class="pill ok">✓ ${esc(e.reference)}</span>`
        : '<span class="pill pending">no reference</span>';

    rows.push(`<div class="erow ${e.kind === 'rest' ? 'rest-row' : ''}">
      <span class="e-time">${esc(e.from)}–${esc(e.to)}</span>
      <span class="e-dur">${hhmm(e.minutes)}</span>
      <span class="e-what">${esc(e.description ?? (e.kind === 'work' ? 'Work' : 'Rest'))}</span>
      ${pill}
    </div>`);
  }

  for (const g of l.gaps ?? []) {
    rows.push(`<div class="erow gap-row">
      <span class="e-time">${esc(g.from)}–${esc(g.to)}</span>
      <span class="e-dur">${hhmm(g.minutes)}</span>
      <span class="e-what">Unaccounted</span>
      <span class="pill open">open</span>
    </div>`);
  }

  entriesEl.innerHTML = rows.join('') || '<div class="erow"><span class="e-what">Nothing filed yet.</span></div>';
  slabTitle.textContent = `${l.context} · ${l.date}`;
  slabHint.textContent = l.logs_externally
    ? 'Work here is logged in another system too, so an entry counts only with a reference and a link.'
    : '';
}

function render(l) {
  ledger = l;

  card.classList.remove(...STATE_CLASSES, 'phase-accounting', 'phase-referencing');
  card.classList.add(LAMP_FOR[l.state] ?? 'lit-wait', `phase-${l.phase}`);

  lamp.textContent = l.phase === 'referencing' ? '印' : '灯';

  if (l.phase === 'referencing' && l.unreferenced_minutes > 0) {
    delta.textContent = hhmm(l.unreferenced_minutes);
    deltaLabel.textContent = 'not in the other system';
  } else if (l.gap_minutes > 0) {
    delta.textContent = hhmm(l.gap_minutes);
    deltaLabel.textContent = 'unaccounted';
  } else if (!l.started_at) {
    delta.textContent = '—:—';
    deltaLabel.textContent = 'day not started';
  } else {
    delta.textContent = hhmm(l.work_minutes);
    deltaLabel.textContent = 'accounted for';
  }

  const bits = [l.context];
  if (l.target_minutes) bits.push(`${hhmm(l.work_minutes)} of ${hhmm(l.target_minutes)} logged`);
  if (l.started_at) bits.push(`started ${l.started_at}`);
  sub.textContent = bits.join(' · ');

  renderTrack(l);
  renderEntries(l);
}

/** A day with no context carrying a target is not an error. */
function renderIdle(message) {
  ledger = null;
  card.classList.remove(...STATE_CLASSES);
  card.classList.add('lit-off');
  lamp.textContent = '灯';
  delta.textContent = '—:—';
  deltaLabel.textContent = '';
  sub.textContent = message;
  track.innerHTML = '';
  entriesEl.innerHTML = '';
}

async function refresh() {
  try {
    const day = await invoke('fetch_day', { date: null });
    const leading = pickLeading(day.ledgers ?? []);

    if (!leading) {
      renderIdle('Nothing carries a target today.');
      return schedule(CALM_MS);
    }

    render(leading);
    schedule(['call', 'attention'].includes(leading.state) ? LOUD_MS : CALM_MS);
  } catch (e) {
    // A network blip must not blank the card: keep the last reading and say
    // it is stale rather than pretending the day is suddenly empty.
    sub.textContent = ledger ? `${sub.textContent} · not reachable` : String(e);
    schedule(LOUD_MS);
  }
}

/** A card this small asks one question at a time; the loudest wins. */
function pickLeading(ledgers) {
  const rank = { call: 0, attention: 1, waiting: 2, running: 3, quiet: 4 };
  return [...ledgers].sort((a, b) => (rank[a.state] ?? 9) - (rank[b.state] ?? 9))[0] ?? null;
}

function schedule(ms) {
  clearTimeout(timer);
  timer = setTimeout(refresh, ms);
}

async function toggle() {
  expanded = !expanded;
  slab.hidden = !expanded;
  actions.hidden = !expanded;

  try {
    await invoke('place_window', { expanded });
  } catch (e) {
    console.error('place_window failed', e);
  }
}

card.addEventListener('click', (event) => {
  if (event.target.closest('.btn') || setup.hidden === false) return;
  toggle();
});

discussBtn.addEventListener('click', async (event) => {
  event.stopPropagation();
  const original = discussBtn.textContent;

  try {
    const { prompt, bootstrap } = await invoke('fetch_prompt', { date: null });
    await navigator.clipboard.writeText(prompt);
    discussBtn.textContent = bootstrap ? 'Copied · it will ask how' : 'Copied · paste it in';
  } catch (e) {
    discussBtn.textContent = 'Could not copy';
    console.error(e);
  }

  setTimeout(() => { discussBtn.textContent = original; }, 4000);
});

connectBtn.addEventListener('click', async () => {
  const server = serverInput.value.trim();
  if (!server) return;

  connectBtn.disabled = true;
  connectBtn.textContent = 'Waiting for your browser…';
  setupNote.textContent = 'Approve it in the tab that just opened.';

  try {
    await invoke('connect', { server });
    setup.hidden = true;
    await refresh();
  } catch (e) {
    setupNote.textContent = String(e);
  } finally {
    connectBtn.disabled = false;
    connectBtn.textContent = 'Connect';
  }
});

(async () => {
  let config = {};

  try {
    config = await invoke('load_config');
  } catch (e) {
    console.error('load_config failed', e);
  }

  if (!config.server_url || !config.client_id) {
    setup.hidden = false;
    renderIdle('Not connected yet.');
    serverInput.focus();
    return;
  }

  setup.hidden = true;
  await refresh();
})();
