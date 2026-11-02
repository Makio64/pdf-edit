// « PDF Edit »: one question per screen.
// 1. choose a PDF, 2. choose what to do with it, 3. choose where to save it; then the new file and
// where it is. Runs in the Mac app's window and the local web server (both talk to /api/*), and in
// the Chrome extension (talks to the service worker), which skips 3: Chrome only saves into
// Downloads.

const $ = (id) => document.getElementById(id);

const screens = { file: $('stepFile'), choice: $('stepChoice'), place: $('stepPlace'), result: $('stepResult') };
const views = { busy: $('busyView'), password: $('passwordView'), error: $('errorView'), done: $('doneView') };
const progressBar = $('progressBar');
const stepCount = $('stepCount');
const backBtn = $('backBtn');
const dropZone = $('dropZone');
const chooseFileBtn = $('chooseFileBtn');
const fileInput = $('fileInput');
const fileError = $('fileError');
const samplesBox = $('samplesBox');
const sourceName = $('sourceName');
const editOption = $('editOption');
const chosenOption = $('chosenOption');
const chosenTitle = $('chosenTitle');
const chosenText = $('chosenText');
const pickFolderBtn = $('pickFolderBtn');
const pickFolderTitle = $('pickFolderTitle');
const pickFolderText = $('pickFolderText');
const busyTitle = $('busyTitle');
const busySpinner = $('busySpinner');
const busyLead = $('busyLead');
const busySteps = $('busySteps');
const passwordInput = $('passwordInput');
const submitPasswordBtn = $('submitPasswordBtn');
const passwordError = $('passwordError');
const errorTitle = $('errorTitle');
const errorText = $('errorText');
const errorRestartBtn = $('errorRestartBtn');
const otherPlaceBtn = $('otherPlaceBtn');
const doneText = $('doneText');
const savedName = $('savedName');
const savedPlace = $('savedPlace');
const fileCard = $('fileCard');
const savedExt = $('savedExt');
const showInFinderBtn = $('showInFinderBtn');
const openAgainBtn = $('openAgainBtn');

const isChromeExtension =
  typeof chrome !== 'undefined' && Boolean(chrome.runtime && chrome.runtime.id);
// Set by the Mac app's window before the page loads (app_window.rs).
const isMacApp = Boolean(window.modifierMesPdfApp);
// Online static web hosting mode (forced with `?wasm=1` or detected when `/api/app-info` is absent).
let isOnlineWasm = new URLSearchParams(location.search).get('wasm') === '1';
let wasmClient = null;

async function getWasmClient() {
  if (!wasmClient) {
    wasmClient = await import('./wasm-client.js');
  }
  return wasmClient;
}

const STEP_COUNT = isChromeExtension ? 2 : 3;
const STEP_NUMBER = { file: 1, choice: 2, place: 3, result: STEP_COUNT };
const PREVIOUS_SCREEN = { choice: 'file', place: 'choice', result: isChromeExtension ? 'choice' : 'place' };
let currentScreen = 'file';
// The chosen PDF: bytes the person picked, or one of the app's examples.
let source = null;
// What the person chose on screen 2: 'edit', 'pages' or 'docx'.
let chosenAction = null;
// Where, on screen 3: 'desktop', 'downloads' or 'chosen' (the folder picked last).
let chosenPlace = 'downloads';
// The new file.
let saved = null;

function focusHeading(container) {
  container.querySelector('h1')?.focus({ preventScroll: true });
}

function showScreen(name) {
  currentScreen = name;
  for (const [key, el] of Object.entries(screens)) {
    el.hidden = key !== name;
  }
  const n = STEP_NUMBER[name];
  stepCount.textContent = `Étape ${n} sur ${STEP_COUNT}`;
  progressBar.style.width = `${(n / STEP_COUNT) * 100}%`;
  backBtn.hidden = name === 'file';
  window.scrollTo(0, 0);
  if (name !== 'result') {
    focusHeading(screens[name]);
  }
}

// Screen 3 shows one view at a time: waiting, password, error or done.
function showView(name) {
  showScreen('result');
  for (const [key, el] of Object.entries(views)) {
    el.hidden = key !== name;
  }
  backBtn.hidden = name === 'busy';
  focusHeading(views[name]);
}

function isBusy() {
  return currentScreen === 'result' && !views.busy.hidden;
}

function arrayBufferToBase64(buffer) {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary);
}

