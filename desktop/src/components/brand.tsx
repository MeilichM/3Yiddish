import { cn } from "@/lib/utils";

/**
 * 3Yiddish's logo, from the terminal app (src/art.rs and assets/readme/logo.png): the pixel
 * wordmark, drawn here as crisp SVG from the same bitmaps.
 */

const GLYPHS: Record<string, string[]> = {
  "3": ["####.", "....#", "....#", ".###.", "....#", "....#", "####."],
  Y: ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."],
  I: ["###", ".#.", ".#.", ".#.", ".#.", ".#.", "###"],
  D: ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."],
  S: [".####", "#....", "#....", ".###.", "....#", "....#", "####."],
  H: ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
};

/** The rows of a bitmap as runs of filled pixels, so each run is one rectangle. */
function runs(rows: string[], x0: number) {
  const out: { x: number; y: number; w: number }[] = [];
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; ) {
      if (row[x] !== "#") {
        x++;
        continue;
      }
      let w = 0;
      while (row[x + w] === "#") w++;
      out.push({ x: x0 + x, y, w });
      x += w;
    }
  });
  return out;
}

/** "3YIDDISH": the 3 in the brand red, the letters in the brand blue. */
export function Wordmark({ className, title = "3Yiddish" }: { className?: string; title?: string }) {
  const word = "3YIDDISH";
  const parts: { x: number; y: number; w: number; red: boolean }[] = [];
  let x = 0;
  for (const letter of word) {
    const rows = GLYPHS[letter];
    for (const r of runs(rows, x)) parts.push({ ...r, red: letter === "3" });
    x += rows[0].length + 1;
  }
  const width = x - 1;
  return (
    <svg
      role="img"
      aria-label={title}
      viewBox={`0 0 ${width} 7`}
      shapeRendering="crispEdges"
      className={cn("h-4 w-auto", className)}
    >
      {parts.map((p, i) => (
        <rect key={i} x={p.x} y={p.y} width={p.w} height={1} fill={p.red ? "var(--brand-red)" : "var(--brand-blue)"} />
      ))}
    </svg>
  );
}
