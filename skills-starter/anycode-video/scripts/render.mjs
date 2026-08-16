#!/usr/bin/env node
/**
 * anycode-video local MP4 renderer.
 * Adapted from nexu-io/html-video packages/adapter-hyperframes/src/render.ts (Apache-2.0).
 *
 * Usage:
 *   node scripts/render.mjs doctor
 *   node scripts/render.mjs render --input video/index.html --output video/out.mp4 \
 *        --width 1080 --height 1920 --duration 4 --fps 30
 */
import { spawn } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const ADAPTER_VERSION = "anycode-video-playwright@1.0.0";

function usage() {
  console.error(`usage:
  node render.mjs doctor
  node render.mjs render --input <html> --output <mp4> [--width N] [--height N] [--duration SEC] [--fps N]`);
}

function parseArgs(argv) {
  const out = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2);
      const val = argv[i + 1] && !argv[i + 1].startsWith("--") ? argv[++i] : true;
      out[key] = val;
    } else {
      out._.push(a);
    }
  }
  return out;
}

function runCmd(cmd, args) {
  return new Promise((resolveP, reject) => {
    const proc = spawn(cmd, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (c) => {
      stdout += c.toString("utf8");
    });
    proc.stderr.on("data", (c) => {
      stderr += c.toString("utf8");
    });
    proc.on("error", (err) => reject(err));
    proc.on("exit", (code) => resolveP({ code: code ?? 1, stdout, stderr }));
  });
}

async function doctor() {
  const checks = [];
  const nodeV = process.version;
  checks.push({ name: "node", ok: true, value: nodeV });

  let ffmpegOk = false;
  let ffmpegVal = "";
  try {
    const r = await runCmd("ffmpeg", ["-version"]);
    ffmpegOk = r.code === 0;
    ffmpegVal = (r.stdout || r.stderr).split("\n")[0] || "";
  } catch {
    ffmpegOk = false;
  }
  checks.push({
    name: "ffmpeg",
    ok: ffmpegOk,
    value: ffmpegVal,
    hint: ffmpegOk
      ? null
      : "Install ffmpeg: macOS `brew install ffmpeg`; Debian `sudo apt install ffmpeg`",
  });

  let pwOk = false;
  let pwVal = "";
  try {
    const playwright = await import("playwright");
    const browser = await playwright.chromium.launch({ headless: true });
    await browser.close();
    pwOk = true;
    pwVal = "chromium launch ok";
  } catch (err) {
    pwOk = false;
    pwVal = err instanceof Error ? err.message : String(err);
  }
  checks.push({
    name: "playwright-chromium",
    ok: pwOk,
    value: pwVal,
    hint: pwOk
      ? null
      : "Install: `npm i -g playwright` then `npx playwright install chromium` (or in project: `npm i playwright && npx playwright install chromium`)",
  });

  const ok = checks.every((c) => c.ok);
  console.log(
    JSON.stringify(
      { status: ok ? "ok" : "error", checks, version: ADAPTER_VERSION },
      null,
      2,
    ),
  );
  if (!ok) {
    for (const c of checks.filter((x) => !x.ok)) {
      console.error(`[doctor] missing ${c.name}: ${c.hint || c.value}`);
    }
    process.exit(1);
  }
}

function runFfmpeg(args) {
  return new Promise((resolveP, reject) => {
    const proc = spawn("ffmpeg", args, { stdio: ["ignore", "pipe", "pipe"] });
    let stderr = "";
    proc.stderr.on("data", (chunk) => {
      stderr += chunk.toString("utf8");
    });
    proc.on("error", (err) => {
      if (err.code === "ENOENT") {
        reject(
          new Error(
            "ffmpeg not found on PATH. Install: brew install ffmpeg (macOS)",
          ),
        );
      } else reject(err);
    });
    proc.on("exit", (code) => {
      if (code === 0) resolveP();
      else reject(new Error(`ffmpeg exited ${code}: ${stderr.slice(-2000)}`));
    });
  });
}

