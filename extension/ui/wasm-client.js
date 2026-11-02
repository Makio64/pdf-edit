// 100% Online Client-Side WebAssembly Engine Bridge (`p2d_wasm`).
// Powers both `drop.html` (1:1 Word/Pages conversion) and `edit.html` (Direct In-Place 1:1 PDF Editor)
// when hosted on any static web server (Netlify, GitHub Pages, Cloudflare Pages, S3, etc.) without a backend.

let wasmMod = null;
let wasmInitPromise = null;
let lastSavedBlobUrl = null;
let lastSavedFilename = null;
let chosenDirHandle = null;
let chosenDirName = null;
const fetchedFontFamilies = new Set();

export async function ensureWasm() {
  if (wasmMod) {
    return wasmMod;
  }
  if (!wasmInitPromise) {
    wasmInitPromise = (async () => {
      let mod;
      try {
        mod = await import('./pkg/p2d_wasm.js');
        await mod.default('./pkg/p2d_wasm_bg.wasm');
      } catch {
        mod = await import('../pkg/p2d_wasm.js');
        await mod.default('../pkg/p2d_wasm_bg.wasm');
      }
      wasmMod = mod;
      return mod;
    })();
  }
  return wasmInitPromise;
}

// IndexedDB session store so large PDFs (10–100 MB) persist across `drop.html` -> `edit.html` navigation.
function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open('modifier_mes_pdf_wasm', 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains('sessions')) {
        db.createObjectStore('sessions');
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export async function storePdfSession(sessionData) {
  try {
    const db = await openDb();
    await new Promise((resolve, reject) => {
      const tx = db.transaction('sessions', 'readwrite');
      tx.objectStore('sessions').put(sessionData, 'active');
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
    db.close();
  } catch {
    // Fallback if IndexedDB is blocked in private browsing.
  }
}

export async function loadPdfSession() {
  try {
    const db = await openDb();
    const res = await new Promise((resolve, reject) => {
      const tx = db.transaction('sessions', 'readonly');
      const getReq = tx.objectStore('sessions').get('active');
      getReq.onsuccess = () => resolve(getReq.result || null);
      getReq.onerror = () => reject(getReq.error);
    });
    db.close();
    return res;
  } catch {
    return null;
  }
}

export function base64ToUint8Array(b64) {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) {
    out[i] = bin.charCodeAt(i);
  }
  return out;
}

async function resolveSourceBytes(source) {
  if (source.bytes instanceof Uint8Array) {
    return source.bytes;
  }
  if (source.base64) {
    return base64ToUint8Array(source.base64);
  }
  if (source.sample) {
    const urls = [`./samples/${source.sample}`, `../qa/corpus/synthetic/${source.sample}`];
    for (const url of urls) {
      try {
        const resp = await fetch(url);
        if (resp.ok) {
          return new Uint8Array(await resp.arrayBuffer());
        }
      } catch {
        // try next
      }
    }
    throw new Error('Exemple introuvable sur ce serveur.');
  }
  throw new Error('Aucun fichier PDF sélectionné.');
}

// Maps standard/commercial PDF font families to their open-source metric/visual equivalents on Fontsource CDN.
function fontsourceSlugFor(family) {
  const base = family
    .replace(/\s+(ExtraLight|UltraLight|Light|Thin|Medium|SemiBold|ExtraBold|Black)$/i, '')
    .trim();
  const lower = base.toLowerCase();
  const map = {
    helvetica: 'arimo',
    'helvetica neue': 'arimo',
    arial: 'arimo',
    times: 'tinos',
    'times new roman': 'tinos',
    courier: 'cousine',
    'courier new': 'cousine',
    calibri: 'carlito',
    cambria: 'caladea',
    consolas: 'inconsolata',
    tahoma: 'pt-sans',
    verdana: 'arimo',
    georgia: 'tinos',
    perpetua: 'eb-garamond',
    garamond: 'eb-garamond',
    'trade gothic': 'libre-franklin',
    'franklin gothic': 'libre-franklin',
    gotham: 'montserrat',
    'proxima nova': 'montserrat',
    'segoe ui': 'open-sans',
    myriad: 'open-sans',
    'myriad pro': 'open-sans'
  };
  return map[lower] || lower.replace(/\s+/g, '-');
}

// Fetches missing PDF font families from Fontsource/Google Fonts CDN (`Access-Control-Allow-Origin: *`)
// and registers their `.ttf` bytes in WASM memory + `document.fonts`.
export async function ensurePdfFontsInWasm(pdfBytes, password) {
  const wasm = await ensureWasm();
  let families = [];
  try {
    families = JSON.parse(wasm.wasm_missing_font_families(pdfBytes, password || undefined));
  } catch {
    families = [];
  }
  const tasks = [];
  for (const family of families) {
    if (!family || fetchedFontFamilies.has(family)) {
      continue;
    }
    fetchedFontFamilies.add(family);
    const slug = fontsourceSlugFor(family);
    const subWeight = /\bLight$/i.test(family)
      ? 300
      : /\bMedium$/i.test(family)
        ? 500
        : /\bSemiBold$/i.test(family)
          ? 600
          : /\bExtraBold$/i.test(family)
            ? 800
            : /\bBlack$/i.test(family)
              ? 900
              : null;
    const variants = subWeight
      ? [
          { weight: subWeight, style: 'normal', bold: false, italic: false },
          { weight: subWeight, style: 'italic', bold: false, italic: true }
        ]
      : [
          { weight: 400, style: 'normal', bold: false, italic: false },
          { weight: 700, style: 'normal', bold: true, italic: false },
          { weight: 400, style: 'italic', bold: false, italic: true },
          { weight: 700, style: 'italic', bold: true, italic: true }
        ];
    for (const v of variants) {
      const url = `https://cdn.jsdelivr.net/fontsource/fonts/${slug}@latest/latin-${v.weight}-${v.style}.ttf`;
      tasks.push(
        (async () => {
          try {
            const resp = await fetch(url);
            if (!resp.ok) {
              return;
            }
            const buf = await resp.arrayBuffer();
            const bytes = new Uint8Array(buf);
            if (bytes.length < 256) {
              return;
            }
            wasm.wasm_register_font(family, v.bold, v.italic, bytes);
            if (typeof FontFace !== 'undefined' && typeof document !== 'undefined') {
              const blobUrl = URL.createObjectURL(new Blob([bytes], { type: 'font/ttf' }));
              const face = new FontFace(family, `url("${blobUrl}")`, {
                weight: v.bold ? '700' : '400',
                style: v.italic ? 'italic' : 'normal'
              });
              document.fonts.add(await face.load());
            }
          } catch {
            // Offline or unavailable font: WASM falls back to PDF glyph widths.
          }
        })()
      );
    }
  }
  await Promise.allSettled(tasks);
}

async function saveBytesInBrowser(bytes, filename, mimeType, place) {
  if (lastSavedBlobUrl) {
    URL.revokeObjectURL(lastSavedBlobUrl);
  }
  const blob = new Blob([bytes], { type: mimeType });
  lastSavedBlobUrl = URL.createObjectURL(blob);
  lastSavedFilename = filename;

  if (place === 'chosen' && chosenDirHandle) {
    try {
      const fileHandle = await chosenDirHandle.getFileHandle(filename, { create: true });
      const writable = await fileHandle.createWritable();
      await writable.write(blob);
      await writable.close();
      return {
        saved_path: `wasm:${filename}`,
        place: { parts: [chosenDirName || 'Dossier choisi'] }
      };
    } catch {
      // Fall back to standard browser download if permission was revoked.
    }
  }

  triggerBrowserDownload(lastSavedBlobUrl, filename);
  return {
    saved_path: `wasm:${filename}`,
    place: { parts: ['Téléchargements'] }
  };
}

function triggerBrowserDownload(blobUrl, filename) {
  const a = document.createElement('a');
  a.href = blobUrl;
  a.download = filename;
  a.style.display = 'none';
  document.body.appendChild(a);
  a.click();
  a.remove();
}

export function redownloadLastSaved() {
  if (lastSavedBlobUrl && lastSavedFilename) {
    triggerBrowserDownload(lastSavedBlobUrl, lastSavedFilename);
  }
}

export async function pickBrowserFolder() {
  if (typeof window.showDirectoryPicker === 'function') {
    try {
      const handle = await window.showDirectoryPicker({ mode: 'readwrite' });
      chosenDirHandle = handle;
      chosenDirName = handle.name || 'Dossier choisi';
      return { ok: true, chosen: { parts: [chosenDirName] } };
    } catch {
      return { ok: false };
    }
  }
  return { ok: false };
}

export function getBrowserSavePlaces() {
  return {
    chosen: chosenDirName ? { parts: [chosenDirName] } : null
  };
}

export async function convertPdfWasm({ source, format, profile, preset, password, place }) {
  const wasm = await ensureWasm();
  const pdfBytes = await resolveSourceBytes(source);
  const inspect = JSON.parse(wasm.wasm_inspect_pdf(pdfBytes, password || undefined));
  if (inspect.needs_password) {
    return {
      ok: false,
      needs_password: true,
      bad_password: Boolean(inspect.bad_password || password)
    };
  }
  await ensurePdfFontsInWasm(pdfBytes, password);

  // In online WASM mode, both `pages` and `docx` produce the calibrated 1:1 `.docx` that opens
  // in both Apple Pages and Microsoft Word.
  const outBytes = wasm.wasm_convert_pdf(
    pdfBytes,
    password || undefined,
    format,
    profile,
    preset || 'exact'
  );
  const ext = format === 'pages' && preset !== 'exact' ? 'pages' : 'docx';
  const mimeType =
    ext === 'pages'
      ? 'application/vnd.apple.pages'
      : 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';
  const stem = (source.name || 'document')
    .replace(/\.pdf$/i, '')
    .replace(/[/:*?"<>|\\]/g, ' ')
    .trim();
  const outFilename = `${stem}.${ext}`;
  const savedInfo = await saveBytesInBrowser(outBytes, outFilename, mimeType, place);

  return {
    ok: true,
    filename: outFilename,
    saved_path: savedInfo.saved_path,
    place: savedInfo.place,
    opens_in: format === 'pages' ? 'pages' : 'word',
    needsUserGesture: true
  };
}

export async function openEditSessionWasm({ source, password }) {
  const wasm = await ensureWasm();
  const pdfBytes = await resolveSourceBytes(source);
  const viewJson = JSON.parse(wasm.wasm_edit_open(source.name || 'document.pdf', pdfBytes, password || undefined));
  if (!viewJson.ok) {
    return viewJson;
  }
  await storePdfSession({
    name: source.name || 'document.pdf',
    bytes: pdfBytes,
    password: password || null
  });
  return { ok: true, session: 'wasm' };
}

async function hydrateWasmView(wasm, view) {
  for (let i = 0; i < (view.fonts || []).length; i += 1) {
    const f = view.fonts[i];
    if (f.url && f.url.startsWith('wasm-font:')) {
      const idx = Number(f.url.slice('wasm-font:'.length));
      const fontBytes = wasm.wasm_edit_font_bytes(idx);
      if (fontBytes && fontBytes.length > 0) {
        f.url = URL.createObjectURL(new Blob([fontBytes], { type: 'font/ttf' }));
      } else {
        f.url = '';
      }
    }
  }
  for (let i = 0; i < (view.pages || []).length; i += 1) {
    if (i > 0) {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    const pngBytes = wasm.wasm_edit_render_page(i, 150.0);
    view.pages[i].imageUrl = URL.createObjectURL(new Blob([pngBytes], { type: 'image/png' }));
  }
  return view;
}

export async function loadEditViewWasm() {
  const wasm = await ensureWasm();
  const stored = await loadPdfSession();
  if (!stored || !stored.bytes) {
    return {
      ok: false,
      error: 'Aucun document en mémoire. Revenez à l’accueil et choisissez un PDF.'
    };
  }
  const pdfBytes =
    stored.bytes instanceof Uint8Array ? stored.bytes : new Uint8Array(stored.bytes);
  await ensurePdfFontsInWasm(pdfBytes, stored.password);
  const view = JSON.parse(
    wasm.wasm_edit_open(stored.name || 'document.pdf', pdfBytes, stored.password || undefined)
  );
  if (!view.ok) {
    return view;
  }
  return hydrateWasmView(wasm, view);
}

export async function renderPreviewPageWasm(pageIndex, editsArray, hasEditsOnPage) {
  const wasm = await ensureWasm();
  const pngBytes = hasEditsOnPage
    ? wasm.wasm_edit_preview_page(pageIndex, JSON.stringify(editsArray), 150.0)
    : wasm.wasm_edit_render_page(pageIndex, 150.0);
  return URL.createObjectURL(new Blob([pngBytes], { type: 'image/png' }));
}

export async function saveEditSessionWasm(editsArray, currentFilename, place) {
  const wasm = await ensureWasm();
  const newPdfBytes = wasm.wasm_edit_save(JSON.stringify(editsArray));
  const stem = (currentFilename || 'document')
    .replace(/\.pdf$/i, '')
    .replace(/\s*\(modifié\)$/i, '')
    .replace(/[/:*?"<>|\\]/g, ' ')
    .trim();
  const savedFilename = `${stem} (modifié).pdf`;
  await storePdfSession({
    name: savedFilename,
    bytes: newPdfBytes,
    password: null
  });
  const savedInfo = await saveBytesInBrowser(
    newPdfBytes,
    savedFilename,
    'application/pdf',
    place
  );
  const view = JSON.parse(wasm.wasm_edit_view_json());
  await hydrateWasmView(wasm, view);
  view.saved_path = savedInfo.saved_path;
  view.saved_filename = savedFilename;
  view.place = savedInfo.place;
  return view;
}
