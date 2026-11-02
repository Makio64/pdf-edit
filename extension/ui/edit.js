// Direct PDF editor: shows each page exactly as rendered from the PDF, with every text line
// clickable. A line being retyped is drawn in the PDF's own embedded font, at its size, letter
// and word spacing, stretch and alignment, so it looks the same before and after the click. Once
// a line is validated, the page image is rendered again from the PDF with the edits applied
// (/api/edit/preview): what the page shows is exactly what /api/edit/save will write. Validated
// lines can be taken back and redone (« Annuler », « Rétablir », ⌘Z, ⇧⌘Z) and shown next to the PDF
// as it was (« Comparer »).

const pagesEl = document.getElementById('pages');
const saveBtn = document.getElementById('saveBtn');
const statusEl = document.getElementById('editStatus');
const docNameEl = document.getElementById('docName');
const savedBox = document.getElementById('savedBox');
const savedSummary = document.getElementById('savedSummary');
const openSavedBtn = document.getElementById('openSavedBtn');
const revealSavedBtn = document.getElementById('revealSavedBtn');
const errorBox = document.getElementById('errorBox');
const errorText = document.getElementById('errorText');
const editView = document.getElementById('editView');
const saveBar = document.getElementById('saveBar');
const continueBtn = document.getElementById('continueBtn');
const savedPlace = document.getElementById('savedPlace');
const undoBtn = document.getElementById('undoBtn');
const redoBtn = document.getElementById('redoBtn');
const compareBtn = document.getElementById('compareBtn');
const revertBtn = document.getElementById('revertBtn');
const compareView = document.getElementById('compareView');
const comparePages = document.getElementById('comparePages');
// Where the home page's step 3 said to save: 'desktop', 'downloads' or 'chosen' (another folder
// once saving there was refused).
let savePlace = new URLSearchParams(location.search).get('place') || 'downloads';

const sessionId = new URLSearchParams(location.search).get('session');
let view = null;
let version = 0;
let savedPath = null;
// "page:line" -> { text, runs: [[style index in line.runs, character count], …], x0, x1 }, where
// x0 and x1 are how far the new text reaches across the page, in points.
const edits = new Map();
// ⌘Z and ⇧⌘Z: each step is the lines it changed, [{ key, before, after }, …], with the edits
// before and after (undefined: the PDF's own text).
const undoSteps = [];
const redoSteps = [];
// "page:line" -> where the line is shown: { lineEl, pageEl, pageIndex, line, page }; and the
// lines in reading order, for Tab.
const lineViews = new Map();
const lineOrder = [];
// The line being typed: whether its text differs from when typing started, and whether anything
// was typed (the browser's own undo may have taken it back).
let typing = null;
// CSS families of the PDF's fonts the browser accepted, and the faces added to the document.
const loadedFonts = new Set();
const fontFaces = [];
// Older browsers ignore `size-adjust`: lines in fonts that need it are then sized for that font.
const sizeAdjustSupported = 'sizeAdjust' in FontFace.prototype;
// Page index -> its <img>, and the pages whose image shows pending edits.
const pageImages = new Map();
const previewPages = new Set();
let previewSeq = 0;
// Covers and texts of finished edits, kept until a page image showing them has arrived.
const pendingOverlays = [];
// The text being retyped, by line.
const editingText = new WeakMap();
// The page images with every edit sent so far, once they have arrived.
let previewDone = Promise.resolve();

function showError(message) {
  document.getElementById('pagesLoading')?.remove();
  errorText.textContent = message;
  errorBox.hidden = false;
}

function cssColor(c) {
  return `rgb(${c.r}, ${c.g}, ${c.b})`;
}

// The fonts for one style of a line: the PDF's own (when the browser accepted it), then the
// installed look-alike the PDF's characters missing from it are also drawn in.
function fontStack(line, runIndex) {
  const run = line.runs[runIndex];
  const generic = run.monospace ? 'monospace' : run.serif ? 'serif' : 'sans-serif';
  const web = line.run_fonts[runIndex];
  const own = web && loadedFonts.has(web) ? `"${web}", ` : '';
  return `${own}"${run.font_family}", ${generic}`;
}

// Load the fonts embedded in the PDF; one the browser refuses is simply left out of the stacks.
async function loadPdfFonts(fonts) {
  fontFaces.splice(0).forEach((face) => document.fonts.delete(face));
  loadedFonts.clear();
  await Promise.all(
    (fonts || []).map(async (f) => {
      const face = new FontFace(f.family, `url("${f.url}")`, {
        weight: f.bold ? '700' : '400',
        style: f.italic ? 'italic' : 'normal',
        sizeAdjust: `${f.size_adjust_pct}%`,
        display: 'block'
      });
      try {
        document.fonts.add(await face.load());
        fontFaces.push(face);
        loadedFonts.add(f.family);
      } catch {
        // Not a font the browser can use: the installed look-alike is shown instead.
      }
    })
  );
}

