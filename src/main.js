// Kaizen Desktop: the lamp.
//
// Almost no logic lives here. Kaizen's own App\Support\Ledger decides what a
// hole is, which state the lamp is in and when the question turns from 灯 to
// 印; this file renders the answer and asks Rust to place the window. Two
// implementations of those rules would drift within a week.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const stack = document.getElementById('stack');
const card = document.getElementById('card');
const lamp = document.getElementById('lamp');
const delta = document.getElementById('delta');
const deltaLabel = document.getElementById('deltaLabel');
const sub = document.getElementById('sub');
const track = document.getElementById('track');
const scale = document.getElementById('scale');
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
const editorSave = document.getElementById('editorSave');
const editorOpen = document.getElementById('editorOpen');
const editorSplit = document.getElementById('editorSplit');
const editorDelete = document.getElementById('editorDelete');
const editFrom = document.getElementById('editFrom');
const editTo = document.getElementById('editTo');
const editFromLabel = document.getElementById('editFromLabel');

// Both time fields, wired the moment they exist rather than on first open,
// so no path into the editor can reach an unmasked one.
[editFrom, editTo].forEach(maskTimeField);

// Reachable from the frontend tests, which have no other way to reach a
// module-scoped helper. Costs nothing and keeps the parsing rules honest.
window.__normalise = (text) => normaliseClock(text);
const editQuick = document.getElementById('editQuick');
const editKind = document.getElementById('editKind');
const editTitle = document.getElementById('editTitle');
const editDescription = document.getElementById('editDescription');
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
const rec = document.getElementById('rec');
const recMenu = document.getElementById('recMenu');
const recWrap = document.getElementById('recWrap');
const recState = document.getElementById('recState');
const popTitle = document.getElementById('popTitle');
const popSpan = document.getElementById('popSpan');
const popBody = document.getElementById('popBody');
const addBtn = document.getElementById('addBtn');
const historyBtn = document.getElementById('historyBtn');
const startBtn = document.getElementById('startBtn');
const endBtn = document.getElementById('endBtn');

const STATE_CLASSES = ['lit-wait', 'lit-ok', 'lit-warm', 'lit-call', 'lit-off'];
const LAMP_FOR = { waiting: 'lit-wait', running: 'lit-ok', attention: 'lit-warm', call: 'lit-call', quiet: 'lit-off' };

// One minute, whatever the lamp is saying.
//
// This used to back off to five minutes under the threshold, on the reasoning
// that a quiet lamp has little to report. What that missed is that the number
// on the card is minutes of unaccounted time, which grows by one every minute
// whether or not anyone is listening: a five-minute poll made it visibly jump
// in five-minute steps, so the one figure the widget exists to show was wrong
// for most of the time it was on screen. A request a minute is nothing; a
// counter that lies is the whole product.
const POLL_MS = 60 * 1000;

let expanded = false;
let ledger = null;
let timer = null;

/** The day on screen. `null` means today, which follows the clock past midnight. */
let viewDate = null;
/** Which month the grid is showing, `YYYY-MM`. */
let viewMonth = null;
/** The entry being edited, or null for a new one. */
let editingId = null;

/**
 * A DURATION. Five and a half hours is 5:30, never 05:30: padding an amount
 * makes it look like a time of day.
 */
const hhmm = (m) => `${Math.floor(Math.max(0, m) / 60)}:${String(Math.max(0, m) % 60).padStart(2, '0')}`;

/**
 * A TIME OF DAY, always padded.
 *
 * These two were one function, and every quick action that landed before ten
 * in the morning produced "9:17". Kaizen validates `H:i`, which requires the
 * zero, so the entry was refused by the server after the widget had already
 * accepted it, which reads as the app being broken rather than the format
 * being wrong.
 */
const clock = (m) => {
  const total = ((Math.round(m) % 1440) + 1440) % 1440;

  return `${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`;
};

/** Whatever the user typed, in the form Kaizen accepts. `9:5` is 09:05. */
/**
 * A typed time, however it was typed: 09:45, 9.45, 0945, or 930.
 *
 * Candidates are tried in the order a person most likely meant them and the
 * first legal one wins. That is what lets 930 be half past nine: read as
 * hours-first it is 93:0, and 93 is not an hour, so the only reading left is
 * 9:30. It is also what makes 9301 an error rather than a guess, which is the
 * point: a field that silently reinterprets a fourth digit would file the
 * wrong span without ever saying so.
 */
const normaliseClock = (text) => {
  const raw = String(text ?? '').trim();
  const digits = raw.replace(/\D/g, '');
  const candidates = [];

  // A separator says where the user meant to split, so it is tried first.
  const split = /^(\d{1,2})\s*[:.]\s*(\d{1,2})$/.exec(raw);
  if (split) candidates.push([Number(split[1]), Number(split[2])]);

  if (digits.length === 3) {
    candidates.push([Number(digits.slice(0, 2)), Number(digits.slice(2))]);
    candidates.push([Number(digits.slice(0, 1)), Number(digits.slice(1))]);
  }
  if (digits.length === 4) {
    candidates.push([Number(digits.slice(0, 2)), Number(digits.slice(2))]);
  }

  for (const [hours, minutes] of candidates) {
    if (hours <= 23 && minutes <= 59) return clock(hours * 60 + minutes);
  }

  return null;
};

/**
 * Make a time field behave like one: four digits, and the colon looks after
 * itself.
 *
 * Focus selects, because every edit here is a replacement rather than a
 * correction: the field arrives holding a real time already, and the fastest
 * thing anyone does with it is type over the whole thing.
 *
 * The value is re-derived from its digits on every keystroke, which is what
 * makes backspace skip the colon rather than land on it: there is no colon to
 * delete, only a digit count that shrinks. Four digits is the ceiling, so a
 * fifth is simply not accepted rather than accepted and later refused.
 */
