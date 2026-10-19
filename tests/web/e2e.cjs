// End-to-end test of the web build (ROADMAP M3.3): serve the built site
// with the cross-origin isolation headers, open it in headless Chromium,
// and check that the Start, Guitar Pedalboard (with a fake microphone), and
// Subtractive Synth examples load from the site and play: the audio
// worklet's output has the expected level and pitch, the transport moves,
// the editor's telemetry follows the worklet, and the page logs no errors.
//
//   node tests/web/e2e.cjs [site dir]        (default target/web)
//
// Needs Playwright (`npm install playwright`, or NODE_PATH pointing at an
// installed one) and its Chromium. Screenshots go to target/web-e2e/.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawn } = require("child_process");
const net = require("net");
const { chromium } = require("playwright");

const ROOT = path.resolve(__dirname, "..", "..");
const SITE = path.resolve(process.argv[2] || path.join(ROOT, "target", "web"));
const SHOTS = path.join(ROOT, "target", "web-e2e");

// A tone for the fake microphone: 1 kHz, half scale, 48 kHz mono, 16-bit.
function writeTone(file, freq, seconds) {
  const rate = 48000;
  const n = rate * seconds;
  const data = Buffer.alloc(44 + n * 2);
  data.write("RIFF", 0);
  data.writeUInt32LE(36 + n * 2, 4);
  data.write("WAVEfmt ", 8);
  data.writeUInt32LE(16, 16);
  data.writeUInt16LE(1, 20);
  data.writeUInt16LE(1, 22);
  data.writeUInt32LE(rate, 24);
  data.writeUInt32LE(rate * 2, 28);
  data.writeUInt16LE(2, 32);
  data.writeUInt16LE(16, 34);
  data.write("data", 36);
  data.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) {
    data.writeInt16LE(Math.round(0.5 * 32767 * Math.sin((2 * Math.PI * freq * i) / rate)), 44 + i * 2);
  }
  fs.writeFileSync(file, data);
}

function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
    s.on("error", reject);
  });
}

