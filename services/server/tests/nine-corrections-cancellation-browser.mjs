import assert from 'node:assert/strict';
import { once } from 'node:events';
import { chromium } from 'playwright';

const config=JSON.parse(process.env.LATEX_CORE_NINE_CONFIG);
const browser=await chromium.launch({headless:true,args:['--no-sandbox'],executablePath:process.env.PLAYWRIGHT_CHROMIUM_PATH||chromium.executablePath()});
try {
  const context=await browser.newContext();
  const [name,...value]=config.writerCookie.split('=');
  await context.addCookies([{name,value:value.join('='),url:config.base}]);
  const page=await context.newPage(); await page.goto(`${config.base}/write?paper=${config.paperId}`);
  await page.locator('#compileSpinner').waitFor({state:'visible'});
  console.log('CANCELLATION_READY');
  // The Rust harness cancels this real queued job through the existing durable queue.
  await once(process.stdin,'data');
  await page.waitForFunction(()=>document.querySelector('#buildStatus').textContent==='Compilation cancelled'&&document.querySelector('#compileSpinner').hidden,{},{timeout:30000});
  const response=await context.request.get(`${config.base}/api/v2/papers/${config.paperId}/builds`);
  const state=(await response.json()).build;
  assert.equal(state.active_status,'cancelled');
  console.log('NINE_BROWSER_PASS: actual durable cancellation stops spinner and shows cancelled status');
} finally { await browser.close(); }