// The Mac app asks before closing when corrections are not saved: it is told whenever that changes.
let reportedUnsaved = false;
function reportUnsaved(unsaved) {
  if (unsaved !== reportedUnsaved && window.ipc) {
    reportedUnsaved = unsaved;
    window.ipc.postMessage(JSON.stringify({ unsaved }));
  }
}

function updateStatus() {
  const n = edits.size;
  const comparing = !compareView.hidden;
  reportUnsaved(n > 0);
  saveBtn.disabled = n === 0;
  // A line being typed counts as the last correction: « Annuler » takes it back.
  undoBtn.disabled = comparing || (undoSteps.length === 0 && !typing?.changed());
  redoBtn.disabled = comparing || redoSteps.length === 0;
  compareBtn.disabled = n === 0 && !comparing;
  revertBtn.hidden = n === 0 || comparing;
  statusEl.textContent =
    n === 0
      ? 'Aucune modification pour l’instant.'
      : `${n} ligne${n > 1 ? 's' : ''} modifiée${n > 1 ? 's' : ''} — pensez à enregistrer.`;
}

// `x`, `y`: where the person clicked; null: at the end of the text.
function placeCaret(el, x, y) {
  const range = x !== null && document.caretRangeFromPoint ? document.caretRangeFromPoint(x, y) : null;
  const sel = window.getSelection();
  if (range && el.contains(range.startContainer)) {
    sel.removeAllRanges();
    sel.addRange(range);
  } else {
    const end = document.createRange();
    end.selectNodeContents(el);
    end.collapse(false);
    sel.removeAllRanges();
    sel.addRange(end);
  }
}

// The line's text as it stands, in styled pieces: [[style index, text], …].
function linePieces(pageIndex, line) {
  const edit = edits.get(`${pageIndex}:${line.id}`);
  const chars = [...(edit ? edit.text : line.text)];
  const runs = edit ? edit.runs : line.runs.map((r, i) => [i, r.len]);
  let at = 0;
  return runs.map(([run, len]) => {
    const text = chars.slice(at, at + len).join('');
    at += len;
    return [run, text];
  });
}

function lineText(pageIndex, line) {
  return linePieces(pageIndex, line)
    .map(([, text]) => text)
    .join('');
}

// Page points -> container units of the page element (the overlay scales with the page).
function cq(pt, page) {
  return `${(pt / page.width_pt) * 100}cqw`;
}

// The band to paint over while a line is retyped: its glyphs with accents and descenders, but not
// past the lines just above and below.
function coverBand(line, page) {
  let top = line.baseline - 0.95 * line.font_size_pt;
  let bottom = line.baseline + 0.28 * line.font_size_pt;
  for (const other of page.lines) {
    const overlaps = other.bbox.x0 < line.bbox.x1 && other.bbox.x1 > line.bbox.x0;
    if (other === line || !overlaps) {
      continue;
    }
    if (other.baseline < line.baseline) {
      top = Math.max(top, Math.min(other.baseline + 0.25 * other.font_size_pt, line.bbox.y0));
    } else if (other.baseline > line.baseline) {
      bottom = Math.min(bottom, Math.max(other.baseline - 0.8 * other.font_size_pt, line.bbox.y1));
    }
  }
  return { top, bottom };
}

// The retyped text, typeset like the PDF line: each style in its own font, size and colour,
// with the line's letter and word spacing, stretch and anchored edge.
function makeTextEl(pageIndex, line, page) {
  const h = line.horiz_scale > 0.05 ? line.horiz_scale : 1;
  const textEl = document.createElement('span');
  textEl.className = 'edit-text';
  textEl.style.fontSize = cq(line.font_size_pt, page);
  // Text typed outside every style's span (into a line cleared earlier) reads as the first style.
  if (line.runs.length > 0) {
    setRunStyle(textEl, line, 0, page);
  }
  for (const [runIndex, text] of linePieces(pageIndex, line)) {
    const span = document.createElement('span');
    span.dataset.run = runIndex;
    span.textContent = text;
    setRunStyle(span, line, runIndex, page);
    textEl.appendChild(span);
  }
  // CSS spacing is stretched by the transform below, PDF spacing is not: divide it out.
  textEl.style.letterSpacing = cq(line.char_spacing_pt / h, page);
  textEl.style.wordSpacing = cq(line.word_spacing_pt / h, page);
  // CSS also adds the letter spacing after the last character, which the PDF line's edge leaves
  // out: right-aligned and centred text moves over by it.
  const b = line.bbox;
  const trailing = line.char_spacing_pt;
  if (line.align === 'Right') {
    textEl.style.right = cq(page.width_pt - b.x1 - trailing, page);
    textEl.style.transformOrigin = 'right';
    textEl.style.transform = `scaleX(${h})`;
  } else if (line.align === 'Center') {
    textEl.style.left = cq(0.5 * (b.x0 + b.x1 + trailing), page);
    textEl.style.transformOrigin = 'left';
    textEl.style.transform = `scaleX(${h}) translateX(-50%)`;
  } else {
    textEl.style.left = cq(b.x0, page);
    textEl.style.transformOrigin = 'left';
    textEl.style.transform = `scaleX(${h})`;
  }
  return textEl;
}