function maskTimeField(input) {
  input.addEventListener('focus', () => input.select());

  input.addEventListener('input', () => {
    const raw = input.value;
    const colon = raw.indexOf(':');

    // A colon with something after it was put there on purpose, by a paste or
    // by someone who types the separator, and it says where the split goes:
    // 9:5 is five past nine, which the digits alone (95) cannot express.
    if (colon > 0 && /\d/.test(raw.slice(colon + 1))) {
      const hours = raw.slice(0, colon).replace(/\D/g, '').slice(0, 2);
      const minutes = raw.slice(colon + 1).replace(/\D/g, '').slice(0, 2);
      input.value = `${hours}:${minutes}`;
    } else {
      // Nothing after it means the colon is being backspaced through, so it
      // goes rather than sitting there needing a second press to clear.
      const digits = raw.replace(/\D/g, '').slice(0, 4);
      input.value = digits.length > 2 ? `${digits.slice(0, 2)}:${digits.slice(2)}` : digits;
    }

    // Typing here is left to right, and the caret has to follow the colon the
    // mask just inserted rather than stay behind it.
    input.setSelectionRange(input.value.length, input.value.length);
  });
}
const toMinutes = (t) => {
  const [h, m] = String(t ?? '0:00').split(':').map(Number);
  return (h || 0) * 60 + (m || 0);
};
const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

/**
 * What the bar spans.
 *
 * The context's window is the frame: 08:00 to 18:00 stays 08:00 to 18:00 all
 * day, so the picture does not rescale under you every time something is
 * filed. The one thing that may stretch it is evidence. Work at 19:30 is real
 * work, and a bar that stopped at 18:00 would simply not draw it, so anything
 * actually recorded pulls the edge out to meet it.
 *
 * `now` deliberately does NOT stretch it. A bar that grew all evening because
 * the clock kept moving would rescale the whole day for nothing; when the time
 * is off the end, the now-line is simply not drawn.
 */
function dayBounds(l) {
  const match = /(\d{1,2}:\d{2})\D+(\d{1,2}:\d{2})/.exec(l.window ?? '');
  let open = match ? toMinutes(match[1]) : 8 * 60;
  let close = match ? toMinutes(match[2]) : 18 * 60;

  const edges = [];
  for (const e of l.entries ?? []) edges.push(toMinutes(e.from), toMinutes(e.to));
  for (const g of l.gaps ?? []) edges.push(toMinutes(g.from), toMinutes(g.to));
  if (l.started_at) edges.push(toMinutes(l.started_at));
  if (l.ended_at) edges.push(toMinutes(l.ended_at));

  for (const at of edges) {
    open = Math.min(open, at);
    close = Math.max(close, at);
  }

  return { open, span: Math.max(60, close - open) };
}

function place(from, to, open, span) {
  const left = Math.max(0, Math.min(100, ((from - open) / span) * 100));
  const width = Math.max(0.6, Math.min(100 - left, ((to - from) / span) * 100));
  return `left:${left}%;width:${width}%`;
}

const at = (minutes, open, span) => `left:${Math.max(0, Math.min(100, ((minutes - open) / span) * 100))}%`;

/** Hour ticks, drawn from the real bounds rather than assumed to be ten. */
function hourTicks(open, span) {
  const ticks = [];

  for (let minute = Math.ceil(open / 60) * 60; minute < open + span; minute += 60) {
    ticks.push(`<span class="tick" style="${at(minute, open, span)}"></span>`);
  }

  return ticks.join('');
}

/** The clock beneath the bar, at whatever spacing keeps it readable. */
function renderScale(l, open, span) {
  if (!expanded) {
    scale.hidden = true;
    return;
  }

  const width = track.clientWidth || 1000;
  const step = Math.max(60, Math.ceil(span / Math.max(2, Math.floor(width / 110)) / 60) * 60);
  const marks = [];
  const edges = [];
  if (l.started_at) edges.push(['start', toMinutes(l.started_at), l.started_at]);
  if (l.ended_at) edges.push(['end', toMinutes(l.ended_at), l.ended_at]);

  // A minute of the day is worth about this many pixels, so a reading needs
  // roughly this much room before it collides with its neighbour.
  const crowded = (minute) => edges.some(([, when]) =>
    Math.abs(((minute - when) / span) * width) < 62);

  for (let minute = Math.ceil(open / 60) * 60; minute <= open + span; minute += step) {
    if (crowded(minute)) continue;
    marks.push(`<span style="${at(minute, open, span)}">${clock(minute)}</span>`);
  }

  // The two readings that are not hours: when the day actually began and ended.
  for (const [name, when, label] of edges) {
    marks.push(`<span class="edge" style="${at(when, open, span)}">${name} ${esc(label)}</span>`);
  }

  scale.innerHTML = marks.join('');
  scale.hidden = false;
}

function renderTrack(l) {
  const { open, span } = dayBounds(l);
  const parts = [];
  const wide = expanded;
  const pixels = track.clientWidth || 292;

  track.classList.toggle('tall', wide);
  if (wide) parts.push(`<span class="ticks">${hourTicks(open, span)}</span>`);

  // Time that has not happened yet is not a hole. Kaizen only reports gaps up
  // to now, so without this the rest of the day is bare track, which is what
  // an unaccounted span looks like at a glance.
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

    // A label only where one fits. Two clipped letters inside a fifteen
    // minute block is noise pretending to be information.
    const room = ((to - from) / span) * pixels;
    const name = e.title ?? (e.kind === 'work' ? 'Work' : 'Rest');
    const label = wide && room > 78 ? `<span class="seg-label">${esc(name)}</span>` : '';

    if (e.kind === 'work') {
      alt = !alt;
      const unref = l.logs_externally && !e.referenced ? ' unref' : '';
      parts.push(`<span class="seg work${alt ? ' alt' : ''}${unref}" data-kind="entry" data-index="${index}" style="${place(from, to, open, span)}">${label}</span>`);
    } else {
      alt = false;
      parts.push(`<span class="seg rest" data-kind="entry" data-index="${index}" style="${place(from, to, open, span)}">${label}</span>`);
    }
  }

  for (const [index, g] of (l.gaps ?? []).entries()) {
    const room = ((toMinutes(g.to) - toMinutes(g.from)) / span) * pixels;
    const label = wide && room > 78 ? `<span class="seg-label">${hhmm(g.minutes)} unaccounted</span>` : '';
    parts.push(`<span class="seg gap" data-kind="gap" data-index="${index}" style="${place(toMinutes(g.from), toMinutes(g.to), open, span)}">${label}</span>`);
  }

  // The day's own edges, as marks rather than as shading. The end is drawn
  // lighter than the start: one is a fact about when you arrived, the other a
  // decision you made to stop.
  if (l.started_at) parts.push(`<span class="mark" style="${at(toMinutes(l.started_at), open, span)}"></span>`);
  if (l.ended_at) parts.push(`<span class="mark soft" style="${at(toMinutes(l.ended_at), open, span)}"></span>`);

  // Now keeps showing after the day is called: the day being over does not
  // stop it being half past four. It is dropped only when the time is off the
  // end of the bar, where drawing it would mean drawing it in the wrong place.
  if (!viewDate) {
    const nowMin = nowMinutes();
    if (nowMin > open && nowMin < open + span) {
      parts.push(`<span class="now" style="${at(nowMin, open, span)}"></span>`);
    }
  }

  track.innerHTML = parts.join('');
  renderScale(l, open, span);
}