// Kept for this tab only, so « Retour » from the editor (drop.html#choix) comes back to screen 2.
// A PDF too large for sessionStorage is simply not kept.
function rememberSource() {
  try {
    sessionStorage.setItem('source', JSON.stringify(source));
  } catch {
    try {
      sessionStorage.removeItem('source');
    } catch {
      // Storage unavailable: « Retour » then starts again at screen 1.
    }
  }
}

function rememberedSource() {
  try {
    return JSON.parse(sessionStorage.getItem('source'));
  } catch {
    return null;
  }
}

function sourceChosen() {
  rememberSource();
  fileError.hidden = true;
  passwordInput.value = '';
  sourceName.textContent = source.name;
  showScreen('choice');
}

async function useFile(file) {
  if (!file || isBusy()) {
    return;
  }
  if (file.type !== 'application/pdf' && !/\.pdf$/i.test(file.name)) {
    showScreen('file');
    fileError.hidden = false;
    return;
  }
  source = { name: file.name, base64: arrayBufferToBase64(await file.arrayBuffer()), sample: null };
  sourceChosen();
}

function useSample(sample) {
  source = { name: sample, base64: null, sample };
  sourceChosen();
}

// A PDF the Mac sent to the app: « Ouvrir avec », dropped on its Dock icon, or ⌘O. While a
// document is being prepared, it waits until that is done.
let openedPdfWaiting = false;

// Whether it took one. `force`: even while busy (the run that was busy gives way).
async function takeOpenedPdf(force = false) {
  if (isBusy() && !force) {
    openedPdfWaiting = true;
    return false;
  }
  openedPdfWaiting = false;
  try {
    const opened = await (await fetch('/api/opened-pdf')).json();
    if (!opened.ok) {
      return false;
    }
    if (!/\.pdf$/i.test(opened.name)) {
      showScreen('file');
      fileError.hidden = false;
      return true;
    }
    source = { name: opened.name, base64: opened.base64, sample: null };
    sourceChosen();
    return true;
  } catch {
    // Nothing waiting.
    return false;
  }
}

function restart() {
  source = null;
  saved = null;
  fileInput.value = '';
  showScreen('file');
  if (openedPdfWaiting) {
    takeOpenedPdf();
  }
}

async function postJson(url, body) {
  const resp = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  });
  return resp.json();
}

// What the waiting screen lists for each choice of screen 2, and which of them each step the
// server reports (/api/progress) belongs to.
const STEPS = {
  pages: [
    ['read', 'Lecture du PDF'],
    ['layout', 'Mise en page du texte'],
    ['write', 'Création du document'],
    ['pages', 'Enregistrement par Pages']
  ],
  docx: [
    ['read', 'Lecture du PDF'],
    ['layout', 'Mise en page du texte'],
    ['write', 'Création du fichier Word']
  ],
  edit: [
    ['read', 'Lecture du PDF'],
    ['layout', 'Repérage des lignes de texte'],
    ['colors', 'Préparation des pages']
  ]
};
const STEP_OF = { draw: 'read', font: 'layout' };
const BUSY_LEAD = 'Vous n’avez rien à faire. Un long document peut prendre une minute.';

// Where the current step is, in words.
function stepDetail(progress) {
  const of = (n, total) => (total > 1 ? `page ${n} sur ${total}` : '');
  switch (progress.step) {
    case 'read':
    case 'layout':
    case 'colors':
      return of(progress.page, progress.pages);
    case 'draw':
      return progress.pages > 1 ? `dessins et images : ${progress.done} pages sur ${progress.pages}` : 'dessins et images';
    case 'font':
      return `téléchargement de la police « ${progress.family} »`;
    case 'pages':
      return 'Pages travaille en arrière-plan : la première fois, cela peut prendre une minute.';
    default:
      return '';
  }
}

// The list of steps, with `progress` (null: not started) as the current one.
function showSteps(action, progress) {
  const steps = STEPS[action] || [];
  const current = progress ? STEP_OF[progress.step] || progress.step : steps[0]?.[0];
  const at = Math.max(0, steps.findIndex(([key]) => key === current));
  busySteps.textContent = '';
  steps.forEach(([, label], i) => {
    const li = document.createElement('li');
    li.className = i < at ? 'step is-done' : i === at ? 'step is-current' : 'step';
    const mark = document.createElement('span');
    mark.className = 'step-mark';
    mark.setAttribute('aria-hidden', 'true');
    const text = document.createElement('span');
    text.textContent = label;
    const detail = i === at && progress ? stepDetail(progress) : '';
    if (detail) {
      const small = document.createElement('span');
      small.className = 'step-detail';
      small.textContent = detail;
      text.append(small);
    }
    li.append(mark, text);
    busySteps.append(li);
  });
}

