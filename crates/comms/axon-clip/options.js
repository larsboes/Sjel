document.addEventListener('DOMContentLoaded', async () => {
  const serverUrlInput = document.getElementById('server-url');
  const apiSecretInput = document.getElementById('api-secret');
  const form = document.getElementById('options-form');
  const statusDiv = document.getElementById('status');

  // Load existing settings
  const settings = await chrome.storage.local.get({
    serverUrl: 'http://127.0.0.1:8083',
    apiSecret: ''
  });

  serverUrlInput.value = settings.serverUrl;
  apiSecretInput.value = settings.apiSecret;

  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const serverUrl = serverUrlInput.value.trim().replace(/\/+$/, '');
    const apiSecret = apiSecretInput.value.trim();

    await chrome.storage.local.set({ serverUrl, apiSecret });

    statusDiv.className = 'status-bar success';
    statusDiv.textContent = '✓ Settings saved successfully!';
    setTimeout(() => {
      statusDiv.className = 'status-bar';
    }, 3000);
  });
});