// Font, size, weight, slant and colour of one of the line's styles.
function setRunStyle(el, line, runIndex, page) {
  const run = line.runs[runIndex];
  el.style.fontFamily = fontStack(line, runIndex);
  const web = line.run_fonts[runIndex];
  const ownFont = web && loadedFonts.has(web);
  const size = ownFont && !sizeAdjustSupported ? run.nominal_font_size_pt : run.font_size_pt;
  el.style.fontSize = cq(size, page);
  el.style.fontWeight = run.bold ? '700' : '400';
  el.style.fontStyle = run.italic ? 'italic' : 'normal';
  el.style.color = cssColor(run.color);
}

// What was typed, in styled pieces: text typed inside a style's span keeps that style, text
// outside any (after clearing the line) takes the style before it.
function readPieces(textEl) {
  const pieces = [];
  const walker = document.createTreeWalker(textEl, NodeFilter.SHOW_TEXT);
  let run = 0;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const owner = node.parentElement.closest('[data-run]');
    if (owner && textEl.contains(owner)) {
      run = Number(owner.dataset.run);
    }
    const text = node.textContent.replace(/\n/g, ' ');
    const last = pieces[pieces.length - 1];
    if (last && last[0] === run) {
      last[1] += text;
    } else if (text) {
      pieces.push([run, text]);
    }
  }
  return pieces;
}

// Put the text's baseline exactly on the PDF line's baseline (font metrics vary).
function alignBaseline(textEl, line, page, pageEl) {
  textEl.style.top = '0px';
  const probe = document.createElement('span');
  probe.style.cssText = 'display:inline-block;width:0;height:0;vertical-align:baseline';
  textEl.appendChild(probe);
  const baselinePx = probe.getBoundingClientRect().bottom - textEl.getBoundingClientRect().top;
  probe.remove();
  const scale = pageEl.clientWidth / page.width_pt;
  textEl.style.top = `${((line.baseline * scale - baselinePx) / pageEl.clientWidth) * 100}cqw`;
}

// Horizontal extent of `textEl` on the page, in points.
function textExtent(textEl, page, pageEl) {
  const scale = pageEl.clientWidth / page.width_pt;
  const left = pageEl.getBoundingClientRect().left + pageEl.clientLeft;
  const box = textEl.getBoundingClientRect();
  return { x0: (box.left - left) / scale, x1: (box.right - left) / scale };
}

// Paint the line's background over its original glyphs, over the text the page image shows for
// it (`shown`: an earlier edit), and wherever the new text reaches.
function fitCover(coverEl, textEl, line, page, pageEl, shown) {
  const band = coverBand(line, page);
  const text = textExtent(textEl, page, pageEl);
  const x0 = Math.min(line.bbox.x0 - 0.5, shown.x0 - 0.5, text.x0);
  const x1 = Math.max(line.bbox.x1 + 0.5, shown.x1 + 0.5, text.x1);
  coverEl.style.left = cq(x0, page);
  coverEl.style.width = cq(x1 - x0, page);
  coverEl.style.top = cq(band.top, page);
  coverEl.style.height = cq(band.bottom - band.top, page);
}

// The clickable zone of a line: the PDF's line, and as far as its new text reaches.
function placeLineEl(lineEl, line, page, edit) {
  const b = line.bbox;
  const x0 = Math.min(b.x0, edit?.x0 ?? b.x0);
  const x1 = Math.max(b.x1, edit?.x1 ?? b.x1);
  lineEl.style.left = `${(x0 / page.width_pt) * 100}%`;
  lineEl.style.top = `${(b.y0 / page.height_pt) * 100}%`;
  lineEl.style.width = `${((x1 - x0) / page.width_pt) * 100}%`;
  lineEl.style.height = `${((b.y1 - b.y0) / page.height_pt) * 100}%`;
}

// A line's zone, outline and tooltip for its current text.
function showLineState(view) {
  const edit = edits.get(`${view.pageIndex}:${view.line.id}`);
  view.lineEl.classList.toggle('is-edited', Boolean(edit));
  view.lineEl.title = edit ? `Avant : « ${view.line.text} ». Cliquer pour modifier.` : 'Cliquer pour modifier cette ligne';
  placeLineEl(view.lineEl, view.line, view.page, edit);
}