// Asks the server where job `job` is until the returned function is called.
function followProgress(job, action) {
  let stopped = false;
  let timer = null;
  const ask = async () => {
    try {
      const data = await (await fetch(`/api/progress?job=${encodeURIComponent(job)}`)).json();
      if (!stopped && data.progress) {
        showSteps(action, data.progress);
      }
    } catch {
      // Asked again next time.
    }
    if (!stopped) {
      timer = setTimeout(ask, 400);
    }
  };
  timer = setTimeout(ask, 250);
  return () => {
    stopped = true;
    clearTimeout(timer);
  };
}

// Pages gets macOS fonts, Word gets the fonts Word has everywhere.
async function convert(format, password, job) {
  const profile = format === 'docx' ? 'word' : 'pages';
  if (isChromeExtension) {
    return chrome.runtime.sendMessage({
      type: 'CONVERT_PDF_BYTES',
      base64Pdf: source.base64,
      suggestedName: source.name,
      preset: 'exact',
      profile,
      format,
      password
    });
  }
  if (isOnlineWasm) {
    const wasm = await getWasmClient();
    return wasm.convertPdfWasm({
      source,
      format,
      profile,
      preset: 'exact',
      password,
      place: chosenPlace
    });
  }
  return postJson('/api/convert', {
    filename: source.name,
    base64_pdf: source.base64,
    sample_file: source.sample,
    preset: 'exact',
    profile,
    format,
    password,
    open_in_pages: false,
    place: chosenPlace,
    job
  });
}

async function openForEditing(password, job) {
  if (isOnlineWasm) {
    const wasm = await getWasmClient();
    return wasm.openEditSessionWasm({ source, password });
  }
  return postJson('/api/edit/open', {
    filename: source.name,
    base64_pdf: source.base64,
    sample_file: source.sample,
    password,
    job
  });
}

async function run(action, password = null) {
  if (!source) {
    return;
  }
  chosenAction = action;
  busyTitle.textContent = action === 'edit' ? 'Ouverture du PDF…' : 'Préparation de votre document…';
  busyLead.textContent = BUSY_LEAD;
  // The steps come from the Mac app's server; the extension and the in-browser mode only wait.
  const job = isChromeExtension || isOnlineWasm ? null : `${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`;
  busySteps.hidden = !job;
  busySpinner.hidden = Boolean(job);
  if (job) {
    showSteps(action, null);
  }
  showView('busy');
  const stopProgress = job ? followProgress(job, action) : () => {};
  const slow = setTimeout(() => {
    busyLead.textContent = 'C’est un long document : encore un peu de patience. Vous n’avez rien à faire.';
  }, 20000);
  try {
    const response = action === 'edit' ? await openForEditing(password, job) : await convert(action, password, job);
    if (response?.needsPassword || response?.needs_password) {
      passwordError.hidden = !(password || response.badPassword || response.bad_password);
      showView('password');
      passwordInput.focus();
      return;
    }
    if (!response?.ok) {
      showError(response?.error || 'Ce fichier n’a pas pu être lu.', Boolean(response?.save_failed));
      return;
    }
    if (action === 'edit') {
      // A PDF the Mac sent meanwhile comes first, rather than waiting behind the editor.
      if (openedPdfWaiting && (await takeOpenedPdf(true))) {
        return;
      }
      location.href = `edit.html?session=${encodeURIComponent(response.session)}&place=${chosenPlace}`;
      return;
    }
    showDone(action, response);
  } catch (err) {
    showError(String(err?.message || err));
  } finally {
    stopProgress();
    clearTimeout(slow);
    // Not over « C'est prêt »: that screen says where the new file is. It waits for « Modifier un
    // autre PDF » (restart).
    if (openedPdfWaiting && !isBusy() && views.done.hidden) {
      takeOpenedPdf();
    }
  }
}

// The PDF could not be read, or the new file could not be written where the person chose: then
// the way out is another place, not another PDF.
function showError(message, saveFailed = false) {
  errorTitle.textContent = saveFailed ? 'Le nouveau fichier n’a pas pu être enregistré.' : 'Ce PDF n’a pas pu être ouvert.';
  errorText.textContent = message;
  errorRestartBtn.hidden = saveFailed;
  otherPlaceBtn.hidden = !saveFailed;
  showView('error');
}

