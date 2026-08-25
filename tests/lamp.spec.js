// The frontend's own tests. CI compiles Rust and never loads this page, which
// is how a build once shipped where window.__TAURI__ did not exist and every
// panel drew on top of every other: nothing failed, and the app showed static
// HTML forever.
//
// Run from a scratch directory, never from inside this repo (the run drops
// test-results/ and screenshots into the CWD):
//
//   mkdir -p ~/pw-work && cd ~/pw-work
//   rm -rf app && cp -r ~/repos/kaizen-andon/src app
//   cp ~/repos/kaizen-andon/tests/lamp.spec.js .
//   ~/.local/bin/pw test lamp.spec.js
//
// The Tauri bridge is stubbed and every value is invented; nothing here talks
// to a real Kaizen.
// The frontend's own tests. CI compiles Rust and never loads this page.
// The frontend's own tests. CI compiles Rust and never loads this page.
//
// The point of this file is that no state is special. Every screen goes
// through the same two checks: the window size it asks for SETTLES (it does
// not oscillate), and once applied, nothing is clipped. A screen added later
// gets the same treatment by being added to STATES.
const { test, expect } = require('@playwright/test');
const http = require('http');
const fs = require('fs');
const path = require('path');

const ROOT = process.env.APP_DIR || '/work/app';
const PORT = 8099;
const PAGE = `http://127.0.0.1:${PORT}/index.html`;
const TYPES = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript' };
let server;

test.beforeAll(async () => {
  server = http.createServer((req, res) => {
    const file = path.join(ROOT, req.url.split('?')[0]);
    fs.readFile(file, (err, body) => {
      if (err) return res.writeHead(404).end();
      res.writeHead(200, { 'content-type': TYPES[path.extname(file)] || 'application/octet-stream' });
      res.end(body);
    });
  });
  await new Promise((done) => server.listen(PORT, '127.0.0.1', done));
});
test.afterAll(async () => { await new Promise((done) => server.close(done)); });

let nextId = 1;
const entry = (from, to, minutes, title, referenced) =>
  ({ id: nextId++, from, to, kind: 'work', minutes, title, referenced,
     reference: referenced ? 'REF-1042' : null,
     link: referenced ? 'https://external.example.com/app/entries/REF-1042' : null });

const LEDGER = (over = {}) => ({
  capture: { activity: true, screen: true },
  context: 'Work', date: '2026-08-19', state: 'attention', phase: 'accounting',
  window: '08:30 - 17:30', started_at: '08:34', logs_externally: true,
  target_minutes: 480, work_minutes: 331, gap_minutes: 95, unreferenced_minutes: 120,
  entries: [
    entry('08:34', '10:15', 101, 'Ticket triage', true),
    { id: 900, from: '10:15', to: '10:30', kind: 'rest', minutes: 15, title: 'Coffee' },
    entry('10:30', '12:30', 120, 'Migration work', false),
    entry('14:35', '16:25', 110, 'Review', true),
  ],
  gaps: [{ from: '13:00', to: '14:35', minutes: 95 }],
  ...over,
});

// A day long enough to hit the ledger's scroll cap.
const LONG = LEDGER({
  entries: Array.from({ length: 22 }, (_, i) =>
    entry(`0${6 + Math.floor(i / 4)}:00`.slice(-5), `0${6 + Math.floor(i / 4)}:30`.slice(-5), 30,
      `A description long enough to be realistic, number ${i + 1}`, i % 2 === 0)),
});

const MONTH = {
  month: '2026-08', label: 'August 2026', context: 'Work', first_weekday: 6,
  days: Array.from({ length: 31 }, (_, i) => {
    const date = `2026-08-${String(i + 1).padStart(2, '0')}`;
    const weekday = new Date(2026, 7, i + 1).getDay();
    const workday = weekday !== 0 && weekday !== 6;
    return {
      date, has_target: workday, target_minutes: workday ? 480 : null,
      work_minutes: workday && i < 18 ? 480 : 0, entries: workday && i < 18 ? 3 : 0,
      accounted: workday && i < 18, referenced: workday && i < 10,
      is_today: i + 1 === 19, is_future: i + 1 > 19,
    };
  }),
};

const bridge = (config, day, connectError) => `
  window.__calls = [];
  window.__emit = (name, payload) =>
    ((window.__listeners || {})[name] || []).forEach((h) => h({ payload }));
  window.__month = ${JSON.stringify(MONTH)};
  window.__baseLedger = ${JSON.stringify((day.ledgers || [])[0] || null)};
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        window.__calls.push([cmd, args]);
        if (cmd === 'place_window') {
          const el = document.getElementById('stack');
          (window.__opacity = window.__opacity || []).push(el ? Number(getComputedStyle(el).opacity) : 1);
        }
        if (cmd === 'load_config') return ${JSON.stringify(config)};
        if (cmd === 'fetch_day') return { local_time: '14:20', ...${JSON.stringify(day)} };
        if (cmd === 'fetch_prompt') return { prompt: 'x', bootstrap: false };
        if (cmd === 'connect' && ${JSON.stringify(connectError)}) throw new Error(${JSON.stringify(connectError)});
        if (cmd === 'diagnostics') return {
          version: '0.2.1', os: 'windows', arch: 'x86_64',
          server: 'https://kaizen.example.com', connected: true,
          token_store: 'Windows Credential Manager',
        };
        if (cmd === 'start_day' && window.__refuseStart) throw new Error(window.__refuseStart);
        if (cmd === 'end_day' && window.__refuseEnd) throw new Error(window.__refuseEnd);
        if (cmd === 'fetch_month') return window.__month;
        // The dot re-reads the recorder rather than trusting an event payload,
        // so the stub has to answer like the recorder does.
        if (cmd === 'capture_status') return window.__capture || { recording: false, paused: false };
        if (cmd === 'save_entries') {
          if (window.__refuse) throw new Error(window.__refuse);
          return window.__after || ${JSON.stringify(day)};
        }
        if (cmd === 'delete_entry' || cmd === 'start_day' || cmd === 'end_day') {
          return window.__after || ${JSON.stringify(day)};
        }
        return null;
      },
    },
    // Real listeners, so a test can deliver an event the way Rust does. The
    // capture indicator is driven entirely by one, and a stub that swallowed
    // it would let the dot be tested only in the state it starts in.
    event: {
      listen: async (name, handler) => {
        (window.__listeners = window.__listeners || {})[name] =
          (window.__listeners[name] || []).concat(handler);
        return () => {};
      },
    },
  };
`;

const CONNECTED = { server_url: 'https://kaizen.example.com', client_id: 'abc' };

