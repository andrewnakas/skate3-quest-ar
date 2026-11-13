// Headless side-by-side: the original (the port's web page) and the clean game (sk8's web page),
// both on KenneyPark in one headless Chrome, fed the SAME pad input one tick at a time. Prints each
// game's tricks and bails and whether they agree, and saves side-by-side frames + a report page.
//
//   node scripts/cleanroom/web/run.mjs <script.txt | --fuzz SEED | --demo> [--ticks N] [--every K] [--out DIR]
//
// Script lines are the probes' format: <from> <to> lx ly rx ry buttons [lt rt]  (ticks inclusive,
// sticks -1..1 +y up, Xbox bits A 0x1000 B 0x2000 X 0x4000 Y 0x8000 LB 0x100 RB 0x200).
// --fuzz SEED uses edge/fuzz.py's random input for that seed. --every K saves a frame pair every K
// ticks (0 = none). Start the pages first: scripts/cleanroom/web/serve.sh
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync, execSync } from 'node:child_process';
import { createRequire } from 'node:module';

const here = path.dirname(new URL(import.meta.url).pathname);
const require = createRequire(execSync('npm root -g').toString().trim() + '/');
const puppeteer = require('puppeteer');

const args = process.argv.slice(2);
const opt = (k, d) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
const PORT = process.env.PORT || 8771;
const ticks = Number(opt('--ticks', 180));
const every = Number(opt('--every', 10));
let script, label;
if (args.includes('--fuzz')) {
  const seed = opt('--fuzz');
  script = execFileSync('python3', ['-c', `import random, fuzz; print(fuzz.gen(random.Random(${seed})), end='')`],
    { cwd: path.join(here, '..', 'edge') }).toString();
  label = `fuzz-${seed}`;
} else if (args.includes('--demo')) {
  // push twice, an ollie, a kickflip, a carve
  script = '0 8 0 0 0 0 0x1000\n40 48 0 0 0 0 0x1000\n90 101 0 0 0 -1 0\n102 102 0 0 0 0 0\n103 108 0 0 0 1 0\n' +
    '160 171 0 0 0 -1 0\n172 172 0 0 0 0 0\n173 176 0 0 0.707 0.707 0\n230 260 0.6 0 0 0 0\n';
  label = 'demo';
} else if (args[0] && !args[0].startsWith('--')) {
  script = fs.readFileSync(args[0], 'utf8');
  label = path.basename(args[0]).replace(/\.\w+$/, '');
} else {
  console.error('usage: run.mjs <script.txt | --fuzz SEED | --demo> [--ticks N] [--every K] [--out DIR]');
  process.exit(2);
}
const out = opt('--out', path.join(process.env.HOME, 'skate-compare', 'runs', label));
fs.mkdirSync(out, { recursive: true });

