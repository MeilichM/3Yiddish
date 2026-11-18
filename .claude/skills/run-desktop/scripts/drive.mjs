// Drive the running desktop app through WebView2's debug port (see launch.cmd).
//   node drive.mjs <screenshot name or ""> "<async JS body; has `page` and `fs`; may return a value>"
// Screenshots go to <test home>\shots\<name>.png (TESTHOME env, else %TEMP%\3y-desktop-test).
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright-core";

globalThis.fs = fs;
const [name, code = ""] = process.argv.slice(2);
const home = process.env.TESTHOME || path.join(process.env.TEMP || ".", "3y-desktop-test");
const browser = await chromium.connectOverCDP("http://localhost:9333");
const page = browser.contexts()[0].pages()[0];
const logs = [];
page.on("console", (m) => logs.push(`${m.type()}: ${m.text()}`));
try {
  const out = await new Function("page", `return (async () => { ${code} })()`)(page);
  if (out !== undefined) console.log("RESULT:", typeof out === "string" ? out : JSON.stringify(out, null, 1));
} catch (e) {
  console.log("ERROR:", e.message);
}
if (name) {
  const file = path.join(home, "shots", `${name}.png`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  await page.screenshot({ path: file });
  console.log("SHOT:", file);
}
if (logs.length) console.log(logs.join("\n"));
// Disconnects only; the app keeps running.
await browser.close().catch(() => {});