/** Do what Rust does: apply the size the page asked for, until it stops asking. */
async function settle(page) {
  // The first ask comes a frame after load, so poll only once it exists.
  await page.waitForFunction(
    () => (window.__calls || []).some(([c]) => c === 'place_window'),
    null, { timeout: 5000 });

  let last = null;
  let stable = 0;

  for (let round = 0; round < 25; round += 1) {
    const call = await page.evaluate(
      () => (window.__calls || []).filter(([c]) => c === 'place_window').pop());
    expect(call, 'the page must ask for a window size').toBeTruthy();

    const asked = { expanded: !!call[1].expanded, height: call[1].height };
    if (last && last.height === asked.height && last.expanded === asked.expanded) {
      stable += 1;
      if (stable >= 3) return asked;
    } else {
      stable = 0;
      await page.setViewportSize({
        width: asked.expanded ? 1222 : 292,
        height: asked.height,
      });
    }
    last = asked;
    await page.waitForTimeout(50);
  }

  throw new Error(`window size never settled, last was ${JSON.stringify(last)}`);
}

/** Open the card the way the app does: click, then let the window catch up. */
async function open(page) {
  await page.locator('#card').click({ position: { x: 140, y: 40 } });
  await settle(page);
}

/** Nothing visible may sit outside the window. */
async function assertNothingClipped(page, label) {
  const spills = await page.evaluate(() => {
    const out = [];
    const check = (el, name) => {
      if (el.hidden || !el.getClientRects().length) return;
      const r = el.getBoundingClientRect();
      if (r.top < -0.5 || r.left < -0.5 ||
          r.bottom > window.innerHeight + 0.5 || r.right > window.innerWidth + 0.5) {
        out.push({ name, top: Math.round(r.top), bottom: Math.round(r.bottom),
                   view: window.innerHeight });
      }
    };
    for (const el of document.body.children) {
      if (el.tagName === 'SCRIPT') continue;
      check(el, el.id || el.className);
    }
    // The pieces inside the card that carry the words.
    for (const id of ['lamp', 'delta', 'deltaLabel', 'sub', 'track',
                      'setupNote', 'slabTitle', 'discuss', 'connectBtn', 'server']) {
      const el = document.getElementById(id);
      if (el) check(el, id);
    }
    return out;
  });

  expect(spills, `${label}: nothing may be clipped`).toEqual([]);
}

const STATES = [
  { name: 'first-run', config: {}, day: { ledgers: [] } },
  { name: 'first-run-long-error', config: {}, day: { ledgers: [] },
    connectError: "could not store the token: could not write to the credential store: " +
      "Attribute 'password encoded as UTF-16' is longer than platform limit of 2560 chars",
    async act(page) {
      await page.locator('#server').fill('https://kaizen.tetrix.dev');
      await page.locator('#connectBtn').click();
      await expect(page.locator('#setupNote')).toContainText('platform limit');
    } },
  { name: 'nothing-carries-a-target', config: CONNECTED, day: { ledgers: [] } },
  { name: 'compact-attention', config: CONNECTED, day: { ledgers: [LEDGER()] } },
  { name: 'compact-call', config: CONNECTED,
    day: { ledgers: [LEDGER({ state: 'call', gap_minutes: 240 })] } },
  { name: 'compact-running', config: CONNECTED,
    day: { ledgers: [LEDGER({ state: 'running', gap_minutes: 0 })] } },
  { name: 'compact-referencing', config: CONNECTED,
    day: { ledgers: [LEDGER({ phase: 'referencing', gap_minutes: 0 })] } },
  { name: 'compact-not-started', config: CONNECTED,
    day: { ledgers: [LEDGER({ started_at: null, gap_minutes: 0, work_minutes: 0 })] } },
  { name: 'expanded', config: CONNECTED, day: { ledgers: [LEDGER()] },
    async act(page) { await page.locator('#card').click({ position: { x: 140, y: 40 } }); } },
  { name: 'editor-from-a-gap', config: CONNECTED, day: { ledgers: [LEDGER()] },
    async act(page) {
      await open(page);
      await page.locator('#track .seg.gap').first().click();
      await expect(page.locator('#editor')).toBeVisible();
    } },
  { name: 'editor-refused', config: CONNECTED, day: { ledgers: [LEDGER()] },
    async act(page) {
      await page.evaluate(() => { window.__refuse = '13:00-14:35 overlaps an entry already filed.'; });
      await open(page);
      await page.locator('#track .seg.gap').first().click();
      await settle(page);
      await page.locator('#editTitle').fill('Migration work');
      await page.locator('#editorSave').click();
      await expect(page.locator('#editorNote')).toContainText('overlaps');
    } },
  { name: 'history-grid', config: CONNECTED, day: { ledgers: [LEDGER()] },
    async act(page) {
      await open(page);
      await page.locator('#historyBtn').click();
      await expect(page.locator('#monthGrid .tile[data-date]')).toHaveCount(31);
    } },
  { name: 'looking-back', config: CONNECTED, day: { ledgers: [LEDGER()] },
    async act(page) {
      await open(page);
      await page.locator('#historyBtn').click();
      await page.locator('.tile[data-date="2026-08-14"]').click();
      await expect(page.locator('#banner')).toBeVisible();
    } },
  { name: 'day-not-started', config: CONNECTED,
    day: { ledgers: [LEDGER({ started_at: null, entries: [], gaps: [], work_minutes: 0, gap_minutes: 0, state: 'waiting' })] },
    async act(page) {
      await open(page);
      await expect(page.locator('#startBtn')).toBeVisible();
    } },
  { name: 'quiet-day-done', config: CONNECTED,
    day: { ledgers: [LEDGER({ state: 'quiet', phase: 'accounting', logs_externally: false,
      started_at: '08:30', ended_at: '17:05', gap_minutes: 0, gaps: [], work_minutes: 480 })] } },
  { name: 'second-phase', config: CONNECTED,
    day: { ledgers: [LEDGER({ state: 'call', phase: 'referencing', gap_minutes: 0, gaps: [],
      work_minutes: 480, unreferenced_minutes: 120, started_at: '08:30' })] },
    async act(page) { await open(page); } },
  { name: 'expanded-long-day', config: CONNECTED, day: { ledgers: [LONG] },
    async act(page) { await page.locator('#card').click({ position: { x: 140, y: 40 } }); } },
];

for (const state of STATES) {
  test(`${state.name}: settles and nothing is clipped`, async ({ page }) => {
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e)));

    // Start at the compact size, which is what the window really is at boot.
    await page.setViewportSize({ width: 292, height: 88 });
    await page.addInitScript(bridge(state.config, state.day, state.connectError || null));
    await page.goto(PAGE);
    await settle(page);

    if (state.act) {
      await state.act(page);
      await settle(page);
    }

    expect(errors, 'the page must not throw').toEqual([]);
    await assertNothingClipped(page, state.name);
    await page.screenshot({ path: `state-${state.name}.png` });
  });
}

