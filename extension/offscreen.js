// Offscreen Document (MV3) — "PDF Edit"
// Strictly restricted to chrome.runtime.{onMessage,sendMessage,getURL} + Web APIs (fetch, Worker).

let workerInstance = null;
let nextRequestId = 1;
const pendingWorkerRequests = new Map();

function getEngineWorker() {
  if (!workerInstance) {
    workerInstance = new Worker(chrome.runtime.getURL('engine.worker.js'), { type: 'module' });
    workerInstance.addEventListener('message', (event) => {
      const data = event.data || {};
      const resolver = pendingWorkerRequests.get(data.requestId);
      if (resolver) {
        pendingWorkerRequests.delete(data.requestId);
        resolver(data);
      }
    });
  }
  return workerInstance;
}

async function runWorkerConversion({ pdfBytes, preset, profile, format, password }) {
  const worker = getEngineWorker();
  const requestId = nextRequestId++;
  return await new Promise((resolve) => {
    pendingWorkerRequests.set(requestId, resolve);
    const copy = new Uint8Array(pdfBytes);
    worker.postMessage(
      {
        requestId,
        type: 'WORKER_CONVERT',
        pdfBytes: copy,
        preset: preset || 'editable',
        profile: profile || 'pages',
        format: format || 'docx',
        password: password || null
      },
      [copy.buffer]
    );
  });
}

function uint8ToBase64(bytes) {
  let binary = '';
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    const slice = bytes.subarray(i, i + chunkSize);
    binary += String.fromCharCode(...slice);
  }
  return btoa(binary);
}

function base64ToUint8(base64) {
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}

function hasPdfMagicHeader(bytes) {
  if (!bytes || bytes.length < 5) {
    return false;
  }
  // Scan first 64 bytes for "%PDF-"
  const maxScan = Math.min(bytes.length - 5, 64);
  for (let i = 0; i <= maxScan; i++) {
    if (
      bytes[i] === 0x25 && // %
      bytes[i + 1] === 0x50 && // P
      bytes[i + 2] === 0x44 && // D
      bytes[i + 3] === 0x46 && // F
      bytes[i + 4] === 0x2d // -
    ) {
      return true;
    }
  }
  return false;
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || typeof message.type !== 'string') {
    return false;
  }
  if (!message.type.startsWith('OFFSCREEN_')) {
    return false;
  }

  (async () => {
    try {
      if (message.type === 'OFFSCREEN_CONVERT_URL') {
        const response = await fetch(message.downloadUrl, {
          credentials: 'include',
          redirect: 'follow'
        });
        if (!response.ok) {
          sendResponse({
            ok: false,
            error: `Impossible de récupérer la pièce jointe Gmail (HTTP ${response.status}).`
          });
          return;
        }
        const buffer = await response.arrayBuffer();
        const pdfBytes = new Uint8Array(buffer);
        if (!hasPdfMagicHeader(pdfBytes)) {
          sendResponse({
            ok: false,
            error: 'La session Gmail a expiré ou le fichier téléchargé n’est pas un PDF valide.'
          });
          return;
        }

        const result = await runWorkerConversion({
          pdfBytes,
          preset: message.preset || 'editable',
          profile: message.profile || 'pages',
          format: 'docx',
          password: message.password || null
        });

        if (!result.ok) {
          sendResponse(result);
          return;
        }

        sendResponse({
          ok: true,
          base64Data: uint8ToBase64(result.outBytes),
          mimeType: result.mimeType,
          formatExt: result.formatExt,
          report: result.report
        });
        return;
      }

      if (message.type === 'OFFSCREEN_CONVERT_BYTES') {
        const pdfBytes = base64ToUint8(message.base64Pdf);
        if (!hasPdfMagicHeader(pdfBytes)) {
          sendResponse({
            ok: false,
            error: 'Ce fichier ne semble pas être un document PDF valide.'
          });
          return;
        }

        const result = await runWorkerConversion({
          pdfBytes,
          preset: message.preset || 'editable',
          profile: message.profile || 'pages',
          format: message.format || 'docx',
          password: message.password || null
        });

        if (!result.ok) {
          sendResponse(result);
          return;
        }

        sendResponse({
          ok: true,
          base64Data: uint8ToBase64(result.outBytes),
          mimeType: result.mimeType,
          formatExt: result.formatExt,
          report: result.report
        });
        return;
      }

      sendResponse({ ok: false, error: `Commande offscreen inconnue : ${message.type}` });
    } catch (err) {
      sendResponse({
        ok: false,
        error: String(err?.message || err)
      });
    }
  })();

  return true;
});
