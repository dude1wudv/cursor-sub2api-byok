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
  let models=[{model_hash:'gpt',display_name:'GPT · Research',model_id:'gpt-fixture',type:'openai',openai_endpoint:'/v1/responses',reasoning_effort:'high',anthropic_thinking_effort:null,allowed_reasoning_efforts:all,context_window_tokens:null,max_completion_tokens:null,thinking_budget_tokens:null,sort_order:0,group_name:null}];
  let consent=false, ca='missing', integration='disabled', cursorRunning=false;
  let subscriptionInjected=false, subscriptionPending=false, subscriptionError=false;
  let consentCalls=0, enableCalls=0, installationCancelled=false;
  let discoveryError=true, editorError=true;
  let usageCase='normal';
  let network={ports:{proxy_port:43121,service_port:43122},outbound:{mode:'custom',address:'http://127.0.0.1:7890',auth_enabled:true,username:'synthetic-user',has_password:true},actual_service_port:43122,actual_proxy_port:null};
  let priceSettings={currency:'USD',models:{}};
  let pendingTests=new Map(), testPosts=[], testDeletes=[];
  let callQueries=[], diagnosticQueries=[], usageModelFilters=[], subscriptionCalls=[];
  const status = () => ({integration,ca,certificate_consent:consent,warnings:[],restart_required:cursorRunning,subscription_injected:subscriptionInjected,subscription_recovery_pending:subscriptionPending});
  await page.route('**/__byok-api__/api/**', async route => {
    const req=route.request(); assert.equal(req.headers()['x-sub2api-control-token'],'synthetic-ui-token');
    const url=new URL(req.url()), endpoint=url.pathname.split('/api/')[1];
    const send=(body,status=200)=>route.fulfill({status,contentType:'application/json',body:JSON.stringify(body)});
    if(endpoint==='updates' && req.method()==='GET') return send({current_version:'0.3.0-rc.2',preferences:{automatic:false,include_prereleases:true},checked_at_ms:null,error:null,latest:null,update_available:false});
    if(endpoint==='pricing/sync' && req.method()==='GET') return send({settings:{enabled:false,source_path:'C:\\Synthetic\\model-pricing.json'},checked_at_ms:null,synced_at_ms:null,stale:false,error:null,matched:{},unmatched:[],ambiguous:[]});
    if(endpoint==='harness/cursor/status') return send(status());
    if(endpoint==='harness/cursor/subscription' && req.method()==='PUT') {
      const body=req.postDataJSON();subscriptionCalls.push(body);
      if(subscriptionError && body.enabled) return send({message:'合成订阅运行时错误'},400);
      subscriptionInjected=body.enabled;subscriptionPending=body.enabled;
      return send(status());
    }
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
    if(endpoint.startsWith('models/') && endpoint.includes('/test/')) {
      const id=endpoint.split('/test/')[1];
      if(req.method()==='POST') {
        testPosts.push(endpoint);
        return await new Promise(resolve=>pendingTests.set(id,()=>{void send({message:'run was cancelled'},400).then(resolve);}));
      }
      if(req.method()==='DELETE') {
        testDeletes.push(endpoint); const cancel=pendingTests.get(id); if(cancel){pendingTests.delete(id);cancel();}
        return route.fulfill({status:204,body:''});
      }
    }
    if(endpoint.startsWith('models/') && endpoint!=='models/order' && req.method()==='PUT') {
      if(editorError){editorError=false;return send({message:'合成保存错误'},400);}
      const body=req.postDataJSON(); assert.ok(Array.isArray(body.allowed_reasoning_efforts),JSON.stringify(body));assert.deepEqual(body.allowed_reasoning_efforts,['low','xhigh']);assert.equal(body.reasoning_effort,'xhigh');assert.equal(body.group_name,'Core');
      models[0]={...models[0],...body};return send(models[0]);
    }
    if(endpoint==='models/order' && req.method()==='PUT') {
      const hashes=req.postDataJSON().model_hashes; assert.equal(hashes.length,models.length);
      models.sort((a,b)=>hashes.indexOf(a.model_hash)-hashes.indexOf(b.model_hash));models.forEach((m,i)=>m.sort_order=i);
      return send(models);
    }
    if(endpoint==='models') {
      if(req.method()==='POST'){const body=req.postDataJSON();if(body.models.length===2)models.push(...body.models.map((m,i)=>({...m,model_hash:`new-${i}`,sort_order:models.length+i,group_name:null,anthropic_thinking_effort:m.reasoning_effort})));else {assert.equal(body.models.length,1);const m=body.models[0];assert.equal(m.group_name,'Core');assert.equal(m.sort_order,models.length);models.push({...m,model_hash:'copy-gpt',anthropic_thinking_effort:m.reasoning_effort});}}
      return send(models);
    }
    if(endpoint==='llm-calls') {
      callQueries.push(Object.fromEntries(url.searchParams));
      const failed=url.searchParams.get('status')==='failed';
      const item=(index,status='completed')=>({call_id:`call-${index}`,created_at_ms:Date.now()-index*1000,display_name:'GPT · Research',model_id:'gpt-fixture',status,http_status:status==='error'?502:200,duration_ms:30,ttfb_ms:12,ttfr_ms:18,ttft_ms:20,input_tokens:100,output_tokens:50,cache_read_tokens:0,cache_write_tokens:0,request_id:'req-child',conversation_id:'conv-fixture',parent_request_id:'req-parent',parent_tool_call_id:'task-fixture',request_url:'https://sub2api.example/v1/responses',method:'POST',route:'sub2api',error_message:status==='error'?'上游身份验证失败':null,error_kind:status==='error'?'upstream_auth':null});
      let items=failed?[item(0,'error'),item(1,'cancelled')]:Array.from({length:25},(_,i)=>item((Number(url.searchParams.get('page')||1)-1)*25+i));
      const total=failed?2:26;
      return send({items,total,page:Number(url.searchParams.get('page')||1),limit:25});
    }
    if(endpoint==='route-diagnostics') {
      diagnosticQueries.push(Object.fromEntries(url.searchParams));
      const rows=[{id:102,created_at_ms:Date.now(),request_id:'req-child',conversation_id:'conv-fixture',parent_request_id:'req-parent',parent_tool_call_id:'task-fixture',method:'POST',path:'/v1/responses',route:'sub2api',stage:'upstream_response',http_status:502,duration_ms:30},{id:101,created_at_ms:Date.now()-1000,request_id:'req-parent',conversation_id:'conv-fixture',parent_request_id:null,parent_tool_call_id:null,method:'POST',path:'/v1/responses',route:'sub2api',stage:'connect',http_status:200,duration_ms:20}];
      const filtered=url.searchParams.get('request_id')?rows.filter(r=>r.request_id===url.searchParams.get('request_id')):rows;
      return send(filtered);
    }
    if(endpoint==='network') return send(network);
    if(endpoint==='network/proxy' && req.method()==='PUT') {
      const body=req.postDataJSON();assert.equal(body.address,'http://127.0.0.1:7891');assert.equal(body.username,'synthetic-user');assert.equal(body.password,'synthetic-new-password');assert.equal(body.has_password,undefined);
      network={...network,outbound:{...body,password:undefined,has_password:true}};return send(network);
    }
    if(endpoint==='network/ports' && req.method()==='PUT') {
      const body=req.postDataJSON();assert.deepEqual(body,{proxy_port:0,service_port:0});network={...network,ports:body};return send(network);
    }
    if(endpoint==='pricing' && req.method()==='GET') return send(priceSettings);
    if(endpoint==='pricing/estimate') return send({currency:'USD',amount:usageCase==='filtered'?0.001:1.2345,covered_tokens:1840000,unpriced_tokens:0,unknown_usage_calls:0,models:[{model_hash:'gpt',display_name:'GPT · Research',calls:128,recorded_tokens:1840000,unknown_usage_calls:0,amount:usageCase==='filtered'?0.001:1.2345}]});
    if(endpoint==='pricing' && req.method()==='PUT') {priceSettings=req.postDataJSON();return send(priceSettings);}
    if(endpoint==='overview') {
      assert(url.searchParams.get('start_ms'));const now=Date.now();
      if(usageCase==='empty') return send({metrics:{llm_calls:0,successful_calls:0,failed_calls:0,token_usage:0,input_tokens:0,output_tokens:0,cache_read_tokens:0,cache_write_tokens:0},token_usage_granularity:'day',token_usage_series:[],calendar:[]});
      if(usageCase==='filtered') {
        const hashes=JSON.parse(url.searchParams.get('model_hashes'));usageModelFilters.push(hashes);
        assert(hashes.every(hash=>['gpt','new-0'].includes(hash)));
        return send({metrics:{llm_calls:2,successful_calls:2,failed_calls:0,token_usage:200,input_tokens:100,output_tokens:50,cache_read_tokens:20,cache_write_tokens:30},token_usage_granularity:'day',token_usage_series:[],calendar:Array.from({length:3},(_,i)=>({date:`2026-10-0${7+i}`,end_ms:Date.parse(`2026-10-0${8+i}T00:00:00Z`),calls:i+1,input_tokens:100,cache_read_tokens:20,cache_write_tokens:30,output_tokens:50}))});
      }
      return send({metrics:{llm_calls:128,successful_calls:126,failed_calls:2,token_usage:1840000,input_tokens:650000,output_tokens:230000,cache_read_tokens:850000,cache_write_tokens:110000},token_usage_granularity:'day',token_usage_series:Array.from({length:14},(_,i)=>({bucket_start_ms:now-(13-i)*86400000,input_tokens:(i%3+1)*12000,cache_read_tokens:i*4400,cache_write_tokens:i*1700,output_tokens:9000+i*1200})),calendar:Array.from({length:14},(_,i)=>({date:new Date(now-(13-i)*86400000).toISOString().slice(0,10),end_ms:now-(12-i)*86400000,calls:i,input_tokens:1000,cache_read_tokens:500,cache_write_tokens:100,output_tokens:400}))});
    }
    return send({message:`Unexpected ${endpoint}`},404);
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/__byok-api__/`);
  await page.getByRole('dialog').waitFor();assert(await page.getByRole('button',{name:'同意并安装证书',exact:true}).isDisabled());
  await page.screenshot({path:path.join(evidence,'setup.png')});
  await page.getByRole('button',{name:'稍后',exact:true}).click();
  await page.getByRole('button',{name:'开启接管',exact:true}).click();
  await page.getByRole('dialog').waitFor();
  await page.getByRole('checkbox',{name:/允许持续保留专属证书/}).check();await page.getByRole('button',{name:'同意并安装证书',exact:true}).click();
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
  await page.locator('.model-card').first().getByRole('button',{name:'编辑',exact:true}).click();
  for(const effort of ['medium','high','max']) await page.getByRole('checkbox',{name:effort,exact:true}).uncheck();
  assert.equal(await page.getByLabel('默认强度',{exact:true}).inputValue(),'');
  await page.getByLabel('默认强度',{exact:true}).selectOption('xhigh');
  await page.getByLabel('分组',{exact:true}).fill('Core');
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
  // Copy and reorder stay within their assigned group; batch cancellation reaches every started POST.
  await page.getByRole('button',{name:'复制',exact:true}).first().click();
  await page.getByText('GPT · Research 副本',{exact:true}).waitFor();assert.equal(models.length,4);assert.equal(models[3].group_name,'Core');
  await page.getByRole('button',{name:'上移 GPT · Research 副本',exact:true}).click();
  assert.deepEqual(models.filter(m=>m.group_name==='Core').map(m=>m.model_hash),['copy-gpt','gpt']);
  await page.getByLabel('筛选模型分组',{exact:true}).selectOption('Core');
  assert.equal(await page.locator('.model-card').count(),2);
  await page.getByLabel('搜索已添加模型',{exact:true}).fill('副本');assert.equal(await page.locator('.model-card').count(),1);
  await page.getByLabel('搜索已添加模型',{exact:true}).fill('');
  await page.getByRole('checkbox',{name:'选择 GPT · Research',exact:true}).check();
  await page.getByRole('checkbox',{name:'选择 GPT · Research 副本'}).check();
  await page.getByRole('button',{name:'测试所选 · 产生用量',exact:true}).click();
  await page.getByText('正在测试…',{exact:true}).first().waitFor();
  await page.getByRole('button',{name:'取消全部测试',exact:true}).click();
  await page.getByText('已取消',{exact:true}).first().waitFor();
  assert.equal(testPosts.length,2);assert.equal(testDeletes.length,2);assert.equal(pendingTests.size,0);
  await page.getByLabel('筛选模型分组',{exact:true}).selectOption('');
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
  usageCase='filtered';await page.getByRole('checkbox',{name:'GPT · Research',exact:true}).check();await page.getByRole('checkbox',{name:'claude-fixture',exact:true}).check();
  await rateCard.getByText('13.3%',{exact:true}).waitFor();await cacheCard.getByText('20',{exact:true}).waitFor();
  assert(usageModelFilters.some(hashes=>JSON.stringify(hashes)===JSON.stringify(['gpt','new-0'])));
  await page.getByLabel('统计时间范围',{exact:true}).selectOption('custom');
  const dateInputs=page.locator('input[type=date]');assert.equal(await dateInputs.count(),2);
  await dateInputs.nth(0).fill('2026-10-07');await dateInputs.nth(1).fill('2026-10-09');
  await page.locator('.usage-calendar-day').first().waitFor();
  await page.getByRole('button',{name:'配置模型单价',exact:true}).click();
  const priceToggle=page.getByRole('checkbox',{name:/GPT · Research/}).last();await priceToggle.check();
  await page.getByLabel('GPT · Research 输入（非缓存）每百万Token价格',{exact:true}).fill('2.5');
  await page.getByLabel('GPT · Research 输出每百万Token价格',{exact:true}).fill('10');
  await page.getByLabel('GPT · Research 缓存读取每百万Token价格',{exact:true}).fill('0.25');
  await page.getByLabel('GPT · Research 缓存写入每百万Token价格',{exact:true}).fill('1');
  await page.getByRole('button',{name:'保存单价',exact:true}).click();
  await page.getByText('单价已保存，当前筛选区间已按新单价重新估算。',{exact:true}).waitFor();
  assert.equal(priceSettings.models.gpt.input_per_million,2.5);assert.equal(priceSettings.currency,'USD');
  await page.getByRole('button',{name:'调用记录',exact:true}).click();
  await page.getByText('第 1 页 · 共 26 条',{exact:true}).waitFor();
  await page.getByRole('button',{name:'下一页',exact:true}).click();await page.getByText('第 2 页 · 共 26 条',{exact:true}).waitFor();
  assert.equal(callQueries.at(-1).page,'2');
  await page.locator('.inspection-filters select').nth(1).selectOption('failed');
  await page.getByText('error / 502',{exact:true}).waitFor();await page.getByText('cancelled / 200',{exact:true}).waitFor();
  await page.locator('.inspection-filters select').nth(0).selectOption('gpt');
  await page.locator('.inspection-filters input').nth(2).fill('conv-fixture');
  await page.locator('.inspection-filters input[type=datetime-local]').nth(0).fill('2026-10-07T00:00');
  await page.locator('.inspection-filters input[type=datetime-local]').nth(1).fill('2026-10-09T00:00');
  await page.getByRole('button',{name:'查看',exact:true}).first().click();
  await page.getByText('请求详情',{exact:true}).waitFor();
  await page.getByRole('button',{name:'定位父请求及子请求',exact:true}).click();
  await page.getByRole('button',{name:'req-parent',exact:true}).waitFor();
  assert.equal(diagnosticQueries.at(-1).request_id,'req-parent');
  assert.equal(callQueries.at(-1).status,'failed');assert.equal(callQueries.at(-1).model_hash,'gpt');
  assert.equal(callQueries.at(-1).conversation_id,'conv-fixture');assert(Number(callQueries.at(-1).start_ms)<Number(callQueries.at(-1).end_ms));
  await page.getByRole('button',{name:'网络与账号',exact:true}).click();
  await page.getByRole('heading',{name:'应用出站代理',exact:true}).waitFor();
  await page.getByLabel('代理地址',{exact:true}).fill('http://127.0.0.1:7891');
  await page.getByLabel('用户名',{exact:true}).fill('synthetic-user');
  await page.getByLabel('密码',{exact:true}).fill('synthetic-new-password');
  await page.getByRole('button',{name:'保存出站设置',exact:true}).click();
  await page.getByText('出站代理已保存，新发起的应用 HTTP 请求使用新设置。',{exact:true}).waitFor();
  await page.getByPlaceholder('已用 DPAPI 保存；留空保留',{exact:true}).waitFor();
  const portInputs=page.locator('input[type=number]');await portInputs.nth(0).fill('0');await portInputs.nth(1).fill('0');
  await page.getByRole('button',{name:'保存端口',exact:true}).click();
  await page.getByText('端口已保存：管理端口在重启应用后生效；接管端口在下次开启接管时生效。',{exact:true}).waitFor();
  await page.getByRole('heading',{name:'临时订阅缓存',exact:true}).waitFor();
  await page.screenshot({path:path.join(evidence,'subscription-default.png')});
  const subscriptionButton=page.getByRole('button',{name:'启用临时缓存',exact:true});
  assert(await subscriptionButton.isDisabled());
  const subscriptionConsent=page.getByRole('checkbox',{name:/我了解这只是本地缓存/});
  await subscriptionConsent.check();assert.equal(await subscriptionButton.isDisabled(),false);
  await subscriptionButton.click();
  await page.getByText('等待恢复 · 官方刷新可能已更改缓存',{exact:true}).waitFor();
  await page.screenshot({path:path.join(evidence,'subscription-pending.png')});
  assert.deepEqual(subscriptionCalls[0],{enabled:true,consent:true});assert.equal(subscriptionInjected,true);assert.equal(subscriptionPending,true);
  await page.getByRole('button',{name:'关闭注入并恢复',exact:true}).click();
  await page.getByText('默认关闭 · 每次手动启用',{exact:true}).waitFor();
  assert.deepEqual(subscriptionCalls[1],{enabled:false,consent:false});assert.equal(subscriptionInjected,false);assert.equal(subscriptionPending,false);
  subscriptionError=true;
  await subscriptionConsent.check();await subscriptionButton.click();
  await page.getByText('合成订阅运行时错误',{exact:true}).waitFor();
  await page.screenshot({path:path.join(evidence,'subscription-error.png')});
  assert.deepEqual(subscriptionCalls[2],{enabled:true,consent:true});
  subscriptionError=false;
  await page.setViewportSize({width:900,height:700});await page.getByRole('button',{name:'模型与连接',exact:true}).click();
  assert(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth));
  await page.screenshot({path:path.join(evidence,'models-compact.png')});
  assert.deepEqual(errors,[]);console.log('PASS: consent/restore regressions, model editing/discovery/copy/group/order/batch cancel, usage/calendar/pricing, call filters/pagination/diagnostic relation, network settings, subscription consent/pending/restore/runtime error and narrow layout.');
})().catch(e=>{console.error(e);process.exitCode=1;}).finally(async()=>{if(browser)await browser.close();server.close();});