function showDone(format, response) {
  saved = {
    path: response.saved_path || null,
    downloadId: response.downloadId ?? null,
    action: format === 'docx' ? 'word' : 'pages'
  };
  const name = (response.filename || '').split('/').pop();
  savedName.textContent = name;
  // The file icon shows the type (PAGES, DOCX) in its colour.
  const ext = name.includes('.') ? name.split('.').pop().toLowerCase() : '';
  fileCard.dataset.ext = ext;
  savedExt.textContent = ext.toUpperCase();
  // The extension's downloads keep the default text: Téléchargements › Documents modifiables.
  if (response.place?.parts?.length) {
    showPlaceLabel(savedPlace, response.place.parts);
  }
  const opened = (!isChromeExtension && !isOnlineWasm) || !response.needsUserGesture;
  // When Pages could not make the file, the app gives the same document as a Word file.
  const opensInWord = response.opens_in ? response.opens_in === 'word' : format === 'docx';
  doneText.textContent = isOnlineWasm
    ? 'Votre document a été téléchargé.'
    : opened
      ? opensInWord
        ? 'Votre document Word s’ouvre.'
        : 'Votre document s’ouvre dans Pages.'
      : 'Cliquez sur « Ouvrir le fichier » pour le voir.';
  if (response.pages_refused) {
    doneText.textContent +=
      ' Il est au format Word, car le Mac n’a pas autorisé PDF Edit à utiliser Pages : vous pouvez l’autoriser dans Réglages Système › Confidentialité et sécurité › Automatisation.';
  }
  showInFinderBtn.hidden = isOnlineWasm;
  showView('done');
  // The extension and online WASM mode already triggered their download; the local app opens the file now.
  if (!isChromeExtension && !isOnlineWasm) {
    openSaved();
  }
}

// « Sur le Bureau », « Dans Téléchargements », « Dans Documents › Factures »…
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

// Screen 3: Desktop, Downloads, the folder picked last time if any, and « Choisir un dossier… ».
async function showPlaces() {
  showScreen('place');
  if (isOnlineWasm) {
    const wasm = await getWasmClient();
    showChosenFolder(wasm.getBrowserSavePlaces().chosen);
    return;
  }
  try {
    const data = await (await fetch('/api/save-places')).json();
    showChosenFolder(data.chosen);
  } catch {
    showChosenFolder(null);
  }
}

function showChosenFolder(chosen) {
  const parts = chosen?.parts || [];
  chosenOption.hidden = parts.length === 0;
  if (parts.length > 0) {
    chosenTitle.textContent = `Dans « ${parts.at(-1)} »`;
    chosenText.textContent = `${parts.join(' › ')}, le dossier choisi la dernière fois.`;
  }
  pickFolderTitle.textContent = parts.length > 0 ? 'Choisir un autre dossier…' : 'Choisir un dossier…';
}

// The Mac's own folder dialog; the server remembers the answer for next time.
async function pickFolder() {
  if (isOnlineWasm) {
    const wasm = await getWasmClient();
    const picked = await wasm.pickBrowserFolder();
    if (picked.ok) {
      showChosenFolder(picked.chosen);
      chosenPlace = 'chosen';
      run(chosenAction);
    } else {
      chosenPlace = 'downloads';
      run(chosenAction);
    }
    return;
  }
  const options = screens.place.querySelectorAll('.option');
  options.forEach((option) => {
    option.disabled = true;
  });
  pickFolderText.textContent = 'Choisissez le dossier dans la fenêtre qui vient de s’ouvrir.';
  try {
    const data = await postJson('/api/choose-folder', {});
    if (data.ok) {
      showChosenFolder(data.chosen);
      chosenPlace = 'chosen';
      run(chosenAction);
    }
  } catch (err) {
    showError(String(err?.message || err));
  } finally {
    options.forEach((option) => {
      option.disabled = false;
    });
    pickFolderText.textContent = 'Une fenêtre s’ouvre pour choisir où le ranger.';
  }
}

async function openSaved() {
  if (!saved) {
    return;
  }
  if (isChromeExtension) {
    chrome.runtime.sendMessage({ type: 'OPEN_DOWNLOAD_WITH_GESTURE', downloadId: saved.downloadId });
  } else if (isOnlineWasm) {
    const wasm = await getWasmClient();
    wasm.redownloadLastSaved();
  } else {
    postJson('/api/action', { saved_path: saved.path, action: saved.action });
  }
}