// Draw the line's current text over the page image until an image showing it arrives. `shownEdit`:
// the edit that image shows for the line (undefined: the PDF's own text).
function showPendingLine(view, shownEdit) {
  const { pageEl, pageIndex, line, page } = view;
  const coverEl = document.createElement('div');
  coverEl.className = 'edit-cover';
  coverEl.style.background = cssColor(line.background);
  const textEl = makeTextEl(pageIndex, line, page);
  textEl.style.pointerEvents = 'none';
  pageEl.append(coverEl, textEl);
  alignBaseline(textEl, line, page, pageEl);
  fitCover(coverEl, textEl, line, page, pageEl, shownEdit || line.bbox);
  pendingOverlays.push(coverEl, textEl);
}

function startEditing(lineEl, pageEl, pageIndex, line, page, clickX, clickY) {
  if (lineEl.classList.contains('is-editing') || !compareView.hidden) {
    return;
  }
  lineEl.classList.add('is-editing');
  const coverEl = document.createElement('div');
  coverEl.className = 'edit-cover';
  coverEl.style.background = cssColor(line.background);
  const textEl = makeTextEl(pageIndex, line, page);
  pageEl.append(coverEl, textEl);
  alignBaseline(textEl, line, page, pageEl);
  // Until it is retyped, the text is what the page image shows for the line.
  const shown = textExtent(textEl, page, pageEl);
  fitCover(coverEl, textEl, line, page, pageEl, shown);
  editingText.set(lineEl, textEl);

  textEl.contentEditable = 'plaintext-only';
  textEl.spellcheck = true;
  textEl.focus({ preventScroll: true });
  placeCaret(textEl, clickX, clickY);
  // Reached with Tab: brought into view, clear of the save bar.
  const box = textEl.getBoundingClientRect();
  if (box.top < 80 || box.bottom > window.innerHeight - saveBar.offsetHeight - 16) {
    textEl.scrollIntoView({ block: 'center' });
  }

  const key = `${pageIndex}:${line.id}`;
  const startPieces = JSON.stringify(readPieces(textEl));
  typing = { changed: () => JSON.stringify(readPieces(textEl)) !== startPieces, typed: false };
  const onInput = () => {
    fitCover(coverEl, textEl, line, page, pageEl, shown);
    typing.typed = true;
    // A line being typed counts as a correction not saved.
    reportUnsaved(true);
    updateStatus();
  };
  const finish = (keep) => {
    textEl.removeEventListener('keydown', onKey);
    textEl.removeEventListener('blur', onBlur);
    textEl.removeEventListener('input', onInput);
    textEl.contentEditable = 'false';
    lineEl.classList.remove('is-editing');
    editingText.delete(lineEl);
    typing = null;
    const before = edits.get(key);
    const beforePieces = JSON.stringify(linePieces(pageIndex, line));
    if (keep) {
      const pieces = readPieces(textEl);
      const text = pieces.map(([, t]) => t).join('');
      if (text === line.text) {
        edits.delete(key);
      } else {
        const { x0, x1 } = textExtent(textEl, page, pageEl);
        edits.set(key, { text, runs: pieces.map(([run, t]) => [run, [...t].length]), x0, x1 });
      }
    }
    showLineState(lineViews.get(key));
    if (JSON.stringify(linePieces(pageIndex, line)) === beforePieces) {
      // Same text: the edit stays as it was (and so does how far it reaches).
      if (before) {
        edits.set(key, before);
        showLineState(lineViews.get(key));
      }
      updateStatus();
      coverEl.remove();
      textEl.remove();
      return;
    }
    recordStep([{ key, before, after: edits.get(key) }]);
    // Keep showing the typed text until a page image with the edit has arrived; clicks go to the
    // line below it.
    textEl.style.pointerEvents = 'none';
    pendingOverlays.push(coverEl, textEl);
    refreshPreview([pageIndex]);
  };
  const onKey = (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      finish(true);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      finish(false);
    } else if (event.key === 'Tab') {
      // The next line (Maj: the one before), as in a form.
      event.preventDefault();
      finish(true);
      editNeighbour(key, event.shiftKey ? -1 : 1);
    }
  };
  const onBlur = () => finish(true);
  textEl.addEventListener('keydown', onKey);
  textEl.addEventListener('blur', onBlur);
  textEl.addEventListener('input', onInput);
  updateStatus();
}

// Start typing in the line `step` lines after the one of `key`, with the caret at its end.
function editNeighbour(key, step) {
  const next = lineViews.get(lineOrder[lineOrder.indexOf(key) + step]);
  if (next) {
    startEditing(next.lineEl, next.pageEl, next.pageIndex, next.line, next.page, null, null);
  }
}

function recordStep(changes) {
  undoSteps.push(changes);
  redoSteps.length = 0;
  updateStatus();
}

