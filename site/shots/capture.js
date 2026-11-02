// Screenshots of the real app pages for the PDF Edit website (see README.md in this folder).
//
// It drives Google Chrome (puppeteer-core, no bundled browser) through the app served by the
// developer's server, `p2d-cli app <PORT> --no-open`, with the fictional letter of
// demo-letter.html. Every call that would write, open or reveal a file on this Mac
// (/api/convert, /api/action, /api/choose-folder, /api/edit/save) is answered here with a canned
// reply: nothing is written and no app opens. Raw 2× PNGs go to raw/ (1100×720 CSS px);
// convert.py turns the ones the site shows into ../src/img/*.webp.
//
// Environment: PORT (default 8893), BASE_URL (default http://127.0.0.1:<PORT>),
// CHROME (default /Applications/Google Chrome.app/Contents/MacOS/Google Chrome).
import fs from 'node:fs';
import path from 'node:path';
import puppeteer from 'puppeteer-core';

const HERE = import.meta.dirname;
const PORT = process.env.PORT || '8893';
const BASE = (process.env.BASE_URL || `http://127.0.0.1:${PORT}`).replace(/\/$/, '');
const CHROME = process.env.CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const OUT = path.join(HERE, 'raw');
const PDF = path.join(HERE, 'Lettre de l’association.pdf');
const W = 1100;
const H = 720;

if (!fs.existsSync(PDF)) {
  console.error(`Missing ${PDF}: print demo-letter.html to PDF first (README.md, step 2).`);
  process.exit(1);
}
fs.mkdirSync(OUT, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function newPage(browser) {
  const page = await browser.newPage();
  await page.setViewport({ width: W, height: H, deviceScaleFactor: 2 });
  await page.emulateMediaFeatures([{ name: 'prefers-color-scheme', value: 'light' }]);
  await page.setRequestInterception(true);
  page.on('request', (req) => {
    const url = new URL(req.url());
    const json = (body) => req.respond({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    switch (url.pathname) {
      case '/api/save-places':
        return json({ chosen: null });
      case '/api/convert':
        return json({
          ok: true,
          filename: 'Lettre de l’association.docx',
          saved_path: '/Users/marie/Desktop/Lettre de l’association.docx',
          place: { parts: ['Bureau'] },
          opens_in: 'word'
        });
      case '/api/action':
      case '/api/choose-folder':
      case '/api/edit/save':
        return json({ ok: true });
      default:
        return req.continue();
    }
  });
  // The Mac app hides the developer's examples; so do the screenshots.
  await page.evaluateOnNewDocument(() => {
    document.addEventListener('DOMContentLoaded', () => {
      const s = document.createElement('style');
      s.textContent = '#samplesBox{display:none!important} *{caret-color:auto}';
      document.head.append(s);
    });
  });
  return page;
}

async function shot(page, name) {
  await sleep(700); // screen transitions (0.35 s) settle
  await page.screenshot({ path: path.join(OUT, `${name}.png`) });
  console.log('saved', path.join('raw', `${name}.png`));
}

async function toStep2(page) {
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle0' });
  await page.waitForSelector('#stepFile:not([hidden])');
  const input = await page.$('#fileInput');
  await input.uploadFile(PDF);
  await page.waitForSelector('#stepChoice:not([hidden])');
}

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: true,
  args: ['--hide-scrollbars', '--force-color-profile=srgb']
});
try {
  const page = await newPage(browser);

  // 1. Choose the PDF
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle0' });
  await page.waitForSelector('#stepFile:not([hidden])');
  await shot(page, '1-choose');

  // 2. What to do
  await toStep2(page);
  await shot(page, '2-action');

  // 3. Where to save (Word)
  await page.click('[data-action="docx"]');
  await page.waitForSelector('#stepPlace:not([hidden])');
  await shot(page, '3-place');

  // Done: « C'est prêt ! »
  await page.click('[data-place="desktop"]');
  await page.waitForSelector('#doneView:not([hidden])', { timeout: 20000 });
  await shot(page, '4-done');

  // Editor: correct the date line in the PDF itself
  await toStep2(page);
  await page.click('[data-action="edit"]');
  await page.waitForSelector('#stepPlace:not([hidden])');
  await page.click('[data-place="desktop"]');
  await page.waitForFunction(() => location.pathname.endsWith('edit.html'), { timeout: 30000 });
  await page.waitForSelector('.edit-line', { timeout: 60000 });
  await page.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0), { timeout: 60000 });
  await sleep(800);
  await shot(page, '5-editor');

  const lines = await page.$$('.edit-line');
  let found = false;
  for (const el of lines) {
    const box = await el.boundingBox();
    if (!box || box.y < 0 || box.y > H - 120) continue;
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await sleep(250);
    const text = await page.evaluate(() => document.activeElement?.textContent || '');
    if (text.includes('3 novembre 2026')) {
      // Select the « 3 » and type « 10 » over it.
      await page.evaluate(() => {
        const el = document.activeElement;
        const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        let node;
        while ((node = walker.nextNode())) {
          const i = node.data.indexOf('3 novembre');
          if (i >= 0) {
            const r = document.createRange();
            r.setStart(node, i);
            r.setEnd(node, i + 1);
            const sel = getSelection();
            sel.removeAllRanges();
            sel.addRange(r);
            return;
          }
        }
      });
      await page.keyboard.type('10', { delay: 60 });
      await sleep(400);
      await shot(page, '6-typing');
      await page.keyboard.press('Enter');
      found = true;
      break;
    }
    await page.keyboard.press('Escape');
  }
  if (!found) throw new Error('date line « Paris, le 3 novembre 2026 » not found in the editor');
  // The page is redrawn from the edited PDF.
  await sleep(6000);
  await page.mouse.move(W - 10, 10);
  await shot(page, '7-corrected');

  // « Comparer »: before and after, side by side
  await page.waitForSelector('#compareBtn:not([disabled])', { timeout: 20000 });
  await page.click('#compareBtn');
  await page.waitForSelector('#compareView:not([hidden])');
  await page.waitForFunction(() => [...document.querySelectorAll('#compareView img')].every((i) => i.complete && i.naturalWidth > 0), { timeout: 30000 });
  await sleep(1000);
  await page.mouse.move(W - 10, 10);
  await shot(page, '8-compare');
} catch (e) {
  console.error(e);
  process.exitCode = 1;
} finally {
  await browser.close();
}