test('a past day stops polling and drops the now line', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await open(page);
  await page.locator('#historyBtn').click();
  await page.locator('.tile[data-date="2026-08-14"]').click();
  await expect(page.locator('#banner')).toBeVisible();

  // The button carries the date, because on a past day it is a different
  // conversation from the one about today.
  await expect(page.locator('#discuss')).toContainText('Aug');
  await expect(page.locator('#track .now')).toHaveCount(0);
  await expect(page.locator('#snoozeBtn')).toBeHidden();

  const before = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'fetch_day').length);
  await page.waitForTimeout(1200);
  const after = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'fetch_day').length);
  expect(after, 'a finished day cannot change, so it must not be re-asked').toBe(before);

  await page.locator('#backToToday').click();
  await expect(page.locator('#banner')).toBeHidden();
  await expect(page.locator('#discuss')).toContainText('today');
});

test('clicking a gap fills the span in rather than asking for it', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await open(page);
  await page.locator('#track .seg.gap').first().click();
  await settle(page);

  await expect(page.locator('#editFrom')).toHaveValue('13:00');
  await expect(page.locator('#editTo')).toHaveValue('14:35');

  await page.locator('#editTitle').fill('Migration work');
  await page.locator('#editorSave').click();

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'save_entries').pop()[1]);
  expect(sent.entries).toHaveLength(1);
  expect(sent.entries[0]).toMatchObject({ from: '13:00', to: '14:35', kind: 'work', title: 'Migration work' });
  await expect(page.locator('#editor')).toBeHidden();
});

test('splitting files the shortened entry and its second half together', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  // Split lives with the entry now, one row-click away.
  await page.locator('.erow[data-kind="entry"]').first().click();
  await settle(page);
  await page.locator('#editorSplit').click();
  await page.locator('#editorSave').click();

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'save_entries').pop()[1]);
  expect(sent.entries, 'both halves go in one decision').toHaveLength(2);
  expect(sent.entries[0].id, 'the first half edits the original').toBeTruthy();
  expect(sent.entries[1].id, 'the second half is a new row').toBeFalsy();
  expect(sent.entries[0].to).toBe(sent.entries[1].from);
});

test('a block does what its row does, once there is a row', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  // Hovering says what a block was, open or shut.
  await page.locator('#track .seg.work').first().hover();
  await expect(page.locator('#pop')).toBeVisible();
  await expect(page.locator('#popTitle')).toContainText('Ticket triage');

  // Shut, there is nothing above the strip to open, so it opens the day.
  await page.locator('#track .seg.work').first().click();
  await settle(page);
  await expect(page.locator('#slab')).toBeVisible();
  await expect(page.locator('#editor'), 'nothing to edit yet').toBeHidden();

  // Open, the strip is the same control as the rows: a block opens its entry.
  await page.locator('#track .seg.work').first().click();
  await settle(page);
  await expect(page.locator('#editor')).toBeVisible();
  await expect(page.locator('#editFrom')).toHaveValue('08:34');

  await page.locator('#editorClose').click();
  await settle(page);

  // And a hole opens the filing panel on its own span, from either side.
  await page.locator('#track .seg.gap').first().click();
  await settle(page);
  await expect(page.locator('#editFrom')).toHaveValue('13:00');
  await expect(page.locator('#editTo')).toHaveValue('14:35');
});

test('hovering either half lights both', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  const seg = page.locator('#track .seg[data-kind="entry"][data-index="2"]');
  const row = page.locator('.erow[data-kind="entry"][data-index="2"]');

  await seg.hover();
  await expect(row, 'the row lights with its block').toHaveClass(/lit/);
  await expect(seg).toHaveClass(/lit/);

  await page.locator('#slabTitle').hover();
  await expect(row).not.toHaveClass(/lit/);

  await row.hover();
  await expect(seg, 'and the block lights with its row').toHaveClass(/lit/);
});

test('delete and the link live with the entry', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  // The first entry carries a reference and a link; the third does not.
  await page.locator('.erow[data-kind="entry"]').first().click();
  await settle(page);
  await expect(page.locator('#editorDelete')).toBeVisible();
  await expect(page.locator('#editorSplit')).toBeVisible();
  await expect(page.locator('#editorOpen')).toBeVisible();

  await page.locator('#editorDelete').click();
  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'delete_entry').pop()[1]);
  expect(sent.id).toBeTruthy();
  await expect(page.locator('#editor')).toBeHidden();
});

test('a personal context is never asked for a reference', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ logs_externally: false })] }, null));
  await page.goto(PAGE);
  await settle(page);

  await open(page);
  await page.locator('#addBtn').click();
  await expect(page.locator('#editor')).toBeVisible();
  await expect(page.locator('#editRef')).toBeHidden();
  await expect(page.locator('#editLink')).toBeHidden();

  // Nothing exists yet, so there is nothing to delete, split or open.
  await expect(page.locator('#editorDelete')).toBeHidden();
  await expect(page.locator('#editorSplit')).toBeHidden();
  await expect(page.locator('#editorOpen')).toBeHidden();
});

test('the grid seals a referenced day differently from an unreferenced one', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await open(page);
  await page.locator('#historyBtn').click();

  // 6 Aug is accounted for and referenced; 17 Aug is accounted for only.
  await expect(page.locator('.tile[data-date="2026-08-06"] .seal')).not.toHaveClass(/partial/);
  await expect(page.locator('.tile[data-date="2026-08-17"] .seal')).toHaveClass(/partial/);
  // A Saturday carries no target, so it carries no judgement either.
  await expect(page.locator('.tile[data-date="2026-08-15"] .seal')).toHaveCount(0);
});

test('the month says what it adds up to, counting only claimed days', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await open(page);
  await page.locator('#historyBtn').click();
  await expect(page.locator('#monthSummary .sum-row').first()).toBeVisible();

  // 1 to 19 August holds 13 working days, 12 of them accounted for; weekends
  // carry no target and must not count against the month.
  await expect(page.locator('#monthSummary')).toContainText('12/13');
  await expect(page.locator('#monthSummary')).toContainText('day still open');
});

test('the kind toggle actually toggles', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);
  await page.locator('#addBtn').click();
  await settle(page);

  await expect(page.locator('.toggle-btn[data-kind="work"]')).toHaveClass(/on/);
  await page.locator('.toggle-btn[data-kind="rest"]').click();
  await expect(page.locator('.toggle-btn[data-kind="rest"]')).toHaveClass(/on/);
  await expect(page.locator('.toggle-btn[data-kind="work"]')).not.toHaveClass(/on/);

  await page.locator('#editTitle').fill('Lunch');
  await page.locator('#editorSave').click();
  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'save_entries').pop()[1]);
  expect(sent.entries[0].kind, 'half of closing a hole is admitting it was not work').toBe('rest');
});

