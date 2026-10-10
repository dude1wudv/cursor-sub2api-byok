// Synthetic browser checks for update preferences, model-filter layout and CC Switch pricing sync.
// Usage: node scripts/verify-updates-layout.cjs tmp/validation-rc2 C:/path/to/playwright
const { chromium } = require(process.argv[3] || 'playwright');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http'), assert = require('node:assert/strict');
const dist = path.resolve(__dirname, '../apps/desktop/dist');
const evidence = path.resolve(process.argv[2] || 'tmp/validation-rc2'); fs.mkdirSync(evidence, {recursive:true});
const server = http.createServer((req,res) => {
  const file = path.join(dist, req.url.startsWith('/__byok-api__/assets/') ? req.url.slice('/__byok-api__/'.length) : 'index.html');
  res.setHeader('content-type', file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html');
  res.end(fs.readFileSync(file));
});
const now = Date.now();
const modelNames = [
  'claude-haiku-5-5', 'claude-opus-5', 'claude-sonnet-5-5', 'deepseek',
  'deepseek-v4.1-flash', 'deepseek-v4.1-flash-fast', 'gpt-6-astra', 'gpt-6-luna',
  'gpt-6.1-sol', 'z-ai', 'glm-5.3-flash',
  'model-with-an-extremely-long-identifier-that-has-no-spaces-and-must-wrap-within-the-available-label-width-0123456789abcdefghijklmnopqrstuvwxyz',
];
const models = modelNames.map((name,index) => ({
  model_hash:`hash-${index}`, display_name:name, model_id:name, type:'openai',
  openai_endpoint:'/v1/responses', reasoning_effort:'high', anthropic_thinking_effort:null,
  allowed_reasoning_efforts:['low','medium','high','xhigh','max'], context_window_tokens:null,
  max_completion_tokens:null, thinking_budget_tokens:null, sort_order:index, group_name:null,
}));
let updateStatus = {
  current_version:'0.3.0-rc.2', preferences:{automatic:true,include_prereleases:true}, checked_at_ms:now,
  error:null, update_available:true,
  latest:{version:'0.4.0-rc.1',prerelease:true,release_url:'https://example.invalid/releases/tag/v0.4.0-rc.1',download_url:'https://example.invalid/downloads/app.zip'},
};
let updateGets=0, updatePosts=0, updatePuts=[], failManualCheck=false;
const usageQueries=[];
const pricingSettings={currency:'USD',models:{'hash-0':{input_per_million:1,output_per_million:2,cache_read_per_million:0.2,cache_write_per_million:0.4}}};
const baseSync={settings:{enabled:true,source_path:'C:\\Users\\test\\.cc-switch\\model-pricing.json'},checked_at_ms:now,synced_at_ms:now,stale:false,error:null,matched:{'hash-0':'claude-haiku-5-5'},unmatched:[],ambiguous:[]};
let pricingSync={...baseSync};
const syncPuts=[];
let browser;
(async () => {
  await new Promise(resolve => server.listen(0,'127.0.0.1',resolve));
  browser=await chromium.launch({headless:true,channel:'msedge'});
  const page=await browser.newPage({viewport:{width:1200,height:850}});
  const errors=[]; page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(() => Object.defineProperty(window,'__SUB2API_CONTROL_TOKEN__',{value:'synthetic-ui-token',writable:false}));
  await page.route('**/__byok-api__/api/**', async route => {
    const req=route.request(); assert.equal(req.headers()['x-sub2api-control-token'],'synthetic-ui-token');
    const url=new URL(req.url()), endpoint=url.pathname.split('/api/')[1];
    const send=(body,status=200)=>route.fulfill({status,contentType:'application/json',body:JSON.stringify(body)});
    if(endpoint==='harness/cursor/status') return send({integration:'disabled',ca:'ready',certificate_consent:true,warnings:[],restart_required:false,settings_path:'C:\\Synthetic\\settings.json',proxy_url:'http://127.0.0.1:43121',ca_sha256:null});
    if(endpoint==='sub2api/connection') return send({base_url:'https://sub2api.example/v1',has_api_key:true});
    if(endpoint==='models') return send(models);
    if(endpoint==='updates' && req.method()==='GET') { updateGets++; return send(updateStatus); }
    if(endpoint==='updates' && req.method()==='POST') {
      updatePosts++;
      if(failManualCheck) return send({message:'合成更新源暂不可用'},503);
      updateStatus={...updateStatus,checked_at_ms:Date.now(),error:null}; return send(updateStatus);
    }
    if(endpoint==='updates' && req.method()==='PUT') {
      const body=req.postDataJSON(); updatePuts.push(body);
      updateStatus={...updateStatus,preferences:body}; return send(updateStatus);
    }
    if(endpoint==='overview') {
      usageQueries.push(Object.fromEntries(url.searchParams));
      return send({metrics:{llm_calls:3,successful_calls:3,failed_calls:0,token_usage:900,input_tokens:400,output_tokens:300,cache_read_tokens:100,cache_write_tokens:100},token_usage_granularity:'day',token_usage_series:[],calendar:[],timezone:'UTC'});
    }
    if(endpoint==='pricing') {
      if(req.method()==='GET') return send(pricingSettings);
      return send(req.postDataJSON());
    }
    if(endpoint==='pricing/estimate') return send({currency:'USD',amount:0.01,covered_tokens:900,unpriced_tokens:0,unknown_usage_calls:0,models:[{model_hash:'hash-0',display_name:modelNames[0],calls:3,recorded_tokens:900,unknown_usage_calls:0,amount:0.01}]});
    if(endpoint==='pricing/sync' && req.method()==='GET') return send(pricingSync);
    if(endpoint==='pricing/sync' && req.method()==='PUT') {
      const {enabled,source_path}=req.postDataJSON();
      syncPuts.push({enabled,source_path});
      pricingSync={...baseSync,settings:{enabled,source_path},checked_at_ms:Date.now(),synced_at_ms:now,
        stale:enabled,error:enabled?'合成价格文件有未匹配模型':null,
        unmatched:enabled?['unknown-price-model']:[],ambiguous:enabled?['ambiguous-price-model']:[]};
      return send(pricingSync);
    }
    return send({message:`Unexpected ${req.method()} ${endpoint}`},404);
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/__byok-api__/`);
  await page.getByRole('button',{name:'关于',exact:true}).click();
  await page.getByRole('heading',{name:'软件更新',exact:true}).waitFor();
  await page.getByText('发现新版本 v0.4.0-rc.1',{exact:true}).waitFor();
  assert(updateGets>=1); assert.equal(updatePosts,1,'automatic startup check should be singleflight even if React effects issue duplicate GETs');
  assert.equal(await page.getByRole('link',{name:'版本说明 ↗'}).getAttribute('href'),updateStatus.latest.release_url);
  assert.equal(await page.getByRole('link',{name:'下载 Windows ZIP ↗'}).getAttribute('href'),updateStatus.latest.download_url);
  await page.getByText('预发布',{exact:true}).waitFor();
  const automatic=page.getByRole('checkbox',{name:/启动时及运行期间每 6 小时自动检查/});
  await automatic.uncheck();
  await page.getByText('发现新版本 v0.4.0-rc.1',{exact:true}).waitFor();
  assert.deepEqual(updatePuts.at(-1),{automatic:false,include_prereleases:true});
  const postsBeforeReload=updatePosts;
  await page.reload(); await page.getByRole('button',{name:'关于',exact:true}).click();
  await page.getByRole('heading',{name:'软件更新',exact:true}).waitFor();
  await page.getByRole('checkbox',{name:/启动时及运行期间每 6 小时自动检查/}).waitFor({state:'visible'});
  assert.equal(await page.getByRole('checkbox',{name:/启动时及运行期间每 6 小时自动检查/}).isChecked(),false);
  assert.equal(updatePosts,postsBeforeReload,'disabled automatic updates must not POST after reload');
  failManualCheck=true;
  await page.getByRole('button',{name:'检查更新',exact:true}).click();
  await page.getByRole('alert').getByText('合成更新源暂不可用',{exact:false}).waitFor();
  assert.equal(await page.getByText(/当前无更高版本|当前已是最新版本/).count(),0,'failed manual check must not claim latest');
  assert.equal(updatePosts,postsBeforeReload+1);
  await page.screenshot({path:path.join(evidence,'updates-error.png')});

  await page.getByRole('button',{name:'用量统计',exact:true}).click();
  await page.getByText('模型筛选 · 未勾选时统计全部模型（包含历史记录）',{exact:true}).waitFor();
  for(const name of modelNames) await page.getByRole('checkbox',{name,exact:true}).waitFor();
  const selected=[modelNames[0],modelNames[5],modelNames[11]];
  for(const name of selected) await page.getByRole('checkbox',{name,exact:true}).check();
  await page.waitForTimeout(200);
  const expectedHashes=selected.map(name=>`hash-${modelNames.indexOf(name)}`);
  assert(usageQueries.some(query=>query.model_hashes===JSON.stringify(expectedHashes)),`expected model_hashes ${JSON.stringify(expectedHashes)}; received ${JSON.stringify(usageQueries)}`);
  for(const width of [780,940,1200,1920]) {
    await page.setViewportSize({width,height:850});
    const layout=await page.locator('.usage-model-options label').evaluateAll(labels=>labels.map(label=>{
      const input=label.querySelector('input'), span=label.querySelector('span');
      const parent=label.getBoundingClientRect(), box=input.getBoundingClientRect(), text=span.getBoundingClientRect();
      return {name:span.textContent, labelWidth:parent.width, labelClient:label.clientWidth,labelScroll:label.scrollWidth,
        boxWidth:box.width,boxHeight:box.height,boxFlex:getComputedStyle(input).flexShrink,
        textRight:text.right,parentRight:parent.right,textScroll:span.scrollWidth,textClient:span.clientWidth};
    }));
    assert.equal(layout.length,modelNames.length);
    for(const item of layout) {
      assert(item.boxWidth>=13,`${item.name}: checkbox shrank to ${item.boxWidth}px at ${width}px`);
      assert.equal(item.boxFlex,'0',`${item.name}: checkbox flex-shrink is not disabled`);
      assert(item.labelScroll<=item.labelClient,`${item.name}: label content overflows at ${width}px`);
      assert(item.textRight<=item.parentRight+0.5,`${item.name}: text escapes label at ${width}px`);
      assert(item.textScroll<=item.textClient,`${item.name}: text overflows its wrapping box at ${width}px`);
    }
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`page horizontal overflow at ${width}px`);
    if(width===940) await page.screenshot({path:path.join(evidence,'usage-940.png'),fullPage:true});
  }
  for(const name of selected) assert(await page.getByRole('checkbox',{name,exact:true}).isChecked(),`${name} selection changed during resizing`);
  await page.getByRole('button',{name:'配置模型单价',exact:true}).click();
  await page.getByLabel(`${modelNames[0]} 输入（非缓存）每百万Token价格`,{exact:true}).waitFor();
  assert(await page.getByLabel(`${modelNames[0]} 输入（非缓存）每百万Token价格`,{exact:true}).isDisabled(),'sync enabled must lock price inputs');
  assert(await page.getByRole('button',{name:'保存单价',exact:true}).isDisabled(),'sync enabled must lock save action');
  const sourcePathInput=page.getByLabel('CC Switch 价格文件路径');
  await sourcePathInput.fill('');
  const syncToggle=page.getByRole('checkbox',{name:/自动同步 CC Switch 单价/});
  await syncToggle.focus(); await page.keyboard.press('Space');
  await page.getByText('已关闭同步，保留最后有效单价，可手动编辑。',{exact:true}).waitFor();
  assert.deepEqual(syncPuts.at(-1),{enabled:false,source_path:baseSync.settings.source_path},'toggling sync must use the saved path, even when the path input is blank');
  assert.equal(await page.getByLabel(`${modelNames[0]} 输入（非缓存）每百万Token价格`,{exact:true}).isDisabled(),false);
  assert.equal(await page.getByRole('button',{name:'保存单价',exact:true}).isDisabled(),false);
  const customPath='C:\\Users\\test\\custom\\model-pricing.json';
  await sourcePathInput.fill(customPath);
  await page.getByRole('button',{name:'保存路径并刷新',exact:true}).click();
  await page.waitForFunction(path => document.querySelector('.pricing-sync p.hint')?.textContent?.includes(path), customPath);
  assert((await page.locator('.pricing-sync p.hint').first().innerText()).includes(customPath),`pricing path hint did not update; actual=${await page.locator('.pricing-sync p.hint').first().innerText()}`);
  assert.deepEqual(syncPuts.at(-1),{enabled:false,source_path:customPath},'the path button must independently submit the edited path');
  assert.equal(await syncToggle.isChecked(),false);
  await syncToggle.focus(); await page.keyboard.press('Space');
  await page.getByText('合成价格文件有未匹配模型').waitFor();
  assert.deepEqual(syncPuts.at(-1),{enabled:true,source_path:customPath},'re-enabling sync must use the saved path');
  await page.getByText(/unknown-price-model/).waitFor();
  await page.getByText(/ambiguous-price-model/).waitFor();
  assert(await page.getByLabel(`${modelNames[0]} 输入（非缓存）每百万Token价格`,{exact:true}).isDisabled());
  await page.screenshot({path:path.join(evidence,'pricing-sync-stale.png'),fullPage:true});
  assert.deepEqual(errors,[]);
  console.log(`PASS: update startup GET/automatic singleflight POST, saved automatic preference and reload, release/download/channel links, failed manual check wording; ${modelNames.length} model labels at 780/940/1200/1920px, preserved model_hashes, CC Switch lock/unlock and stale/unmatched/ambiguous state.`);
})().catch(e=>{console.error(e);process.exitCode=1;}).finally(async()=>{if(browser)await browser.close();server.close();});
