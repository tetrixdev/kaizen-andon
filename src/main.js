// Kaizen Desktop: the lamp.
//
// Almost no logic lives here. Kaizen's own App\Support\Ledger decides what a
// hole is, which state the lamp is in and when the question turns from 灯 to
// 印; this file renders the answer and asks Rust to place the window. Two
// implementations of those rules would drift within a week.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

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
const snoozeBtn = document.getElementById('snoozeBtn');
const editor = document.getElementById('editor');
const editorTitle = document.getElementById('editorTitle');
const editorNote = document.getElementById('editorNote');
const editFrom = document.getElementById('editFrom');
const editTo = document.getElementById('editTo');
const editQuick = document.getElementById('editQuick');
const editKind = document.getElementById('editKind');
const editWhat = document.getElementById('editWhat');
const editRef = document.getElementById('editRef');
const editLink = document.getElementById('editLink');
const history = document.getElementById('history');
const historyTitle = document.getElementById('historyTitle');
const weekdays = document.getElementById('weekdays');
const monthGrid = document.getElementById('monthGrid');
const monthSummary = document.getElementById('monthSummary');
const banner = document.getElementById('banner');
const bannerText = document.getElementById('bannerText');
const pop = document.getElementById('pop');
const popTitle = document.getElementById('popTitle');
const popSpan = document.getElementById('popSpan');
const popBody = document.getElementById('popBody');
const popActions = document.getElementById('popActions');
const addBtn = document.getElementById('addBtn');
const historyBtn = document.getElementById('historyBtn');
const startBtn = document.getElementById('startBtn');
const endBtn = document.getElementById('endBtn');

const STATE_CLASSES = ['lit-wait', 'lit-ok', 'lit-warm', 'lit-call', 'lit-off'];
const LAMP_FOR = { waiting: 'lit-wait', running: 'lit-ok', attention: 'lit-warm', call: 'lit-call', quiet: 'lit-off' };

// Under the threshold the lamp is barely saying anything, so asking often is
// waste. Once it has something to say, look more often.
const CALM_MS = 5 * 60 * 1000;
const LOUD_MS = 60 * 1000;

let expanded = false;
let ledger = null;
let timer = null;

/** The day on screen. `null` means today, which follows the clock past midnight. */
let viewDate = null;
/** Which month the grid is showing, `YYYY-MM`. */
let viewMonth = null;
/** The pinned segment, by entry id or gap key, or null. */
let pinned = null;
/** The entry being edited, or null for a new one. */
let editingId = null;

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

/**
 * Where "now" falls on the track, in minutes past midnight.
 *
 * Kaizen's clock, not this machine's. The server knows the account's timezone
 * and the day being drawn is its day, so a laptop set to another zone would
 * otherwise put the now-line and the not-yet cover hours away from the truth
 * while every number beside them stayed right. The browser clock is only a
 * fallback for the moment before the first answer arrives.
 */