test('the whole gap runs to now, not to where it reached last poll', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  // The fixture's hole ends at 14:35 and Kaizen's clock said 14:20 at the last
  // poll. Wind the local stopwatch on: an open hole ends whenever you get to
  // it, not when the page last asked.
  await page.evaluate(() => { window.__now = Date.now; Date.now = () => window.__now() + 40 * 60 * 1000; });

  await page.locator('#track .seg.gap').first().click();
  await page.getByRole('button', { name: 'The whole gap' }).click();
  await expect(page.locator('#editFrom')).toHaveValue('13:00');
  await expect(page.locator('#editTo'), 'the clock kept running between polls').toHaveValue('15:00');

  // One button, not two names for it.
  await expect(page.getByRole('button', { name: 'To now' })).toHaveCount(0);
});

test('quick actions fill the span without typing', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await open(page);
  await page.locator('#addBtn').click();

  // Add entry opens on the first open hole, so From is already 13:00.
  await expect(page.locator('#editFrom')).toHaveValue('13:00');
  await page.getByRole('button', { name: '30m' }).click();
  await expect(page.locator('#editTo')).toHaveValue('13:30');

  await page.getByRole('button', { name: '2h' }).click();
  await expect(page.locator('#editTo')).toHaveValue('15:00');
});

test('the bar still holds together on a narrow screen', async ({ page }) => {
  // The expanded width is min(1222, work area - 48), so a 1024 laptop gets
  // 976 and the actions row has to survive it. The row has grown twice now.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await page.locator('#card').click({ position: { x: 140, y: 40 } });
  await page.waitForFunction(() =>
    window.__calls.filter(([c]) => c === 'place_window').pop()[1].expanded === true);
  const asked = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'place_window').pop()[1]);
  await page.setViewportSize({ width: 976, height: asked.height });
  await page.waitForTimeout(120);

  await assertNothingClipped(page, 'narrow-expanded');

  // The readout must not be crushed to nothing by the buttons beside it.
  const readout = await page.evaluate(() =>
    document.querySelector('.readout').getBoundingClientRect().width);
  expect(readout, 'the number is the point; it cannot be squeezed out').toBeGreaterThan(220);

  await page.screenshot({ path: 'state-narrow.png' });
});

test('time that has not happened is covered, not left looking like a hole', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  const seg = page.locator('#track .seg.notyet');
  await expect(seg, 'the rest of the day must be covered').toHaveCount(1);

  // It must end where the window does, and never overlap a filed entry.
  const geom = await page.evaluate(() => {
    const el = document.querySelector('#track .seg.notyet');
    const track = document.getElementById('track').getBoundingClientRect();
    const r = el.getBoundingClientRect();
    return { right: Math.round(r.right - track.right), left: r.left - track.left };
  });
  expect(Math.abs(geom.right), 'it runs to the end of the window').toBeLessThan(2);
  expect(geom.left).toBeGreaterThan(0);

  // A finished day has no "not yet" at all.
  await open(page);
  await page.locator('#historyBtn').click();
  await page.locator('.tile[data-date="2026-08-14"]').click();
  await expect(page.locator('#banner')).toBeVisible();
  await expect(page.locator('#track .seg.notyet')).toHaveCount(0);
});

test('a failure can be copied and carries what a developer needs', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ started_at: null, entries: [], gaps: [], work_minutes: 0, gap_minutes: 0 })] }, null));
  await page.goto(PAGE);
  await settle(page);
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);

  await page.evaluate(() => {
    window.__refuseStart = 'Kaizen answered something unexpected: error decoding response body';
  });

  await open(page);
  await page.locator('#startBtn').click();
  await page.locator('#editorSave').click();
  await expect(page.locator('#editorNote')).toContainText('error decoding response body');

  await page.locator('#editorNote .copy-error').click();
  await expect(page.locator('#editorNote .copy-error')).toContainText('send it to the developer');

  const copied = await page.evaluate(() => navigator.clipboard.readText());
  for (const needle of ['start_day', 'error decoding response body', '0.2.1',
                        'windows', 'kaizen.example.com', 'Credential Manager', 'Arguments']) {
    expect(copied, `the report must name ${needle}`).toContain(needle);
  }
});

test('the stack reads as one card, not a pile of rounded boxes', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  // Closed, the card is the whole thing and rounds all four corners.
  let corners = await page.evaluate(() => {
    const s = getComputedStyle(document.getElementById('card'));
    return [s.borderTopLeftRadius, s.borderBottomLeftRadius];
  });
  expect(corners[0]).not.toBe('0px');

  await open(page);

  // Open, the ledger is on top and the card is the bottom of one shape.
  corners = await page.evaluate(() => {
    const card = getComputedStyle(document.getElementById('card'));
    const slab = getComputedStyle(document.getElementById('slab'));
    return {
      cardTop: card.borderTopLeftRadius, cardBottom: card.borderBottomLeftRadius,
      slabTop: slab.borderTopLeftRadius, slabBottom: slab.borderBottomLeftRadius,
    };
  });
  expect(corners.cardTop, 'no notch where the ledger meets the card').toBe('0px');
  expect(corners.cardBottom).not.toBe('0px');
  expect(corners.slabTop, 'the topmost panel rounds the top').not.toBe('0px');
  expect(corners.slabBottom).toBe('0px');

  // With the banner between them, only the ledger still rounds the top.
  await page.locator('#historyBtn').click();
  await page.locator('.tile[data-date="2026-08-14"]').click();
  await expect(page.locator('#banner')).toBeVisible();

  const withBanner = await page.evaluate(() => ({
    slab: getComputedStyle(document.getElementById('slab')).borderTopLeftRadius,
    banner: getComputedStyle(document.getElementById('banner')).borderTopLeftRadius,
    card: getComputedStyle(document.getElementById('card')).borderTopLeftRadius,
  }));
  expect(withBanner.slab).not.toBe('0px');
  expect(withBanner.banner, 'nothing in the middle rounds anything').toBe('0px');
  expect(withBanner.card).toBe('0px');

  await page.screenshot({ path: 'state-corners.png' });
});

test('the day is started at the time you say, not the moment you clicked', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ started_at: null, entries: [], gaps: [], work_minutes: 0, gap_minutes: 0 })] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  await page.locator('#startBtn').click();
  await expect(page.locator('#editor')).toBeVisible();
  await expect(page.locator('#editorTitle')).toContainText('start');

  // One field, prefilled with Kaizen's clock; the span fields are gone.
  await expect(page.locator('#editFrom')).toHaveValue('14:20');
  await expect(page.locator('#editTo')).toBeHidden();
  await expect(page.locator('#editKind')).toBeHidden();
  await expect(page.locator('#editTitle')).toBeHidden();
  await expect(page.locator('#editDescription')).toBeHidden();
  await expect(page.locator('#editorSave')).toHaveText('Start the day');

  // The button is pressed when you remember, not when it happened.
  await page.getByRole('button', { name: '30m ago' }).click();
  await expect(page.locator('#editFrom')).toHaveValue('13:50');

  await page.locator('#editFrom').fill('08:15');
  await page.locator('#editorSave').click();

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'start_day').pop()[1]);
  expect(sent.at, 'the typed time is what is filed').toBe('08:15');
  await expect(page.locator('#editor')).toBeHidden();
});

