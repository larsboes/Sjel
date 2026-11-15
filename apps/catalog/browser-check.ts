/** Browser acceptance against Rust-served static assets and fictional classified reads. */
import { chromium } from 'playwright';
import { strict as assert } from 'node:assert';
import { resolve } from 'node:path';
import { mkdirSync } from 'node:fs';
import fixture from './fixtures/projects.json';

const root = resolve(import.meta.dir, '../..');
const artifactDir = resolve(import.meta.dir, 'dist/acceptance');
mkdirSync(artifactDir, { recursive: true });
const server = Bun.spawn(['sjel', 'cargo', 'test', '--locked', '-p', 'sjel-status', 'catalog_browser_fixture', '--', '--ignored', '--nocapture'], { cwd: root, stdout: 'pipe', stderr: 'inherit' });
const decoder = new TextDecoder();
const reader = server.stdout.getReader();
let output = '';
let base: URL | undefined;
let browser: Awaited<ReturnType<typeof chromium.launch>> | undefined;
try {
  const deadline = setTimeout(() => { void reader.cancel('fixture startup timeout'); }, 120_000);
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) throw new Error(`Rust fixture exited before readiness: ${output}`);
      output += decoder.decode(value, { stream: true });
      const match = output.match(/CATALOG_FIXTURE=(http:\/\/127\.0\.0\.1:\d+\/catalog\/)/);
      if (match) { base = new URL(match[1]); break; }
    }
  } finally { clearTimeout(deadline); }
  browser = await chromium.launch({ headless: true, ...(process.env.SJEL_BROWSER_EXECUTABLE ? { executablePath: process.env.SJEL_BROWSER_EXECUTABLE } : { channel: 'chrome' }) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const failures: string[] = [];
  const requests: { url: string; method: string }[] = [];
  page.on('pageerror', error => failures.push(error.message));
  page.on('request', request => requests.push({ url: request.url(), method: request.method() }));
  const projectsRoute = /\/projects\/api\/projects(?:\?.*)?$/;
  const response = await page.goto(base.href);
  assert.equal(response?.status(), 200);
  assert.match(response?.headers()['content-security-policy'] ?? '', /script-src 'self'/);
  await page.getByRole('list', { name: 'Projects' }).waitFor();
  assert.equal(await page.locator('li').count(), 2);
  await page.screenshot({ path: resolve(artifactDir, 'projects-desktop.png') });

  const project = page.getByRole('button', { name: 'Kitchen renovation c1', exact: true });
  await project.focus();
  await page.keyboard.press('Enter');
  await page.getByRole('list', { name: 'Records' }).waitFor();
  assert.equal(await page.locator('li').count(), 2);
  assert.ok(requests.some(request => request.url.includes('content=false')));
  await page.getByRole('button', { name: 'Kitchen renovation c1', exact: true }).click();
  await page.getByRole('heading', { name: 'Kitchen renovation', exact: true }).waitFor();
  assert.ok((await page.locator('dd').allTextContents()).some(value => value.includes('<script>Never execute record content</script>')));
  assert.equal(await page.locator('main script').count(), 0);
  await page.setViewportSize({ width: 375, height: 812 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), true);
  await page.screenshot({ path: resolve(artifactDir, 'record-mobile.png') });
  await page.getByRole('button', { name: '← Back', exact: true }).click();
  await page.getByRole('list', { name: 'Records' }).waitFor();
  await page.getByRole('button', { name: '← Back', exact: true }).click();
  await page.getByRole('list', { name: 'Projects' }).waitFor();
  await page.getByRole('button', { name: /Kitchen renovation c2/ }).click();
  await page.getByRole('list', { name: 'Records' }).waitFor();
  assert.equal(await page.locator('li').count(), 1);
  await page.getByRole('button', { name: /^Earlier plan c2/ }).click();
  await page.getByRole('heading', { name: 'Earlier plan', exact: true }).waitFor();

  await page.route(projectsRoute, route => route.fulfill({ status: 503, body: 'Unavailable' }));
  await page.goto(base.href);
  await page.getByRole('alert').waitFor();
  assert.match(await page.getByRole('alert').innerText(), /root: read failed \(503\)/);
  await page.unroute(projectsRoute);
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await page.getByRole('list', { name: 'Projects' }).waitFor();

  await page.route(projectsRoute, route => route.fulfill({ json: { projects: [] } }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByText('No projects found.', { exact: true }).waitFor();
  await page.unroute(projectsRoute);
  await page.route(projectsRoute, route => route.fulfill({ json: { projects: Array.from({ length: 55 }, (_, i) => ({ ...fixture.projects[0], name: `Project ${i + 1}` })) } }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByRole('navigation', { name: 'List pages' }).waitFor();
  assert.equal(await page.locator('li').count(), 50);
  await page.getByRole('button', { name: 'Next', exact: true }).click();
  assert.equal(await page.locator('li').count(), 5);
  await page.unroute(projectsRoute);
  await page.route(projectsRoute, route => route.fulfill({ json: { projects: [{ ...fixture.projects[0], data_class: undefined }] } }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByRole('alert').waitFor();
  assert.match(await page.getByRole('alert').innerText(), /root: invalid classified object/);
  assert.equal(await page.locator('li').count(), 0);
  assert.deepEqual(failures, []);
  assert.ok(requests.every(request => request.method === 'GET' && new URL(request.url).origin === base!.origin));
  const missing = await fetch(new URL('/catalog/assets/missing.js', base));
  assert.equal(missing.status, 404);
  console.log(`Browser acceptance passed: keyboard navigation, list/detail/back, archived identity, omitted list content, escaped text, narrow layout, error/retry, empty, pagination, invalid classification, local GET-only traffic. Screenshots: ${artifactDir}`);
} finally {
  await browser?.close();
  if (base) await fetch(new URL('/__fixture/stop', base), { method: 'POST' }).catch(() => {});
  else server.kill();
  const exitCode = await server.exited;
  await reader.cancel();
  if (exitCode !== 0) throw new Error(`Rust fixture failed (${exitCode})`);
}