function showSavedInFinder() {
  if (!saved) {
    return;
  }
  if (isChromeExtension) {
    chrome.runtime.sendMessage({ type: 'SHOW_DOWNLOAD_IN_FINDER', downloadId: saved.downloadId });
  } else {
    postJson('/api/action', { saved_path: saved.path, action: 'finder' });
  }
}

// Screen 1
chooseFileBtn.addEventListener('click', (event) => {
  event.stopPropagation();
  fileInput.click();
});

dropZone.addEventListener('click', () => fileInput.click());

dropZone.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    fileInput.click();
  }
});

fileInput.addEventListener('change', () => useFile(fileInput.files?.[0]));

for (const btn of document.querySelectorAll('[data-sample]')) {
  btn.addEventListener('click', () => useSample(btn.dataset.sample));
}

// A PDF can be dropped anywhere in the window, at any step.
let dragDepth = 0;

function endDrag() {
  dragDepth = 0;
  document.body.classList.remove('is-dragging');
}

document.addEventListener('dragenter', (event) => {
  if (event.dataTransfer?.types?.includes('Files') && !isBusy()) {
    dragDepth += 1;
    document.body.classList.add('is-dragging');
  }
});

document.addEventListener('dragleave', () => {
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) {
    endDrag();
  }
});

document.addEventListener('dragover', (event) => event.preventDefault());

document.addEventListener('drop', (event) => {
  event.preventDefault();
  endDrag();
  useFile(event.dataTransfer?.files?.[0]);
});

// Screen 2
for (const option of document.querySelectorAll('[data-action]')) {
  option.addEventListener('click', () => {
    chosenAction = option.dataset.action;
    if (isChromeExtension) {
      run(chosenAction);
    } else {
      showPlaces();
    }
  });
}

// Screen 3
for (const option of document.querySelectorAll('[data-place]')) {
  option.addEventListener('click', () => {
    chosenPlace = option.dataset.place;
    run(chosenAction);
  });
}

pickFolderBtn.addEventListener('click', pickFolder);
otherPlaceBtn.addEventListener('click', showPlaces);

// The result
submitPasswordBtn.addEventListener('click', () => run(chosenAction, passwordInput.value));

passwordInput.addEventListener('keydown', (event) => {
  if (event.key === 'Enter') {
    event.preventDefault();
    run(chosenAction, passwordInput.value);
  }
});

showInFinderBtn.addEventListener('click', showSavedInFinder);
openAgainBtn.addEventListener('click', openSaved);

for (const btn of document.querySelectorAll('[data-restart]')) {
  btn.addEventListener('click', restart);
}

backBtn.addEventListener('click', () => {
  const previous = PREVIOUS_SCREEN[currentScreen] || 'file';
  if (previous === 'place') {
    showPlaces();
  } else {
    showScreen(previous);
  }
});

// Correcting the PDF itself works both in the local app and in Online WASM mode.
editOption.hidden = isChromeExtension;
samplesBox.hidden = isChromeExtension || isMacApp;

function enableOnlineWasmMode() {
  isOnlineWasm = true;
  for (const btn of document.querySelectorAll('[data-sample^="dropbox_"]')) {
    btn.hidden = true;
  }
  const desktopOption = document.querySelector('[data-place="desktop"]');
  if (desktopOption) {
    desktopOption.hidden = true;
  }
  getWasmClient().then((w) => w.ensureWasm()).catch(() => {});
}

// Probe whether `/api/app-info` is served by the native Mac backend; if not, switch to 100% Online WASM mode.
if (isOnlineWasm) {
  enableOnlineWasmMode();
} else if (!isChromeExtension) {
  fetch('/api/app-info')
    .then((resp) => {
      if (!resp.ok) {
        throw new Error('static host');
      }
      return resp.json();
    })
    .then((info) => {
      if (info.pages === false) {
        document.querySelector('[data-action="pages"]').hidden = true;
      }
    })
    .catch(() => {
      enableOnlineWasmMode();
    });
}

if (location.hash === '#choix') {
  history.replaceState(null, '', location.pathname);
  source = rememberedSource();
}
if (source) {
  sourceChosen();
} else {
  showScreen('file');
}

if (isMacApp) {
  window.addEventListener('modifier-mes-pdf:opened', () => takeOpenedPdf());
  takeOpenedPdf();
  // No « Recharger » / « Précédent » menu in the window; the usual one stays in text fields.
  document.addEventListener('contextmenu', (event) => {
    if (!event.target.closest('input, textarea')) {
      event.preventDefault();
    }
  });
}