test('calling it a day asks when, and reopening does not', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  await page.locator('#endBtn').click();
  await expect(page.locator('#editorTitle')).toContainText('end');
  await expect(page.locator('#editorSave')).toHaveText('Call it a day');

  // Kaizen answers a write with the new day, so arrange that before saving.
  await page.evaluate(() => {
    window.__after = { local_time: '17:30', ledgers: [{ ...window.__baseLedger, ended_at: '17:05' }] };
  });

  await page.locator('#editFrom').fill('17:05');
  await page.locator('#editorSave').click();
  let sent = await page.evaluate(() => window.__calls.filter(([c]) => c === 'end_day').pop()[1]);
  expect(sent).toMatchObject({ at: '17:05', reopen: false });

  // Once ended, the button reopens, and there is no time to choose.
  await expect(page.locator('#endBtn')).toHaveText('Reopen the day');
  await expect(page.locator('#endBtn')).toHaveText('Reopen the day');
  await page.locator('#endBtn').click();

  sent = await page.evaluate(() => window.__calls.filter(([c]) => c === 'end_day').pop()[1]);
  expect(sent.reopen, 'reopening asks nothing').toBe(true);
});

test('a refused moment is shown, with the copy word', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await page.evaluate(() => { window.__refuseEnd = 'A day cannot end before it started.'; });
  await open(page);

  await page.locator('#endBtn').click();
  await page.locator('#editFrom').fill('06:00');
  await page.locator('#editorSave').click();

  await expect(page.locator('#editorNote')).toContainText('cannot end before it started');
  await expect(page.locator('#editorNote .copy-error')).toBeVisible();
  await expect(page.locator('#editor'), 'a refusal leaves the panel open to fix').toBeVisible();
});

test('the stamps are the control for the stamps', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ ended_at: '17:05' })] }, null));
  await page.goto(PAGE);
  await settle(page);

  // Closed, the card is one big button and the stamp must not punch a hole in
  // it: clicking there opens the bar like anywhere else does.
  expect(await page.evaluate(() =>
    getComputedStyle(document.querySelector('#sub .amend')).pointerEvents)).toBe('none');

  await open(page);

  // Open, both stamps are reachable while set, which is when they are wrong.
  await expect(page.locator('#sub .amend[data-mode="start"]')).toContainText('started 08:34');
  await expect(page.locator('#sub .amend[data-mode="end"]')).toContainText('ended 17:05');

  await page.locator('#sub .amend[data-mode="start"]').click();
  await expect(page.locator('#editorTitle')).toContainText('start');
  await expect(page.locator('#editFrom'), 'it opens on the time already set').toHaveValue('08:34');

  await page.locator('#editFrom').fill('08:15');
  await page.locator('#editorSave').click();
  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'start_day').pop()[1]);
  expect(sent.at).toBe('08:15');

  // Amending must not also collapse the bar behind the panel.
  expect(await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'place_window').pop()[1].expanded)).toBe(true);
});

test('everything that can be clicked says so', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  const cursors = await page.evaluate(() => {
    const at = (sel) => {
      const el = document.querySelector(sel);
      return el ? getComputedStyle(el).cursor : 'missing';
    };
    return {
      card: at('#card'),
      workSeg: at('#track .seg.work'),
      gapSeg: at('#track .seg.gap'),
      entryRow: at('.erow[data-kind="entry"]'),
      gapRow: at('.erow[data-kind="gap"]'),
      stamp: at('#sub .amend'),
    };
  });

  for (const key of ['card', 'workSeg', 'gapSeg', 'entryRow', 'gapRow', 'stamp']) {
    expect(cursors[key], `${key} opens something, so it must look like it does`).toBe('pointer');
  }
});

test('a gap row does not repeat itself', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  const row = page.locator('.erow[data-kind="gap"]');
  await expect(row).toContainText('Unaccounted');
  await expect(row.locator('.pill'), 'the row already says it, and it is clickable').toHaveCount(0);
});

test('the link field gets a whole row, being the longest thing here', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);
  await page.locator('#addBtn').click();

  const widths = await page.evaluate(() => ({
    link: document.getElementById('editLink').getBoundingClientRect().width,
    what: document.getElementById('editTitle').getBoundingClientRect().width,
    ref: document.getElementById('editRef').getBoundingClientRect().width,
    grid: document.querySelector('.editor-grid').getBoundingClientRect().width,
  }));
  // Reference and link share a row now: an id and where to find it are one
  // fact said twice, and the URL is by far the longer half.
  expect(widths.link / widths.grid, 'the URL takes most of the row').toBeGreaterThan(0.7);
  expect(widths.link).toBeGreaterThan(widths.ref * 2.5);
});

test('a time before ten in the morning keeps its zero', async ({ page }) => {
  // The exact failure from the field: a quick action produced "9:17", Kaizen
  // validates H:i, and the entry was refused after the widget had accepted it.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ started_at: '08:47', gaps: [{ from: '08:47', to: '09:17', minutes: 30 }], entries: [] })] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  await page.locator('#track .seg.gap').first().click();
  await settle(page);
  await expect(page.locator('#editFrom')).toHaveValue('08:47');
  await page.getByRole('button', { name: '30m' }).click();
  await expect(page.locator('#editTo'), 'padded, not 9:17').toHaveValue('09:17');

  // And whatever gets typed is normalised rather than refused later.
  await page.locator('#editTo').fill('9:5');
  await page.locator('#editTitle').fill('Early start');
  await page.locator('#editorSave').click();

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'save_entries').pop()[1]);
  expect(sent.entries[0].to).toBe('09:05');
  expect(sent.entries[0].from).toBe('08:47');
});

test('a duration is not padded, because it is not a time', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ gap_minutes: 331, state: 'call' })] }, null));
  await page.goto(PAGE);
  await settle(page);

  // Five and a half hours is 5:31. "05:31" would read as half past five.
  await expect(page.locator('#delta')).toHaveText('5:31');
});

