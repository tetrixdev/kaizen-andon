// The frontend's own tests. CI compiles Rust and never loads this page, which
// is how a build once shipped where `window.__TAURI__` did not exist and every
// panel drew on top of every other: nothing failed, and the app simply showed
// static HTML forever.
//
// Run from a scratch directory, never from inside this repo (the run drops
// test-results/ and screenshots into the CWD):
//
//   mkdir -p ~/pw-work && cd ~/pw-work
//   rm -rf app && cp -r ~/repos/kaizen-andon/src app
//   cp ~/repos/kaizen-andon/tests/lamp.spec.js .
//   ~/.local/bin/pw test lamp.spec.js
//
// The Tauri bridge is stubbed and the data is invented; nothing here talks to
// a real Kaizen.
// Renders the REAL src/index.html at the exact window sizes the Rust side
// asks for, with the Tauri bridge stubbed. Dummy data only.
const { test, expect } = require('@playwright/test');
const http = require('http');
const fs = require('fs');
const path = require('path');

// Chromium refuses ES modules over file://, and main.js is a module, so the
// page has to come over HTTP or none of the app's JS runs at all.
const PORT = 8099;
const PAGE = `http://127.0.0.1:${PORT}/index.html`;
const TYPES = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript' };
let server;

test.beforeAll(async () => {
  server = http.createServer((req, res) => {
    const file = path.join(process.env.APP_DIR || '/work/app', req.url.split('?')[0]);
    fs.readFile(file, (err, body) => {
      if (err) return res.writeHead(404).end();
      res.writeHead(200, { 'content-type': TYPES[path.extname(file)] || 'application/octet-stream' });
      res.end(body);
    });
  });
  await new Promise((done) => server.listen(PORT, '127.0.0.1', done));
});

test.afterAll(async () => {
  await new Promise((done) => server.close(done));
});

const LEDGER = {
  context: 'Work',
  date: '2026-08-19',
  state: 'attention',
  phase: 'accounting',
  window: '08:30 - 17:30',
  started_at: '08:34',
  logs_externally: true,
  target_minutes: 480,
  work_minutes: 331,
  gap_minutes: 95,
  unreferenced_minutes: 120,
  entries: [
    { from: '08:34', to: '10:15', kind: 'work', minutes: 101, description: 'Ticket triage', referenced: true, reference: 'REF-1042' },
    { from: '10:15', to: '10:30', kind: 'rest', minutes: 15, description: 'Coffee' },
    { from: '10:30', to: '12:30', kind: 'work', minutes: 120, description: 'Migration work', referenced: false },
    { from: '12:30', to: '13:00', kind: 'rest', minutes: 30, description: 'Lunch' },
    { from: '14:35', to: '16:25', kind: 'work', minutes: 110, description: 'Review', referenced: true, reference: 'REF-1088' },
  ],
  gaps: [{ from: '13:00', to: '14:35', minutes: 95 }],
};

const stub = (config) => `
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        (window.__calls = window.__calls || []).push([cmd, args]);
        if (cmd === 'load_config') return ${JSON.stringify(config)};
        if (cmd === 'fetch_day') return { ledgers: [${JSON.stringify(LEDGER)}] };
        if (cmd === 'fetch_prompt') return { prompt: 'x', bootstrap: false };
        return null;
      },
    },
    event: { listen: async () => () => {} },
  };
`;

const CONNECTED = { server_url: 'https://kaizen.example.com', client_id: 'abc' };

test('compact: nothing overlaps in 292x88', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(stub(CONNECTED));

  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));

  await page.goto(PAGE);
  await expect(page.locator('#delta')).not.toHaveText('—:—');
  expect(errors, 'the page must not throw').toEqual([]);

  // The three panels that fought each other must be genuinely gone, not
  // merely transparent: a visible one is what put three layers in one window.
  await expect(page.locator('#setup')).toBeHidden();
  await expect(page.locator('#slab')).toBeHidden();
  await expect(page.locator('#actions')).toBeHidden();

  // Nothing may spill out of the window the Rust side sized.
  const overflow = await page.evaluate(() => ({
    w: document.documentElement.scrollWidth,
    h: document.documentElement.scrollHeight,
  }));
  expect(overflow).toEqual({ w: 292, h: 88 });

  await page.screenshot({ path: 'compact.png' });
});

test('expanded: the ledger opens without spilling', async ({ page }) => {
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(stub(CONNECTED));
  await page.goto(PAGE);
  await expect(page.locator('#delta')).not.toHaveText('—:—');

  // Clicking is what the user does; the Rust resize is stubbed out, so do
  // here what place_window would have done.
  await page.locator('#card').click({ position: { x: 140, y: 40 } });
  await page.setViewportSize({ width: 1222, height: 420 });

  await expect(page.locator('#slab')).toBeVisible();
  await expect(page.locator('#actions')).toBeVisible();
  await expect(page.locator('.erow')).toHaveCount(6);

  await page.screenshot({ path: 'expanded.png' });
});

test('first run asks for a window its own panel fits in', async ({ page }) => {
  // Deliberately the compact height: this is the size the window really is
  // when the page boots, and the panel is a good deal taller than it. If the
  // page ever stops asking for more room, the whole setup panel sits above
  // the top edge and there is no way left to connect.
  await page.setViewportSize({ width: 292, height: 88 });
  await page.addInitScript(stub({}));
  await page.goto(PAGE);

  await expect(page.locator('#setup')).toBeVisible();
  await expect(page.locator('#slab')).toBeHidden();
  await expect(page.locator('#actions')).toBeHidden();

  const asked = await page.evaluate(() =>
    (window.__calls || []).filter(([cmd]) => cmd === 'place_window').pop());
  expect(asked, 'first run must reposition the window').toBeTruthy();
  expect(asked[1].mode).toBe('setup');

  const needed = await page.evaluate(() => {
    const s = getComputedStyle(document.body);
    return Math.ceil(
      document.getElementById('setup').getBoundingClientRect().height +
      document.getElementById('card').getBoundingClientRect().height +
      parseFloat(s.paddingTop) + parseFloat(s.paddingBottom) + (parseFloat(s.rowGap) || 0));
  });
  expect(asked[1].height).toBeGreaterThanOrEqual(needed);

  // Now give it the size it asked for and confirm nothing is clipped.
  await page.setViewportSize({ width: 292, height: asked[1].height });
  const spill = await page.evaluate(() => ({
    top: document.getElementById('setup').getBoundingClientRect().top,
    bottom: document.getElementById('card').getBoundingClientRect().bottom,
    view: window.innerHeight,
  }));
  expect(spill.top, 'the panel must not sit above the top edge').toBeGreaterThanOrEqual(0);
  expect(spill.bottom).toBeLessThanOrEqual(spill.view);

  await page.screenshot({ path: 'firstrun.png' });
});