function renderEntries(l) {
  // One list in clock order. Entries and holes arrive as two arrays and used
  // to be printed one after the other, so every hole sat below every entry
  // regardless of when it happened: a morning gap appeared under an afternoon
  // meeting, and the column of times ran backwards.
  //
  // The index kept here is the index into the ORIGINAL array, because that is
  // what a click resolves against. Sorting the display must not renumber it.
  const timeline = [
    ...(l.entries ?? []).map((item, index) => ({ item, index, kind: 'entry' })),
    ...(l.gaps ?? []).map((item, index) => ({ item, index, kind: 'gap' })),
  ].sort((a, b) => toMinutes(a.item.from) - toMinutes(b.item.from)
    || toMinutes(a.item.to) - toMinutes(b.item.to));

  const rows = timeline.map(({ item, index, kind }) => {
    if (kind === 'gap') {
      return `<div class="erow gap-row" data-kind="gap" data-index="${index}">
      <span class="e-time">${esc(item.from)}–${esc(item.to)}</span>
      <span class="e-dur">${hhmm(item.minutes)}</span>
      <span class="e-what">Unaccounted</span>
    </div>`;
    }

    const pill = !l.logs_externally || item.kind !== 'work'
      ? (item.kind === 'rest' ? '<span class="pill">rest</span>' : '')
      : item.referenced
        ? `<span class="pill ok">✓ ${esc(item.reference)}</span>`
        : '<span class="pill pending">no reference</span>';

    return `<div class="erow ${item.kind === 'rest' ? 'rest-row' : ''}" data-kind="entry" data-index="${index}">
      <span class="e-time">${esc(item.from)}–${esc(item.to)}</span>
      <span class="e-dur">${hhmm(item.minutes)}</span>
      <span class="e-what">${esc(item.title ?? (item.kind === 'work' ? 'Work' : 'Rest'))}</span>
      ${pill}
    </div>`;
  });

  entriesEl.innerHTML = rows.join('') || '<div class="erow"><span class="e-what">Nothing filed yet.</span></div>';
  slabTitle.textContent = `${l.area} · ${l.date}`;
  slabHint.textContent = l.logs_externally
    ? 'Work here is logged in another system too, so an entry counts only with a reference and a link.'
    : '';
}

function render(l) {
  ledger = l;

  stack.classList.remove(...STATE_CLASSES, 'phase-accounting', 'phase-referencing');
  stack.classList.add(LAMP_FOR[l.state] ?? 'lit-wait', `phase-${l.phase}`);

  lamp.textContent = l.phase === 'referencing' ? '印' : '灯';

  if (l.phase === 'referencing' && l.unreferenced_minutes > 0) {
    delta.textContent = hhmm(l.unreferenced_minutes);
    deltaLabel.textContent = 'not in the other system';
  } else if (liveGapMinutes(l) > 0) {
    delta.textContent = hhmm(liveGapMinutes(l));
    deltaLabel.textContent = 'unaccounted';
  } else if (!l.started_at) {
    delta.textContent = '–:–';
    deltaLabel.textContent = 'day not started';
  } else {
    delta.textContent = hhmm(l.work_minutes);
    deltaLabel.textContent = 'accounted for';
  }

  // The stamps are the way to change the stamps. A day's start is wrong more
  // often than it is missing, because it is set from memory, and hunting for a
  // button that only exists before it is set is the wrong shape: the thing to
  // correct is right there on screen, saying the wrong time.
  // The stamps are live only on the open bar, and the stylesheet enforces it.
  // Closed, this line truncates and the card is one big button, so a live
  // target sitting in the middle of it would be both unreachable when clipped
  // and a hole in the click area when not. Opening is the natural first step
  // anyway: you cannot read a time you want to correct until you can see it.
  const bits = [esc(l.area)];
  if (l.target_minutes) bits.push(`${hhmm(l.work_minutes)} of ${hhmm(l.target_minutes)} logged`);
  if (l.started_at) bits.push(`<button type="button" class="amend" data-mode="start">started ${esc(l.started_at)}</button>`);
  if (l.ended_at) bits.push(`<button type="button" class="amend" data-mode="end">ended ${esc(l.ended_at)}</button>`);

  // A fresh reading replaces a failure, so the red must go with it.
  sub.classList.remove('bad');
  sub.innerHTML = bits.join(' · ');

  renderTrack(l);
  renderEntries(l);

  // The card can be hidden or snoozed; the tray is the one surface always
  // there, so it carries the number.
  const [amount, what] = [delta.textContent, deltaLabel.textContent];
  invoke('set_tooltip', { text: `Kaizen · ${amount} ${what} · ${l.area}` }).catch(() => {});

  // The tray's Hide item follows the same rule the lamp does: amber is a nudge
  // and may be put away, red insists. Pushed from here because this is the
  // thing that just read the day; Rust asking again would be a second answer.
  // capture is Kaizen's answer to whether this minute is inside a context that
  // earns recording. Rust deliberately does not work it out for itself: the
  // rule lives in the ledger, and a second copy here would be free to drift
  // from it. Rust treats a stale answer as no.


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
  stack.classList.remove(...STATE_CLASSES);
  stack.classList.add('lit-off');
  lamp.textContent = '灯';
  delta.textContent = '–:–';
  deltaLabel.textContent = '';
  sub.textContent = message;
  track.innerHTML = '';
  entriesEl.innerHTML = '';
}