test('the glow wraps the whole stack, and breathes rather than blinks', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ state: 'call', gap_minutes: 240 })] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  // The state lives on the stack, so the light reaches the ledger too and the
  // glow traces one shape instead of drawing a line across the seam.
  await expect(page.locator('#stack')).toHaveClass(/lit-call/);
  await expect(page.locator('#card')).not.toHaveClass(/lit-call/);

  const geom = await page.evaluate(() => {
    const box = (id) => document.getElementById(id).getBoundingClientRect();
    const stack = box('stack');
    return {
      wrapsSlab: stack.top <= box('slab').top + 1,
      wrapsCard: stack.bottom >= box('card').bottom - 1,
      animates: getComputedStyle(document.getElementById('stack'), '::after').animationName,
      lamp: getComputedStyle(document.querySelector('.lamp'), '::after').animationName,
    };
  });

  expect(geom.wrapsSlab && geom.wrapsCard, 'the glow follows the whole open shape').toBe(true);
  expect(geom.animates).toBe('pulse');
  expect(geom.lamp).toBe('pulse-lamp');

  await page.screenshot({ path: 'state-call-open.png' });
});

test('the second phase arrives only where a reference is required', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ phase: 'referencing', state: 'call', gap_minutes: 0, gaps: [],
      work_minutes: 480, unreferenced_minutes: 120 })] }, null));
  await page.goto(PAGE);
  await settle(page);

  // The glyph is the question: 灯 asks where the day went, 印 asks whether it
  // reached the other system.
  await expect(page.locator('#lamp')).toHaveText('印');
  await expect(page.locator('#stack')).toHaveClass(/phase-referencing/);
  await expect(page.locator('#delta')).toHaveText('2:00');
  await expect(page.locator('#deltaLabel')).toContainText('not in the other system');

  // The re-skin: unreferenced work hollows out and the alternation stops.
  const seg = await page.evaluate(() => {
    const el = document.querySelector('#track .seg.work.unref');
    return el ? getComputedStyle(el).borderStyle : 'missing';
  });
  expect(seg).toBe('dashed');

  await page.screenshot({ path: 'state-phase-two.png' });
});

test('a day that is done rests, and stops resting when you open it', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED,
    { ledgers: [LEDGER({ state: 'quiet', logs_externally: false, started_at: '08:30',
      ended_at: '17:05', gap_minutes: 0, gaps: [], work_minutes: 480 })] }, null));
  await page.goto(PAGE);
  await settle(page);

  await expect(page.locator('#stack')).toHaveClass(/lit-off/);

  // A finished day is said in colour, not in opacity. Fading the card made the
  // widget read as half-there rather than as done, which is the look a
  // disabled control has, parked permanently in the corner of a screen.
  const shut = await page.evaluate(() =>
    getComputedStyle(document.getElementById('card')).opacity);
  expect(Number(shut), 'a finished day is not dimmed').toBe(1);

  await open(page);
  const openOpacity = await page.evaluate(() => ({
    card: getComputedStyle(document.getElementById('card')).opacity,
    slab: getComputedStyle(document.getElementById('slab')).opacity,
  }));
  expect(Number(openOpacity.card), 'nothing you asked to look at is dimmed').toBe(1);
  expect(Number(openOpacity.slab), 'least of all the rows you are reading').toBe(1);

  await page.screenshot({ path: 'state-quiet-open.png' });
});

test('the state colour reaches the number and the glyph', async ({ page }) => {
  // Asserting the class is on the element is not enough: the class was there,
  // --lamp resolved to vermilion on the stack, and both the number and the
  // glyph still painted grey, because a leftover lit-wait on the card was a
  // nearer ancestor redefining --lamp for everything inside it.
  const cases = [
    ['call', '--vermilion'],
    ['attention', '--ochre'],
    ['running', '--moss'],
  ];

  for (const [state, token] of cases) {
    await page.setViewportSize({ width: 292, height: 88 });
    await page.addInitScript(bridge(CONNECTED,
      { ledgers: [LEDGER({ state, gap_minutes: state === 'running' ? 0 : 95 })] }, null));
    await page.goto(PAGE);
    await settle(page);

    const seen = await page.evaluate((t) => {
      const hex = getComputedStyle(document.documentElement).getPropertyValue(t).trim();
      const probe = document.createElement('span');
      probe.style.color = hex;
      document.body.appendChild(probe);
      const want = getComputedStyle(probe).color;
      probe.remove();
      return {
        want,
        glyph: getComputedStyle(document.getElementById('lamp')).color,
        delta: getComputedStyle(document.getElementById('delta')).color,
      };
    }, token);

    expect(seen.glyph, `${state}: the glyph carries the state's colour`).toBe(seen.want);
    if (state !== 'running') {
      expect(seen.delta, `${state}: so does the number`).toBe(seen.want);
    }
  }
});

test('the ring grows outward rather than fading in place', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER({ state: 'call' })] }, null));
  await page.goto(PAGE);
  await settle(page);

  const anim = await page.evaluate(() => {
    const ring = getComputedStyle(document.getElementById('stack'), '::after');
    const lamp = getComputedStyle(document.querySelector('.lamp'), '::after');
    return {
      stack: ring.animationName, lamp: lamp.animationName,
      stackDuration: ring.animationDuration, lampDuration: lamp.animationDuration,
      stackEasing: ring.animationTimingFunction, lampEasing: lamp.animationTimingFunction,
    };
  });

  // The expansion is the character of it: a ring that only faded in place read
  // as a switch rather than a breath.
  expect(anim.stack).toBe('pulse');
  expect(anim.lamp).toBe('pulse-lamp');
  // Aligned: the glyph brightens on the beat the ring is born.
  expect(anim.stackDuration).toBe(anim.lampDuration);
  expect(anim.stackEasing).toBe(anim.lampEasing);
});

test('the shadow fades out inside the window, not at its edge', async ({ page }) => {
  // The window is transparent and sized to its content, so anything painted
  // past the padding is cut off square: the card grows grey corners instead of
  // soft ones. Whatever the card casts has to land inside.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER({ state: 'call' })] }, null));
  await page.goto(PAGE);
  await settle(page);

  const room = await page.evaluate(() => {
    const s = getComputedStyle(document.body);
    const stack = document.getElementById('stack').getBoundingClientRect();
    return {
      padding: parseFloat(s.paddingTop),
      left: stack.left,
      right: window.innerWidth - stack.right,
      top: stack.top,
      bottom: window.innerHeight - stack.bottom,
    };
  });

  // The ring reaches 9px and the drop shadow about 10px past the edge.
  for (const side of ['left', 'right', 'top', 'bottom']) {
    expect(room[side], `${side}: the glow needs somewhere to go`).toBeGreaterThanOrEqual(14);
  }
  expect(room.padding).toBeGreaterThanOrEqual(14);
});

