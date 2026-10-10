// Isolated native EXE + SQLite acceptance; no real accounts, CA writes or model requests.
// node SCRIPT EXE EVIDENCE PLAYWRIGHT_PACKAGE
const {chromium}=require(process.argv[4] || 'playwright');
const {DatabaseSync}=require('node:sqlite');
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),cp=require('node:child_process'),net=require('node:net'),assert=require('node:assert/strict');
const exe=path.resolve(process.argv[2]),evidence=path.resolve(process.argv[3]);
fs.mkdirSync(evidence,{recursive:true});
const root=fs.mkdtempSync(path.join(os.tmpdir(),'BYOK subscription isolated '));
const data=path.join(root,'Data'),profile=path.join(root,'Cursor'),settings=path.join(profile,'User','settings.json'),database=path.join(profile,'User','globalStorage','state.vscdb'),journal=path.join(data,'subscription.dpapi');
fs.mkdirSync(path.dirname(database),{recursive:true});fs.writeFileSync(settings,'{\n// preserve\n"editor.fontSize":17\n}');
const originalSettings=fs.readFileSync(settings),db=new DatabaseSync(database);
db.exec('PRAGMA journal_mode=WAL; CREATE TABLE ItemTable(key TEXT PRIMARY KEY,value BLOB)');
const member='cursorAuth/stripeMembershipType',status='cursorAuth/stripeSubscriptionStatus',mirror='src.vs.platform.reactivestorage.browser.reactiveStorageServiceImpl.persistentStorage.applicationUser';
const put=(k,v)=>db.prepare('INSERT INTO ItemTable VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value').run(k,v);
const get=k=>db.prepare('SELECT value FROM ItemTable WHERE key=?').get(k)?.value;
put(member,'free');put(status,'none');put('cursorAuth/stripeMembershipAuthId','synthetic-owner');put('cursorAuth/accessToken','synthetic-credential-must-stay');put(mirror,JSON.stringify({membershipType:'free',subscriptionStatus:null,keep:'original'}));
let child,browser;const args=['--data-dir',data,'--cursor-user-data-dir',profile];
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function wait(fn,label){const end=Date.now()+40000;while(Date.now()<end){if(await fn())return;await delay(100);}throw Error('Timeout: '+label);}
async function start(){
 const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
 child=cp.spawn(exe,args,{windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`}});
 await wait(async()=>{try{browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`,{timeout:1000});return true;}catch{return false;}},'WebView');
 let page;await wait(()=>{page=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('/__byok-api__/'));return !!page;},'panel');
 page.setDefaultTimeout(15000);
 await page.getByRole('heading',{name:'模型与连接',exact:true}).waitFor();
 if(await page.getByRole('button',{name:'稍后',exact:true}).isVisible())await page.getByRole('button',{name:'稍后',exact:true}).click();
 await page.getByRole('button',{name:'网络与账号',exact:true}).click();return page;
}
async function crash(){if(child&&child.exitCode===null){child.kill();await wait(()=>child.exitCode!==null||child.signalCode!==null,'isolated process exit');}child=null;if(browser){await browser.close().catch(()=>{});browser=null;}}
function restore(){return cp.spawnSync(exe,['--restore',...args],{windowsHide:true,timeout:30000}).status;}
async function inject(page){await page.getByRole('checkbox',{name:/我了解这只是本地缓存/}).check();await page.getByRole('button',{name:'启用临时缓存',exact:true}).click();await page.getByRole('button',{name:'关闭注入并恢复',exact:true}).waitFor();assert.equal(get(member),'ultra');assert.equal(get(status),'active');assert(fs.existsSync(journal));}
(async()=>{
 let page=await start();assert(await page.getByRole('button',{name:'启用临时缓存',exact:true}).isDisabled());await inject(page);
 assert.equal(get('cursorAuth/accessToken'),'synthetic-credential-must-stay');assert(!fs.readFileSync(journal).includes(Buffer.from('synthetic-owner')));
 put(mirror,JSON.stringify({membershipType:'ultra',subscriptionStatus:'active',keep:'third-party',added:42}));
 await page.getByRole('button',{name:'关闭注入并恢复',exact:true}).click();await page.getByRole('button',{name:'启用临时缓存',exact:true}).waitFor();
 assert.equal(get(member),'free');assert.equal(get(status),'none');assert.deepEqual(JSON.parse(get(mirror)),{membershipType:'free',subscriptionStatus:null,keep:'third-party',added:42});assert(!fs.existsSync(journal));
 await inject(page);put(member,'pro');put(mirror,JSON.stringify({membershipType:'pro',subscriptionStatus:'active',keep:'newer'}));
 await crash();assert.equal(restore(),0);assert.equal(restore(),0);assert.equal(get(member),'pro');assert.equal(get(status),'none');assert.equal(JSON.parse(get(mirror)).membershipType,'pro');
 page=await start();await inject(page);await crash();
 page=await start();assert.equal(get(member),'pro');assert(!fs.existsSync(journal));assert(await page.getByRole('button',{name:'启用临时缓存',exact:true}).isDisabled());
 assert.deepEqual(fs.readFileSync(settings),originalSettings);assert.equal(get('cursorAuth/accessToken'),'synthetic-credential-must-stay');
 await page.screenshot({path:path.join(evidence,'native-subscription-restored.png')});await crash();db.close();
 const report={passed:true,temporaryDirectory:root,checks:['explicit default-off consent','native API writes two cache fields only','DPAPI journal','JSON leaf restore preserves third-party keys/null','crash offline restore and idempotence','third-party subscription preserved','startup recovery','settings and synthetic credential unchanged'],realAccountsTouched:false};
 fs.writeFileSync(path.join(evidence,'native-subscription.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
})().catch(async e=>{await crash();const recovered=restore();db.close();console.error(JSON.stringify({passed:false,error:e.message,recoveryExit:recovered,temporaryDirectory:root}));process.exitCode=1;});
