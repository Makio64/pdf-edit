const params = new URLSearchParams(window.location.search);
const downloadId = Number(params.get('downloadId') || '0');
const openBtn = document.getElementById('openBtn');

if (openBtn && downloadId > 0) {
  openBtn.addEventListener('click', () => {
    // Synchronous sendMessage inside extension-frame user gesture
    chrome.runtime.sendMessage({
      type: 'OPEN_DOWNLOAD_WITH_GESTURE',
      downloadId
    });
    openBtn.textContent = '✓ Document ouvert dans Pages';
  });
}