function nowMinutes() {
  if (localNow) return toMinutes(localNow);
  const now = new Date();
  return now.getHours() * 60 + now.getMinutes();
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

  // Time that has not happened yet is not a hole and must not read like one.
  // Kaizen only ever reports gaps up to now, so without this the rest of the
  // day is bare track, which is the same thing an unaccounted span looks like
  // at a glance.
  if (!viewDate) {
    const nowMin = nowMinutes();
    if (nowMin < open + span) {
      parts.push(`<span class="seg notyet" style="${place(Math.max(nowMin, open), open + span, open, span)}"></span>`);
    }
  }

  // The alternation is positional only: it flips on consecutive work and
  // resets after a rest or a hole, where the boundary is already obvious.
  let alt = false;
  for (const [index, e] of (l.entries ?? []).entries()) {
    const from = toMinutes(e.from);
    const to = toMinutes(e.to);
    const key = e.id != null ? `e${e.id}` : `i${index}`;
    const pin = pinned === key ? ' pinned' : '';

    if (e.kind === 'work') {
      alt = !alt;
      // Phase 2 turns the alternation off: once the question is whether it
      // reached the other system, which block is which stops mattering.
      const unref = l.logs_externally && !e.referenced ? ' unref' : '';
      parts.push(`<span class="seg work${alt ? ' alt' : ''}${unref}${pin}" data-kind="entry" data-key="${key}" data-index="${index}" style="${place(from, to, open, span)}"></span>`);
    } else {
      alt = false;
      parts.push(`<span class="seg rest${pin}" data-kind="entry" data-key="${key}" data-index="${index}" style="${place(from, to, open, span)}"></span>`);
    }
  }

  for (const [index, g] of (l.gaps ?? []).entries()) {
    const key = `g${index}`;
    parts.push(`<span class="seg gap${pinned === key ? ' pinned' : ''}" data-kind="gap" data-key="${key}" data-index="${index}" style="${place(toMinutes(g.from), toMinutes(g.to), open, span)}"></span>`);
  }

  // A finished day has no now, so a past day gets no hairline. Drawing one
  // would put "now" somewhere inside a day that ended hours ago.
  if (!viewDate) {
    const nowMin = nowMinutes();
    if (nowMin > open && nowMin < open + span) {
      parts.push(`<span class="now" style="left:${((nowMin - open) / span) * 100}%"></span>`);
    }
  }

  track.innerHTML = parts.join('');
}

