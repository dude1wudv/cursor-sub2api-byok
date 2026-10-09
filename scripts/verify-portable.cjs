// Isolated native EXE acceptance. No live Cursor account or external provider requests.
// Requires a user to confirm the Windows install/delete dialogs for the dedicated CA.
// Usage: node verify-portable.cjs EXE EVIDENCE_DIR PLAYWRIGHT_PACKAGE
const { chromium } = require(process.argv[4] || 'playwright');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const cp = require('node:child_process');
const assert = require('node:assert/strict');
const net = require('node:net');
const exe = path.resolve(process.argv[2]);
const evidence = path.resolve(process.argv[3]);
fs.mkdirSync(evidence, { recursive: true });
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'Sub2API Native Space '));
const data = path.join(root, 'Controller Data'), cursor = path.join(root, 'Cursor Profile');
const settings = path.join(cursor, 'User', 'settings.json');
const journal = path.join(data, 'takeover-journal.dpapi');
const original = Buffer.from('\ufeff{\r\n // isolated original\r\n "editor.fontSize":17,\r\n "http.proxy":null\r\n}\r\n');
fs.mkdirSync(path.dirname(settings), { recursive: true });
fs.writeFileSync(settings, original);
const sentinel = path.join(path.dirname(settings), 'globalStorage', 'state.vscdb');
fs.mkdirSync(path.dirname(sentinel), {recursive:true});
fs.writeFileSync(sentinel, 'synthetic sentinel');
const args = ['--data-dir', data, '--cursor-user-data-dir', cursor];
function ps(command) { return cp.execFileSync('powershell.exe', ['-NoProfile', '-Command', command], { windowsHide:true, encoding:'utf8' }).trim(); }
function roots() { return JSON.parse(ps("$ErrorActionPreference='Stop'; $store=[System.Security.Cryptography.X509Certificates.X509Store]::new('Root','CurrentUser'); $store.Open('ReadOnly'); $thumbprints=@($store.Certificates | ForEach-Object Thumbprint | Sort-Object); $store.Close(); ConvertTo-Json -Compress -InputObject $thumbprints")); }
const baseline = roots();
const results = [];
let child, browser;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function waitFor(fn, label) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) { if (await fn()) return; await delay(100); }
  throw new Error(`Timed out: ${label}`);
}
function restore() {
  return cp.spawnSync(exe, ['--restore', ...args], { windowsHide:true, timeout:120000 });
}
async function start() {
  const listener = net.createServer();
  await new Promise(r => listener.listen(0, '127.0.0.1', r));
  const port = listener.address().port;
  await new Promise(r => listener.close(r));
  child = cp.spawn(exe, args, { windowsHide:true, stdio:'ignore', env:{ ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1` } });
  await waitFor(async () => {
    try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout:1000 }); return true; } catch { return false; }
  }, 'native WebView2');
  let page;
  await waitFor(() => { page=browser.contexts().flatMap(c => c.pages()).find(p => p.url().includes('/__byok-api__/')); return !!page; }, 'controller page');
  page.setDefaultTimeout(Number(process.env.NATIVE_TIMEOUT_MS || 120000));
  await page.getByRole('heading', { name:'Cursor 接管', exact:true }).waitFor();
  return page;
}
async function terminate() {
  if (child && child.exitCode === null && child.signalCode === null) { child.kill(); await waitFor(() => child.exitCode !== null || child.signalCode !== null, 'forced interruption'); }
  child=null;
  if (browser) { await Promise.race([browser.close().catch(() => {}),delay(3000)]); browser=null; }
}
(async () => {
  assert(!ps('tasklist /FI "IMAGENAME eq Cursor.exe" /NH /FO CSV').toLowerCase().includes('cursor.exe'), 'Close real Cursor before isolated takeover verification');
  let page = await start();
  await page.getByText('未接管', {exact:true}).waitFor();
  assert.deepEqual(fs.readFileSync(settings), original);
  assert.deepEqual(roots(), baseline);
  await page.getByLabel('Base URL', {exact:true}).fill('http://127.0.0.1:9/v1');
  await page.getByLabel('API Key', {exact:true}).fill('synthetic-native-acceptance-key');
  await page.getByRole('button', {name:'保存连接',exact:true}).click();
  await page.getByText('连接已保存，所有模型已同步。').waitFor();
  await page.getByRole('button', {name:'添加模型'}).click();
  await page.getByLabel('显示名称', {exact:true}).fill('GPT fixture');
  await page.getByLabel('模型 ID', {exact:true}).fill('gpt-fixture');
  await page.getByRole('button', {name:'保存模型',exact:true}).click();
  await page.getByText('GPT fixture', {exact:true}).waitFor();
  await page.screenshot({path:path.join(evidence,'controller-disabled.png')});
  const descriptor = await page.evaluate(() => {
    const d=Object.getOwnPropertyDescriptor(window,'__SUB2API_CONTROL_TOKEN__');
    return {exists:!!d,writable:d?.writable,configurable:d?.configurable};
  });
  assert.deepEqual(descriptor,{exists:true,writable:false,configurable:false});
  await page.getByRole('button', {name:'开启接管',exact:true}).click();
  await page.getByText('接管中', {exact:true}).waitFor();
  assert(fs.existsSync(journal));assert(!fs.readFileSync(settings).equals(original));
  assert.equal(roots().filter(t=>!baseline.includes(t)).length,1);
  await page.screenshot({path:path.join(evidence,'controller-enabled.png')});
  await page.getByRole('button', {name:'关闭并恢复',exact:true}).click();
  await page.getByText('未接管', {exact:true}).waitFor();
  assert.deepEqual(fs.readFileSync(settings),original);assert.deepEqual(roots(),baseline);assert(!fs.existsSync(journal));
  console.log('Checkpoint:', results.length + 1); results.push('Native UI enable/disable: settings bytes and CurrentUser Root set restored exactly; token readonly.');
  // Simulate process termination with an active durable journal, then recover offline.
  await page.getByRole('button', {name:'开启接管',exact:true}).click();
  await page.getByText('接管中', {exact:true}).waitFor();
  const savedJournal=fs.readFileSync(journal);
  await terminate();
  fs.writeFileSync(journal, Buffer.from('synthetic-corrupt-journal'));
  assert.notEqual(restore().status,0);assert.equal(fs.readFileSync(journal).toString(),'synthetic-corrupt-journal');
  fs.writeFileSync(journal,savedJournal);
  assert.equal(restore().status,0);assert.deepEqual(fs.readFileSync(settings),original);assert.deepEqual(roots(),baseline);assert(!fs.existsSync(journal));
  assert.equal(restore().status,0);
  console.log('Checkpoint:', results.length + 1); results.push('Forced interruption + offline --restore: corrupted journal retained/nonzero; valid journal restored/idempotent.');
  // Startup recovery must not restart takeover. Closing the real window must restore before exit.
  page=await start();
  await page.getByText('未接管', {exact:true}).waitFor();
  await page.getByRole('button', {name:'开启接管',exact:true}).click();
  await page.getByText('接管中', {exact:true}).waitFor();
  await terminate();
  page=await start();
  await page.getByText('未接管', {exact:true}).waitFor();
  assert.deepEqual(fs.readFileSync(settings),original);assert.deepEqual(roots(),baseline);
  await page.getByRole('button', {name:'开启接管',exact:true}).click();
  await page.getByText('接管中', {exact:true}).waitFor();
  const closeRequested=ps(`(Get-Process -Id ${child.pid}).CloseMainWindow()`);
  assert.equal(closeRequested,'True');
  await waitFor(()=>child.exitCode !== null,'window close recovery');child=null;browser=null;
  assert.deepEqual(fs.readFileSync(settings),original);assert.deepEqual(roots(),baseline);assert(!fs.existsSync(journal));
  assert.equal(fs.readFileSync(sentinel).toString(),'synthetic sentinel');
  console.log('Checkpoint:', results.length + 1); results.push('Startup recovery and native window X: restored before exit; state.vscdb sentinel unchanged.');
  for (const name of ['cursor-sub2api.db','cursor-sub2api.db-wal']) { // gitleaks:allow -- database filenames, no credential
    const file=path.join(data,name);
    if(fs.existsSync(file)) assert(!fs.readFileSync(file).includes(Buffer.from('synthetic-native-acceptance-key')));
  }
  for (const name of fs.readdirSync(path.join(data,'logs'))) assert(!fs.readFileSync(path.join(data,'logs',name)).includes(Buffer.from('synthetic-native-acceptance-key')));
  assert(!fs.readFileSync(path.join(data,'ca','ca.key.dpapi')).includes(Buffer.from('PRIVATE KEY')));
  console.log('Checkpoint:', results.length + 1); results.push('No plaintext fixture Key in DB/WAL/logs; CA private key is DPAPI protected.');
  const report={passed:true,temporaryDirectory:root,results,realCursorGptClaudeMcp:'NOT VERIFIED: no dedicated account/key supplied'};
  fs.writeFileSync(path.join(evidence,'native-verification.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report));
})().catch(async error => {
  if(browser) { const p=browser.contexts().flatMap(c=>c.pages())[0]; if(p) await p.screenshot({path:path.join(evidence,"failure.png")}).catch(()=>{}); }
  await terminate();
  const recovery=restore();
  const report={passed:false,error:error.message,recoveryExit:recovery.status,certificateSetRestored:JSON.stringify(roots())===JSON.stringify(baseline),temporaryDirectory:root,results};
  fs.writeFileSync(path.join(evidence,'native-verification.json'),JSON.stringify(report,null,2));
  console.error(JSON.stringify(report));process.exitCode=1;
});
