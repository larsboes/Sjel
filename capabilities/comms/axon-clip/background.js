// Background service worker for Sjel Clip.
//
// Everything that clips outside the popup lives here: the context menus and the
// keyboard shortcut. Those paths have no popup to write into, so each one ends
// in a notification carrying the stored item's id — a capture you cannot look
// up afterwards is a capture you cannot trust.

// Named so the server can record which client handed the content over, instead
// of storing it as indistinguishable from a page it fetched itself.
const CLIENT_ID = 'axon-clip';

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: 'axon-clip-page',
    title: 'Clip Page to Sjel',
    contexts: ['page']
  });

  chrome.contextMenus.create({
    id: 'axon-clip-selection',
    title: 'Clip Selection to Sjel',
    contexts: ['selection']
  });

  chrome.contextMenus.create({
    id: 'axon-clip-link',
    title: 'Clip Link to Sjel',
    contexts: ['link']
  });
});

async function sendIngestRequest(payload) {
  const { serverUrl = 'http://127.0.0.1:8083', apiSecret = '' } = await chrome.storage.local.get([
    'serverUrl',
    'apiSecret'
  ]);

  const endpoint = `${serverUrl.replace(/\/+$/, '')}/ingest`;
  const headers = {
    'Content-Type': 'application/json'
  };

  if (apiSecret) {
    headers['Authorization'] = `Bearer ${apiSecret}`;
    headers['X-Sjel-Token'] = apiSecret;
  }

  const response = await fetch(endpoint, {
    method: 'POST',
    headers,
    body: JSON.stringify({ ...payload, client: CLIENT_ID })
  });

  if (!response.ok) {
    const errorJson = await response.json().catch(() => ({}));
    const msg = errorJson.error || `HTTP ${response.status} ${response.statusText}`;
    throw new Error(msg);
  }

  return await response.json();
}

async function flashBadge(text, color) {
  await chrome.action.setBadgeText({ text });
  await chrome.action.setBadgeBackgroundColor({ color });
  setTimeout(async () => {
    await chrome.action.setBadgeText({ text: '' });
  }, 2500);
}

// The id is the point of this notification: it is what finds the item in the
// feed, and what shows that two clips of the same URL landed on one row.
async function report(item, error) {
  if (error) {
    await flashBadge('ERR', '#EF4444');
    await chrome.notifications.create({
      type: 'basic',
      iconUrl: 'icons/icon-48.png',
      title: 'Sjel Clip failed',
      message: error
    });
    return;
  }

  await flashBadge('✓', '#10B981');
  await chrome.notifications.create({
    type: 'basic',
    iconUrl: 'icons/icon-48.png',
    title: 'Clipped to Sjel Feed',
    message: `${item.title || item.url}\nid ${item.id}`
  });
}

// Read the rendered DOM of a tab. This is the capability the server does not
// have: the page as the operator's own session sees it, login and all.
async function readPage(tabId) {
  const [{ result }] = await chrome.scripting.executeScript({
    target: { tabId },
    func: () => ({
      html: document.documentElement.outerHTML,
      title: document.title,
      url: window.location.href
    })
  });
  return result;
}

async function clipActivePage(tab) {
  const page = await readPage(tab.id);
  return sendIngestRequest({
    url: page.url || tab.url,
    content: page.html,
    title: page.title || tab.title
  });
}

chrome.commands.onCommand.addListener(async (command) => {
  if (command !== 'clip-page') return;
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab || !tab.id) return;

  try {
    await report(await clipActivePage(tab));
  } catch (err) {
    console.error('Sjel Clip shortcut ingest failed:', err);
    await report(null, err.message);
  }
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (!tab || !tab.id) return;

  try {
    if (info.menuItemId === 'axon-clip-link' && info.linkUrl) {
      await report(await sendIngestRequest({ url: info.linkUrl }));
    } else if (info.menuItemId === 'axon-clip-selection' && info.selectionText) {
      await report(
        await sendIngestRequest({
          url: tab.url || info.pageUrl,
          content: info.selectionText,
          title: tab.title
        })
      );
    } else if (info.menuItemId === 'axon-clip-page') {
      await report(await clipActivePage(tab));
    }
  } catch (err) {
    console.error('Sjel Clip context menu ingest failed:', err);
    await report(null, err.message);
  }
});

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.action === 'ingest') {
    (async () => {
      try {
        const res = await sendIngestRequest(message.payload);
        sendResponse({ success: true, data: res });
      } catch (err) {
        sendResponse({ success: false, error: err.message });
      }
    })();
    return true; // Keep response channel open for async sendResponse
  }
});
