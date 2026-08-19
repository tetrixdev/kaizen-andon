// Kaizen Desktop: the lamp.
//
// Almost no logic lives here. The rules are server-side in Kaizen's own
// App\Support\Ledger, so this file only renders what came back and asks Rust
// to place the window. That is deliberate: two implementations of "what counts
// as a hole" would drift within a week.

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
const entries = document.getElementById('entries');
const actions = document.getElementById('actions');

const STATE_CLASSES = ['lit-wait', 'lit-ok', 'lit-warm', 'lit-call', 'lit-off'];

let expanded = false;
let ledger = null;

const hhmm = (m) => `${Math.floor(Math.max(0, m) / 60)}:${String(Math.max(0, m) % 60).padStart(2, '0')}`;
const toMinutes = (t) => {
  const [h, m] = String(t ?? '0:00').split(':').map(Number);
  return (h || 0) * 60 + (m || 0);
};

/** The window's span, from the context's own hours. */
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

  // Alternating weight is positional only: it flips on consecutive work and
  // resets whenever a rest or a gap sits between, because the boundary is
  // already obvious there.
  let alt = false;
  for (const e of l.entries ?? []) {
    const from = toMinutes(e.from);
    const to = toMinutes(e.to);

    if (e.kind === 'work') {
      alt = !alt;
      const unref = l.logs_externally && !e.referenced ? ' unref' : '';
      parts.push(`<span class="seg work${alt ? ' alt' : ''}${unref}" style="${place(from, to, open, span)}" title="${e.from}–${e.to} · ${e.description ?? 'Work'}"></span>`);
    } else {
      alt = false;
      parts.push(`<span class="seg rest" style="${place(from, to, open, span)}" title="${e.from}–${e.to} · ${e.description ?? 'Rest'}"></span>`);
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
        ? `<span class="pill ok">✓ ${e.reference}</span>`
        : '<span class="pill pending">no reference</span>';

    rows.push(`<div class="erow ${e.kind === 'rest' ? 'rest-row' : ''}">
      <span class="e-time">${e.from}–${e.to}</span>
      <span class="e-dur">${hhmm(e.minutes)}</span>
      <span class="e-what">${e.description ?? (e.kind === 'work' ? 'Work' : 'Rest')}</span>
      ${pill}
    </div>`);
  }

  for (const g of l.gaps ?? []) {
    rows.push(`<div class="erow gap-row">
      <span class="e-time">${g.from}–${g.to}</span>
      <span class="e-dur">${hhmm(g.minutes)}</span>
      <span class="e-what">Unaccounted</span>
      <span class="pill open">open</span>
    </div>`);
  }

  entries.innerHTML = rows.join('') || '<div class="erow"><span class="e-what">Nothing filed yet.</span></div>';
  slabTitle.textContent = `${l.context} · ${l.date}`;
  slabHint.textContent = l.logs_externally
    ? 'Work here is logged in another system too, so an entry counts only with a reference and a link.'
    : '';
}

function render(l) {
  ledger = l;

  card.classList.remove(...STATE_CLASSES, 'phase-accounting', 'phase-referencing');
  card.classList.add(
    { waiting: 'lit-wait', running: 'lit-ok', attention: 'lit-warm', call: 'lit-call', quiet: 'lit-off' }[l.state] ?? 'lit-wait',
    `phase-${l.phase}`,
  );

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
  actions.hidden = !expanded;
}

/** Opening moves both axes on one curve; the anchor is bottom-right, so the
 *  lamp does not move. Rust owns the geometry. */
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
  // The drag region and the buttons are not the toggle.
  if (event.target.closest('.btn')) return;
  toggle();
});

document.getElementById('discuss').addEventListener('click', (event) => {
  event.stopPropagation();
  // Filled in once the API client lands: this copies the day, its gaps and the
  // context's own integration prompt to the clipboard.
});

// Until the connection exists, show what an unconnected widget honestly is.
(async () => {
  let config = {};
  try {
    config = await invoke('load_config');
  } catch (e) {
    console.error('load_config failed', e);
  }

  if (!config.server_url) {
    render({
      context: 'Kaizen',
      date: '',
      window: null,
      state: 'waiting',
      phase: 'accounting',
      gap_minutes: 0,
      work_minutes: 0,
      unreferenced_minutes: 0,
      logs_externally: false,
      entries: [],
      gaps: [],
    });
    sub.textContent = 'Not connected. Open Kaizen in your browser to connect.';
    return;
  }

  // The API client is the next piece; until then the card sits waiting rather
  // than pretending to know anything.
  sub.textContent = config.server_url;
})();
