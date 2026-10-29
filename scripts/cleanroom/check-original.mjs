// Open the local original (port) page headless, wait for it to finish loading, screenshot it.
import { createRequire } from 'node:module';
import { execSync } from 'node:child_process';
const require = createRequire(execSync('npm root -g').toString().trim() + '/');
const puppeteer = require('puppeteer');
const url = process.argv[2], out = process.argv[3];
const browser = await puppeteer.launch({ headless: 'new', executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', userDataDir: process.env.TMPDIR + '/pptr-check', args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--window-size=1280,720'] });
const page = await browser.newPage();
await page.setViewport({ width: 1280, height: 720 });
const logs = [];
page.on('console', (m) => logs.push(`${m.type()}: ${m.text()}`));
page.on('pageerror', (e) => logs.push(`pageerror: ${e.message}`));
await page.goto(url);
const t0 = Date.now();
let status = '';
while (Date.now() - t0 < 120000) {
  status = await page.evaluate(() => document.body.innerText.slice(0, 300));
  if (!/downloading|loading|installing|decoding/i.test(status)) break;
  await new Promise((r) => setTimeout(r, 2000));
}
await new Promise((r) => setTimeout(r, 4000));
await page.screenshot({ path: out });
console.log('status:', JSON.stringify(status), 'after', ((Date.now() - t0) / 1000).toFixed(0), 's');
console.log(logs.filter((l) => /error|fail/i.test(l)).slice(0, 10).join('\n') || 'no console errors');
await browser.close();