// Put the lines of `changes` back as they were (`undo`), or as they became again.
function applyStep(changes, undo) {
  const pages = new Set();
  for (const { key, before, after } of changes) {
    const shown = edits.get(key);
    const target = undo ? before : after;
    if (target) {
      edits.set(key, target);
    } else {
      edits.delete(key);
    }
    const view = lineViews.get(key);
    if (view) {
      showLineState(view);
      showPendingLine(view, shown);
      pages.add(view.pageIndex);
    }
  }
  updateStatus();
  refreshPreview(pages);
}

function undo() {
  commitTyping();
  const step = undoSteps.pop();
  if (step) {
    redoSteps.push(step);
    applyStep(step, true);
  }
}

function redo() {
  commitTyping();
  const step = redoSteps.pop();
  if (step) {
    undoSteps.push(step);
    applyStep(step, false);
  }
}

// « Tout annuler »: every line back to the PDF's own text, in one step that « Annuler » brings back.
function revertAll() {
  commitTyping();
  if (edits.size === 0) {
    return;
  }
  const changes = [...edits].map(([key, before]) => ({ key, before, after: undefined }));
  undoSteps.push(changes);
  redoSteps.length = 0;
  applyStep(changes, false);
}

function editsPayload() {
  return [...edits].map(([key, edit]) => {
    const [page, line] = key.split(':').map(Number);
    return { page_index: page, line_id: line, text: edit.text, runs: edit.runs };
  });
}

function pageImageUrl(pageIndex, preview) {
  const p = preview ? `&preview=1&p=${previewSeq}` : '';
  return `/api/edit/page.png?session=${encodeURIComponent(view.session)}&page=${pageIndex}&v=${version}${p}`;
}

function dropOverlays(overlays) {
  for (const el of overlays) {
    el.remove();
    const i = pendingOverlays.indexOf(el);
    if (i >= 0) {
      pendingOverlays.splice(i, 1);
    }
  }
}

let wasmClient = null;
async function getWasmClient() {
  if (!wasmClient) {
    wasmClient = await import('./wasm-client.js');
  }
  return wasmClient;
}

function refreshPreview(changedPages) {
  previewDone = showPreview(changedPages);
  return previewDone;
}

// Render the PDF again with the pending edits, and show the pages they touch. A newer call
// includes this one's edits: only the latest swaps the images and takes the overlays off. When it
// fails, the overlays stay: they show the text that will be saved.
async function showPreview(changedPages) {
  const seq = ++previewSeq;
  const overlays = [...pendingOverlays];
  const payloadEdits = editsPayload();
  const edited = new Set(payloadEdits.map((e) => e.page_index));
  const pages = new Set([...previewPages, ...edited, ...changedPages]);

  if (view.session === 'wasm') {
    try {
      const wasm = await getWasmClient();
      const images = await Promise.all(
        [...pages].map(async (pageIndex) => {
          const img = pageImages.get(pageIndex);
          if (!img) {
            return null;
          }
          const url = await wasm.renderPreviewPageWasm(pageIndex, payloadEdits, edited.has(pageIndex));
          const next = new Image();
          next.src = url;
          try {
            await next.decode();
          } catch {
            return null;
          }
          return [img, next.src];
        })
      );
      if (seq !== previewSeq) {
        return;
      }
      for (const [img, src] of images.filter(Boolean)) {
        img.src = src;
      }
      previewPages.clear();
      edited.forEach((p) => previewPages.add(p));
      dropOverlays(overlays);
    } catch (err) {
      showError(String(err?.message || err));
    }
    return;
  }

  let data;
  try {
    const resp = await fetch('/api/edit/preview', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ session: view.session, edits: payloadEdits })
    });
    data = await resp.json();
  } catch (err) {
    data = { ok: false, error: String(err?.message || err) };
  }
  if (seq !== previewSeq) {
    return;
  }
  if (!data.ok) {
    showError(data.error || 'Impossible d’afficher la modification.');
    return;
  }
  const images = await Promise.all(
    [...pages].map(async (pageIndex) => {
      const img = pageImages.get(pageIndex);
      if (!img) {
        return null;
      }
      const next = new Image();
      next.src = pageImageUrl(pageIndex, edited.has(pageIndex));
      try {
        await next.decode();
      } catch {
        return null;
      }
      return [img, next.src];
    })
  );
  if (seq !== previewSeq) {
    return;
  }
  for (const [img, src] of images.filter(Boolean)) {
    img.src = src;
  }
  previewPages.clear();
  edited.forEach((p) => previewPages.add(p));
  // The zones of the edited lines reach as far as the PDF now draws their text (characters the
  // PDF's font lacks come from another font, wider or narrower than the one typed in).
  const sent = new Map(payloadEdits.map((e) => [`${e.page_index}:${e.line_id}`, e.text]));
  for (const { page_index: page, line_id: line, x0, x1 } of data.reach || []) {
    const key = `${page}:${line}`;
    const edit = edits.get(key);
    if (edit && edit.text === sent.get(key) && lineViews.has(key)) {
      Object.assign(edit, { x0, x1 });
      showLineState(lineViews.get(key));
    }
  }
  dropOverlays(overlays);
}

