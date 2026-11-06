// Syntax-checks the JavaScript inside the floor's HTML pages, which no compiler sees.
// Run: node ci/check_pages.mjs   (exits 1 on any error; no dependencies)
//
// It compiles each inline <script> without running it. That catches the mistake that broke
// the desk page once: an apostrophe inside a single-quoted string.

import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const root = join(fileURLToPath(import.meta.url), "..", "..");
const pages = [
  join(root, "opencry-visual", "index.html"),
  ...readdirSync(join(root, "opencry-visual", "static"))
    .filter((f) => f.endsWith(".html"))
    .map((f) => join(root, "opencry-visual", "static", f)),
];

let failed = 0;
let checked = 0;
for (const page of pages) {
  const html = readFileSync(page, "utf8");
  const name = relative(root, page);
  for (const m of html.matchAll(/<script(\s[^>]*)?>([\s\S]*?)<\/script>/gi)) {
    const attrs = m[1] || "";
    if (/\bsrc\s*=/i.test(attrs)) continue; // a vendored file, not ours
    const type = (attrs.match(/\btype\s*=\s*["']?([^"'\s>]+)/i) || [])[1];
    if (type && !/^(text|application)\/javascript$/i.test(type)) continue;
    const line = html.slice(0, m.index).split("\n").length; // the <script> tag's line
    // Pad with newlines so a reported line number is the page's own.
    const code = "\n".repeat(line - 1) + m[2];
    checked++;
    try {
      new vm.Script(code, { filename: name });
    } catch (e) {
      failed++;
      const where = (e.stack || "").split("\n")[0];
      console.error(`${where}\n  ${e.name}: ${e.message}`);
    }
  }
}
console.log(`${checked} inline scripts in ${pages.length} pages, ${failed} with errors`);
process.exit(failed ? 1 : 0);