async function serve(port) {
  const py = process.platform === "win32" ? "python" : "python3";
  const child = spawn(py, [path.join(ROOT, "scripts", "serve_web.py"), SITE, "--port", String(port)], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  await new Promise((resolve, reject) => {
    child.stdout.on("data", (d) => {
      if (String(d).includes("Serving")) resolve();
    });
    child.on("exit", (code) => reject(new Error(`the server exited (${code})`)));
  });
  return child;
}

let failures = 0;
function check(ok, what, detail) {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) failures++;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function status(page) {
  return JSON.parse(await page.evaluate(() => window.audioConstructor.status()));
}

// Wait until `pred(status)` holds (or time out and return the last one).
async function until(page, pred, ms = 20000) {
  const t0 = Date.now();
  let s = null;
  while (Date.now() - t0 < ms) {
    s = await page.evaluate(() => (window.audioConstructor ? window.audioConstructor.status() : null));
    if (s) {
      s = JSON.parse(s);
      if (pred(s)) return s;
    }
    await sleep(100);
  }
  return s;
}

// The output as the page's analyser sees it: RMS, and the strongest
// frequency with its level.
async function listen(page) {
  return page.evaluate(() => {
    const a = globalThis.audioConstructorAudio.analyser;
    const t = new Float32Array(a.fftSize);
    a.getFloatTimeDomainData(t);
    let sum = 0;
    for (const v of t) sum += v * v;
    const f = new Float32Array(a.frequencyBinCount);
    a.getFloatFrequencyData(f);
    let best = 1;
    for (let i = 2; i < f.length; i++) if (f[i] > f[best]) best = i;
    const rate = a.context.sampleRate;
    return { rms: Math.sqrt(sum / t.length), freq: (best * rate) / a.fftSize, db: f[best] };
  });
}

async function open(page, port, example) {
  await page.goto(`http://127.0.0.1:${port}/?example=${example}`);
  return until(page, (s) => s.worklet === "ready" && s.audio === "running" && s.inStep && s.project !== "Untitled");
}

// A project to open: the Pedalboard example, renamed.
async function projectText(page, name) {
  return page.evaluate(async (name) => {
    const p = JSON.parse(await (await fetch("examples/guitar-pedalboard.acproj")).text());
    p.name = name;
    return JSON.stringify(p);
  }, name);
}

// What the stand-in pickers saw, once `n` files are saved (or 5 s on).
async function files_after(page, n) {
  const t0 = Date.now();
  let f = null;
  while (Date.now() - t0 < 5000) {
    f = await page.evaluate(() => window.__acFiles);
    if (f.saved.length >= n) break;
    await sleep(100);
  }
  return f;
}

async function ready(page, port) {
  await page.goto(`http://127.0.0.1:${port}/?example=start`);
  await until(page, (s) => s.worklet === "ready" && s.project === "Start");
  // Keys go to the editor once its canvas has focus.
  await page.mouse.click(700, 760);
}

// Opening and saving through the File System Access API (pickers stood in
// for, as a person's choices), then through the upload-and-download
// fallback for browsers without it.
async function files(browser, port, errors) {
  // The API: Save writes back to the file the project was opened from.
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  await ctx.addInitScript(() => {
    const store = { saved: [], asked: 0, next: null };
    window.__acFiles = store;
    const writable = (name) => ({
      write: async (d) => store.saved.push({ name, text: new TextDecoder().decode(d) }),
      close: async () => {},
    });
    window.showOpenFilePicker = async () => {
      const f = store.next;
      return [{ kind: "file", name: f.name, getFile: async () => f, createWritable: async () => writable(f.name) }];
    };
    window.showSaveFilePicker = async (o) => {
      store.asked++;
      return { kind: "file", name: o.suggestedName, createWritable: async () => writable(o.suggestedName) };
    };
  });
  let page = await ctx.newPage();
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  await ready(page, port);
  const text = await projectText(page, "Mine");
  await page.evaluate((t) => {
    window.__acFiles.next = new File([t], "mine.acproj", { type: "application/json" });
  }, text);
  await page.keyboard.press("Control+o");
  let s = await until(page, (x) => x.project === "Mine", 10000);
  check(s && s.project === "Mine", "File ▸ Open reads a project through the file picker", s && s.project);
  await page.keyboard.press("Control+s");
  let saved = await files_after(page, 1);
  const back = saved.saved.length ? JSON.parse(saved.saved[0].text) : {};
  check(
    saved.saved.length === 1 && saved.saved[0].name === "mine.acproj" && back.name === "Mine",
    "File ▸ Save writes it back to the same file",
    JSON.stringify(saved.saved.map((x) => x.name)),
  );
  check(saved.asked === 0, "without asking where");
  await page.keyboard.press("Control+Shift+s");
  saved = await files_after(page, 2);
  check(saved.asked === 1 && saved.saved.length === 2, "Save As asks where", saved.asked);
  // MIDI through a patch: MIDI In straight to MIDI Out comes back to the
  // page, for the outputs checked in the Devices tab.
  const thru = JSON.stringify({
    schema_version: 2,
    name: "MIDI Thru",
    graph: {
      nodes: [
        { id: "1", kind: "midi_in" },
        { id: "2", kind: "midi_out", ui: { pos: [300, 0] } },
      ],
      wires: [{ from: "1:midi", to: "2:midi" }],
    },
  });
  await page.evaluate((t) => {
    window.__acFiles.next = new File([t], "thru.acproj", { type: "application/json" });
  }, thru);
  await page.keyboard.press("Control+o");
  s = await until(page, (x) => x.project === "MIDI Thru" && x.inStep, 10000);
  const sentBefore = s ? s.midiSent : 0;
  await page.evaluate(() => {
    window.audioConstructor.midi(new Uint8Array([0x90, 64, 90]));
    window.audioConstructor.midi(new Uint8Array([0x80, 64, 0]));
  });
  s = await until(page, (x) => x.midiSent >= sentBefore + 2, 5000);
  check(s && s.midiSent >= sentBefore + 2, "MIDI Out nodes' messages come back to the page", s && s.midiSent - sentBefore);
  // An edit makes the page's title say so, and leaving asks first.
  await page.keyboard.press("a");
  await page.keyboard.press("x");
  s = await until(page, (x) => x.dirty, 5000);
  const title = await page.title();
  check(s && s.dirty && title.startsWith("●"), "an unsaved edit shows in the title", title);
  let asked = false;
  page.on("dialog", async (d) => {
    asked = d.type() === "beforeunload";
    await d.accept();
  });
  await page.close({ runBeforeUnload: true });
  await sleep(500);
  check(asked, "leaving with unsaved changes asks first");
  await ctx.close();

  // Without the API: an upload and a download.
  const ctx2 = await browser.newContext({ viewport: { width: 1440, height: 900 }, acceptDownloads: true });
  await ctx2.addInitScript(() => {
    window.showOpenFilePicker = undefined;
    window.showSaveFilePicker = undefined;
  });
  page = await ctx2.newPage();
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  await ready(page, port);
  const text2 = await projectText(page, "Uploaded");
  const [chooser] = await Promise.all([page.waitForEvent("filechooser"), page.keyboard.press("Control+o")]);
  await chooser.setFiles({ name: "uploaded.acproj", mimeType: "application/json", buffer: Buffer.from(text2) });
  s = await until(page, (x) => x.project === "Uploaded", 10000);
  check(s && s.project === "Uploaded", "without the API, File ▸ Open takes an upload", s && s.project);
  const [download] = await Promise.all([page.waitForEvent("download"), page.keyboard.press("Control+s")]);
  const file = await download.path();
  const got = JSON.parse(fs.readFileSync(file, "utf8"));
  check(
    download.suggestedFilename() === "uploaded.acproj" && got.name === "Uploaded",
    "and File ▸ Save downloads the project",
    download.suggestedFilename(),
  );
  await ctx2.close();
}

async function main() {
  if (!fs.existsSync(path.join(SITE, "index.html"))) throw new Error(`no site in ${SITE}: run scripts/build_web.py`);
  fs.mkdirSync(SHOTS, { recursive: true });
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ac-web-e2e-"));
  const tone = path.join(tmp, "tone-1k.wav");
  writeTone(tone, 1000, 10);
  const port = await freePort();
  const server = await serve(port);
  const browser = await chromium.launch({
    headless: true,
    executablePath: process.env.AC_CHROMIUM || undefined,
    args: [
      "--autoplay-policy=no-user-gesture-required",
      "--use-fake-ui-for-media-stream",
      "--use-fake-device-for-media-stream",
      `--use-file-for-fake-audio-capture=${tone}`,
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
    ],
  });
  const errors = [];
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await context.newPage();
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    page.on("pageerror", (e) => errors.push(String(e)));

    // Start: a 220 Hz saw at −18 dB through a meter and a scope.
    let s = await open(page, port, "start");
    check(s && s.project === "Start", "the Start example loads from the site", s && s.project);
    check(s && s.errors === 0, "it compiles without errors", s && s.errors);
    await page.evaluate(() => window.audioConstructor.play());
    await sleep(1500);
    s = await status(page);
    const rate = s.sampleRate;
    check(s.playing && s.position > rate, "the transport plays and moves", `${s.seconds.toFixed(2)} s`);
    check(s.inStep, "the editor shows the worklet's telemetry for this project");
    check(s.peak[0] > 0.08 && s.peak[0] < 0.2, "the master meter reads a −18 dB saw", s.peak[0].toFixed(3));
    let heard = await listen(page);
    check(heard.rms > 0.03 && heard.rms < 0.15, "the output is a quiet saw", heard.rms.toFixed(4));
    check(Math.abs(heard.freq - 220) < 8, "at 220 Hz", `${heard.freq.toFixed(1)} Hz`);
    const before = s.position;
    await sleep(500);
    s = await status(page);
    check(s.position > before, "the position keeps moving", s.position - before);
    // An edit from the page reaches the worklet: the oscillator to 440 Hz.
    const took = await page.evaluate(() => window.audioConstructor.set_param("2", "freq", "440.0"));
    check(took, "a parameter change is taken");
    await sleep(600);
    heard = await listen(page);
    check(Math.abs(heard.freq - 440) < 8, "and heard: 440 Hz", `${heard.freq.toFixed(1)} Hz`);
    await page.screenshot({ path: path.join(SHOTS, "start.png") });

    // Guitar Pedalboard: the (fake) microphone through tone, echo, level.
    s = await open(page, port, "guitar-pedalboard");
    check(s && s.project === "Guitar Pedalboard", "the Pedalboard example loads", s && s.project);
    s = await until(page, (x) => x.mic === "on", 10000);
    check(s && s.mic === "on", "the page asks for the microphone and gets it", s && s.mic);
    await sleep(1500);
    heard = await listen(page);
    s = await status(page);
    check(heard.rms > 0.05, "the input comes through", heard.rms.toFixed(4));
    check(Math.abs(heard.freq - 1000) < 8, "as the 1 kHz input tone", `${heard.freq.toFixed(1)} Hz`);
    check(s.peak[0] > 0.1, "and the master meter shows it", s.peak[0].toFixed(3));
    await page.screenshot({ path: path.join(SHOTS, "pedalboard.png") });

    // Subtractive Synth: silent until a note comes in (Demo off).
    s = await open(page, port, "subtractive-synth");
    check(s && s.project === "Subtractive Synth", "the Synth example loads", s && s.project);
    const off = await page.evaluate(() => window.audioConstructor.set_param("d", "value", "false"));
    check(off, "Demo turns off");
    await page.evaluate(() => window.audioConstructor.play());
    await sleep(1200);
    const quiet = await listen(page);
    check(quiet.rms < 0.003, "silent with no note", quiet.rms.toFixed(5));
    await page.evaluate(() => window.audioConstructor.midi(new Uint8Array([0x90, 69, 110])));
    await sleep(250);
    heard = await listen(page);
    check(heard.rms > 0.01 && heard.rms > 5 * quiet.rms, "a MIDI note sounds", heard.rms.toFixed(4));
    check(Math.abs(heard.freq - 440) < 8, "at A4, 440 Hz", `${heard.freq.toFixed(1)} Hz`);
    await page.screenshot({ path: path.join(SHOTS, "synth.png") });
    await page.evaluate(() => window.audioConstructor.midi(new Uint8Array([0x80, 69, 0])));
    await sleep(1500);
    const after = await listen(page);
    check(after.rms < heard.rms / 4, "and stops after note off", after.rms.toFixed(5));
    s = await status(page);
    check(s.dropped === 0, "no ring records were dropped", s.dropped);

    // Four Tracks: an example with audio files, which come from the site too.
    s = await open(page, port, "four-tracks");
    check(s && s.project === "Four Tracks", "the Four Tracks example loads", s && s.project);
    check(s && s.missingFiles === 0, "with its audio files", s && s.missingFiles);
    await page.evaluate(() => window.audioConstructor.play());
    await sleep(1500);
    heard = await listen(page);
    check(heard.rms > 0.01, "and its clips play", heard.rms.toFixed(4));

    await files(browser, port, errors);
  } finally {
    await browser.close();
    server.kill();
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  check(errors.length === 0, "the page logged no errors", errors.join(" | "));
  if (failures) {
    console.log(`\n${failures} check(s) failed`);
    process.exit(1);
  }
  console.log("\nall checks passed");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
