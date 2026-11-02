// Content Script for Gmail (`https://mail.google.com/*`) — "PDF Edit"
// 100% Trusted-Types safe: zero innerHTML / outerHTML / insertAdjacentHTML.

const PROCESSED_ATTR = 'data-p2d-attached';
const COMPOSE_ATTR = 'data-p2d-compose-attached';

function extractPdfAttachmentInfo(node) {
  // Primary anchor: [download_url^="application/pdf:"]
  const rawAttr = node.getAttribute?.('download_url') || '';
  if (rawAttr.toLowerCase().startsWith('application/pdf:')) {
    // Format: application/pdf:filename.pdf:https://mail.google.com/mail/u/0?ui=2&...
    const firstColon = rawAttr.indexOf(':');
    const secondColon = rawAttr.indexOf(':', firstColon + 1);
    if (firstColon !== -1 && secondColon !== -1) {
      const filename = decodeURIComponent(rawAttr.slice(firstColon + 1, secondColon));
      const downloadUrl = rawAttr.slice(secondColon + 1);
      return { filename, downloadUrl };
    }
  }

  // Secondary anchor: link href with view=att or download="*.pdf"
  const link =
    node.matches?.('a[href]') ? node : node.querySelector?.('a[href*="view=att"], a[download$=".pdf"]');
  if (link) {
    const href = link.href || '';
    const nameAttr =
      link.getAttribute('download') ||
      node.getAttribute('aria-label') ||
      link.textContent?.trim() ||
      'Document.pdf';
    if (href && /\.pdf/i.test(nameAttr)) {
      return { filename: nameAttr, downloadUrl: href };
    }
  }

  return null;
}

function createShadowActionWidget({ filename, downloadUrl }) {
  const host = document.createElement('div');
  host.style.display = 'inline-block';
  host.style.marginTop = '6px';
  host.style.marginBottom = '4px';
  host.style.zIndex = '20';

  const shadow = host.attachShadow({ mode: 'closed' });

  const style = document.createElement('style');
  style.textContent = `
    :host {
      all: initial;
      font-family: -apple-system, BlinkMacSystemFont, "Helvetica Neue", Arial, sans-serif;
    }
    .p2d-container {
      display: inline-flex;
      flex-direction: column;
      gap: 6px;
    }
    .p2d-btn {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      min-height: 44px;
      padding: 8px 16px;
      border-radius: 10px;
      border: 2px solid #0f4c96;
      background: #1a5fb4;
      color: #ffffff;
      font-size: 16px;
      font-weight: 700;
      line-height: 1.2;
      cursor: pointer;
      box-shadow: 0 2px 5px rgba(0, 0, 0, 0.16);
      transition: background 0.15s ease;
    }
    .p2d-btn:hover {
      background: #154c91;
    }
    .p2d-btn:disabled {
      background: #5e5c64;
      border-color: #3d3846;
      cursor: wait;
    }
    .p2d-btn:focus-visible {
      outline: 3px solid #f5c211;
      outline-offset: 2px;
    }
    .p2d-status {
      font-size: 14px;
      font-weight: 600;
      color: #1e1e1e;
      background: #f6f5f4;
      border-radius: 8px;
      padding: 6px 10px;
      border: 1px solid #deddda;
    }
    .p2d-status.error {
      color: #a51d2d;
      background: #fce8e8;
      border-color: #f66151;
    }
    .p2d-iframe {
      border: none;
      width: 290px;
      height: 52px;
      overflow: hidden;
    }
  `;

  const container = document.createElement('div');
  container.className = 'p2d-container';

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'p2d-btn';
  button.textContent = '✏️ Modifier ce document';
  button.title = `Convertir « ${filename} » et l’ouvrir dans Pages`;

  const statusBox = document.createElement('div');
  statusBox.className = 'p2d-status';
  statusBox.style.display = 'none';

  button.addEventListener('click', async (event) => {
    event.stopPropagation();
    event.preventDefault();

    button.disabled = true;
    button.textContent = '⏳ Préparation pour Pages…';
    statusBox.style.display = 'none';

    try {
      const response = await chrome.runtime.sendMessage({
        type: 'CONVERT_GMAIL_PDF',
        downloadUrl,
        suggestedName: filename,
        preset: 'editable',
        profile: 'pages'
      });

      if (!response || !response.ok) {
        button.disabled = false;
        button.textContent = '✏️ Réessayer la conversion';
        statusBox.className = 'p2d-status error';
        statusBox.textContent =
          response?.error || 'Impossible de convertir ce PDF pour le moment.';
        statusBox.style.display = 'block';
        return;
      }

      if (response.needsUserGesture && response.downloadId) {
        button.style.display = 'none';
        const iframe = document.createElement('iframe');
        iframe.className = 'p2d-iframe';
        iframe.src = chrome.runtime.getURL(
          `ui/open-gesture.html?downloadId=${encodeURIComponent(String(response.downloadId))}`
        );
        container.appendChild(iframe);
      } else {
        button.disabled = false;
        button.textContent = '✓ Ouvert dans Pages';
        statusBox.className = 'p2d-status';
        statusBox.textContent = `Enregistré dans Téléchargements / ${response.filename}`;
        statusBox.style.display = 'block';
      }
    } catch (err) {
      button.disabled = false;
      button.textContent = '✏️ Modifier ce document';
      statusBox.className = 'p2d-status error';
      statusBox.textContent = String(err?.message || err);
      statusBox.style.display = 'block';
    }
  });

  container.appendChild(button);
  container.appendChild(statusBox);
  shadow.appendChild(style);
  shadow.appendChild(container);
  return host;
}