function renderEntries(l) {
  const rows = [];

  for (const [index, e] of (l.entries ?? []).entries()) {
    const pill = !l.logs_externally || e.kind !== 'work'
      ? (e.kind === 'rest' ? '<span class="pill">rest</span>' : '')
      : e.referenced
        ? `<span class="pill ok">✓ ${esc(e.reference)}</span>`
        : '<span class="pill pending">no reference</span>';

    rows.push(`<div class="erow ${e.kind === 'rest' ? 'rest-row' : ''}" data-kind="entry" data-index="${index}">
      <span class="e-time">${esc(e.from)}–${esc(e.to)}</span>
      <span class="e-dur">${hhmm(e.minutes)}</span>
      <span class="e-what">${esc(e.description ?? (e.kind === 'work' ? 'Work' : 'Rest'))}</span>
      ${pill}
    </div>`);
  }

  for (const [index, g] of (l.gaps ?? []).entries()) {
    rows.push(`<div class="erow gap-row" data-kind="gap" data-index="${index}">
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

  // The card can be hidden or snoozed; the tray is the one surface always
  // there, so it carries the number.
  const [amount, what] = [delta.textContent, deltaLabel.textContent];
  invoke('set_tooltip', { text: `Kaizen · ${amount} ${what} · ${l.context}` }).catch(() => {});

  // A day that never started can hold nothing, so that is the only thing
  // worth offering until it has.
  startBtn.hidden = !!l.started_at;

  // Calling it a day closes the accounting question whatever the total. A day
  // ended short is complete by decision, not a failure, which is the ordinary
  // case of working two hours less today and ten tomorrow.
  endBtn.hidden = !l.started_at;
  endBtn.textContent = l.ended_at ? 'Reopen the day' : 'Call it a day';
  endBtn.dataset.reopen = l.ended_at ? 'yes' : '';

  // The button carries the date, because on a past day it is a different
  // conversation and a prompt about "today" would be the wrong one.
  discussBtn.textContent = viewDate
    ? `Discuss ${dayLabel(viewDate)} with AI`
    : 'Discuss today with AI';

  // Putting it down is a Running-only privilege. From attention up there is no
  // hiding, which is the rule that makes the end of the day always show. A
  // finished day is not something to put down either.
  snoozeBtn.hidden = l.state !== 'running' || !!viewDate;
  snoozeBtn.dataset.state = l.state;
  snoozeBtn.dataset.date = l.date;
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
    const day = await invoke('fetch_day', { date: viewDate });
    const leading = apply(day);

    // A finished day cannot change on its own, so looking back stops the
    // polling entirely rather than asking the same question every minute.
    if (viewDate) return;
    if (!leading) return schedule(CALM_MS);

    // A snooze lapses the moment the lamp has something else to say, and that
    // is exactly the case this widget exists for, so it must not stay hidden.
    if (['call', 'attention'].includes(leading.state)) {
      invoke('wake').catch(() => {});
    }

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

// ── Sizing ──────────────────────────────────────────────────────────────
//
// NOTHING HERE DECLARES A HEIGHT. The page measures whatever it currently is
// and the window follows, for every state and every message inside a state. A
// declared height is a guess about how text wraps in a font this machine may
// not have, and it is wrong the first time an error runs to three lines: the
// card is anchored to the bottom, so the extra lines grow off the TOP and take
// the title with them, which reads as a cut-off window rather than a long
// message.

let lastHeight = 0;
let queued = false;

function contentHeight() {
  const style = getComputedStyle(document.body);
  // [data-floating] is out of flow (the segment popover), so counting it would
  // make the window grow by the height of a card that is drawn over the top.
  const panels = [...document.body.children].filter(
    (el) => el.tagName !== 'SCRIPT' && !el.hidden && !el.hasAttribute('data-floating'),
  );

  return Math.ceil(
    panels.reduce((total, el) => total + el.getBoundingClientRect().height, 0) +
      (parseFloat(style.rowGap) || 0) * Math.max(0, panels.length - 1) +
      parseFloat(style.paddingTop) +
      parseFloat(style.paddingBottom),
  );
}

/**
 * Ask for a window that fits the page.
 *
 * Coalesced to one call per frame: showing a panel and writing its text are
 * two changes that must not become two resizes. `force` is for the case where
 * the height is unchanged but the WIDTH is not, which is opening and closing.
 */
function fit(force = false) {
  if (queued) return;
  queued = true;

  requestAnimationFrame(async () => {
    queued = false;
    const height = contentHeight();
    if (height === lastHeight && !force) return;
    lastHeight = height;

    try {
      await invoke('place_window', { expanded, height });
    } catch (e) {
      console.error('place_window failed', e);
    }
  });
}

// Any panel changing size for any reason resizes the window. This is what
// makes it unnecessary to remember to call anything after changing any text,
// and what makes a screen that does not exist yet work without being taught.
const sizes = new ResizeObserver(() => fit());
for (const panel of [slab, setup, card]) sizes.observe(panel);

function toggle() {
  expanded = !expanded;
  actions.hidden = !expanded;

  // Collapsing puts everything away: the editor and the grid only make sense
  // beside the rows they belong to.
  closePop();
  if (!expanded) {
    editor.hidden = true;
    history.hidden = true;
  }
  slab.hidden = !expanded || !editor.hidden || !history.hidden;

  fit(true);
}

function showSetup(show) {
  setup.hidden = !show;
  fit(true);
}


card.addEventListener('click', (event) => {
  if (event.target.closest('.btn') || event.target.closest('.seg') || setup.hidden === false) return;
  toggle();
});

snoozeBtn.addEventListener('click', async (event) => {
  event.stopPropagation();

  try {
    await invoke('snooze', { state: snoozeBtn.dataset.state, today: snoozeBtn.dataset.date });
  } catch (e) {
    console.error(e);
  }
});

// A deep link, from Kaizen's own Connect button. The address arrives from the
// link that was clicked rather than from the binary that was downloaded.
listen('deep-link', async ({ payload }) => {
  if (payload?.action === 'connect' && payload.server) {
    serverInput.value = payload.server;
    showSetup(true);
    connectBtn.click();
  }
}).catch((e) => console.error('deep-link listener', e));

discussBtn.addEventListener('click', async (event) => {
  event.stopPropagation();
  const original = discussBtn.textContent;

  try {
    const { prompt, bootstrap } = await invoke('fetch_prompt', { date: viewDate });
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
    showSetup(false);
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
    renderIdle('Not connected yet.');
    showSetup(true);
    serverInput.focus();
    return;
  }

  setup.hidden = true;
  await refresh();
})();

// ── Segment detail ──────────────────────────────────────────────────────
//
// Two levels on purpose. Hover reads; clicking pins. Only a pinned card has a
// live link and reachable actions, because a link inside something that closes
// the moment the cursor drifts is a target you would miss half the time.

/** Middle truncation, keeping the end: the id is the only part that differs. */
function shortLink(url) {
  const text = String(url).replace(/^https?:\/\//, '');
  if (text.length <= 46) return text;
  return `${text.slice(0, 20)}…${text.slice(-24)}`;
}

function segmentAt(kind, index) {
  if (!ledger) return null;
  const list = kind === 'gap' ? ledger.gaps : ledger.entries;
  return (list ?? [])[index] ?? null;
}

function showPop(target, kind, index, pin) {
  const item = segmentAt(kind, index);
  if (!item) return;

  const isGap = kind === 'gap';
  popTitle.textContent = isGap
    ? 'Unaccounted'
    : item.description || (item.kind === 'work' ? 'Work' : 'Rest');
  popSpan.textContent = `${item.from}–${item.to} · ${hhmm(item.minutes)}`;

  const lines = [];
  if (!isGap && ledger.logs_externally && item.kind === 'work') {
    lines.push(item.reference
      ? `Reference ${esc(item.reference)}`
      : 'No reference yet, so it does not count.');
    if (item.link) {
      lines.push(`<a class="pop-link" href="${esc(item.link)}" title="${esc(item.link)}" target="_blank" rel="noreferrer">${esc(shortLink(item.link))}</a>`);
    }
  }
  if (isGap) lines.push('Click to account for it.');
  popBody.innerHTML = lines.join('<br>');

  pop.classList.toggle('pinned', !!pin);
  popActions.hidden = !pin || isGap;
  pop.hidden = false;

  // Above the segment, centred, and never off an edge.
  const rect = target.getBoundingClientRect();
  const box = pop.getBoundingClientRect();
  const left = Math.max(6, Math.min(window.innerWidth - box.width - 6,
    rect.left + rect.width / 2 - box.width / 2));
  const top = rect.top - box.height - 8;
  pop.style.left = `${left}px`;
  pop.style.top = `${top < 6 ? rect.bottom + 8 : top}px`;
}

function closePop() {
  pop.hidden = true;
  pop.classList.remove('pinned');
  if (pinned) {
    pinned = null;
    if (ledger) renderTrack(ledger);
  }
}

track.addEventListener('mouseover', (event) => {
  if (pinned) return;
  const seg = event.target.closest('.seg');
  if (!seg) return;
  showPop(seg, seg.dataset.kind, Number(seg.dataset.index), false);
});

track.addEventListener('mouseleave', () => {
  if (!pinned) pop.hidden = true;
});

track.addEventListener('click', (event) => {
  event.stopPropagation();
  const seg = event.target.closest('.seg');
  if (!seg) return;

  const kind = seg.dataset.kind;
  const index = Number(seg.dataset.index);

  // A gap has no detail worth reading first, so it skips straight to filing.
  if (kind === 'gap') {
    closePop();
    return openEditor({ gap: segmentAt('gap', index) });
  }

  pinned = seg.dataset.key;
  renderTrack(ledger);
  const again = track.querySelector(`[data-key="${pinned}"]`);
  showPop(again ?? seg, kind, index, true);
});

// A row is the same thing said in words, so it does the same thing.
entriesEl.addEventListener('click', (event) => {
  event.stopPropagation();
  const row = event.target.closest('.erow');
  if (!row || !row.dataset.kind) return;

  const index = Number(row.dataset.index);
  if (row.dataset.kind === 'gap') return openEditor({ gap: segmentAt('gap', index) });
  openEditor({ entry: segmentAt('entry', index) });
});

document.addEventListener('click', (event) => {
  if (pinned && !pop.contains(event.target)) closePop();
});

popEdit.addEventListener('click', (event) => {
  event.stopPropagation();
  const key = pinned;
  const seg = track.querySelector(`[data-key="${key}"]`);
  const entry = segmentAt('entry', Number(seg?.dataset.index));
  closePop();
  if (entry) openEditor({ entry });
});

// Splitting is filing two entries where there was one: the same editor, with
// the second half offered as soon as the first is saved.
popSplit.addEventListener('click', (event) => {
  event.stopPropagation();
  const seg = track.querySelector(`[data-key="${pinned}"]`);
  const entry = segmentAt('entry', Number(seg?.dataset.index));
  closePop();
  if (!entry) return;

  const middle = hhmm(Math.round((toMinutes(entry.from) + toMinutes(entry.to)) / 2));
  openEditor({ entry, splitAt: middle });
});

popDelete.addEventListener('click', async (event) => {
  event.stopPropagation();
  const seg = track.querySelector(`[data-key="${pinned}"]`);
  const entry = segmentAt('entry', Number(seg?.dataset.index));
  closePop();
  if (!entry?.id) return;

  try {
    apply(await invoke('delete_entry', { id: entry.id }));
  } catch (e) {
    sub.textContent = String(e);
  }
});

// ── Filing ──────────────────────────────────────────────────────────────

/** The clock as Kaizen reports it, so "to now" agrees with the server's day. */
let localNow = null;

/** Render a whole day response, or say plainly that nothing carries a target. */
function apply(day) {
  localNow = day.local_time ?? localNow;
  const leading = pickLeading(day.ledgers ?? []);

  if (!leading) {
    renderIdle(viewDate ? 'Nothing was filed that day.' : 'Nothing carries a target today.');
    return null;
  }

  render(leading);
  return leading;
}

function onlyPanel(which) {
  slab.hidden = which !== 'slab' || !expanded;
  editor.hidden = which !== 'editor';
  history.hidden = which !== 'history';
  fit(true);
}

function setKind(kind) {
  for (const button of editKind.querySelectorAll('.toggle-btn')) {
    button.classList.toggle('on', button.dataset.kind === kind);
  }
}

function currentKind() {
  return editKind.querySelector('.toggle-btn.on')?.dataset.kind ?? 'work';
}

function quickChips(gap) {
  const chips = [];
  if (gap) chips.push(['The whole gap', () => { editFrom.value = gap.from; editTo.value = gap.to; }]);
  if (localNow && !viewDate) chips.push(['To now', () => { editTo.value = localNow; }]);

  for (const [label, minutes] of [['15m', 15], ['30m', 30], ['1h', 60], ['2h', 120]]) {
    chips.push([label, () => {
      const from = toMinutes(editFrom.value || gap?.from || '09:00');
      editTo.value = hhmm(from + minutes);
    }]);
  }

  editQuick.innerHTML = '';
  for (const [label, act] of chips) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'chip';
    button.textContent = label;
    button.addEventListener('click', (event) => { event.stopPropagation(); act(); });
    editQuick.appendChild(button);
  }
}

/**
 * Open the editor.
 *
 * From a gap the span is already filled: typing a time you can see on screen
 * is the friction that ends a habit.
 */
let splitAt = null;

function openEditor({ gap = null, entry = null, splitAt: at = null } = {}) {
  editingId = entry?.id ?? null;
  splitAt = at;

  editor.classList.toggle('external', !!ledger?.logs_externally);
  editorNote.classList.remove('bad');
  editorNote.textContent = '';

  if (at) {
    editorTitle.textContent = 'Split this in two';
    editorNote.textContent = `Everything after ${at} becomes a second entry.`;
  } else {
    editorTitle.textContent = entry ? 'Edit this entry' : 'Account for a span';
  }

  editFrom.value = entry?.from ?? gap?.from ?? localNow ?? '';
  editTo.value = at ?? entry?.to ?? gap?.to ?? localNow ?? '';
  editWhat.value = entry?.description ?? '';
  editRef.value = entry?.reference ?? '';
  editLink.value = entry?.link ?? '';
  setKind(entry?.kind ?? 'work');
  quickChips(gap);

  onlyPanel('editor');
  editFrom.focus();
}

function closeEditor() {
  editingId = null;
  splitAt = null;
  onlyPanel(expanded ? 'slab' : null);
}

async function fileEntries() {
  const from = editFrom.value.trim();
  const to = editTo.value.trim();
  const shape = /^\d{1,2}:\d{2}$/;

  if (!shape.test(from) || !shape.test(to)) {
    editorNote.classList.add('bad');
    editorNote.textContent = 'Both times read as 13:45.';
    return;
  }

  const common = {
    kind: currentKind(),
    description: editWhat.value.trim() || null,
    reference: editRef.value.trim() || null,
    link: editLink.value.trim() || null,
  };

  // A split is the original shortened plus a second entry, filed together, so
  // Kaizen accepts or refuses the pair as one decision.
  const entries = splitAt
    ? [
        // Only the half that already exists carries an id. A reference belongs
        // to the work it was raised for, so the new half starts without one.
        { ...(editingId ? { id: editingId } : {}), from, to: splitAt, ...common },
        { from: splitAt, to, ...common, reference: null, link: null },
      ]
    : [{ ...(editingId ? { id: editingId } : {}), from, to, ...common }];

  const button = document.getElementById('editorSave');
  button.disabled = true;
  editorNote.classList.remove('bad');
  editorNote.textContent = 'Filing…';

  try {
    const day = await invoke('save_entries', { entries, date: viewDate });
    apply(day);
    closeEditor();
  } catch (e) {
    // Kaizen's own words. The overlap rule is the one that gets met most, and
    // "09:00-10:00 overlaps an entry already filed" says what to change.
    editorNote.classList.add('bad');
    editorNote.textContent = String(e).replace(/^Error:\s*/, '');
  } finally {
    button.disabled = false;
  }
}

document.getElementById('editorSave').addEventListener('click', (e) => { e.stopPropagation(); fileEntries(); });
document.getElementById('editorClose').addEventListener('click', (e) => { e.stopPropagation(); closeEditor(); });
editor.addEventListener('click', (e) => e.stopPropagation());
editor.addEventListener('keydown', (e) => { if (e.key === 'Enter') fileEntries(); });

addBtn.addEventListener('click', (event) => {
  event.stopPropagation();
  // The first open hole is almost always the one meant.
  openEditor({ gap: (ledger?.gaps ?? [])[0] ?? null });
});

endBtn.addEventListener('click', async (event) => {
  event.stopPropagation();
  try {
    apply(await invoke('end_day', {
      at: null,
      reopen: endBtn.dataset.reopen === 'yes',
      date: viewDate,
    }));
  } catch (e) {
    sub.textContent = String(e).replace(/^Error:\s*/, '');
  }
});

startBtn.addEventListener('click', async (event) => {
  event.stopPropagation();
  try {
    apply(await invoke('start_day', { at: null, date: viewDate }));
  } catch (e) {
    sub.textContent = String(e);
  }
});

// ── Looking back ────────────────────────────────────────────────────────

const MONTH_DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

function dayLabel(date) {
  const [y, m, d] = String(date).split('-').map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}

function shiftMonth(month, by) {
  const [y, m] = month.split('-').map(Number);
  const moved = new Date(y, m - 1 + by, 1);
  return `${moved.getFullYear()}-${String(moved.getMonth() + 1).padStart(2, '0')}`;
}

function renderMonth(data) {
  historyTitle.textContent = data.label ?? '';
  weekdays.innerHTML = MONTH_DAYS.map((d) => `<span>${d}</span>`).join('');

  const cells = [];
  // The 1st rarely falls on a Monday, so the row is padded to line the
  // weekdays up with their columns.
  for (let i = 1; i < (data.first_weekday ?? 1); i += 1) {
    cells.push('<span class="tile blank"></span>');
  }

  for (const day of data.days ?? []) {
    const number = Number(day.date.slice(-2));
    const classes = ['tile'];
    if (day.entries > 0) classes.push('held');
    if (day.accounted) classes.push('done');
    if (day.is_today) classes.push('today');
    if (!day.has_target) classes.push('untargeted');
    if (day.is_future) classes.push('future');

    // No badge on a day with no target: a weekend is not a day you failed.
    const seal = day.has_target && day.accounted
      ? `<span class="seal${day.referenced ? '' : ' partial'}">印</span>`
      : '';

    cells.push(`<button type="button" class="${classes.join(' ')}" data-date="${day.date}"${day.is_future ? ' disabled' : ''}>${number}${seal}</button>`);
  }

  monthGrid.innerHTML = cells.join('');
  renderMonthSummary(data.days ?? []);
}

/**
 * What the month adds up to.
 *
 * The grid is seven columns wide and the bar is not, so this is the space
 * that would otherwise be empty. Only days that CARRY a target are counted:
 * a month is not worse because it contained weekends.
 */
function renderMonthSummary(days) {
  const claimed = days.filter((d) => d.has_target && !d.is_future);
  const accounted = claimed.filter((d) => d.accounted);
  const sealed = claimed.filter((d) => d.accounted && d.referenced);
  const minutes = claimed.reduce((total, d) => total + (d.work_minutes ?? 0), 0);

  const rows = [
    [`${accounted.length}/${claimed.length}`, 'days accounted for', false],
    [hhmm(minutes), 'filed this month', false],
  ];

  // The second phase is a question some contexts are never asked.
  if (ledger?.logs_externally) {
    rows.push([`${sealed.length}/${claimed.length}`, 'also in the other system', false]);
  }

  const untouched = claimed.length - accounted.length;
  if (untouched > 0) {
    rows.push([String(untouched), untouched === 1 ? 'day still open' : 'days still open', true]);
  }

  monthSummary.innerHTML = rows.map(([figure, label, faint]) =>
    `<div class="sum-row${faint ? ' faint' : ''}"><span class="sum-figure">${esc(figure)}</span><span>${esc(label)}</span></div>`,
  ).join('');
}

async function openHistory(month) {
  viewMonth = month ?? viewMonth ?? (viewDate ? viewDate.slice(0, 7) : null);
  onlyPanel('history');
  historyTitle.textContent = 'Reading…';

  try {
    const data = await invoke('fetch_month', { month: viewMonth });
    viewMonth = data.month ?? viewMonth;
    renderMonth(data);
  } catch (e) {
    historyTitle.textContent = String(e).replace(/^Error:\s*/, '');
  }

  fit(true);
}

/** Load a day. Passing null returns to today and to the live polling. */
async function look(date) {
  viewDate = date;
  closePop();

  banner.hidden = !date;
  document.body.classList.toggle('past', !!date);
  if (date) bannerText.textContent = `Looking at ${dayLabel(date)}`;

  await refresh();
  fit(true);
}

monthGrid.addEventListener('click', async (event) => {
  event.stopPropagation();
  const tile = event.target.closest('.tile[data-date]');
  if (!tile) return;

  const date = tile.dataset.date;
  onlyPanel(expanded ? 'slab' : null);
  // Today is not a past day: picking it goes back to the live view rather
  // than pinning the widget to a date that stops being today at midnight.
  await look(tile.classList.contains('today') ? null : date);
});

historyBtn.addEventListener('click', (event) => {
  event.stopPropagation();
  if (!history.hidden) return onlyPanel(expanded ? 'slab' : null);
  openHistory(viewDate ? viewDate.slice(0, 7) : null);
});

document.getElementById('historyClose').addEventListener('click', (event) => {
  event.stopPropagation();
  onlyPanel(expanded ? 'slab' : null);
});

document.getElementById('monthPrev').addEventListener('click', (event) => {
  event.stopPropagation();
  openHistory(shiftMonth(viewMonth, -1));
});

document.getElementById('monthNext').addEventListener('click', (event) => {
  event.stopPropagation();
  openHistory(shiftMonth(viewMonth, 1));
});

document.getElementById('backToToday').addEventListener('click', (event) => {
  event.stopPropagation();
  look(null);
});

banner.addEventListener('click', (event) => event.stopPropagation());
history.addEventListener('click', (event) => event.stopPropagation());