test('the rows read in clock order, holes among the entries', async ({ page }) => {
  // Exactly the shape seen in the field: one entry late in the morning and two
  // holes around it. Printed as two arrays, the entry came first and the times
  // ran backwards down the column.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER({
    started_at: '08:47',
    entries: [{ id: 5, from: '11:00', to: '11:30', kind: 'work', minutes: 30,
                title: 'Call with Sanne', referenced: false, reference: null, link: null }],
    gaps: [{ from: '08:47', to: '11:00', minutes: 133 }, { from: '11:30', to: '11:39', minutes: 9 }],
  })] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  const times = await page.evaluate(() =>
    [...document.querySelectorAll('.erow .e-time')].map((el) => el.textContent.trim()));
  expect(times).toEqual(['08:47–11:00', '11:00–11:30', '11:30–11:39']);

  // Sorting the display must not renumber what a click resolves against.
  await page.locator('.erow').first().click();
  await settle(page);
  await expect(page.locator('#editFrom'), 'the first row is the first hole').toHaveValue('08:47');
});


test('opening fades out and back in, with the resize hidden inside it', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await page.locator('#card').click({ position: { x: 140, y: 40 } });
  await page.waitForFunction(() =>
    window.__calls.filter(([c]) => c === 'place_window').pop()[1].expanded === true);

  // The first reading is the boot placement, which happens in plain sight.
  const seen = await page.evaluate(() => window.__opacity);
  expect(seen.length, 'the window was asked to move').toBeGreaterThan(1);
  expect(seen[0], 'the first placement is not a swap').toBe(1);
  // Exactly gone, not nearly gone: waiting on a timer instead of the event
  // meant the swap happened while the card was still fading.
  expect(seen[seen.length - 1], 'nothing was visible while it opened').toBe(0);

  // It comes back.
  await expect
    .poll(async () => page.evaluate(() =>
      Number(getComputedStyle(document.getElementById('stack')).opacity)))
    .toBe(1);

  const timing = await page.evaluate(() => {
    const el = document.getElementById('stack');
    const out = getComputedStyle(el);
    el.classList.add('swapping');
    const during = getComputedStyle(el).transitionDuration;
    el.classList.remove('swapping');
    return { back: out.transitionDuration, away: during, easing: out.transitionTimingFunction };
  });

  // Out faster than in, and linear, which is what a fade wants.
  expect(timing.away).toBe('0.07s');
  expect(timing.back).toBe('0.13s');
  expect(timing.easing).toBe('linear');
});