// The page as the PDF has it (the last saved version, after saving).
function originalImageUrl(pageIndex) {
  return view.pages[pageIndex].imageUrl || pageImageUrl(pageIndex, false);
}

function render() {
  pagesEl.textContent = '';
  pageImages.clear();
  lineViews.clear();
  lineOrder.length = 0;
  pendingOverlays.length = 0;
  docNameEl.textContent = view.filename;
  view.pages.forEach((page, pageIndex) => {
    const pageEl = document.createElement('div');
    pageEl.className = 'edit-page';
    pageEl.style.aspectRatio = `${page.width_pt} / ${page.height_pt}`;

    const label = document.createElement('span');
    label.className = 'edit-page-label';
    label.textContent = `Page ${pageIndex + 1} / ${view.pages.length}`;
    pageEl.appendChild(label);

    const img = document.createElement('img');
    img.alt = `Page ${pageIndex + 1}`;
    // A long document: pages render as they come into view, not all at once.
    img.loading = 'lazy';
    img.src = originalImageUrl(pageIndex);
    pageImages.set(pageIndex, img);
    pageEl.appendChild(img);

    for (const line of page.lines) {
      const lineEl = document.createElement('div');
      lineEl.className = 'edit-line';
      const key = `${pageIndex}:${line.id}`;
      lineViews.set(key, { lineEl, pageEl, pageIndex, line, page });
      lineOrder.push(key);
      showLineState(lineViews.get(key));
      lineEl.addEventListener('mousedown', (event) => {
        event.preventDefault();
        const textEl = editingText.get(lineEl);
        if (textEl) {
          // A click in the line beside the text being typed: the caret goes to the nearest point
          // of the text.
          textEl.focus();
          const r = textEl.getBoundingClientRect();
          const x = Math.min(Math.max(event.clientX, r.left + 1), r.right - 1);
          const y = Math.min(Math.max(event.clientY, r.top + 1), r.bottom - 1);
          placeCaret(textEl, x, y);
        } else {
          startEditing(lineEl, pageEl, pageIndex, line, page, event.clientX, event.clientY);
        }
      });
      pageEl.appendChild(lineEl);
    }
    pagesEl.appendChild(pageEl);
  });
  updateStatus();
}

async function load() {
  if (!sessionId) {
    showError('Aucun document ouvert. Revenez à l’accueil et choisissez un PDF.');
    return;
  }
  let data;
  if (sessionId === 'wasm') {
    revealSavedBtn.hidden = true;
    const wasm = await getWasmClient();
    data = await wasm.loadEditViewWasm();
  } else {
    const resp = await fetch(`/api/edit/view?session=${encodeURIComponent(sessionId)}`);
    data = await resp.json();
  }
  if (!data.ok) {
    pagesEl.textContent = '';
    showError(data.error || 'Impossible d’ouvrir ce document.');
    return;
  }
  view = data;
  await loadPdfFonts(view.fonts);
  render();
}

saveBtn.addEventListener('click', async () => {
  saveBtn.disabled = true;
  statusEl.textContent = 'Enregistrement…';
  const payload = { session: view.session, edits: editsPayload(), place: savePlace };
  try {
    let data;
    if (view.session === 'wasm') {
      const wasm = await getWasmClient();
      data = await wasm.saveEditSessionWasm(payload.edits, view.filename, savePlace);
    } else {
      const resp = await fetch('/api/edit/save', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      });
      data = await resp.json();
    }
    if (!data.ok) {
      showError(data.error || 'Impossible d’enregistrer le PDF.');
      otherFolderBox.hidden = !data.save_failed;
      updateStatus();
      return;
    }
    otherFolderBox.hidden = true;
    showCompare(false);
    savedPath = data.saved_path;
    savedSummary.textContent = data.saved_filename;
    if (data.place?.parts?.length) {
      showPlaceLabel(savedPlace, data.place.parts);
    }
    errorBox.hidden = true;
    edits.clear();
    // The saved PDF is the one corrected from now on: its lines are read again.
    undoSteps.length = 0;
    redoSteps.length = 0;
    previewPages.clear();
    version += 1;
    view = data;
    await loadPdfFonts(view.fonts);
    render();
    showSaved(true);
  } catch (err) {
    showError(String(err?.message || err));
    updateStatus();
  }
});

async function action(kind) {
  if (!savedPath) {
    return;
  }
  if (view?.session === 'wasm') {
    const wasm = await getWasmClient();
    wasm.redownloadLastSaved();
    return;
  }
  await fetch('/api/action', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ saved_path: savedPath, action: kind })
  });
}

