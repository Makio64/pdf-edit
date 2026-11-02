// Service Worker (MV3) — "PDF Edit"
// Strictly stateless: all state is persisted in chrome.storage.session / chrome.storage.local.

chrome.action.onClicked.addListener(async () => {
  const dropUrl = chrome.runtime.getURL('ui/drop.html');
  await chrome.tabs.create({ url: dropUrl });
});

async function ensureOffscreenDocument() {
  const offscreenUrl = chrome.runtime.getURL('offscreen.html');
  if (chrome.runtime.getContexts) {
    const existing = await chrome.runtime.getContexts({
      contextTypes: ['OFFSCREEN_DOCUMENT'],
      documentUrls: [offscreenUrl]
    });
    if (existing.length > 0) {
      return;
    }
  }

  const { offscreenLock = 'idle' } = await chrome.storage.session.get('offscreenLock');
  if (offscreenLock === 'creating') {
    // Wait briefly and re-check via storage/contexts without global variables
    await new Promise((resolve) => setTimeout(resolve, 150));
    return;
  }

  await chrome.storage.session.set({ offscreenLock: 'creating' });
  try {
    await chrome.offscreen.createDocument({
      url: 'offscreen.html',
      reasons: ['WORKERS', 'BLOBS'],
      justification: 'Exécuter la conversion locale PDF vers document modifiable Pages/Word dans un Web Worker WASM.'
    });
    await chrome.storage.session.set({ offscreenLock: 'ready' });
  } catch (err) {
    // Ignore error if document already exists
    if (!String(err?.message || '').includes('Only a single offscreen')) {
      await chrome.storage.session.set({ offscreenLock: 'idle' });
      throw err;
    }
    await chrome.storage.session.set({ offscreenLock: 'ready' });
  }
}

function sanitizeFrenchFilename(rawTitle) {
  const cleaned = String(rawTitle || 'Document')
    .replace(/\.pdf$/i, '')
    .replace(/[/:*?"<>|\\]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  const today = new Date().toISOString().slice(0, 10);
  const base = cleaned || 'Document';
  return `Documents modifiables/${today} — ${base}`;
}

async function saveAndOpenArtifact({ base64Data, mimeType, suggestedName, formatExt }) {
  const dataUrl = `data:${mimeType};base64,${base64Data}`;
  const filename = `${sanitizeFrenchFilename(suggestedName)}.${formatExt}`;

  const downloadId = await chrome.downloads.download({
    url: dataUrl,
    filename,
    conflictAction: 'uniquify',
    saveAs: false
  });

  await chrome.storage.session.set({
    lastConvertedDownload: {
      downloadId,
      filename,
      timestamp: Date.now()
    }
  });

  let openedDirectly = false;
  try {
    await chrome.downloads.open(downloadId);
    openedDirectly = true;
  } catch (_err) {
    // Chrome may require a fresh user gesture in an extension frame if conversion took > 3s
    openedDirectly = false;
  }

  return {
    ok: true,
    downloadId,
    filename,
    needsUserGesture: !openedDirectly
  };
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || typeof message.type !== 'string') {
    return false;
  }

  // Must invoke chrome.downloads.open synchronously in the user gesture turn before any await
  if (message.type === 'OPEN_DOWNLOAD_WITH_GESTURE') {
    try {
      chrome.downloads.open(Number(message.downloadId));
      sendResponse({ ok: true });
    } catch (err) {
      chrome.downloads.show(Number(message.downloadId));
      sendResponse({ ok: true, shownInFinder: true, note: String(err?.message || err) });
    }
    return false;
  }

  if (message.type === 'SHOW_DOWNLOAD_IN_FINDER') {
    chrome.downloads.show(Number(message.downloadId));
    sendResponse({ ok: true });
    return false;
  }

  (async () => {
    try {
      if (message.type === 'GET_LAST_CONVERTED') {
        const { lastConvertedDownload = null } = await chrome.storage.session.get('lastConvertedDownload');
        sendResponse({ ok: true, lastConvertedDownload });
        return;
      }

      if (message.type === 'CONVERT_GMAIL_PDF') {
        await chrome.action.setBadgeText({ text: '...' });
        await chrome.action.setBadgeBackgroundColor({ color: '#1A5FB4' });

        await ensureOffscreenDocument();
        const offscreenResp = await chrome.runtime.sendMessage({
          type: 'OFFSCREEN_CONVERT_URL',
          downloadUrl: message.downloadUrl,
          suggestedName: message.suggestedName || 'Document.pdf',
          preset: message.preset || 'editable',
          profile: message.profile || 'pages',
          password: message.password || null
        });

        if (!offscreenResp || !offscreenResp.ok) {
          await chrome.action.setBadgeText({ text: '!' });
          await chrome.action.setBadgeBackgroundColor({ color: '#C01C28' });
          sendResponse(offscreenResp || { ok: false, error: 'Échec de la conversion locale.' });
          return;
        }

        const saved = await saveAndOpenArtifact({
          base64Data: offscreenResp.base64Data,
          mimeType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
          suggestedName: message.suggestedName || 'Document.pdf',
          formatExt: 'docx'
        });

        await chrome.action.setBadgeText({ text: 'OK' });
        await chrome.action.setBadgeBackgroundColor({ color: '#26A269' });

        sendResponse({
          ...saved,
          report: offscreenResp.report
        });
        return;
      }

      if (message.type === 'CONVERT_PDF_BYTES') {
        await chrome.action.setBadgeText({ text: '...' });
        await chrome.action.setBadgeBackgroundColor({ color: '#1A5FB4' });

        await ensureOffscreenDocument();
        const offscreenResp = await chrome.runtime.sendMessage({
          type: 'OFFSCREEN_CONVERT_BYTES',
          base64Pdf: message.base64Pdf,
          suggestedName: message.suggestedName || 'Document.pdf',
          preset: message.preset || 'editable',
          profile: message.profile || 'pages',
          format: message.format || 'docx',
          password: message.password || null
        });

        if (!offscreenResp || !offscreenResp.ok) {
          await chrome.action.setBadgeText({ text: '!' });
          await chrome.action.setBadgeBackgroundColor({ color: '#C01C28' });
          sendResponse(offscreenResp || { ok: false, error: 'Échec de la conversion locale.' });
          return;
        }

        const saved = await saveAndOpenArtifact({
          base64Data: offscreenResp.base64Data,
          mimeType: offscreenResp.mimeType,
          suggestedName: message.suggestedName || 'Document.pdf',
          formatExt: offscreenResp.formatExt
        });

        await chrome.action.setBadgeText({ text: 'OK' });
        await chrome.action.setBadgeBackgroundColor({ color: '#26A269' });

        sendResponse({
          ...saved,
          report: offscreenResp.report
        });
        return;
      }

      sendResponse({ ok: false, error: `Action inconnue : ${message.type}` });
    } catch (err) {
      await chrome.action.setBadgeText({ text: '!' });
      await chrome.action.setBadgeBackgroundColor({ color: '#C01C28' });
      sendResponse({
        ok: false,
        error: String(err?.message || err)
      });
    }
  })();

  return true;
});
