document.addEventListener('DOMContentLoaded', async () => {
  const tabTitleEl = document.getElementById('tab-title');
  const tabUrlEl = document.getElementById('tab-url');
  const optionsBtn = document.getElementById('options-btn');
  const btnClipPage = document.getElementById('btn-clip-page');
  const btnClipSelection = document.getElementById('btn-clip-selection');
  const btnClipLink = document.getElementById('btn-clip-link');
  const statusEl = document.getElementById('status');

  optionsBtn.addEventListener('click', () => {
    chrome.runtime.openOptionsPage();
  });

  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab || !tab.id || !tab.url) {
    tabTitleEl.textContent = 'No active tab';
    tabUrlEl.textContent = '';
    btnClipPage.disabled = true;
    btnClipSelection.disabled = true;
    btnClipLink.disabled = true;
    return;
  }

  tabTitleEl.textContent = tab.title || tab.url;
  tabUrlEl.textContent = tab.url;

  function setStatus(text, type) {
    statusEl.className = `status-bar ${type}`;
    statusEl.textContent = text;
  }

  function setButtonsDisabled(disabled) {
    btnClipPage.disabled = disabled;
    btnClipSelection.disabled = disabled;
    btnClipLink.disabled = disabled;
  }

  async function performIngest(payload) {
    setButtonsDisabled(true);
    setStatus('Clipping to Sjel Feed...', 'info');

    chrome.runtime.sendMessage({ action: 'ingest', payload }, (response) => {
      setButtonsDisabled(false);
      if (chrome.runtime.lastError) {
        setStatus(`Error: ${chrome.runtime.lastError.message}`, 'error');
        return;
      }

      if (response && response.success) {
        // The id, not just a tick: it is what finds the item in the feed, and
        // what shows a re-clip of the same URL updated one row instead of
        // adding a second.
        const id = response.data && response.data.id;
        setStatus(id ? `✓ Clipped — id ${id}` : '✓ Clipped to Feed', 'success');
        setTimeout(() => window.close(), 3000);
      } else {
        const err = response ? response.error : 'Ingest failed';
        setStatus(`Error: ${err}`, 'error');
      }
    });
  }

  btnClipLink.addEventListener('click', async () => {
    await performIngest({ url: tab.url, title: tab.title });
  });

  btnClipPage.addEventListener('click', async () => {
    try {
      const [{ result }] = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: () => ({
          html: document.documentElement.outerHTML,
          title: document.title,
          url: window.location.href
        })
      });

      await performIngest({
        url: result.url || tab.url,
        content: result.html,
        title: result.title || tab.title
      });
    } catch (err) {
      console.error('Extraction failed:', err);
      // Fallback: send link only if scripting blocked on tab
      await performIngest({ url: tab.url, title: tab.title });
    }
  });

  btnClipSelection.addEventListener('click', async () => {
    try {
      const [{ result }] = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: () => ({
          selection: window.getSelection().toString().trim(),
          title: document.title,
          url: window.location.href
        })
      });

      if (!result.selection) {
        setStatus('No text selected on page', 'error');
        return;
      }

      await performIngest({
        url: result.url || tab.url,
        content: result.selection,
        title: result.title || tab.title
      });
    } catch (err) {
      console.error('Selection extraction failed:', err);
      setStatus('Could not read selection from page', 'error');
    }
  });
});