// « Comparer »: each corrected page as the PDF has it and with the corrections, side by side, with
// the changed lines marked and what each of them said before.
async function openCompare() {
  commitTyping();
  if (edits.size === 0) {
    return;
  }
  compareBtn.disabled = true;
  await previewDone.catch(() => {});
  const byPage = new Map();
  for (const [key, edit] of edits) {
    const v = lineViews.get(key);
    if (v) {
      byPage.set(v.pageIndex, [...(byPage.get(v.pageIndex) || []), [v.line, edit]]);
    }
  }
  comparePages.textContent = '';
  for (const pageIndex of [...byPage.keys()].sort((a, b) => a - b)) {
    const changes = byPage.get(pageIndex).sort(([a], [b]) => a.id - b.id);
    const page = view.pages[pageIndex];
    const group = document.createElement('section');
    group.className = 'compare-group';
    const title = document.createElement('h2');
    title.className = 'compare-title';
    title.textContent = `Page ${pageIndex + 1}`;
    const list = document.createElement('ul');
    list.className = 'compare-list';
    for (const [line, edit] of changes) {
      const li = document.createElement('li');
      const was = document.createElement('span');
      was.className = 'compare-was';
      was.textContent = `« ${line.text.trim()} »`;
      const now = document.createElement('span');
      now.className = 'compare-now';
      now.textContent = edit.text.trim() ? `« ${edit.text.trim()} »` : 'rien (ligne effacée)';
      li.append(was, ' devient ', now);
      list.append(li);
    }
    const sides = document.createElement('div');
    sides.className = 'compare-sides';
    sides.append(
      compareSide('Avant', originalImageUrl(pageIndex), pageIndex, page, changes.map(([line]) => [line, undefined])),
      compareSide('Après', pageImages.get(pageIndex)?.src || originalImageUrl(pageIndex), pageIndex, page, changes)
    );
    group.append(title, list, sides);
    comparePages.append(group);
  }
  showCompare(true);
}

// One side of a page in « Comparer »: its image, the changed lines marked on it.
function compareSide(caption, src, pageIndex, page, marks) {
  const figure = document.createElement('figure');
  figure.className = 'compare-side';
  const label = document.createElement('figcaption');
  label.textContent = caption;
  const sheet = document.createElement('div');
  sheet.className = 'compare-sheet';
  sheet.style.aspectRatio = `${page.width_pt} / ${page.height_pt}`;
  const img = document.createElement('img');
  img.src = src;
  img.alt = `Page ${pageIndex + 1}, ${caption.toLowerCase()}`;
  sheet.append(img);
  for (const [line, edit] of marks) {
    const mark = document.createElement('div');
    mark.className = 'compare-mark';
    placeLineEl(mark, line, page, edit);
    sheet.append(mark);
  }
  figure.append(label, sheet);
  return figure;
}

function showCompare(open) {
  compareView.hidden = !open;
  pagesEl.hidden = open;
  editView.classList.toggle('is-comparing', open);
  document.getElementById('compareLabel').textContent = open ? 'Fermer la comparaison' : 'Comparer';
  compareBtn.setAttribute('aria-pressed', String(open));
  updateStatus();
  if (open) {
    window.scrollTo(0, 0);
  }
}

undoBtn.addEventListener('click', undo);
redoBtn.addEventListener('click', redo);
revertBtn.addEventListener('click', revertAll);
compareBtn.addEventListener('click', () => (compareView.hidden ? openCompare() : showCompare(false)));

// ⌘Z, ⇧⌘Z (or ⌘Y) and ⌘S; Échap closes « Comparer ». While a line is being typed, ⌘Z first takes
// back the typing (the browser's own undo); once nothing typed is left, the line before.
document.addEventListener('keydown', (event) => {
  if (leaveDialog.open || editView.hidden) {
    return;
  }
  if (event.key === 'Escape' && !compareView.hidden) {
    event.preventDefault();
    showCompare(false);
    return;
  }
  if (!(event.metaKey || event.ctrlKey) || event.altKey) {
    return;
  }
  const key = event.key.toLowerCase();
  if (key === 's') {
    event.preventDefault();
    commitTyping();
    if (!saveBtn.disabled) {
      saveBtn.click();
    }
  } else if (key === 'z' || key === 'y') {
    const again = key === 'y' || event.shiftKey;
    if (typing && (again ? typing.typed : typing.changed())) {
      return;
    }
    event.preventDefault();
    if (compareView.hidden) {
      (again ? redo : undo)();
    }
  }
});

openSavedBtn.addEventListener('click', () => action('open'));
revealSavedBtn.addEventListener('click', () => action('finder'));

// After saving: « C'est enregistré » and where the file is; « Corriger encore » goes back to the
// pages, which now show the saved PDF.
function showSaved(saved) {
  savedBox.hidden = !saved;
  editView.hidden = saved;
  saveBar.hidden = saved;
  window.scrollTo(0, 0);
}

continueBtn.addEventListener('click', () => showSaved(false));

