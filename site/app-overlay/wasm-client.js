// PDF Edit website: the in-browser app's bridge to the WASM engine, plus anonymous counts.
//
// scripts/build-web.sh copies extension/ui/wasm-client.js to app/wasm-client.core.js and this file
// to app/wasm-client.js, the module drop.js and edit.js import. Every export of the core passes
// through unchanged except the three below, which call the core, return exactly what it returns
// (or rethrow what it throws), and send one event with /assets/track.js: which kind of file, whether
// it worked, roughly how long it took. Nothing about the document: no name, size or content.
// See docs/WEB.md, « Analytics ».

import {
  convertPdfWasm as coreConvertPdfWasm,
  openEditSessionWasm as coreOpenEditSessionWasm,
  saveEditSessionWasm as coreSaveEditSessionWasm
} from './wasm-client.core.js';

// The functions declared below take precedence over these re-exports of the same names.
export * from './wasm-client.core.js';

// Loaded apart from the app: when the beacon cannot load (blocked, offline, missing), nothing is
// counted and the app works the same. It is the module the page already loaded, so no second page
// view is sent.
let trackFn = null;
const trackReady = import('/assets/track.js')
  .then((mod) => {
    if (typeof mod.track === 'function') {
      trackFn = mod.track;
    }
  })
  .catch(() => {});

function send(name, dims) {
  trackReady.then(() => {
    try {
      trackFn?.(name, dims);
    } catch {
      // Counting never gets in the way of the app.
    }
  });
}

function elapsedBucket(startedAt) {
  const seconds = (performance.now() - startedAt) / 1000;
  if (seconds < 5) {
    return 'lt5';
  }
  if (seconds < 15) {
    return '5-15';
  }
  if (seconds < 60) {
    return '15-60';
  }
  return 'gt60';
}

// Screen 2 « Modifier dans Pages » / « Modifier dans Word »: in the browser, both give a .docx.
export async function convertPdfWasm(options, ...rest) {
  const f = options?.format === 'pages' ? 'pages' : 'docx';
  const startedAt = performance.now();
  let result;
  try {
    result = await coreConvertPdfWasm(options, ...rest);
  } catch (err) {
    send('conv', { f, ok: 0, err: 'other' });
    throw err;
  }
  if (result?.needs_password) {
    send('pw', { w: 'conv' });
  } else if (result?.ok) {
    send('conv', { f, ok: 1, t: elapsedBucket(startedAt) });
  } else {
    send('conv', { f, ok: 0, err: 'other' });
  }
  return result;
}

// Screen 2 « Corriger quelques mots »: the PDF opens in the editor (edit.html).
export async function openEditSessionWasm(...args) {
  let result;
  try {
    result = await coreOpenEditSessionWasm(...args);
  } catch (err) {
    send('eopen', { ok: 0 });
    throw err;
  }
  if (result?.needs_password) {
    send('pw', { w: 'edit' });
  } else {
    send('eopen', { ok: result?.ok ? 1 : 0 });
  }
  return result;
}

// « Enregistrer le PDF » in the editor.
export async function saveEditSessionWasm(...args) {
  let result;
  try {
    result = await coreSaveEditSessionWasm(...args);
  } catch (err) {
    send('esave', { ok: 0 });
    throw err;
  }
  send('esave', { ok: result?.ok ? 1 : 0 });
  return result;
}
