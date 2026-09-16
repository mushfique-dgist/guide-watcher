import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import AxeBuilder from '@axe-core/playwright';
import { desktopFixture } from './desktop-fixture.js';
import { createServer } from 'vite';
const server=process.env.GW_TEST_URL?null:await createServer({server:{host:'127.0.0.1',port:0,strictPort:false}});
let browser;
try {
await server?.listen();
browser=await chromium.launch({headless:true,...(process.env.GW_BROWSER_PATH?{executablePath:process.env.GW_BROWSER_PATH}:{channel:'msedge'})});
const context=await browser.newContext();const page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
await page.addInitScript(desktopFixture);
const url=process.env.GW_TEST_URL||server.resolvedUrls.local[0];
async function load(width=1120,height=780,scale=1){await page.setViewportSize({width,height});await page.goto(url);await page.evaluate(s=>{localStorage.setItem('guide-watcher-scale-v2',String(s));localStorage.removeItem('fixture-history');},scale);await page.reload();await page.getByRole('heading',{name:'Turn course material into understanding.'}).waitFor();}
async function nav(name){const opener=page.getByRole('button',{name:'Open navigation',exact:true});if(await opener.isVisible())await opener.click();await page.getByRole('complementary',{name:'Main navigation'}).getByRole('button',{name,exact:false}).first().click();}
async function fits(locator,label){const box=await locator.boundingBox();const viewport=page.viewportSize();assert(box && box.width>0&&box.height>0,label+' has no visible size');assert(box.x>=-1&&box.y>=-1&&box.x+box.width<=viewport.width+1&&box.y+box.height<=viewport.height+1,label+' is clipped '+JSON.stringify(box));}
{

  // Reproduce the supplied screenshot: three active jobs plus a full recent list.
  await load(1116,822,1);
  await page.evaluate(()=>{
    const sample=window.fixture.records[0];
    window.fixture.records.splice(0,window.fixture.records.length,...Array.from({length:21},(_,i)=>({...sample,jobId:'dense-'+i,status:i<3?'working':'interrupted',summary:i<3?'Collecting context from the lecture.':'This attempt stopped before a guide was published.',canOpenOutput:false,canResume:false,canRetry:i>=3,events:[{timestamp:Date.now(),level:'info',message:i<3?'Inspecting source visuals.':'Work stopped before publication.'}],filename:'Lecture '+i+' with a long source filename.pdf',finishedAt:i<3?null:Date.now(),startedAt:Date.now()-i*1000})));
  });
  await nav('History');await page.getByRole('button',{name:'Refresh',exact:true}).click();
  const sidebar=page.getByRole('complementary',{name:'Main navigation'});
  const sidebarGeometry=await sidebar.evaluate(el=>{
    const list=el.querySelector('.run-list'),footer=el.querySelector('.sidebar-bottom');
    const l=list.getBoundingClientRect(),f=footer.getBoundingClientRect();
    return {overflow:getComputedStyle(list).overflowY,listBottom:l.bottom,footerTop:f.top,footerBackground:getComputedStyle(footer).backgroundColor,scrollHeight:list.scrollHeight,clientHeight:list.clientHeight};
  });
  assert(['auto','scroll'].includes(sidebarGeometry.overflow),'Populated job list must contain its own scrolling: '+JSON.stringify(sidebarGeometry));
  assert(sidebarGeometry.listBottom<=sidebarGeometry.footerTop,'Job list overlaps Settings');
  assert(!['rgba(0, 0, 0, 0)','transparent'].includes(sidebarGeometry.footerBackground),'Settings footer is transparent');
  await fits(sidebar.getByRole('button',{name:'Settings & help',exact:true}),'Settings footer with active and recent jobs');
  await sidebar.getByRole('button',{name:'Settings & help',exact:true}).click({trial:true});
  await sidebar.locator('.run').first().focus();
  assert(await sidebar.locator('.run-list').evaluate(el=>parseFloat(getComputedStyle(el).paddingLeft)>=5),'Scrollable list clips keyboard focus outline');
  console.log('PASS screenshot regression: populated sidebar contains overflow and Settings has an opaque, unobscured surface');
  if(process.env.GW_UI_SCREENSHOT){await sidebar.locator('.run').first().click();await page.getByRole('heading',{name:'Lecture 0 with a long source filename.pdf',exact:true}).waitFor();await page.screenshot({path:process.env.GW_UI_SCREENSHOT});}


  for(const [width,height,scale,count,active] of [[1116,822,1,21,3],[1116,822,1.25,50,20],[1920,800,2,100,25],[600,400,2,30,3],[800,600,1.5,21,3],[1120,780,1,0,0]]){
    await load(width,height,scale);
    await page.evaluate(({count,active})=>{const sample=window.fixture.records[0];window.fixture.records.splice(0,window.fixture.records.length,...Array.from({length:count},(_,i)=>({...sample,jobId:'density-'+i,status:i<active?'working':'interrupted',summary:i<active?'Collecting context from the lecture.':'This attempt stopped before a guide was published.',canOpenOutput:false,canResume:false,canRetry:i>=active,events:[{timestamp:Date.now(),level:'info',message:i<active?'Inspecting source visuals.':'Work stopped before publication.'}],filename:'한국어 lecture '+i+' '+('Long source name '.repeat(6))+'.pdf',startedAt:Date.now()-i*1000,finishedAt:i<active?null:Date.now()})));},{count,active});
    await nav('History');await page.getByRole('button',{name:'Refresh',exact:true}).click();
    const opener=page.getByRole('button',{name:'Open navigation',exact:true});if(await opener.isVisible())await opener.click();
    const navigation=page.getByRole('complementary',{name:'Main navigation'});
    const settingsButton=navigation.getByRole('button',{name:'Settings & help',exact:true});
    await settingsButton.scrollIntoViewIfNeeded();await fits(settingsButton,'Settings at '+[width,height,scale,count,active].join('/'));
    assert(await settingsButton.evaluate(el=>{const r=el.getBoundingClientRect();return el.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));}),'Settings is obscured by another element');
    const geometry=await navigation.evaluate(el=>{const list=el.querySelector('.run-list'),footer=el.querySelector('.sidebar-bottom');return {listBottom:list.getBoundingClientRect().bottom,footerTop:footer.getBoundingClientRect().top,background:getComputedStyle(footer).backgroundColor};});
    assert(geometry.listBottom<=geometry.footerTop+1,'Job list overlaps Settings after density/zoom change');assert.notEqual(geometry.background,'rgba(0, 0, 0, 0)');
    await settingsButton.click();await page.getByRole('heading',{name:'Settings & help',exact:true}).waitFor();
    await page.getByLabel('Theme',{exact:true}).selectOption('light');await page.getByRole('button',{name:'Open full history',exact:true}).scrollIntoViewIfNeeded();await fits(page.getByRole('button',{name:'Open full history',exact:true}),'Full history entry');
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'Populated navigation overflows document');
  }
  console.log('PASS dense, empty, short and zoomed navigation: Settings is reachable, opaque, and separate from job content');

  let cases=0;
  for(const [width,height] of [[600,400],[800,600],[1120,780],[1440,900],[1920,1080],[320,640]])for(const scale of [.85,1,1.25,1.5,2]){
    await load(width,height,scale);
    await fits(page.getByRole('button',{name:'Zoom in',exact:true}),'Zoom control');
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'Document overflows horizontally');
    await nav('History');await page.getByRole('heading',{name:'History',exact:true}).waitFor();
    await fits(page.getByRole('button',{name:'Refresh',exact:true}),'History refresh');
    await nav('Settings & help');await page.getByLabel('Theme',{exact:true}).selectOption('light');
    await page.getByLabel('Theme',{exact:true}).selectOption('dark');
    cases++;
  }
  console.log('PASS '+cases+' viewport and zoom combinations across workspace, history, and settings');
  await load();await nav('History');await page.getByRole('searchbox',{name:'Search history'}).fill('does-not-exist');assert(await page.getByText('No matching attempts').isVisible());await page.getByRole('button',{name:'Clear filters'}).click();await page.getByLabel('Filter history by outcome').selectOption('resumable');assert(await page.locator('.history-row').count()===1);await page.locator('.history-row').click();assert(await page.getByRole('button',{name:'Resume writing',exact:true}).isVisible());await page.getByRole('tab',{name:'Technical log'}).click();assert(!(await page.getByText('Guide generated',{exact:true}).count()));
  console.log('PASS history search, filters, recovery affordance, and truthful provider completion');
  await load(600,400,2);await page.getByRole('button',{name:'Open navigation'}).click();const dialog=page.getByRole('dialog',{name:'Navigation'});await dialog.locator('button').last().focus();await page.keyboard.press('Tab');assert(await dialog.evaluate(el=>el.contains(document.activeElement)),'Tab escaped navigation');await page.keyboard.press('Escape');await dialog.waitFor({state:'detached'});await page.waitForFunction(()=>document.activeElement?.getAttribute('aria-label')==='Open navigation');assert(await page.getByRole('button',{name:'Open navigation',exact:true}).evaluate(el=>el===document.activeElement),'Escape did not return focus to Open navigation');
  console.log('PASS compact navigation keyboard trap and Escape');
  // Recreate the same drawer across a breakpoint transition, without reloading.
  await page.getByRole('button',{name:'Open navigation',exact:true}).click();
  await page.getByRole('dialog',{name:'Navigation'}).waitFor();
  await page.setViewportSize({width:1920,height:1080});
  await page.getByRole('button',{name:'Open navigation',exact:true}).waitFor({state:'detached'});
  assert.equal(await page.getByRole('dialog',{name:'Navigation'}).count(),0,'Drawer remained mounted at wide size');
  await page.setViewportSize({width:600,height:400});
  const navigationOpener=page.getByRole('button',{name:'Open navigation',exact:true});
  await navigationOpener.waitFor();
  assert.equal(await navigationOpener.getAttribute('aria-expanded'),'false','Returning to compact size reopened the drawer');
  assert.equal(await page.getByRole('dialog',{name:'Navigation'}).count(),0,'Stale compact drawer returned after resizing');
  await navigationOpener.click();await page.getByRole('dialog',{name:'Navigation'}).waitFor();
  await page.keyboard.press('Escape');await page.waitForFunction(()=>document.activeElement?.getAttribute('aria-label')==='Open navigation');
  console.log('PASS compact open -> wide -> compact stays closed and can reopen');

  // A short window at 200% must permit reaching actual log text, not merely its tab.
  await load(600,400,2);await nav('History');
  await page.getByLabel('Filter history by outcome').selectOption('active');
  await page.locator('.history-row').first().click();
  const logEnd='END OF SCROLL REACHABILITY REGRESSION';
  await page.evaluate(marker=>window.emitFixture('job-output',{job_id:'fixture-5',line:JSON.stringify({type:'assistant',message:{content:[{type:'text',text:Array.from({length:100},(_,i)=>'Diagnostic line '+i).join('\n')+'\n'+marker}]}})}),logEnd);
  await page.getByRole('tab',{name:'Technical log'}).click();
  const logPanel=page.getByRole('tabpanel',{name:/Technical log/});
  const outputDisclosure=logPanel.locator('summary').filter({hasText:'Provider output'});
  await outputDisclosure.click();
  assert(await logPanel.evaluate(el=>el.clientHeight>0),'Technical log has no usable scroll area');
  // Scroll the log and each scrollable ancestor using their native scroll offsets.
  // Measuring a range and hit-testing it catches overflow:hidden ancestors and zero-height feeds.
  const scrollState=await logPanel.evaluate(el=>{
    const positions=[];
    for(let node=el;node;node=node.parentElement){
      if(node.scrollHeight>node.clientHeight && /auto|scroll/.test(getComputedStyle(node).overflowY)){
        node.scrollTop=node.scrollHeight;positions.push({name:node.className,top:node.scrollTop});
      }
    }
    return positions;
  });
  assert(scrollState.some(s=>s.top>0),'Long technical output did not scroll');
  await page.waitForFunction(marker=>{
    const pre=[...document.querySelectorAll('[role="tabpanel"] pre')].find(el=>el.textContent.includes(marker));
    if(!pre?.firstChild)return false;
    const range=document.createRange();const start=pre.firstChild.textContent.lastIndexOf(marker);
    range.setStart(pre.firstChild,start);range.setEnd(pre.firstChild,start+3);
    const r=range.getBoundingClientRect();const x=r.x+r.width/2,y=r.y+r.height/2;
    return r.height>0 && x>=0 && y>=0 && x<innerWidth && y<innerHeight && pre.contains(document.elementFromPoint(x,y));
  },logEnd);
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'Technical output causes horizontal document overflow');
  console.log('PASS technical log final content reachable by scrolling at 600x400 and 200%');
  await load(600,400,2);
  await page.evaluate(()=>window.fixture.sources.push('C:/Courses/'+('Very long folder name /'.repeat(12))+'Long source filename with spaces and 한국어.pdf'));
  await page.getByRole('button',{name:'Choose files',exact:true}).click();
  await page.getByRole('heading',{name:'Review your sources',exact:true}).waitFor();
  const generate=page.getByRole('button',{name:/^Generate from \d+ sources$/});
  await generate.waitFor();
  await generate.scrollIntoViewIfNeeded();await fits(generate,'Confirmation generate footer at 200%');await generate.click({trial:true});
  const dismiss=page.getByRole('button',{name:'Dismiss selection',exact:true});
  await dismiss.scrollIntoViewIfNeeded();await fits(dismiss,'Confirmation dismiss footer at 200%');await dismiss.click({trial:true});
  const prepModel=page.getByLabel('Primary Codex model',{exact:true}).first();
  const writerModel=page.getByLabel('Claude model',{exact:true});
  assert.equal(await prepModel.inputValue(),'gpt-5.6-luna');
  assert.equal(await writerModel.inputValue(),'claude-opus-4-8');
  assert.equal(await page.getByLabel('Primary effort',{exact:true}).first().inputValue(),'medium');
  assert.equal(await page.getByLabel('Effort',{exact:true}).inputValue(),'high');
  await writerModel.selectOption('claude-opus-5');
  assert.equal(await writerModel.inputValue(),'claude-opus-5','Writer model selection did not persist');
  await prepModel.scrollIntoViewIfNeeded();await fits(prepModel,'Context model selector at 200%');
  await page.getByRole('button',{name:'Clear selection',exact:true}).click();
  assert(await generate.isDisabled(),'Empty selection permits generation');
  const sourceSearch=page.getByRole('searchbox',{name:'Find a source'});
  await sourceSearch.fill('CH01_Introduction');await page.getByRole('button',{name:'Select visible',exact:true}).click();
  await sourceSearch.fill('nothing-matches-this-source');
  assert(await page.getByText('No sources match your search.',{exact:true}).isVisible());
  assert(await page.getByRole('button',{name:'Select visible',exact:true}).isDisabled(),'Empty search can change selection');
  await sourceSearch.fill('');assert.equal(await page.getByRole('checkbox',{checked:true}).count(),1,'Filtering lost the chosen source');
  assert.equal(await generate.textContent(),'Generate from 1 sources');
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'Long source path creates horizontal overflow');
  console.log('PASS confirmation long paths, reachable footer, model selectors, empty/search selection at 600x400 and 200%');

  await load();
  await page.evaluate(()=>{
    const invoke=window.__TAURI_INTERNALS__.invoke;
    window.failConfig=true;
    window.__TAURI_INTERNALS__.invoke=(command,args)=>command==='get_config'&&window.failConfig?Promise.reject(new Error('Settings service unavailable')):invoke(command,args);
  });
  await page.getByRole('button',{name:'Choose files',exact:true}).click();
  await page.getByRole('button',{name:'Reload settings',exact:true}).waitFor();
  assert(await page.getByRole('button',{name:/^Generate from/}).isDisabled(),'Configuration failure enables generation');
  assert.equal(await page.getByRole('checkbox',{checked:true}).count(),4,'Configuration failure lost source selection');
  await page.evaluate(()=>window.failConfig=false);
  await page.getByRole('button',{name:'Reload settings',exact:true}).click();
  await page.waitForFunction(()=>[...document.querySelectorAll('button')].some(b=>b.textContent.startsWith('Generate from')&&!b.disabled));
  assert.equal(await page.getByRole('checkbox',{checked:true}).count(),4,'Reloading settings lost source selection');
  console.log('PASS command-specific configuration failure, disabled submission, and settings retry');

  await page.evaluate(()=>{
    const invoke=window.__TAURI_INTERNALS__.invoke;
    window.approvalRequests=[];
    window.__TAURI_INTERNALS__.invoke=async(command,args)=>{
      if(command==='approve_files'){
        window.approvalRequests.push(JSON.parse(JSON.stringify(args)));
        await new Promise(resolve=>window.releaseApproval=resolve);
      }
      return invoke(command,args);
    };
  });
  // Two clicks in the same turn probe the controller guard before the DOM disables the button.
  await page.getByRole('button',{name:'Generate from 4 sources',exact:true}).evaluate(el=>{el.click();el.click();});
  await page.getByRole('button',{name:'Preparing…',exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>window.approvalRequests.length),1,'Duplicate clicks submitted twice');
  await page.getByRole('button',{name:'← Workspace',exact:true}).click();
  await page.getByRole('heading',{name:'Turn course material into understanding.'}).waitFor();
  await page.getByRole('button',{name:'Review sources',exact:true}).click();
  assert(await page.getByRole('button',{name:'Preparing…',exact:true}).isDisabled(),'Remount lost in-flight submission state');
  assert(await page.getByLabel('Course profile',{exact:true}).isDisabled(),'Remount enabled settings during submission');
  await page.evaluate(()=>window.emitFixture('new-files',['C:/Courses/Networks/Arrived during submission.pdf']));
  assert.equal(await page.locator('.source-row input[type=checkbox]').count(),5,'Watcher arrival was dropped during submission');
  await nav('History');await page.getByRole('heading',{name:'History',exact:true}).waitFor();
  await page.evaluate(()=>window.releaseApproval());
  await page.getByText('Additional sources remain under Review sources.',{exact:true}).waitFor();
  await nav('Workspace');await page.getByRole('button',{name:'Review sources',exact:true}).click();
  await page.getByRole('button',{name:'Generate from 1 sources',exact:true}).waitFor();
  assert.equal(await page.locator('.source-row input[type=checkbox]').count(),1,'Submitted sources were not removed exactly once');
  assert(await page.getByText('Arrived during submission.pdf',{exact:true}).isVisible(),'Unsubmitted watcher source did not survive completion');
  const submitted=await page.evaluate(()=>window.approvalRequests);
  assert.equal(submitted.length,1,'Navigation or watcher event triggered a duplicate submission');
  assert.equal(submitted[0].filePaths.length,4,'Watcher arrival changed the in-flight submission snapshot');
  assert(!submitted[0].filePaths.some(path=>path.includes('Arrived during submission')),'Unreviewed watcher source entered the existing request');
  console.log('PASS duplicate click guard, navigation/remount mid-submit, and retained watcher arrival');


  await load();await nav('Settings & help');
  await page.getByText('Model accounts',{exact:true}).waitFor();
  assert.equal(await page.getByText('Connected',{exact:true}).count(),2,'Both native CLI accounts were not shown as connected');
  const codexAccount=page.locator('.account-card').filter({hasText:'OpenAI Codex'});
  await codexAccount.getByRole('button',{name:'Switch account',exact:true}).click();
  await codexAccount.getByRole('button',{name:'Cancel sign-in',exact:true}).waitFor();
  await page.evaluate(()=>window.emitFixture('provider-auth',{provider:'codex-chatgpt',sessionId:'auth-fixture',state:'waiting',message:'Preparing the one-time device code…',verificationUri:'https://auth.openai.com/codex/device',userCode:null}));
  assert(await codexAccount.getByRole('button',{name:'Waiting for device code…',exact:true}).isDisabled(),'Sign-in page became available before the device code');
  await page.evaluate(()=>window.emitFixture('provider-auth',{provider:'codex-chatgpt',sessionId:'auth-fixture',state:'waiting',message:'Open the sign-in page and enter the code shown here.',verificationUri:'https://auth.openai.com/codex/device',userCode:'ABCD-EFGH'}));
  await codexAccount.getByLabel('One-time device code',{exact:true}).waitFor();
  assert.equal(await codexAccount.getByLabel('One-time device code',{exact:true}).inputValue(),'ABCD-EFGH');
  await codexAccount.getByRole('button',{name:'Copy code and open sign-in page',exact:true}).click();
  await codexAccount.getByRole('button',{name:'Code copied — open page again',exact:true}).waitFor();
  await nav('Workspace');await nav('Settings & help');
  assert.equal(await codexAccount.getByLabel('One-time device code',{exact:true}).inputValue(),'ABCD-EFGH');
  await codexAccount.getByRole('button',{name:'Cancel sign-in',exact:true}).waitFor();
  await codexAccount.getByRole('button',{name:'Cancel sign-in',exact:true}).click();
  await page.evaluate(()=>window.emitFixture('provider-auth',{provider:'codex-chatgpt',sessionId:'auth-fixture',state:'cancelled',message:'Sign-in was cancelled.',verificationUri:null,userCode:null}));
  await codexAccount.getByRole('button',{name:'Switch account',exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>window.fixture.calls.filter(call=>call.command==='start_provider_login').length),1,'Account switch did not use the native auth command exactly once');
  assert.equal(await page.evaluate(()=>window.fixture.calls.filter(call=>call.command==='cancel_provider_login').length),1,'Account cancellation was not forwarded exactly once');
  assert.equal(await page.evaluate(()=>window.fixture.calls.filter(call=>call.command==='plugin:clipboard-manager|write_text').length),1,'Device code was not copied through the scoped native clipboard permission');

  const collectionDefaults=page.getByRole('group',{name:'Context collection',exact:true});
  const collectionRecoveryDefaults=page.getByRole('group',{name:'Collection recovery',exact:true});
  const writerDefaults=page.getByRole('group',{name:'Guide writing',exact:true});
  await collectionDefaults.getByLabel('Primary effort',{exact:true}).selectOption('low');
  assert.equal(await collectionRecoveryDefaults.getByLabel('First fallback',{exact:true}).inputValue(),'gpt-5.6-terra');
  assert.equal(await collectionRecoveryDefaults.getByLabel('Second fallback',{exact:true}).inputValue(),'gpt-5.6-sol');
  await writerDefaults.getByLabel('Claude model',{exact:true}).selectOption('claude-opus-5');
  await writerDefaults.getByLabel('Effort',{exact:true}).selectOption('medium');
  await page.getByRole('button',{name:'Save generation defaults',exact:true}).click();
  await page.getByText('Generation defaults saved. New guides and resumes will start with these choices.',{exact:true}).waitFor();
  await nav('Workspace');await page.getByRole('button',{name:'Choose files',exact:true}).click();
  await page.getByRole('heading',{name:'Review your sources',exact:true}).waitFor();
  assert.equal(await page.getByLabel('Primary effort',{exact:true}).first().inputValue(),'low');
  assert.equal(await page.getByLabel('Claude model',{exact:true}).inputValue(),'claude-opus-5');
  assert.equal(await page.getByLabel('Effort',{exact:true}).inputValue(),'medium');
  await page.getByRole('button',{name:'← Workspace',exact:true}).click();await nav('Settings & help');
  await page.getByRole('button',{name:'Restore recommended defaults',exact:true}).click();
  assert.equal(await collectionDefaults.getByLabel('Primary effort',{exact:true}).inputValue(),'medium');
  assert.equal(await writerDefaults.getByLabel('Claude model',{exact:true}).inputValue(),'claude-opus-4-8');
  console.log('PASS native account-switch flow, cancellation, and persistent model/effort defaults');


  await load();
  await page.evaluate(()=>{window.fixture.records[2].finishedAt=null;window.fixture.records[2].summary='Legacy attempt with unknown duration';});
  await nav('History');await page.getByRole('button',{name:'Refresh',exact:true}).click();
  await page.getByRole('button',{name:/Legacy attempt with unknown duration/}).click();
  assert.equal(await page.locator('.run-subtitle > span').count(),2,'Unknown historical duration must not become a running clock');
  console.log('PASS unknown historical duration is not fabricated');

  await load();const accessibility=await new AxeBuilder({page}).withTags(['wcag2a','wcag2aa','wcag21aa']).analyze();assert.deepEqual(accessibility.violations.map(v=>({id:v.id,nodes:v.nodes.map(n=>n.target)})),[],'Workspace accessibility violations');
  await nav('History');const historyAxe=await new AxeBuilder({page}).withTags(['wcag2a','wcag2aa','wcag21aa']).analyze();assert.deepEqual(historyAxe.violations.map(v=>({id:v.id,nodes:v.nodes.map(n=>n.target)})),[],'History accessibility violations');
  await nav('Settings & help');const settingsAxe=await new AxeBuilder({page}).withTags(['wcag2a','wcag2aa','wcag21aa']).analyze();assert.deepEqual(settingsAxe.violations.map(v=>({id:v.id,nodes:v.nodes.map(n=>n.target)})),[],'Settings accessibility violations');
  console.log('PASS workspace, history, and settings accessibility checks');
  await load();await page.locator('.run').first().waitFor();await page.locator('.run').first().click();await page.getByRole('tab',{name:'Technical log'}).click();const burst=await page.evaluate(async()=>{const start=performance.now();for(let i=0;i<3000;i++)window.emitFixture('job-output',{job_id:'fixture-5',line:JSON.stringify({type:'assistant',message:{content:[{type:'text',text:'Progress line '+i}]}})});await new Promise(requestAnimationFrame);return performance.now()-start;});await page.getByText(/Provider output.*characters/).waitFor();console.log('PASS 3000 parsed provider messages in '+Math.round(burst)+' ms; '+await page.getByText(/Provider output.*characters/).textContent());
  assert.deepEqual(errors,[],'Browser runtime errors');
}
} finally {await browser?.close();await server?.close();}