async function prepareSourceHtml(sourcePath) {
  const raw = await readFile(sourcePath, "utf8");
  const srcMatches = Array.from(
    raw.matchAll(/data-composition-src=["']([^"']+)["']/g),
  );
  if (srcMatches.length === 0) return { loadPath: sourcePath };

  const srcDir = dirname(sourcePath);
  const compMap = {};
  for (const m of srcMatches) {
    const rel = m[1];
    if (compMap[rel] !== undefined) continue;
    const compPath = join(srcDir, rel);
    if (!existsSync(compPath)) continue;
    compMap[rel] = await readFile(compPath, "utf8");
  }
  if (Object.keys(compMap).length === 0) return { loadPath: sourcePath };

  const safeJson = JSON.stringify(compMap)
    .replace(/<\//g, "<\\/")
    .replace(/<!--/g, "<\\!--");
  let out = raw;
  const head = `<script>window.__timelines=window.__timelines||{};window.__COMPOSITIONS__=${safeJson};</script>`;
  out = /<head[^>]*>/i.test(out)
    ? out.replace(/<head[^>]*>/i, (mm) => `${mm}\n${head}`)
    : `${head}\n${out}`;

  const player = `
<script>
(function () {
  function reexec(root) {
    root.querySelectorAll('script').forEach(function (old) {
      if (old.src) { old.parentNode.removeChild(old); return; }
      var s = document.createElement('script');
      s.textContent = '{\\n' + old.textContent + '\\n}';
      old.parentNode.replaceChild(s, old);
    });
  }
  function mountOne(host) {
    var src = host.getAttribute('data-composition-src');
    var text = (window.__COMPOSITIONS__ || {})[src];
    if (!text) return;
    var holder = document.createElement('div');
    holder.innerHTML = text;
    var tpl = holder.querySelector('template');
    host.appendChild(tpl ? tpl.content.cloneNode(true) : holder);
    reexec(host);
  }
  window.__hvPlayAll = function () {
    var tls = window.__timelines || {};
    Object.keys(tls).forEach(function (k) {
      var tl = tls[k];
      if (tl && typeof tl.play === 'function') tl.play(0);
    });
  };
  function boot() {
    window.__timelines = window.__timelines || {};
    Array.prototype.slice
      .call(document.querySelectorAll('[data-composition-src]'))
      .forEach(mountOne);
    setTimeout(function () { if (!window.__hvPlayed) window.__hvPlayAll(); }, 250);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else { boot(); }
})();
</script>`;
  out = out.includes("</body>")
    ? out.replace("</body>", `${player}\n</body>`)
    : out + player;

  const loadPath = join(srcDir, `.hv-render-${Date.now()}.html`);
  await writeFile(loadPath, out, "utf8");
  return {
    loadPath,
    cleanup: async () => {
      await rm(loadPath, { force: true }).catch(() => {});
    },
  };
}

async function render(opts) {
  const inputPath = resolve(opts.input);
  const outputPath = resolve(opts.output);
  const width = Number(opts.width) || 1080;
  const height = Number(opts.height) || 1920;
  const fps = Number(opts.fps) || 30;
  let totalDuration = Math.max(0.5, Number(opts.duration) || 5);
  const explicit = opts.duration != null && opts.duration !== "auto";

  if (!existsSync(inputPath)) {
    throw new Error(`Source HTML not found: ${inputPath}`);
  }
  await mkdir(dirname(outputPath), { recursive: true });

  console.error(`[render] launching chromium ${width}x${height} ${totalDuration}s`);
  let playwright;
  try {
    playwright = await import("playwright");
  } catch (err) {
    throw new Error(
      `playwright not installed. Run: npm i playwright && npx playwright install chromium (${err instanceof Error ? err.message : err})`,
    );
  }

  const recordDir = await mkdtemp(join(tmpdir(), "ac-video-"));
  let browser;
  let webmPath;
  let cleanupSrc;
  let leadInMs = 0;
  const t0 = Date.now();

  try {
    browser = await playwright.chromium.launch({
      headless: true,
      args: ["--no-sandbox", "--disable-blink-features=AutomationControlled"],
    });
    const tWebmStart = Date.now();
    const context = await browser.newContext({
      viewport: { width, height },
      deviceScaleFactor: 1,
      recordVideo: { dir: recordDir, size: { width, height } },
    });
    const page = await context.newPage();

    await page.addInitScript(() => {
      const style = document.createElement("style");
      style.id = "__hv_freeze";
      style.textContent =
        "*, *::before, *::after { animation-play-state: paused !important;" +
        " -webkit-animation-play-state: paused !important; }";
      const attach = () =>
        (document.head || document.documentElement).appendChild(style);
      if (document.head || document.documentElement) attach();
      else document.addEventListener("DOMContentLoaded", attach, { once: true });
      window.__hvUnfreeze = () => {
        document.getElementById("__hv_freeze")?.remove();
      };
    });

    const prepared = await prepareSourceHtml(inputPath);
    cleanupSrc = prepared.cleanup;
    const fileUrl = pathToFileURL(prepared.loadPath).href;
    await page.goto(fileUrl, { waitUntil: "domcontentloaded" });

    console.error("[render] loading fonts");
    await page
      .evaluate(
        () =>
          new Promise((resolveP) => {
            const fonts = document.fonts;
            if (!fonts || typeof fonts.ready?.then !== "function") {
              resolveP();
              return;
            }
            let settled = false;
            const finish = () => {
              if (settled) return;
              settled = true;
              requestAnimationFrame(() =>
                requestAnimationFrame(() => resolveP()),
              );
            };
            const cap = setTimeout(finish, 8000);
            const links = Array.from(
              document.querySelectorAll('link[rel="stylesheet"]'),
            );
            const linkDone = links.map((link) => {
              try {
                if (link.sheet && link.sheet.cssRules) return Promise.resolve();
              } catch {
                /* wait */
              }
              return new Promise((r) => {
                const done = () => r();
                link.addEventListener("load", done, { once: true });
                link.addEventListener("error", done, { once: true });
                setTimeout(done, 6000);
              });
            });
            Promise.all(linkDone)
              .then(() => {
                const loads = [];
                fonts.forEach((face) => {
                  try {
                    loads.push(face.load().catch(() => undefined));
                  } catch {
                    /* ignore */
                  }
                });
                return Promise.all(loads);
              })
              .then(() => fonts.ready)
              .then(() => {
                clearTimeout(cap);
                finish();
              })
              .catch(() => {
                clearTimeout(cap);
                finish();
              });
          }),
      )
      .catch(() => {});

    await page.waitForTimeout(100);

    try {
      const animMs = await page.evaluate(() => {
        let maxMs = 0;
        Array.from(document.querySelectorAll("*")).forEach((el) => {
          const s = getComputedStyle(el);
          const durs = (s.animationDuration || "").split(",");
          const dels = (s.animationDelay || "").split(",");
          const iters = (s.animationIterationCount || "").split(",");
          durs.forEach((d, i) => {
            if ((iters[i] || "").trim() === "infinite") return;
            maxMs = Math.max(
              maxMs,
              ((parseFloat(d) || 0) + (parseFloat(dels[i] || "0") || 0)) * 1000,
            );
          });
        });
        const g = window.gsap;
        let gsapMs = 0;
        const children =
          g?.globalTimeline?.getChildren?.(true, true, true) ?? [];
        for (const c of children) {
          const repeat =
            typeof c.repeat === "function" ? c.repeat() : (c.vars?.repeat ?? 0);
          if (repeat === -1) continue;
          const td = typeof c.totalDuration === "function" ? c.totalDuration() : 0;
          if (Number.isFinite(td)) gsapMs = Math.max(gsapMs, td * 1000);
        }
        return Math.max(maxMs, gsapMs);
      });
      const needed = Math.min(30, (animMs + 400) / 1000);
      if (!explicit && needed > totalDuration) {
        console.error(`[render] extending to ${needed.toFixed(1)}s for animation`);
        totalDuration = needed;
      }
    } catch {
      /* probe failed */
    }

    await page
      .evaluate(() => {
        const w = window;
        if (typeof w.__hvPlayAll === "function") {
          w.__hvPlayed = true;
          w.__hvPlayAll();
          return true;
        }
        return false;
      })
      .catch(() => false);

    await page
      .evaluate(() => {
        window.__hvUnfreeze?.();
      })
      .catch(() => {});
    leadInMs = Date.now() - tWebmStart;

    console.error(`[render] recording ${totalDuration}s`);
    const totalMs = Math.round(totalDuration * 1000);
    const tick = 250;
    const start = Date.now();
    while (Date.now() - start < totalMs) {
      await page.waitForTimeout(
        Math.min(tick, totalMs - (Date.now() - start)),
      );
    }

    await context.close();
    const candidates = readdirSync(recordDir).filter((f) => f.endsWith(".webm"));
    if (candidates.length === 0) {
      throw new Error(`Playwright produced no webm in ${recordDir}`);
    }
    candidates.sort();
    webmPath = join(recordDir, candidates[candidates.length - 1]);
  } finally {
    if (browser) await browser.close().catch(() => {});
    if (cleanupSrc) await cleanupSrc().catch(() => {});
  }

  console.error("[render] encoding mp4");
  const seekSec = leadInMs > 200 ? Math.max(0, (leadInMs - 120) / 1000) : 0;
  await runFfmpeg([
    "-y",
    ...(seekSec > 0 ? ["-ss", seekSec.toFixed(3)] : []),
    "-i",
    webmPath,
    ...(explicit
      ? ["-vf", `tpad=stop_mode=clone:stop_duration=${totalDuration}`]
      : []),
    "-t",
    String(totalDuration),
    "-r",
    String(fps),
    "-c:v",
    "libx264",
    "-pix_fmt",
    "yuv420p",
    "-preset",
    "medium",
    "-crf",
    "20",
    "-movflags",
    "+faststart",
    outputPath,
  ]);

  await rm(recordDir, { recursive: true, force: true }).catch(() => {});
  const st = await stat(outputPath);
  const result = {
    status: "ok",
    output_path: outputPath,
    duration_sec: totalDuration,
    file_size_bytes: st.size,
    width,
    height,
    fps,
    render_wall_clock_sec: (Date.now() - t0) / 1000,
    engine: ADAPTER_VERSION,
  };
  console.log(JSON.stringify(result, null, 2));
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const cmd = args._[0];
  if (cmd === "doctor") {
    await doctor();
    return;
  }
  if (cmd === "render") {
    if (!args.input || !args.output) {
      usage();
      process.exit(2);
    }
    await render(args);
    return;
  }
  usage();
  process.exit(2);
}

main().catch((err) => {
  console.error(`[render] failed: ${err instanceof Error ? err.message : err}`);
  process.exit(1);
});
