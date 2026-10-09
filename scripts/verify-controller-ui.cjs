// Browser interaction checks with synthetic responses. Usage: node SCRIPT EVIDENCE PLAYWRIGHT_PACKAGE
const { chromium } = require(process.argv[3] || 'playwright');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http'), assert = require('node:assert/strict');
const dist = path.resolve(__dirname, '../apps/desktop/dist');
const evidence = path.resolve(process.argv[2]); fs.mkdirSync(evidence, {recursive:true});
const server = http.createServer((req,res) => {
  const file = path.join(dist, req.url.startsWith('/__byok-api__/assets/') ? req.url.slice('/__byok-api__/'.length) : 'index.html');
  res.setHeader('content-type', file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html');
  res.end(fs.readFileSync(file));
});
let browser;
(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  browser = await chromium.launch({headless:true,channel:"msedge"});
  const page = await browser.newPage({viewport:{width:1200,height:850}});
  const errors=[]; page.on('pageerror', e=>errors.push(e.message));
  await page.addInitScript(() => Object.defineProperty(window,'__SUB2API_CONTROL_TOKEN__',{value:'synthetic-ui-token',writable:false}));
  const all=['low','medium','high','xhigh','max'];
  let models=[{model_hash:'gpt',display_name:'GPT · Research',model_id:'gpt-fixture',type:'openai',openai_endpoint:'/v1/responses',reasoning_effort:'high',anthropic_thinking_effort:null,allowed_reasoning_efforts:all,context_window_tokens:null,max_completion_tokens:null,thinking_budget_tokens:null}];
  let consent=false, ca='missing', integration='disabled', cursorRunning=false;
  let consentCalls=0, enableCalls=0, installationCancelled=false;
  let discoveryError=true, editorError=true;
  let usageCase='normal';
  const status = () => ({integration,ca,certificate_consent:consent,warnings:[],restart_required:cursorRunning});
  await page.route('**/__byok-api__/api/**', async route => {
    const req=route.request(); assert.equal(req.headers()['x-sub2api-control-token'],'synthetic-ui-token');
    const url=new URL(req.url()), endpoint=url.pathname.split('/api/')[1];
    const send=(body,status=200)=>route.fulfill({status,contentType:'application/json',body:JSON.stringify(body)});
    if(endpoint==='harness/cursor/status') return send(status());
    if(endpoint==='harness/cursor/ca/consent') {
      assert.deepEqual(req.postDataJSON(),{accepted:true,version:1});consentCalls++;consent=true;
      // Even a successful HTTP response must report trusted CA before proceeding.
      ca=consentCalls===1?'untrusted':'ready';return send(status());
    }
    if(endpoint==='harness/cursor/enabled') {
      enableCalls++;
      if(installationCancelled){installationCancelled=false;return send({message:'合成 Windows 安装取消'},400);}
      integration=req.postDataJSON().enabled?'enabled':'disabled';ca='ready';return send(status());
    }
    if(endpoint==='sub2api/connection') return send({base_url:'https://sub2api.example/v1',has_api_key:true});
    if(endpoint==='sub2api/models') {if(discoveryError){discoveryError=false;return send({message:'合成网络错误'},400);}return send([{id:'gpt-fixture'},{id:'claude-fixture'},{id:'gpt-second'}]);}
    if(endpoint.startsWith('models/') && req.method()==='PUT') {
      if(editorError){editorError=false;return send({message:'合成保存错误'},400);}
      const body=req.postDataJSON(); assert.deepEqual(body.allowed_reasoning_efforts,['low','xhigh']);assert.equal(body.reasoning_effort,'xhigh');
      models[0]={...models[0],...body};return send(models[0]);
    }
    if(endpoint==='models') {
      if(req.method()==='POST'){const body=req.postDataJSON();assert.equal(body.models.length,2);models.push(...body.models.map((m,i)=>({...m,model_hash:`new-${i}`,anthropic_thinking_effort:m.reasoning_effort})));}
      return send(models);
    }
    if(endpoint==='overview') {
      assert(url.searchParams.get('start_ms'));const now=Date.now();
      if(usageCase==='empty') return send({metrics:{llm_calls:0,successful_calls:0,failed_calls:0,token_usage:0,input_tokens:0,output_tokens:0,cache_read_tokens:0,cache_write_tokens:0},token_usage_granularity:'day',token_usage_series:[]});
      if(usageCase==='filtered') {
        assert.deepEqual(JSON.parse(url.searchParams.get('model_hashes')),['gpt']);
        return send({metrics:{llm_calls:1,successful_calls:1,failed_calls:0,token_usage:150,input_tokens:100,output_tokens:50,cache_read_tokens:0,cache_write_tokens:0},token_usage_granularity:'day',token_usage_series:[]});
      }
      return send({metrics:{llm_calls:128,successful_calls:126,failed_calls:2,token_usage:1840000,input_tokens:650000,output_tokens:230000,cache_read_tokens:850000,cache_write_tokens:110000},token_usage_granularity:'day',token_usage_series:Array.from({length:14},(_,i)=>({bucket_start_ms:now-(13-i)*86400000,input_tokens:(i%3+1)*12000,cache_read_tokens:i*4400,cache_write_tokens:i*1700,output_tokens:9000+i*1200}))});
    }
    return send({message:`Unexpected ${endpoint}`},404);
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/__byok-api__/`);
  await page.getByRole('dialog').waitFor();assert(await page.getByRole('button',{name:'同意并安装证书',exact:true}).isDisabled());
  await page.screenshot({path:path.join(evidence,'setup.png')});
  await page.getByRole('button',{name:'稍后',exact:true}).click();
  await page.getByRole('button',{name:'开启接管',exact:true}).click();
  await page.getByRole('dialog').waitFor();
  await page.getByRole('checkbox').check();await page.getByRole('button',{name:'同意并安装证书',exact:true}).click();
  await page.getByText('证书尚未完成安装，请完成 Windows 确认后重试。',{exact:true}).waitFor();
  assert.equal(enableCalls,0);
  await page.getByRole('button',{name:'同意并安装证书',exact:true}).click();
  await page.getByRole('dialog').waitFor({state:'hidden'});
  await page.getByText('接管已开启，现在可以打开 Cursor。',{exact:true}).waitFor();
  assert.equal(enableCalls,1);assert.equal(integration,'enabled');
  await page.getByRole('button',{name:'关闭并恢复',exact:true}).click();
  await page.getByText('Cursor 设置已恢复，证书保留供下次使用。',{exact:true}).waitFor();
  await page.getByRole('button',{name:'开启接管',exact:true}).click();
  await page.getByText('接管已开启，现在可以打开 Cursor。',{exact:true}).waitFor();
  assert.equal(consentCalls,2);assert.equal(enableCalls,3);
  await page.getByRole('button',{name:'关闭并恢复',exact:true}).click();
  await page.getByText('Cursor 设置已恢复，证书保留供下次使用。',{exact:true}).waitFor();
  ca='untrusted';installationCancelled=true;
  await page.getByRole('button',{name:'刷新状态',exact:true}).click();
  await page.getByText('证书待安装 · 已授权',{exact:true}).waitFor();
  await page.getByRole('button',{name:'开启接管',exact:true}).click();
  await page.getByText('合成 Windows 安装取消',{exact:true}).waitFor();
  assert.equal(await page.getByRole('dialog').count(),0);assert.equal(consentCalls,2);assert.equal(integration,'disabled');
  await page.getByRole('button',{name:'开启接管',exact:true}).click();
  await page.getByText('接管已开启，现在可以打开 Cursor。',{exact:true}).waitFor();
  assert.equal(enableCalls,6);assert.equal(consentCalls,2);
  await page.getByRole('button',{name:'关闭并恢复',exact:true}).click();
  await page.getByText('Cursor 设置已恢复，证书保留供下次使用。',{exact:true}).waitFor();
  cursorRunning=true;
  await page.getByRole('button',{name:'开启接管',exact:true}).click();
  await page.getByRole('alert').getByText('请保存工作并完全退出 Cursor，再更改接管状态。',{exact:true}).waitFor();
  assert.equal(enableCalls,7);assert.equal(await page.getByRole('dialog').count(),0);
  cursorRunning=false;
  await page.getByRole('button',{name:'编辑配置',exact:true}).click();
  for(const effort of ['medium','high','max']) await page.getByRole('checkbox',{name:effort,exact:true}).uncheck();
  assert.equal(await page.getByLabel('默认强度',{exact:true}).inputValue(),'');
  await page.getByLabel('默认强度',{exact:true}).selectOption('xhigh');
  await page.screenshot({path:path.join(evidence,'model-editor.png')});
  await page.getByRole('button',{name:'保存模型',exact:true}).click();await page.getByRole('dialog').getByText('合成保存错误').waitFor();
  await page.getByRole('button',{name:'保存模型',exact:true}).click();await page.getByRole('dialog').waitFor({state:'hidden'});
  await page.getByRole('button',{name:'获取模型列表'}).click();await page.getByText('合成网络错误').waitFor();
  await page.getByRole('button',{name:'重新获取'}).click();await page.getByText('claude-fixture',{exact:true}).waitFor();
  assert(await page.getByRole('checkbox',{name:/gpt-fixture.*已添加/}).isDisabled());
  await page.getByLabel('搜索模型',{exact:true}).fill('claude');await page.getByRole('button',{name:'选择搜索结果'}).click();
  await page.getByLabel('搜索模型',{exact:true}).fill('');await page.getByRole('checkbox',{name:'gpt-second',exact:true}).check();
  await page.screenshot({path:path.join(evidence,'model-discovery.png')});
  await page.getByRole('button',{name:'添加所选 2 个模型'}).click();await page.getByRole('dialog').waitFor({state:'hidden'});
  assert.equal(models.length,3); assert.equal(models[1].type,'anthropic');
  await page.getByRole('button',{name:'关闭操作提示',exact:true}).click();
  assert.equal(await page.getByRole('alert').count(),0);
  await page.locator('main').evaluate(el=>el.scrollTo(0,0));
  await page.screenshot({path:path.join(evidence,'models.png')});
  await page.getByRole('button',{name:'用量统计',exact:true}).click();await page.getByText('1.8M',{exact:true}).waitFor();
  const cacheCard=page.locator('.metric').filter({has:page.getByText('缓存读取 Token',{exact:true})});
  const rateCard=page.locator('.metric').filter({has:page.getByText('缓存读取率',{exact:true})});
  assert.equal(await page.locator('.metric').count(),6);
  await cacheCard.getByText('850K',{exact:true}).waitFor();await rateCard.getByText('52.8%',{exact:true}).waitFor();
  await page.screenshot({path:path.join(evidence,'usage.png')});
  usageCase='empty';await page.getByRole('button',{name:'刷新',exact:true}).click();
  await rateCard.getByText('—',{exact:true}).waitFor();await cacheCard.getByText('0',{exact:true}).waitFor();
  usageCase='filtered';await page.getByLabel('统计模型',{exact:true}).selectOption('gpt');
  await rateCard.getByText('0.0%',{exact:true}).waitFor();await cacheCard.getByText('0',{exact:true}).waitFor();
  await page.setViewportSize({width:900,height:700});await page.getByRole('button',{name:'模型与连接',exact:true}).click();
  assert(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth));
  await page.screenshot({path:path.join(evidence,'models-compact.png')});
  assert.deepEqual(errors,[]);console.log('PASS: consent-to-enable continuation, persistent consent repair/retry, Cursor guard, effort multi-select/default, editor errors, discovery retry/search/bulk import, usage and layout.');
})().catch(e=>{console.error(e);process.exitCode=1;}).finally(async()=>{if(browser)await browser.close();server.close();});