// « Sur le Bureau », « Dans Téléchargements », « Dans Documents › Factures »… as on the home page.
function showPlaceLabel(el, parts) {
  el.textContent = parts.length === 1 && parts[0] === 'Bureau' ? 'Sur le ' : 'Dans ';
  parts.forEach((part, i) => {
    if (i > 0) {
      el.append(' › ');
    }
    const name = document.createElement('b');
    name.textContent = part;
    el.append(name);
  });
}

// The save button says where the PDF goes.
async function labelSaveButton() {
  if (savePlace === 'desktop') {
    saveBtn.textContent = 'Enregistrer sur le Bureau';
  } else if (savePlace === 'chosen') {
    try {
      const parts = (await (await fetch('/api/save-places')).json()).chosen?.parts || [];
      if (parts.length > 0) {
        saveBtn.textContent = `Enregistrer dans « ${parts.at(-1)} »`;
      }
    } catch {
      // Keeps « Enregistrer le PDF ».
    }
  } else {
    saveBtn.textContent = 'Enregistrer dans Téléchargements';
  }
}

labelSaveButton();

window.addEventListener('beforeunload', (event) => {
  if (edits.size > 0) {
    event.preventDefault();
  }
});

// Leaving with corrections not saved (« ← Retour », « Modifier un autre PDF », a PDF sent by the
// Finder, closing the Mac app): the page asks first, since the Mac app's window has no « Quitter
// cette page ? » prompt.
const leaveDialog = document.getElementById('leaveDialog');
let leaveTo = null;
// Leaving because the Finder sent another PDF: staying drops that PDF.
let leavingForOpenedPdf = false;

// A line still being typed (Entrée not pressed yet) counts too: it is validated first.
function commitTyping() {
  const el = document.activeElement;
  if (el && el.classList.contains('edit-text')) {
    el.blur();
  }
}

// `url`: the page to go to; null: quit the Mac app.
function go(url) {
  if (url === null) {
    window.ipc.postMessage(JSON.stringify({ quit: true }));
  } else {
    location.href = url;
  }
}

function leave(url) {
  commitTyping();
  if (edits.size === 0) {
    go(url);
    return;
  }
  leaveTo = url;
  if (!leaveDialog.open) {
    leaveDialog.showModal();
  }
  document.getElementById('stayBtn').focus();
}

document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href]');
  if (link && edits.size > 0) {
    event.preventDefault();
    leave(link.href);
  }
});

document.getElementById('stayBtn').addEventListener('click', () => leaveDialog.close());
document.getElementById('leaveBtn').addEventListener('click', () => {
  edits.clear();
  reportUnsaved(false);
  go(leaveTo);
});

// Closing the Mac app's window (or quitting) with corrections not saved (app_window.rs).
window.modifierMesPdfConfirmQuit = () => leave(null);

// Saving where the home page said was refused (the Mac's privacy settings): another folder, then
// save there.
const otherFolderBox = document.getElementById('otherFolderBox');
document.getElementById('otherFolderBtn').addEventListener('click', async () => {
  try {
    const resp = await fetch('/api/choose-folder', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{}'
    });
    const picked = await resp.json();
    if (!picked.ok) {
      return;
    }
    savePlace = 'chosen';
    otherFolderBox.hidden = true;
    errorBox.hidden = true;
    await labelSaveButton();
    saveBtn.click();
  } catch (err) {
    showError(String(err?.message || err));
  }
});
// « Continuer à corriger » or Escape.
leaveDialog.addEventListener('close', () => {
  if (leavingForOpenedPdf) {
    leavingForOpenedPdf = false;
    fetch('/api/opened-pdf').catch(() => {});
  }
});

// The home page takes the PDF the Mac sent (see drop.js), including one sent while this page was
// loading.
function leaveForOpenedPdf() {
  leavingForOpenedPdf = true;
  leave('drop.html');
}
window.addEventListener('modifier-mes-pdf:opened', leaveForOpenedPdf);

if (window.modifierMesPdfApp) {
  fetch('/api/opened-pdf?peek=1')
    .then((resp) => resp.json())
    .then((opened) => {
      if (opened.waiting) {
        leaveForOpenedPdf();
      }
    })
    .catch(() => {});
  // No « Recharger » / « Précédent » menu that would lose the corrections; the usual menu
  // (spelling, copy, paste) stays in the line being typed.
  document.addEventListener('contextmenu', (event) => {
    if (!event.target.closest('[contenteditable="plaintext-only"], input, textarea')) {
      event.preventDefault();
    }
  });
}

// Placing the lines needs container units (Safari 16 and later): on an older Mac, say so plainly.
if (!CSS.supports('width', '1cqw')) {
  showError(
    'Cette fonction a besoin d’une version plus récente de Safari. Mettez à jour votre Mac (menu Pomme › Mise à jour de logiciels), ou revenez en arrière et choisissez « Modifier dans Pages ».'
  );
} else {
  load();
}
