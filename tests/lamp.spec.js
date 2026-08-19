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

const entry = (from, to, minutes, description, referenced) =>
  ({ from, to, kind: 'work', minutes, description, referenced, reference: referenced ? 'REF-1042' : null });

const LEDGER = (over = {}) => ({
  context: 'Work', date: '2026-08-19', state: 'attention', phase: 'accounting',
  window: '08:30 - 17:30', started_at: '08:34', logs_externally: true,
  target_minutes: 480, work_minutes: 331, gap_minutes: 95, unreferenced_minutes: 120,
  entries: [
    entry('08:34', '10:15', 101, 'Ticket triage', true),
    { from: '10:15', to: '10:30', kind: 'rest', minutes: 15, description: 'Coffee' },
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

const bridge = (config, day, connectError) => `
  window.__calls = [];
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        window.__calls.push([cmd, args]);
        if (cmd === 'load_config') return ${JSON.stringify(config)};
        if (cmd === 'fetch_day') return ${JSON.stringify(day)};
        if (cmd === 'fetch_prompt') return { prompt: 'x', bootstrap: false };
        if (cmd === 'connect' && ${JSON.stringify(connectError)}) throw new Error(${JSON.stringify(connectError)});
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
