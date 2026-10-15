const pw = require('D:/app/nvm/node_global/node_modules/@playwright/cli/node_modules/playwright');

(async () => {
  const browser = await pw.chromium.launch({ headless: true, channel: 'msedge' });
  const page = await browser.newPage({ viewport: { width: 1500, height: 950 } });
  const logs = [];
  page.on('console', m => logs.push('[console.' + m.type() + '] ' + m.text()));
  page.on('pageerror', e => logs.push('[pageerror] ' + (e.stack || e.message)));
  page.on('requestfailed', r => logs.push('[reqfail] ' + r.url() + ' ' + JSON.stringify(r.failure())));
  page.on('response', r => {
    const u = r.url();
    if (u.includes('user-info') || u.includes('user-action') || u.includes('/users') || u.includes('grants')) {
      logs.push('[resp] ' + r.status() + ' ' + u);
    }
  });

  await page.goto('http://127.0.0.1:20362/#/main', { waitUntil: 'load' });
  await page.waitForTimeout(3500);
  await page.screenshot({ path: 'd:/develop/workspace/product/DBMind/_uitest/repro-1-loaded.png' });
  console.log('=== STEP1 loaded ===');
  console.log(logs.join('\n'));

  // 先在左侧树上点中「公司Mysql」连接，再打开用户管理
  const connNode = page.locator('text=公司Mysql').first();
  await connNode.click().catch(e => console.log('click conn failed: ' + e.message));
  await page.waitForTimeout(1500);
  const mgmtBtn = page.locator('text=用户管理').first();
  console.log('mgmt count=' + (await mgmtBtn.count()));
  await mgmtBtn.click().catch(e => console.log('click 用户管理 failed: ' + e.message));
  await page.waitForTimeout(2500);
  await page.screenshot({ path: 'd:/develop/workspace/product/DBMind/_uitest/repro-2-mgmt.png' });

  // 用户管理页里若需选连接则选择
  const sel = page.locator('.el-select').first();
  if (await sel.count()) {
    await sel.click().catch(e => console.log('click select failed: ' + e.message));
    await page.waitForTimeout(800);
    const opt = page.locator('.el-select-dropdown__item', { hasText: '公司Mysql' }).first();
    if (await opt.count()) {
      await opt.click();
      await page.waitForTimeout(2500);
      console.log('=== connection selected ===');
    } else {
      console.log('=== option 公司Mysql not found (可能已默认选中) ===');
    }
  } else {
    console.log('=== no el-select found ===');
  }
  await page.screenshot({ path: 'd:/develop/workspace/product/DBMind/_uitest/repro-3-conn.png' });

  // 先试「新增用户」做鉴别
  const createBtn = page.locator('button', { hasText: '新增用户' }).first();
  logs.length = 0;
  console.log('=== clicking 新增用户 ===');
  await createBtn.click().catch(e => console.log('click create failed: ' + e.message));
  await page.waitForTimeout(2500);
  console.log('=== logs after create click ===');
  console.log(logs.join('\n'));
  await page.screenshot({ path: 'd:/develop/workspace/product/DBMind/_uitest/repro-3b-create.png' });

  // 装页面内钩子：捕获所有 fetch/XHR、未处理 rejection、错误
  await page.evaluate(() => {
    window.__events = [];
    const push = (s) => window.__events.push(s);
    window.addEventListener('unhandledrejection', e => push('REJ: ' + ((e.reason && e.reason.stack) || e.reason)));
    window.addEventListener('error', e => push('ERR: ' + e.message + ' @' + e.filename + ':' + e.lineno));
    const of = window.fetch;
    if (of) window.fetch = function (...a) { push('FETCH ' + a[0]); return of.apply(this, a); };
    const oo = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function (m, u) { push('XHR ' + m + ' ' + u); return oo.apply(this, arguments); };
  });

  // 点 demo1 行的「编辑 / 授权」
  const row = page.locator('tr', { hasText: 'demo1' }).first();
  const editBtn = row.locator('button', { hasText: /编辑/ }).first();
  if (await editBtn.count()) {
    logs.length = 0;
    console.log('=== clicking edit for demo1 ===');
    await editBtn.click();
    await page.waitForTimeout(4000);
    console.log('=== logs after click ===');
    console.log(logs.join('\n'));
    const ev = await page.evaluate(() => (window.__events || []).join('\n---\n'));
    console.log('=== in-page events ===');
    console.log(ev || '(empty)');
    const toasts = await page.evaluate(() => Array.from(document.querySelectorAll('.el-message, .el-notification')).map(n => n.innerText).join(' | '));
    console.log('=== toasts ===');
    console.log(toasts || '(none)');
  } else {
    console.log('=== edit button for demo1 not found ===');
  }
  await page.screenshot({ path: 'd:/develop/workspace/product/DBMind/_uitest/repro-4-after-click.png' });

  // 页面上是否出现了编辑页签
  const tabText = await page.evaluate(() => document.body.innerText.slice(0, 3000));
  console.log('=== body text head ===');
  console.log(tabText);

  await browser.close();
})().catch(e => { console.error('FATAL: ' + (e.stack || e.message)); process.exit(1); });