const lines = [];
for (const raw of script.split('\n')) {
  const t = raw.replace(/#.*/, '').trim().split(/\s+/);
  if (t.length < 7) continue;
  lines.push({ from: +t[0], to: +t[1], lx: +t[2], ly: +t[3], rx: +t[4], ry: +t[5], buttons: Number(t[6]), lt: +(t[7] || 0), rt: +(t[8] || 0) });
}
const padAt = (k) => {
  let p = { buttons: 0, lx: 0, ly: 0, rx: 0, ry: 0, lt: 0, rt: 0 };
  for (const l of lines) if (k >= l.from && k <= l.to) p = { ...l };
  return p;
};

const browser = await puppeteer.launch({
  headless: 'new', executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  userDataDir: path.join(process.env.TMPDIR || '/tmp', 'pptr-sk8web'),
  args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--window-size=960,540', '--autoplay-policy=no-user-gesture-required'],
});
async function open(url, ready) {
  const page = await browser.newPage();
  await page.setViewport({ width: 960, height: 540 });
  const errs = [];
  page.on('pageerror', (e) => errs.push(e.message));
  page.on('console', (m) => { if (m.type() === 'error' && !/^(SKATE|INFO)/.test(m.text())) errs.push(m.text()); });
  await page.goto(url);
  const t0 = Date.now();
  while (!(await page.evaluate(ready).catch(() => false))) {
    if (Date.now() - t0 > 180000) throw new Error(`${url} never got ready: ${errs.join(' | ') || await page.evaluate(() => document.body.innerText.slice(0, 200))}`);
    await new Promise((r) => setTimeout(r, 500));
  }
  return { page, errs };
}
const B = `http://127.0.0.1:${PORT}`;
const orig = await open(`${B}/original/?base=/original/pack/&map=KenneyPark&capture=1`, () => !!(window.s3cap && window.s3cap.ready()));
await orig.page.evaluate(() => { const h = document.getElementById('help'); if (h) h.classList.add('hidden'); });
const clean = await open(`${B}/clean/?mesh=/kenney_mesh.txt&spawn=13.5,0.1,-1.5,3.14159&capture=1`, () => !!(window.sk8cap && window.sk8cap.ready()));

const rows = [], frames = [];
for (let k = 0; k < ticks; k++) {
  const pad = padAt(k), draw = every > 0 && k % every === 0;
  const [o, c] = await Promise.all([
    orig.page.evaluate((p) => window.s3cap.stepPad(p), { ...pad, draw }),
    clean.page.evaluate((p) => window.sk8cap.stepPad(p), { ...pad, draw }),
  ]);
  rows.push({ k, pad, o, c });
  if (draw) {
    const n = String(k).padStart(4, '0');
    await orig.page.screenshot({ path: path.join(out, `o${n}.png`) });
    await clean.page.screenshot({ path: path.join(out, `c${n}.png`) });
    frames.push(n);
  }
}

// tricks (in order, without repeats) and bails, the same way edge/fuzz.py reads them
const seq = (xs) => xs.filter((x, i) => x && x !== xs[i - 1]);
const oTricks = [...new Set(seq(rows.map((r) => r.o && r.o.trick)))];
const cTricks = [...new Set(seq(rows.flatMap((r) => (r.c ? r.c.events.split('|').filter((e) => /^EV[01]:/.test(e)).map((e) => e.slice(4)) : []))))];
const oBail = rows.some((r) => r.o && r.o.wiping), cBail = rows.some((r) => r.c && r.c.bail);
const canon = (n) => n.toLowerCase().replace(/^(fakie|switch) /, '').replace(/ (180|360|540)$/, '').replace(/[^a-z0-9]/g, '')
  .replace('frontside', 'fs').replace('backside', 'bs').replace('popshoveit', 'bsshuv').replace('shoveit', 'shuv').replace('shuvit', 'shuv');
const skip = /grab|manual|grind|slide|air|50-50|5-0|smith|crook|salad|feeble|willy|^bs_|^fs_/i;
const a = [...new Set(oTricks.filter((x) => !skip.test(x)).map(canon))].sort();
const b = [...new Set(cTricks.filter((x) => !skip.test(x)).map(canon))].sort();
const same = JSON.stringify(a) === JSON.stringify(b) && oBail === cBail;
console.log(`original: ${JSON.stringify(oTricks)}${oBail ? ' BAIL' : ''}`);
console.log(`clean:    ${JSON.stringify(cTricks)}${cBail ? ' BAIL' : ''}`);
console.log(same ? 'MATCH' : 'DIFFER');
const errs = [...orig.errs.map((e) => `original: ${e}`), ...clean.errs.map((e) => `clean: ${e}`)];
if (errs.length) console.log(errs.slice(0, 8).join('\n'));

// per-tick log + a report page with the frames side by side
fs.writeFileSync(path.join(out, 'ticks.csv'), 'tick,buttons,lx,ly,rx,ry,lt,rt,o_state,o_trick,o_speed,o_wiping,o_x,o_y,o_z,c_mode,c_trick,c_events,c_speed,c_bail,c_x,c_y,c_z\n' +
  rows.map(({ k, pad: p, o, c }) => [k, p.buttons, p.lx, p.ly, p.rx, p.ry, p.lt, p.rt, o && o.state, o && o.trick, o && o.speed && o.speed.toFixed(2),
    o && o.wiping ? 1 : 0, ...(o && o.pos ? o.pos.map((x) => x.toFixed(3)) : ['', '', '']), c && c.mode, c && c.trick, c && c.events,
    c && c.speed.toFixed(2), c && c.bail ? 1 : 0, ...(c ? c.pos.map((x) => x.toFixed(3)) : ['', '', ''])].join(',')).join('\n') + '\n');
const at = (n) => rows[+n];
fs.writeFileSync(path.join(out, 'index.html'), `<!doctype html><meta charset="utf-8"><title>${label}</title>
<style>body{background:#15171b;color:#ddd;font:13px system-ui;margin:16px}img{width:480px}td{vertical-align:top;padding:2px 6px}
.d{color:#f87}.m{color:#7d7}</style><h2>${label}: <span class="${same ? 'm' : 'd'}">${same ? 'MATCH' : 'DIFFER'}</span></h2>
<p>original ${oTricks.join(', ')}${oBail ? ' — BAIL' : ''}<br>clean ${cTricks.join(', ')}${cBail ? ' — BAIL' : ''}</p>
<table><tr><th>tick</th><th>original</th><th>clean (sk8)</th></tr>${frames.map((n) => `<tr><td>${+n}<br>${at(n).o ? at(n).o.state : ''}<br>${at(n).c ? at(n).c.mode : ''}</td>
<td><img src="o${n}.png"></td><td><img src="c${n}.png"></td></tr>`).join('')}</table>`);
console.log(`frames + report: ${path.join(out, 'index.html')}`);
await browser.close();
process.exit(same ? 0 : 1);
