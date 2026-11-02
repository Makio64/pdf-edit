// Dedicated Web Worker hosting the compiled Rust WASM engine (`pkg/p2d_wasm.js`).
import initWasm, {
  wasm_inspect_pdf,
  wasm_convert_pdf,
  wasm_convert_report_json
} from './pkg/p2d_wasm.js';

let wasmReady = false;

async function ensureWasmInitialized() {
  if (!wasmReady) {
    await initWasm();
    wasmReady = true;
  }
}

function getFormatMetadata(format) {
  switch (format) {
    case 'pages':
      return { mimeType: 'application/vnd.apple.pages', formatExt: 'pages' };
    case 'html':
      return { mimeType: 'text/html', formatExt: 'html' };
    case 'md':
      return { mimeType: 'text/markdown', formatExt: 'md' };
    case 'txt':
      return { mimeType: 'text/plain', formatExt: 'txt' };
    case 'spdf':
      return { mimeType: 'application/pdf', formatExt: 'pdf' };
    default:
      return {
        mimeType: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
        formatExt: 'docx'
      };
  }
}

self.addEventListener('message', async (event) => {
  const msg = event.data || {};
  const { requestId, type, pdfBytes, preset, profile, format, password } = msg;
  if (type !== 'WORKER_CONVERT') {
    return;
  }

  try {
    await ensureWasmInitialized();

    const inspectJson = wasm_inspect_pdf(pdfBytes, password || undefined);
    const inspectInfo = JSON.parse(inspectJson);
    if (inspectInfo.status === 'needs_password' || inspectInfo.status === 'bad_password') {
      self.postMessage({
        requestId,
        ok: false,
        needsPassword: true,
        badPassword: inspectInfo.status === 'bad_password',
        error:
          inspectInfo.status === 'bad_password'
            ? 'Le mot de passe saisi est incorrect.'
            : 'Ce PDF est protégé par un mot de passe.'
      });
      return;
    }

    const outBytes = wasm_convert_pdf(
      pdfBytes,
      password || undefined,
      format || 'docx',
      profile || 'pages',
      preset || 'editable'
    );

    let report = null;
    try {
      const reportStr = wasm_convert_report_json(
        pdfBytes,
        password || undefined
      );
      report = JSON.parse(reportStr);
    } catch (_err) {
      report = null;
    }

    const { mimeType, formatExt } = getFormatMetadata(format || 'docx');
    self.postMessage(
      {
        requestId,
        ok: true,
        outBytes,
        mimeType,
        formatExt,
        report
      },
      [outBytes.buffer]
    );
  } catch (err) {
    self.postMessage({
      requestId,
      ok: false,
      error: String(err?.message || err)
    });
  }
});
