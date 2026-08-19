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
const entry = (from, to, minutes, description, referenced) =>
  ({ id: nextId++, from, to, kind: 'work', minutes, description, referenced,
     reference: referenced ? 'REF-1042' : null,
     link: referenced ? 'https://external.example.com/app/entries/REF-1042' : null });

const LEDGER = (over = {}) => ({
  context: 'Work', date: '2026-08-19', state: 'attention', phase: 'accounting',
  window: '08:30 - 17:30', started_at: '08:34', logs_externally: true,
  target_minutes: 480, work_minutes: 331, gap_minutes: 95, unreferenced_minutes: 120,
  entries: [
    entry('08:34', '10:15', 101, 'Ticket triage', true),
    { id: 900, from: '10:15', to: '10:30', kind: 'rest', minutes: 15, description: 'Coffee' },
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
  window.__month = ${JSON.stringify(MONTH)};
  window.__baseLedger = ${JSON.stringify((day.ledgers || [])[0] || null)};
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        window.__calls.push([cmd, args]);
        if (cmd === 'load_config') return ${JSON.stringify(config)};
        if (cmd === 'fetch_day') return { local_time: '14:20', ...${JSON.stringify(day)} };
        if (cmd === 'fetch_prompt') return { prompt: 'x', bootstrap: false };
        if (cmd === 'connect' && ${JSON.stringify(connectError)}) throw new Error(${JSON.stringify(connectError)});
        if (cmd === 'fetch_month') return window.__month;
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
    event: { listen: async () => () => {} },
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
      await page.locator('#editorSave').click();
      await expect(page.locator('#editorNote')).toContainText('overlaps');
    } },
  { name: 'pinned-popover', config: CONNECTED, day: { ledgers: [LEDGER()] },
    async act(page) {
      await open(page);
      await page.locator('#track .seg.work').first().click();
      await expect(page.locator('#pop')).toBeVisible();
      await expect(page.locator('#popActions')).toBeVisible();
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

  await expect(page.locator('#editFrom')).toHaveValue('13:00');
  await expect(page.locator('#editTo')).toHaveValue('14:35');

  await page.locator('#editWhat').fill('Migration work');
  await page.locator('#editorSave').click();

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'save_entries').pop()[1]);
  expect(sent.entries).toHaveLength(1);
  expect(sent.entries[0]).toMatchObject({ from: '13:00', to: '14:35', kind: 'work' });
  await expect(page.locator('#editor')).toBeHidden();
});

test('splitting files the shortened entry and its second half together', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await open(page);
  await page.locator('#track .seg.work').first().click();
  await page.locator('#popSplit').click();
  await page.locator('#editorSave').click();

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'save_entries').pop()[1]);
  expect(sent.entries, 'both halves go in one decision').toHaveLength(2);
  expect(sent.entries[0].id, 'the first half edits the original').toBeTruthy();
  expect(sent.entries[1].id, 'the second half is a new row').toBeFalsy();
  expect(sent.entries[0].to).toBe(sent.entries[1].from);
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

test('quick actions fill the span without typing', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await open(page);
  await page.locator('#addBtn').click();
  await page.getByRole('button', { name: 'To now' }).click();
  await expect(page.locator('#editTo')).toHaveValue('14:20');

  await page.getByRole('button', { name: '30m' }).click();
  await expect(page.locator('#editTo')).toHaveValue('13:30');
});

test('a day can be called and reopened from the widget', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);
  await open(page);

  await expect(page.locator('#startBtn')).toBeHidden();
  await expect(page.locator('#endBtn')).toHaveText('Call it a day');

  // Kaizen answers a write with the new day, so the button turns around.
  await page.evaluate(() => {
    window.__after = { local_time: '17:30', ledgers: [{ ...window.__baseLedger, ended_at: '17:30' }] };
  });
  await page.locator('#endBtn').click();
  await expect(page.locator('#endBtn')).toHaveText('Reopen the day');

  const sent = await page.evaluate(() =>
    window.__calls.filter(([c]) => c === 'end_day').pop()[1]);
  expect(sent.reopen).toBe(false);
});

test('the bar still holds together on a narrow screen', async ({ page }) => {
  // The expanded width is min(1222, work area - 48), so a 1024 laptop gets
  // 976 and the actions row has to survive it. The row has grown twice now.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(bridge(CONNECTED, { ledgers: [LEDGER()] }, null));
  await page.goto(PAGE);
  await settle(page);

  await page.locator('#card').click({ position: { x: 140, y: 40 } });
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
