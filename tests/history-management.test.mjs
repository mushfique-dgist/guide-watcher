import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import { desktopFixture } from './desktop-fixture.js';
const server=process.env.GW_TEST_URL?null:await createServer({server:{host:'127.0.0.1',port:0}});
let browser;
try {
 await server?.listen();
 browser=await chromium.launch({headless:true,...(process.env.GW_BROWSER_PATH?{executablePath:process.env.GW_BROWSER_PATH}:{channel:'msedge'})});
 const page=await browser.newPage({viewport:{width:1116,height:822}}); const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(desktopFixture);await page.goto(process.env.GW_TEST_URL||server.resolvedUrls.local[0]);
 async function nav(name){const menu=page.getByRole('button',{name:'Open navigation',exact:true});if(await menu.isVisible())await menu.click();await page.getByRole('complementary').getByRole('button',{name,exact:false}).first().click();}
 const count=async n=>{await page.waitForFunction(n=>document.querySelector('.result-count')?.textContent.startsWith(n+' attempt'),n);};
 const clear=()=>page.locator('main').getByRole('button',{name:'Clear recents',exact:true});
 const remove=()=>page.locator('main').getByRole('button',{name:/^Remove .* from Recents$/});
 const deletion=()=>page.getByRole('button',{name:/^Permanently delete history for /});
 const dialog=page.getByRole('dialog');
 await nav('History');await count(6);await remove().first().click();await count(5);
 await clear().click();await count(1);assert.match(await page.locator('.history-list').innerText(),/In progress/);
 await nav('Settings & help');await page.getByRole('button',{name:'Open full history',exact:true}).click();await count(6);
 await page.getByLabel('Record visibility',{exact:true}).selectOption('archived');await count(5);
 await page.getByRole('button',{name:/^Restore .* to Recents$/}).first().click();await count(4);
 await page.getByLabel('Record visibility',{exact:true}).selectOption('recent');await count(2);
 await page.getByLabel('Record visibility',{exact:true}).selectOption('archived');
 await deletion().first().click();await dialog.waitFor();assert.equal(await page.getByRole('button',{name:'Cancel',exact:true}).evaluate(el=>el===document.activeElement),true);
 await page.keyboard.press('Escape');await dialog.waitFor({state:'detached'});assert.equal(await deletion().first().evaluate(el=>el===document.activeElement),true);
 await deletion().first().click();await page.getByRole('button',{name:'Cancel',exact:true}).click();await count(4);
 await deletion().first().click();await dialog.waitFor();
 await page.evaluate(()=>window.emitFixture('new-files',['C:/Courses/Incoming.pdf']));
 await page.waitForTimeout(100);assert.equal(await dialog.count(),1,'Incoming watcher files must not dismiss a destructive confirmation');
 await page.keyboard.press('Control+h');assert.equal(await dialog.count(),1,'App shortcuts must not dismiss a modal');
 await page.evaluate(()=>{window.fixture.error='Storage is unavailable';});
 await dialog.getByRole('button',{name:'Delete permanently',exact:true}).click();await dialog.getByRole('alert').waitFor();assert.equal(await page.evaluate(()=>window.fixture.records.length),6);
 await page.evaluate(()=>{window.fixture.error=null;window.fixture.delay=150;window.fixture.persistHistory=true;});
 await dialog.getByRole('button',{name:'Delete permanently',exact:true}).dblclick();await dialog.waitFor({state:'detached'});await count(3);
 assert.equal(await page.evaluate(()=>window.fixture.calls.filter(c=>c.command==='delete_history_job').length),2,'One failed attempt and one successful attempt only');
 assert.equal(await page.getByRole('heading',{name:'Full history',exact:true}).evaluate(el=>el===document.activeElement),true);
 await nav('Workspace');await page.getByRole('button',{name:'Review sources',exact:true}).click();await page.getByText('Incoming.pdf',{exact:true}).waitFor();
 await page.reload();await nav('Settings & help');await page.getByRole('button',{name:'Open full history',exact:true}).click();await count(5);
 await nav('Workspace');await page.getByRole('heading',{name:'Turn course material into understanding.'}).waitFor();
 // Tiny, highly zoomed dialog remains opaque and its controls are scrollable and clickable.
 await page.evaluate(()=>localStorage.setItem('guide-watcher-scale-v2','2'));await page.setViewportSize({width:600,height:400});await page.reload();
 await nav('Settings & help');await page.getByRole('button',{name:'Open full history',exact:true}).click();await deletion().first().click();await dialog.waitFor();
 assert.notEqual(await dialog.evaluate(el=>getComputedStyle(el).backgroundColor),'rgba(0, 0, 0, 0)');
 await dialog.getByRole('button',{name:'Cancel',exact:true}).scrollIntoViewIfNeeded();await dialog.getByRole('button',{name:'Cancel',exact:true}).click();
 assert.deepEqual(errors,[]);console.log('PASS history archive, clear, active preservation, full history, restore, confirmation, failure/retry, duplicate guard, restart and compact zoom');
} finally {await browser?.close();await server?.close();}