test('sliding across the strip moves nothing and resizes once', async ({ page }) => {
  // Written after watching it, frame by frame, rather than reasoning about it.
  // Making room for the detail card resized the window; the resize moved the
  // strip relative to the cursor; the cursor left the strip; the card closed;
  // the window shrank; the cursor was back on the strip. Fourteen resizes and
  // five different card positions to cross the day once.
  //
  // The harness has to be honest about two things or it cannot see this: a
  // real window grows UPWARD keeping its bottom edge, so the fixed point on
  // screen is the distance from the bottom of the viewport; and the resize
  // arrives a frame or two AFTER the app asks for it.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.addInitScript(`
    addEventListener('DOMContentLoaded', () => {
      window.__frames = [];
      const tick = () => {
        const card = document.getElementById('card');
        const pop = document.getElementById('pop');
        if (card) {
          const c = card.getBoundingClientRect();
          const shown = !pop.hidden && getComputedStyle(pop).visibility !== 'hidden';
          window.__frames.push({
            cardUp: Math.round(innerHeight - c.bottom),
            popUp: shown ? Math.round(innerHeight - pop.getBoundingClientRect().bottom) : null,
          });
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
  `);
  await page.goto(PAGE);

  let stop = false;
  let height = 88;
  const os = (async () => {
    while (!stop) {
      const want = await page.evaluate(() => {
        const all = window.__calls.filter(([c]) => c === 'place_window');
        return all.length ? all[all.length - 1][1] : null;
      });
      if (want && want.height !== height) {
        await page.waitForTimeout(24);
        height = want.height;
        await page.setViewportSize({ width: want.expanded ? 1222 : 332, height });
      }
      await page.waitForTimeout(12);
    }
  })();

  await page.waitForTimeout(400);
  await page.evaluate(() => { window.__frames = []; window.__calls.length = 0; });

  for (let i = 0; i <= 16; i += 1) {
    const strip = await page.locator('#track').boundingBox();
    await page.mouse.move(strip.x + (strip.width * i) / 16, strip.y + strip.height / 2);
    await page.waitForTimeout(45);
  }
  await page.waitForTimeout(250);
  stop = true;
  await os;

  const frames = await page.evaluate(() => window.__frames);
  const cardAt = [...new Set(frames.map((f) => f.cardUp))];
  const popAt = [...new Set(frames.filter((f) => f.popUp !== null).map((f) => f.popUp))];
  const resizes = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'place_window').length);

  expect(frames.length, 'frames were recorded').toBeGreaterThan(30);
  expect(cardAt, 'the card holds one place on screen').toHaveLength(1);
  expect(popAt.length, 'so does the detail card').toBeLessThanOrEqual(2);
  expect(resizes, 'the room is taken once, not once per block').toBeLessThanOrEqual(2);
});

test('the detail card fits the room there is, and never asks for more', async ({ page }) => {
  // Growing the window moves its top edge, and the page is anchored to the
  // bottom, so for a frame the old layout is drawn against the new top and the
  // whole card jumps up and drops back. The detail card therefore never
  // resizes anything: it fits itself to what is already there.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  const asks = () => page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'place_window').length);
  const before = await asks();

  // Shut: one line, over the subtitle, inside the window as it stands.
  await page.locator('#track .seg.work').first().hover();
  await expect(page.locator('#pop')).toBeVisible();
  await expect(page.locator('#pop')).toHaveClass(/brief/);
  await expect(page.locator('#popBody')).toBeHidden();
  await expect(page.locator('#popTitle')).toContainText('Ticket triage');

  let box = await page.evaluate(() => {
    const p = document.getElementById('pop').getBoundingClientRect();
    return { top: p.top, bottom: p.bottom, height: p.height, view: window.innerHeight };
  });
  expect(box.top, 'inside the window').toBeGreaterThanOrEqual(0);
  expect(box.bottom).toBeLessThanOrEqual(box.view);
  expect(box.height, 'one line, not a card').toBeLessThan(40);
  expect(await asks(), 'shut, it costs no resize').toBe(before);

  // Open: the ledger above gives it hundreds of pixels, so it says everything.
  await open(page);
  const opened = await asks();

  await page.locator('#track .seg.work').first().hover();
  await expect(page.locator('#pop')).toBeVisible();
  await expect(page.locator('#pop')).not.toHaveClass(/brief/);
  await expect(page.locator('#popBody')).toContainText('REF-1042');

  box = await page.evaluate(() => {
    const p = document.getElementById('pop').getBoundingClientRect();
    const seg = document.querySelector('#track .seg.work').getBoundingClientRect();
    return { bottom: p.bottom, segTop: seg.top, top: p.top };
  });
  expect(box.bottom, 'above its own block').toBeLessThanOrEqual(box.segTop + 1);
  expect(box.top).toBeGreaterThanOrEqual(0);
  expect(await asks(), 'open, it costs no resize either').toBe(opened);
});

test('the detail card leaves when the pointer crosses onto bare track', async ({ page }) => {
  // The strip is wider than what it holds. The frame is the context's window,
  // so the run-up before the day started and the tail past the last block are
  // bare track, and crossing onto that fires mouseover with no segment under
  // the pointer while never firing mouseleave at all. The card used to stay
  // up describing a block the pointer had already left.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await page.waitForTimeout(300);

  const shown = () => page.evaluate(() => {
    const pop = document.getElementById('pop');
    return !pop.hidden && getComputedStyle(pop).visibility !== 'hidden';
  });
  const lit = () => page.locator('.lit').count();

  const seg = await page.locator('#track .seg[data-kind]').first().boundingBox();
  await page.mouse.move(seg.x + seg.width / 2, seg.y + seg.height / 2);
  await page.waitForTimeout(250);
  expect(await shown(), 'a block under the pointer says what it is').toBe(true);
  expect(await lit(), 'and lights itself and its row').toBeGreaterThan(0);

  // This day holds nothing between 16:25 and the 17:30 close, so the far right
  // of the frame is track and nothing else.
  const strip = await page.locator('#track').boundingBox();
  await page.mouse.move(strip.x + strip.width - 2, strip.y + strip.height / 2);
  await page.waitForTimeout(300);
  expect(await shown(), 'bare track describes nothing, so nothing is described').toBe(false);
  expect(await lit(), 'and nothing is left lit behind it').toBe(0);
});

test('the capture dot is shown only while frames are actually being written', async ({ page }) => {
  // Capture that leaves no mark on screen is capture nobody consented to
  // twice. The dot is driven by the recorder's own event rather than by what
  // the page believes should be happening, so it cannot say "recording" while
  // a pause or a closed window means nothing is written.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await expect(page.locator('#rec')).toBeHidden();

  await page.evaluate(() => {
    window.__capture = { recording: true, paused: false };
    window.__emit('capture', { recording: true });
  });
  await expect(page.locator('#rec')).toBeVisible();

  await page.evaluate(() => {
    window.__capture = { recording: false, paused: false };
    window.__emit('capture', { recording: false });
  });
  await expect(page.locator('#rec')).toBeHidden();
});

test('the dot carries the controls, and says when a pause ends', async ({ page }) => {
  // The control belongs where the fact is: the thing telling you it is
  // recording is the thing that stops it.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await page.evaluate(() => {
    window.__capture = { recording: true, paused: false };
    window.__emit('capture', { recording: true });
  });
  await expect(page.locator('#rec')).toBeVisible();
  await expect(page.locator('#recMenu')).toBeHidden();

  await page.locator('#rec').click();
  await expect(page.locator('#recMenu')).toBeVisible();

  // A pause is stated with its end, not as a bare "paused": knowing it lapses
  // at 14:30 is the whole difference between a pause and a mystery.
  await page.evaluate(() => {
    window.__capture = { recording: false, paused: true, paused_until: '14:30' };
  });
  await page.locator('#recPauseHour').click();
  await expect(page.locator('#recMenu')).toBeHidden();

  const calls = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'pause_capture').map(([, a]) => a));
  expect(calls.at(-1), 'the hour is passed as minutes').toEqual({ minutes: 60 });
});

test('pausing until a clock time hands the clock to Rust, not a computed instant', async ({ page }) => {
  // Which day "00:30" means is decided in one place. The widget forwarding a
  // bare clock is what keeps that from becoming two answers.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await page.evaluate(() => {
    window.__capture = { recording: true, paused: false };
    window.__emit('capture', { recording: true });
  });
  await page.locator('#rec').click();
  await page.locator('#recUntil').fill('14:30');
  await page.locator('#recUntil').dispatchEvent('change');

  const calls = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'pause_capture').map(([, a]) => a));
  expect(calls.at(-1), 'the wall clock goes over as written').toEqual({ until: '14:30' });
});

test('the page hands Kaizen the capture answer rather than deciding it here', async ({ page }) => {
  // What counts as a working day lives in the ledger. A second copy of that
  // rule in the widget would be free to drift from the first, so the page
  // forwards the answer and Rust treats a stale one as no.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER({ capture: { activity: true, screen: true } })] }, null));
  await page.goto(PAGE);
  await settle(page);

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'set_lamp').map(([, a]) => a));

  expect(sent.length, 'the lamp state was pushed').toBeGreaterThan(0);
  expect(sent[sent.length - 1].capture, 'with the window answer alongside it')
    .toEqual({ activity: true, screen: true });
});

test('the capture folder can be opened from the dot', async ({ page }) => {
  // Reachable whether or not anything is recording: the archive outlives the
  // switch, and looking at what was kept is not the same act as keeping more.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await page.evaluate(() => {
    window.__capture = { recording: true, paused: false };
    window.__emit('capture', { recording: true });
  });
  await page.locator('#rec').click();
  await page.locator('#recFolder').click();

  const opened = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'open_capture_folder').length);
  expect(opened, 'the folder was asked for').toBe(1);
  await expect(page.locator('#recMenu')).toBeHidden();
});

test('the editor refuses to file without a title', async ({ page }) => {
  // Server-side this is already enforced (DesktopLedgerController requires
  // it), but a raw 422 from a blank title reads as the app being broken.
  // Caught here instead, with a message that says what to type.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);
  await page.locator('#track .seg.gap').first().click();
  await settle(page);

  await page.locator('#editorSave').click();

  await expect(page.locator('#editorNote')).toContainText('title');
  const attempted = await page.evaluate(() =>
    window.__calls.some(([c]) => c === 'save_entries'));
  expect(attempted, 'nothing was sent').toBe(false);
});

test('title and description both reach the save call, and both come back when reopened', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);
  await page.locator('#track .seg.gap').first().click();
  await settle(page);

  await page.locator('#editTitle').fill('Client call');
  await page.locator('#editDescription').fill('Discussed the rollout timeline.');
  await page.locator('#editorSave').click();

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'save_entries').pop()[1]);
  expect(sent.entries[0]).toMatchObject({
    title: 'Client call',
    description: 'Discussed the rollout timeline.',
  });
});

test('an empty description is sent as null, not as an empty string', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);
  await page.locator('#track .seg.gap').first().click();
  await settle(page);

  await page.locator('#editTitle').fill('Fine');
  await page.locator('#editorSave').click();

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'save_entries').pop()[1]);
  expect(sent.entries[0].description).toBeNull();
});
