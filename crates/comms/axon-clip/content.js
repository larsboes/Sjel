// Content script for Sjel Clip
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.action === 'extract') {
    const selection = window.getSelection().toString().trim();
    const html = document.documentElement.outerHTML;
    const title = document.title || '';
    let author = '';

    const authorMeta = document.querySelector('meta[name="author"], meta[property="article:author"], meta[name="twitter:creator"]');
    if (authorMeta) {
      author = authorMeta.getAttribute('content') || '';
    }

    sendResponse({
      selection,
      html,
      title,
      author,
      url: window.location.href
    });
  }
  return true; // Keep response channel open
});