/**
 * Tell Rust what the lamp is and whether this minute earns recording.
 *
 * Always about NOW, never about the day on screen. Rust treats an answer
 * older than its TTL as "do not record", so this is a heartbeat as much as a
 * message: it stopping is what stops the recorder.
 */
function pushLamp(l) {
  invoke('set_lamp', { state: l?.state ?? '', capture: l?.capture ?? null }).catch(() => {});
}

async function refresh() {
  try {
    // Always today, whatever is on screen. Capture and the tray follow the
    // clock rather than the view, and a past day is static anyway: fetching
    // the one being looked at would answer the wrong question and, because
    // that answer is also the capture heartbeat, would quietly stop the
    // recorder for as long as the history stayed open.
    const day = await invoke('fetch_day', { date: null });
    const leading = pickLeading(day.ledgers ?? []);
    pushLamp(leading);

    // A day already gone by cannot change, so it is not re-rendered. The
    // timer keeps running regardless, because the heartbeat above has to.
    if (viewDate) return schedule(POLL_MS);

    apply(day);
    if (!leading) return schedule(POLL_MS);

    // A snooze lapses the moment the lamp has something else to say, and that
    // is exactly the case this widget exists for, so it must not stay hidden.
    if (['call', 'attention'].includes(leading.state)) {
      invoke('wake').catch(() => {});
    }

    schedule(POLL_MS);
  } catch (e) {
    // A network blip must not blank the card: keep the last reading and say
    // it is stale rather than pretending the day is suddenly empty.
    if (ledger) {
      sub.textContent = `${sub.textContent} · not reachable`;
    } else {
      failed(sub, `fetch_day for ${viewDate ?? 'today'}`, e, { date: viewDate });
    }
    schedule(POLL_MS);
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

/**
 * Roll the headline over on the minute, independently of the poll.
 *
 * Polling every minute is not the same as being right every minute: a poll can
 * land late, be slow, or fail, and the counter would then sit on a number the
 * user can see is wrong by looking at their own clock. This advances it from
 * the local clock instead and lets the next answer correct it, which is the
 * same bargain the track's now-line already makes.
 *
 * Deliberately touches only the headline. A full re-render would rebuild the
 * entry list under whatever the user is doing with it, and the one figure that
 * goes stale is this one.
 */
let shownMinute = null;

setInterval(() => {
  const minute = nowMinutes();
  if (minute === shownMinute) return;
  shownMinute = minute;

  if (!ledger || viewDate) return;
  if (ledger.phase === 'referencing' && ledger.unreferenced_minutes > 0) return;

  const live = liveGapMinutes(ledger);
  if (live > 0) {
    delta.textContent = hhmm(live);
    deltaLabel.textContent = 'unaccounted';
  }
}, 5000);

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
  const panels = [...stack.children].filter((el) => !el.hidden);

  // The segment popover is out of the page's flow but not out of the window's:
  // the window is sized to its content, so unless its height is counted there
  // is nowhere above the card for it to open into, and it flips downward past
  // the bottom edge where it is simply cut off.
  // Nothing floating counts. The window never changes size to hold the detail
  // card: it is drawn inside whatever room already exists.

  // Which panel is on top changes with what is open, and the stack has to read
  // as one card rather than a pile of separately rounded boxes.
  for (const [index, el] of panels.entries()) {
    el.classList.toggle('stack-top', index === 0);
  }
  card.classList.toggle('stacked', panels[0] !== card);

  // The capture menu is out of flow but not out of the window. It opens
  // upward, and the page is bottom-aligned, so counting it is what puts the
  // room above the card rather than leaving it to be sliced off at the edge.
  const menuRoom = recMenu && !recMenu.hidden
    ? recMenu.getBoundingClientRect().height + 8
    : 0;

  return Math.ceil(
    stack.getBoundingClientRect().height +
      parseFloat(style.paddingTop) +
      parseFloat(style.paddingBottom) +
      menuRoom,
  );
}

/**
 * Ask for a window that fits the page.
 *
 * Coalesced to one call per frame: showing a panel and writing its text are
 * two changes that must not become two resizes. `force` is for the case where
 * the height is unchanged but the WIDTH is not, which is opening and closing.
 */
/** Measure and ask, without coalescing: the caller has already decided. */
async function resize() {
  lastHeight = contentHeight();

  try {
    await invoke('place_window', { expanded, height: lastHeight });
  } catch (e) {
    console.error('place_window failed', e);
  }

  placePop();
}

function fit(force = false) {
  if (queued) return;
  queued = true;

  requestAnimationFrame(async () => {
    queued = false;
    if (contentHeight() === lastHeight && !force) return;

    await resize();
  });
}

// Out faster than in, and linear: a curve on an opacity change reads as a
// stutter. Around a tenth of a second each way is where every house style
// lands for something this small, and the whole swap stays far under the point
// at which a transition starts to feel like waiting.
const FADE_OUT = 70;

/**
 * Wait for the card to have actually gone.
 *
 * Not a timer. A transition does not begin until the next style recalculation
 * after the class lands, which is up to a frame later, so a timer set to the
 * duration fires while the card is still visibly fading and the swap happens
 * in plain sight. The event knows when it is finished; nothing else does.
 */
function faded() {
  return new Promise((done) => {
    const finish = () => {
      stack.removeEventListener('transitionend', watch);
      clearTimeout(failsafe);
      done();
    };

    const watch = (event) => {
      if (event.target === stack && event.propertyName === 'opacity') finish();
    };

    stack.addEventListener('transitionend', watch);

    // Nothing to fade (already invisible, or transitions disabled) means no
    // event will ever arrive, and the card must not be stuck hidden.
    const failsafe = setTimeout(finish, FADE_OUT + 150);
  });
}

/**
 * Change what the card is showing, across a fade.
 *
 * Opening both moves and resizes the window, and those cannot be done as one
 * operation, so some frame always shows it half-changed. Rather than trying to
 * make that frame right, the page stops being visible for it: fade out,
 * rearrange and resize, fade back in with everything already in place.
 */
async function swap(change) {
  const still = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

  if (still) {
    change();

    return resize();
  }

  stack.classList.add('swapping');
  await faded();

  change();
  await resize();

  // One frame at the new size before fading back, so nothing is ever seen
  // arriving at the wrong dimensions.
  await new Promise(requestAnimationFrame);
  stack.classList.remove('swapping');
}

// Any panel changing size for any reason resizes the window. This is what
// makes it unnecessary to remember to call anything after changing any text,
// and what makes a screen that does not exist yet work without being taught.
// EVERY panel, not a hand-picked few. The editor was left out, so an error
// message wrapping to three lines inside it changed the height and nothing
// asked for a bigger window: the panel simply grew off the top of the screen.
const sizes = new ResizeObserver(() => fit());
for (const panel of stack.children) sizes.observe(panel);

/**
 * Redraw the timeline when it actually changes width.
 *
 * How much a block can say depends on how many pixels it gets, and the window
 * resize lands a frame after the click. Drawing once on click meant measuring
 * the 292px card and then being shown at 1222: every label was suppressed and
 * the clock underneath fell back to one reading every five hours.
 */
let trackWidth = 0;
new ResizeObserver(() => {
  const width = Math.round(track.clientWidth);
  if (width === trackWidth || !ledger) return;

  trackWidth = width;
  renderTrack(ledger);
}).observe(track);

function toggle() {
  closePop();

  return swap(() => {
    expanded = !expanded;
    card.classList.toggle('open', expanded);

    // Collapsing puts everything away: the editor and the grid only make sense
    // beside the rows they belong to.
    if (!expanded) {
      editor.hidden = true;
      history.hidden = true;
    }
    slab.hidden = !expanded || !editor.hidden || !history.hidden;

    // The timeline is drawn differently open than closed, so it is redrawn
    // rather than merely restyled: labels only exist where there is room.
    if (ledger) renderTrack(ledger);
  });
}

function showSetup(show) {
  setup.hidden = !show;
  fit(true);
}


sub.addEventListener('click', (event) => {
  const amend = event.target.closest('.amend');
  if (!amend) return;

  event.stopPropagation();
  openEditor({ mode: amend.dataset.mode });
});

card.addEventListener('click', (event) => {
  if (event.target.closest('.btn, .amend') || setup.hidden === false) return;
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
    await failed(setupNote, `connect to ${server}`, e, { server });
    showSetup(true);
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

// ── When something goes wrong ───────────────────────────────────────────
//
// An error the user can read is not the same as an error a developer can act
// on. "Kaizen answered something unexpected" says nothing about which build,
// which call, which arguments or which machine, and by the time anyone asks,
// whoever hit it has moved on. So every failure is shown short and carries a
// word that copies the long version.

let facts = null;

/** Asked once. Nothing here is secret: no token goes near it. */
async function diagnostics() {
  if (!facts) facts = await invoke('diagnostics').catch(() => ({}));
  return facts;
}

function plainly(error) {
  return String(error?.message ?? error).replace(/^Error:\s*/, '');
}

/**
 * Show a failure, with a word that copies the whole story.
 *
 * `where` is the element to say it in; `doing` is what was being attempted,
 * in the developer's terms rather than the user's, because that is the half
 * a bug report is usually missing.
 */
async function failed(where, doing, error, args = null) {
  const message = plainly(error);

  where.innerHTML = `${esc(message)} · <button type="button" class="copy-error">copy</button>`;
  where.classList.add('bad');

  const button = where.querySelector('.copy-error');

  button.addEventListener('click', async (event) => {
    event.stopPropagation();

    const report = [
      'Kaizen Desktop error report',
      '',
      `What it was doing: ${doing}`,
      `What it said:      ${message}`,
      '',
      `When:    ${new Date().toISOString()}`,
      `Viewing: ${viewDate ?? 'today'}${started ? ` (Kaizen clock ${nowLabel()})` : ''}`,
      `Area: ${ledger?.area ?? 'none loaded'}`,
      args ? `Arguments: ${JSON.stringify(args)}` : null,
      '',
      Object.entries(await diagnostics())
        .map(([key, value]) => `${key}: ${value}`)
        .join('\n'),
      `screen: ${window.innerWidth}x${window.innerHeight} @${window.devicePixelRatio}`,
      `agent: ${navigator.userAgent}`,
    ].filter((line) => line !== null).join('\n');

    try {
      await navigator.clipboard.writeText(report);
      button.textContent = 'copied · send it to the developer';
    } catch {
      button.textContent = 'could not copy';
    }
  });
}

/** Put a message back to ordinary, so a stale error does not linger. */
function clearFailure(where) {
  where.classList.remove('bad');
  where.textContent = '';
}

// ── Segment detail ──────────────────────────────────────────────────────
//
// Read-only. Hovering a block says what it was; clicking it does what clicking
// anywhere else on the card does, which is open the day.
//
// There used to be a second level: a click pinned the card so its link could
// be followed without it vanishing under the cursor. That is a whole extra
// interaction mode on a widget the size of a business card, and it made the
// strip the one part of the card that did not open it. The link, Split and
// Delete moved into the editor, which is one click on a row away.

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

function showPop(target, kind, index) {
  const item = segmentAt(kind, index);
  if (!item) return;

  const isGap = kind === 'gap';
  popTitle.textContent = isGap
    ? 'Unaccounted'
    : item.title || (item.kind === 'work' ? 'Work' : 'Rest');
  popSpan.textContent = `${item.from}–${item.to} · ${hhmm(item.minutes)}`;

  // Shut, the whole card is about a hundred pixels tall and there is no room
  // above the strip for anything more than a line.
  pop.classList.toggle('brief', !expanded);

  if (!expanded) {
    popBody.innerHTML = '';
    popKey = `${kind}:${index}`;
    reveal();

    return;
  }

  const lines = [];
  if (!isGap && ledger.logs_externally && item.kind === 'work') {
    lines.push(item.reference
      ? `Reference ${esc(item.reference)}`
      : 'No reference yet, so it does not count.');
    if (item.link) {
      lines.push(`<a class="pop-link" href="${esc(item.link)}" title="${esc(item.link)}" target="_blank" rel="noreferrer">${esc(shortLink(item.link))}</a>`);
    }
  }
  // The fuller account, when there is one. Kept to a preview: this is a
  // hover card, not the place to read a paragraph of markdown.
  if (!isGap && item.description) {
    const preview = item.description.length > 160
      ? item.description.slice(0, 160).trimEnd() + '…'
      : item.description;
    lines.push(esc(preview));
  }
  if (isGap) lines.push('Open the day to account for it.');
  popBody.innerHTML = lines.join('<br>');

  popKey = `${kind}:${index}`;
  reveal();
}

/**
 * The detail card never resizes the window.
 *
 * It used to make room for itself, and that was the whole problem. Growing the
 * window moves its TOP edge up, and the page is anchored to the bottom, so for
 * a frame the webview still holds the old layout and draws it against the new
 * top: the card visibly jumps up and then drops back as the reflow lands. Once
 * per hover, and once per block while crossing the strip.
 *
 * So it fits itself to the room there is. Open, the ledger above the strip is
 * hundreds of pixels and the full card fits easily. Shut, there is almost
 * nothing, so it says one line instead, which fits over the subtitle without
 * needing a single pixel more window.
 */
let revealing = 0;

/**
 * Lay the card out, make room for it, place it, and only then show it.
 *
 * Doing any of that in a different order is what made hovering jump. Shown
 * first, it has nowhere to go and lands clamped against the top edge; the
 * window then grows and it moves to where it belongs. Every block did that,
 * and every block is a different height, so moving along the strip resized
 * the window and re-placed the card over and over.
 *
 * The reserve only ever GROWS while the pointer is on the strip. The first
 * block pays for a resize and the rest ride on the room it took, so sliding
 * across the day is still.
 */
async function reveal() {
  const mine = (revealing += 1);

  // Laid out but not painted, so it can be measured without being seen.
  pop.style.visibility = 'hidden';
  pop.hidden = false;

  // The room is already there, so nothing is resized and nothing moves. All
  // that is left is to put it in the right place before it is painted.
  if (mine !== revealing || pop.hidden) return;

  placePop();
  pop.style.visibility = '';
}

/** Which segment the popover belongs to, so it can be re-placed after a resize. */
let popKey = null;

/**
 * Put the card above its segment.
 *
 * Always upward. The widget lives in the bottom-right corner of the screen, so
 * downward is off the edge of the world; the only reason it ever flipped was
 * that the window had no room above, which is now counted for.
 */
function placePop() {
  if (pop.hidden || !popKey) return;

  const [kind, index] = popKey.split(':');
  const target = track.querySelector(`.seg[data-kind="${kind}"][data-index="${index}"]`);
  if (!target) return;

  const rect = target.getBoundingClientRect();
  const box = pop.getBoundingClientRect();

  // Centred on the segment, then pulled back inside whichever edge it meets.
  // The text wraps rather than the card hanging off the side.
  const left = Math.max(6, Math.min(window.innerWidth - box.width - 6,
    rect.left + rect.width / 2 - box.width / 2));

  pop.style.left = `${left}px`;
  pop.style.top = `${Math.max(6, rect.top - box.height - 8)}px`;
}

function closePop() {
  if (pop.hidden) return;

  revealing += 1;
  pop.hidden = true;
  pop.style.visibility = '';
  popKey = null;
}

/**
 * Light the block and its row together.
 *
 * They are two drawings of one thing: the strip says when, the row says what.
 * Hovering either should say which part of the other it is, or reading across
 * the two means counting rows.
 */
function twinned(kind, index, on) {
  const pair = [
    track.querySelector(`.seg[data-kind="${kind}"][data-index="${index}"]`),
    entriesEl.querySelector(`.erow[data-kind="${kind}"][data-index="${index}"]`),
  ];

  for (const el of pair) el?.classList.toggle('lit', on);
}

function unlight() {
  for (const el of [...track.querySelectorAll('.lit'), ...entriesEl.querySelectorAll('.lit')]) {
    el.classList.remove('lit');
  }
}

let dropping = null;

/**
 * Leaving a block and leaving the strip are not the same event.
 *
 * The strip is wider than what it holds: the frame runs the whole window, so
 * the run-up before the day was started and the tail past the ceiling are bare
 * track. Crossing onto that fires mouseover with no segment under the pointer,
 * and mouseleave does not fire at all, so without this the popover for the
 * block just left stayed up over empty ground.
 *
 * Both routes share one short grace, which is what keeps the seam between two
 * touching blocks from blinking: the next block's mouseover cancels the drop
 * before it runs.
 */
function drop() {
  clearTimeout(dropping);
  dropping = setTimeout(() => {
    unlight();
    closePop();
  }, 120);
}

track.addEventListener('mouseover', (event) => {
  const seg = event.target.closest('.seg[data-kind]');
  if (!seg) return drop();

  clearTimeout(dropping);
  unlight();
  twinned(seg.dataset.kind, seg.dataset.index, true);
  showPop(seg, seg.dataset.kind, Number(seg.dataset.index));
});

track.addEventListener('mouseleave', drop);

// Open, the strip is the same control as the rows above it: a block opens what
// it describes. Closed, there is nothing above it to open, so it falls through
// to the card and opens the day.
track.addEventListener('click', (event) => {
  const seg = event.target.closest('.seg[data-kind]');
  if (!seg || !expanded) return;

  event.stopPropagation();
  closePop();

  const index = Number(seg.dataset.index);
  if (seg.dataset.kind === 'gap') return openEditor({ gap: segmentAt('gap', index) });
  openEditor({ entry: segmentAt('entry', index) });
});

entriesEl.addEventListener('mouseover', (event) => {
  const row = event.target.closest('.erow[data-kind]');
  if (!row) return unlight();

  unlight();
  twinned(row.dataset.kind, row.dataset.index, true);
});

entriesEl.addEventListener('mouseleave', unlight);

// A row is the same thing said in words, so it does the same thing.
entriesEl.addEventListener('click', (event) => {
  event.stopPropagation();
  const row = event.target.closest('.erow');
  if (!row || !row.dataset.kind) return;

  const index = Number(row.dataset.index);
  if (row.dataset.kind === 'gap') return openEditor({ gap: segmentAt('gap', index) });
  openEditor({ entry: segmentAt('entry', index) });
});

// ── Filing ──────────────────────────────────────────────────────────────

/**
 * Kaizen's clock, kept running between polls.
 *
 * Only the offset comes from the server. Reading `local_time` straight off the
 * last answer means every clock in the widget is as stale as the last poll:
 * the now-line stops advancing, the cover over
 * time that has not happened stops moving with it, and "the whole gap" fills
 * in an end that was true when the page last asked. Anchoring the server's
 * time to a local stopwatch keeps the timezone right and the minute live.
 */
let started = null;

function setClock(reported) {
  if (reported) started = { minutes: toMinutes(reported), at: Date.now() };
}

/** Now, in minutes past midnight, in the account's own timezone. */
function nowMinutes() {
  if (!started) {
    const now = new Date();
    return now.getHours() * 60 + now.getMinutes();
  }

  return (started.minutes + Math.floor((Date.now() - started.at) / 60000)) % 1440;
}

/**
 * The unaccounted total, advanced to this minute rather than the last poll's.
 *
 * `gap_minutes` is computed server-side and frozen the moment it is sent, so
 * between polls the headline is stale by however long ago that was. The same
 * problem the local clock above was written for, one value over.
 *
 * It is NOT simply "add the minutes since the poll": the gap only grows while
 * a gap is actually open. A day that has ended, or a minute already covered by
 * a filed entry, does not accrue anything, and adding to those would invent
 * unaccounted time that does not exist. The test is the one the track already
 * relies on: Kaizen reports gaps only up to the moment it answered, so the gap
 * still running is the one whose end IS that moment. If the last gap ended
 * earlier than that, the clock is inside an entry and nothing is accruing.
 */
function liveGapMinutes(l) {
  const gaps = l.gaps ?? [];

  if (viewDate || !l.started_at || l.ended_at || !started || gaps.length === 0) {
    return l.gap_minutes;
  }

  const openEnd = toMinutes(gaps[gaps.length - 1].to);
  if (openEnd < started.minutes) return l.gap_minutes;

  return l.gap_minutes + Math.max(0, nowMinutes() - started.minutes);
}

/** Now as a clock reading, or null before the server has ever answered. */
function nowLabel() {
  return started ? clock(nowMinutes()) : null;
}

/** Render a whole day response, or say plainly that nothing carries a target. */
function apply(day) {
  setClock(day.local_time);
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

  // Marking a moment moves ONE field, and it usually moves backwards: the
  // button is pressed when you remember, not when it happened.
  if (editorMode !== 'entry') {
    if (started) chips.push(['Now', () => { editFrom.value = nowLabel(); }]);

    for (const [label, minutes] of [['5m ago', 5], ['15m ago', 15], ['30m ago', 30], ['1h ago', 60]]) {
      chips.push([label, () => {
        const from = toMinutes(editFrom.value || nowLabel() || '09:00');
        editFrom.value = clock(Math.max(0, from - minutes));
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

    return;
  }

  // The whole gap runs to NOW, not to where the hole reached when the page
  // last asked. An open hole has no end yet: its end is whatever time it is
  // when you decide to account for it. That also makes a separate "to now"
  // chip a second name for the same button, so there is only one.
  if (gap) {
    chips.push(['The whole gap', () => {
      editFrom.value = gap.from;
      editTo.value = viewDate ? gap.to : clock(Math.max(toMinutes(gap.to), nowMinutes()));
    }]);
  }

  for (const [label, minutes] of [['15m', 15], ['30m', 30], ['1h', 60], ['2h', 120]]) {
    chips.push([label, () => {
      const from = toMinutes(editFrom.value || gap?.from || '09:00');
      editTo.value = clock(from + minutes);
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

/** 'entry' fills a span; 'start' and 'end' mark one moment of the day. */
let editorMode = 'entry';

function openEditor({ gap = null, entry = null, splitAt: at = null, mode = 'entry' } = {}) {
  editingId = entry?.id ?? null;
  editing = entry;
  splitAt = at;
  editorMode = mode;

  editor.classList.toggle('external', !!ledger?.logs_externally);
  editor.classList.toggle('moment', mode !== 'entry');
  clearFailure(editorNote);

  // The day's start and end are one field, and prefilled with now because that
  // is usually right. Usually is not always: the button gets pressed when you
  // remember, which is rarely the minute it happened, and a stamp you cannot
  // move is only marginally better than no stamp at all.
  if (mode !== 'entry') {
    editorDelete.hidden = true;
    editorSplit.hidden = true;
    editorOpen.hidden = true;

    const starting = mode === 'start';
    editorTitle.textContent = starting ? 'When did the day start?' : 'When did the day end?';
    editFromLabel.textContent = starting ? 'Started at' : 'Ended at';
    editorSave.textContent = starting ? 'Start the day' : 'Call it a day';
    editFrom.value = (starting ? ledger?.started_at : ledger?.ended_at) ?? nowLabel() ?? '';
    quickChips(null);
    onlyPanel('editor');
    editFrom.focus();
    editFrom.select();

    return;
  }

  editorSave.textContent = 'File it';
  editFromLabel.textContent = 'From';

  // These act on a row that exists, so they appear only when one is open.
  editorDelete.hidden = !entry?.id;
  editorSplit.hidden = !entry?.id || !!at;
  editorOpen.hidden = !entry?.link;

  if (at) {
    editorTitle.textContent = 'Split this in two';
    editorNote.textContent = `Everything after ${at} becomes a second entry.`;
  } else {
    editorTitle.textContent = entry ? 'Edit this entry' : 'Account for a span';
  }

  editFrom.value = entry?.from ?? gap?.from ?? nowLabel() ?? '';
  editTo.value = at ?? entry?.to ?? gap?.to ?? nowLabel() ?? '';
  editTitle.value = entry?.title ?? '';
  editDescription.value = entry?.description ?? '';
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

function complain(text) {
  clearFailure(editorNote);
  editorNote.classList.add('bad');
  editorNote.textContent = text;
}

/** Mark the start or the end of the day, at the time actually typed. */
async function markMoment() {
  const at = normaliseClock(editFrom.value);

  if (!at) return complain('A time reads as 13:45.');

  editFrom.value = at;

  editorSave.disabled = true;
  clearFailure(editorNote);
  editorNote.textContent = 'Saving…';

  const starting = editorMode === 'start';
  const command = starting ? 'start_day' : 'end_day';
  const args = starting ? { at, date: viewDate } : { at, reopen: false, date: viewDate };

  try {
    apply(await invoke(command, args));
    closeEditor();
  } catch (e) {
    // Kaizen refuses an end before its start, which is worth reading rather
    // than silently clamping to something nobody asked for.
    await failed(editorNote, command, e, args);
  } finally {
    editorSave.disabled = false;
  }
}

async function fileEntries() {
  if (editorMode !== 'entry') return markMoment();

  // Normalised before anything else: Kaizen validates H:i, so "9:17" is
  // refused by the server after the widget has already accepted it, which
  // reads as the app being broken rather than the hour needing its zero.
  const from = normaliseClock(editFrom.value);
  const to = normaliseClock(editTo.value);

  if (!from || !to) {
    return complain('Both times read as 13:45.');
  }

  const title = editTitle.value.trim();

  if (!title) {
    return complain('A title: short, business terms, what a colleague would recognise.');
  }

  editFrom.value = from;
  editTo.value = to;

  const common = {
    kind: currentKind(),
    title,
    description: editDescription.value.trim() || null,
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

  editorSave.disabled = true;
  editorNote.classList.remove('bad');
  editorNote.textContent = 'Filing…';

  try {
    const day = await invoke('save_entries', { entries, date: viewDate });
    apply(day);
    closeEditor();
  } catch (e) {
    // Kaizen's own words. The overlap rule is the one that gets met most, and
    // "09:00-10:00 overlaps an entry already filed" says what to change.
    failed(editorNote, 'save_entries', e, { entries, date: viewDate });
  } finally {
    editorSave.disabled = false;
  }
}

/** The entry the editor currently has open, for the actions beside it. */
let editing = null;

editorDelete.addEventListener('click', async (event) => {
  event.stopPropagation();
  if (!editing?.id) return;

  editorDelete.disabled = true;

  try {
    apply(await invoke('delete_entry', { id: editing.id }));
    closeEditor();
  } catch (e) {
    await failed(editorNote, 'delete_entry', e, { id: editing.id });
  } finally {
    editorDelete.disabled = false;
  }
});

// Splitting is filing two entries where there was one: the same panel, with
// the second half offered as soon as the first is saved.
editorSplit.addEventListener('click', (event) => {
  event.stopPropagation();
  if (!editing) return;

  const middle = clock(Math.round((toMinutes(editing.from) + toMinutes(editing.to)) / 2));
  openEditor({ entry: editing, splitAt: middle });
});

editorOpen.addEventListener('click', (event) => {
  event.stopPropagation();
  const url = editLink.value.trim();
  if (!url) return;

  window.__TAURI__?.opener?.openUrl?.(url).catch((e) => console.error('open link', e));
});

editKind.addEventListener('click', (event) => {
  event.stopPropagation();
  const button = event.target.closest('.toggle-btn');
  if (button) setKind(button.dataset.kind);
});

editorSave.addEventListener('click', (e) => { e.stopPropagation(); fileEntries(); });
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

  // Reopening has no time to choose: it only takes the end off again.
  if (endBtn.dataset.reopen === 'yes') {
    try {
      apply(await invoke('end_day', { at: null, reopen: true, date: viewDate }));
    } catch (e) {
      failed(sub, 'end_day (reopen)', e, { reopen: true, date: viewDate });
    }

    return;
  }

  openEditor({ mode: 'end' });
});

startBtn.addEventListener('click', (event) => {
  event.stopPropagation();
  openEditor({ mode: 'start' });
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
    failed(historyTitle, 'fetch_month', e, { month: viewMonth });
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

  // The day being looked at is fetched here rather than by refresh(), which
  // now only ever speaks for today because it carries the capture heartbeat.
  if (date) {
    try {
      apply(await invoke('fetch_day', { date }));
    } catch (e) {
      await failed(sub, `fetch_day for ${date}`, e, { date });
    }
  }

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

// ── The capture indicator ───────────────────────────────────────────────
//
// Driven by an event from the recorder rather than by polling, so the dot
// reflects what is actually being written rather than what the page believes
// should be. Asked once at startup too, since the event only fires on change
// and the app may launch mid-window with capture already running.

function showRecording(on) {
  if (recWrap) recWrap.hidden = !on;
  if (!on) closeRecMenu();
}

function closeRecMenu() {
  if (!recMenu || recMenu.hidden) return;
  recMenu.hidden = true;
  // The window was made taller to hold it, so it has to come back down.
  fit(true);
}

/** Read the truth from the recorder, never from what the page last assumed. */
async function refreshCapture() {
  try {
    const status = await invoke('capture_status');
    showRecording(Boolean(status?.recording));

    if (recState) {
      recState.textContent = status?.paused
        ? `Paused until ${status.paused_until ?? 'later'}`
        : 'Capturing this screen';
    }
    return status;
  } catch {
    return null;
  }
}

listen('capture', () => refreshCapture()).catch(() => {});
refreshCapture();

// The control lives on the dot: the thing that tells you it is recording is
// the thing that stops it. The tray carries the same actions, because the
// moment this most needs stopping is a screenshare, and that is exactly when
// clicking around the widget on a shared screen is the last thing you want.
rec?.addEventListener('click', async (event) => {
  event.stopPropagation();
  if (!recMenu) return;

  const opening = recMenu.hidden;
  await refreshCapture();
  recMenu.hidden = !opening;
  // Absolutely positioned, so the page does not grow for it on its own and
  // the window would clip it. contentHeight() counts it while it is open.
  fit(true);
});

document.getElementById('recPauseHour')?.addEventListener('click', async (event) => {
  event.stopPropagation();
  await invoke('pause_capture', { minutes: 60 }).catch(() => {});
  closeRecMenu();
  refreshCapture();
});

document.getElementById('recUntil')?.addEventListener('change', async (event) => {
  event.stopPropagation();
  const until = event.target.value;
  if (!until) return;

  // A clock already gone today means the next one; Rust decides that, so the
  // widget does not carry a second opinion about which day is meant.
  await invoke('pause_capture', { until }).catch(() => {});
  closeRecMenu();
  refreshCapture();
});

document.getElementById('recStop')?.addEventListener('click', async (event) => {
  event.stopPropagation();
  await invoke('set_capture', { enabled: false }).catch(() => {});
  closeRecMenu();
  refreshCapture();
});

document.getElementById('recFolder')?.addEventListener('click', async (event) => {
  event.stopPropagation();
  await invoke('open_capture_folder').catch(() => {});
  closeRecMenu();
});

recMenu?.addEventListener('click', (event) => event.stopPropagation());
document.addEventListener('click', closeRecMenu);