function createComposeReturnWidget() {
  const host = document.createElement('span');
  host.style.display = 'inline-block';
  host.style.marginLeft = '8px';

  const shadow = host.attachShadow({ mode: 'closed' });
  const style = document.createElement('style');
  style.textContent = `
    :host {
      all: initial;
      font-family: -apple-system, BlinkMacSystemFont, "Helvetica Neue", Arial, sans-serif;
    }
    .p2d-compose-btn {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      min-height: 36px;
      padding: 6px 12px;
      border-radius: 8px;
      border: 1.5px solid #1a5fb4;
      background: #eef4fc;
      color: #1a5fb4;
      font-size: 14px;
      font-weight: 700;
      cursor: pointer;
    }
    .p2d-compose-btn:hover {
      background: #dce8fa;
    }
  `;

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'p2d-compose-btn';
  btn.textContent = '📎 Joindre mon document modifié';
  btn.title = 'Affiche votre dernier document converti dans le Finder pour le glisser dans ce message';

  btn.addEventListener('click', async (event) => {
    event.stopPropagation();
    event.preventDefault();
    const resp = await chrome.runtime.sendMessage({ type: 'GET_LAST_CONVERTED' });
    if (resp?.lastConvertedDownload?.downloadId) {
      await chrome.runtime.sendMessage({
        type: 'SHOW_DOWNLOAD_IN_FINDER',
        downloadId: resp.lastConvertedDownload.downloadId
      });
      btn.textContent = '✓ Fichier affiché dans le Finder (glissez-le ici)';
    } else {
      btn.textContent = 'Aucun document converti récemment';
    }
  });

  shadow.appendChild(style);
  shadow.appendChild(btn);
  return host;
}

async function scanAndEnhanceGmailDom() {
  const candidates = Array.from(
    document.querySelectorAll(
      `[download_url^="application/pdf:"]:not([${PROCESSED_ATTR}]), a[download$=".pdf"]:not([${PROCESSED_ATTR}])`
    )
  );

  const BATCH = 12;
  for (let i = 0; i < candidates.length; i += BATCH) {
    const slice = candidates.slice(i, i + BATCH);
    await new Promise((resolve) =>
      requestAnimationFrame(() => {
        for (const node of slice) {
          node.setAttribute(PROCESSED_ATTR, '1');
          const info = extractPdfAttachmentInfo(node);
          if (info) {
            const widget = createShadowActionWidget(info);
            node.appendChild(widget);
          }
        }
        resolve();
      })
    );
    if (globalThis.scheduler?.yield) {
      await globalThis.scheduler.yield();
    }
  }

  // Also enhance Gmail compose/reply toolbars with the 1-click return-trip helper
  const composeToolbars = Array.from(
    document.querySelectorAll(`td.gU.Up:not([${COMPOSE_ATTR}])`)
  );
  if (composeToolbars.length > 0) {
    const resp = await chrome.runtime.sendMessage({ type: 'GET_LAST_CONVERTED' });
    if (resp?.lastConvertedDownload?.downloadId) {
      await new Promise((resolve) =>
        requestAnimationFrame(() => {
          for (const bar of composeToolbars) {
            bar.setAttribute(COMPOSE_ATTR, '1');
            bar.appendChild(createComposeReturnWidget());
          }
          resolve();
        })
      );
    }
  }
}

let scanScheduled = false;
function scheduleScan() {
  if (scanScheduled) {
    return;
  }
  scanScheduled = true;
  requestAnimationFrame(async () => {
    scanScheduled = false;
    await scanAndEnhanceGmailDom();
  });
}

const observer = new MutationObserver(() => {
  scheduleScan();
});

observer.observe(document.documentElement, {
  childList: true,
  subtree: true
});

scheduleScan();
