import type { Mark, TakeStatus } from "@/lib/api";

/** Why a span is marked: the phone page's list, in its order (assets/phone/app.js). */
export const REASONS: [string, string][] = [
  ["wrong", "Wrong word"],
  ["weird", "Weird"],
  ["fluency", "Not fluent / my fault"],
  ["accent", "Wrong accent"],
  ["loshn-koydesh", "Loshn koydesh"],
  ["name", "Name"],
  ["english", "English word"],
  ["custom", "Custom…"],
];
export const LABEL: Record<string, string> = {
  ...Object.fromEntries(REASONS),
  dropped: "Dropped word",
  custom: "Custom",
};
/** Each reason's color (the phone page's). */
export const COLOR: Record<string, string> = {
  wrong: "#f0525f",
  weird: "#a56ff0",
  fluency: "#d9962c",
  accent: "#1fa9a3",
  "loshn-koydesh": "#4f86f0",
  name: "#2fa86d",
  english: "#ef7d36",
  custom: "#8a96a6",
  dropped: "#e85aa5",
};
export const STATUS: Record<TakeStatus, string> = {
  pending: "To review",
  correct: "Correct",
  fixed: "Fixed",
  tossed: "Tossed",
};

export function sortMarks(marks: Mark[]) {
  return [...marks].sort((a, b) => a.start - b.start || a.end - b.end);
}

/** The draft with the marks applied: the core's rule (src/freebench.rs `apply`). */
export function applyMarks(words: string[], marks: Mark[]) {
  const out: string[] = [];
  let at = 0;
  for (const m of sortMarks(marks)) {
    out.push(...words.slice(at, m.start));
    if (m.right == null) out.push(...words.slice(m.start, m.end));
    else out.push(...m.right.split(/\s+/).filter(Boolean));
    at = m.end;
  }
  out.push(...words.slice(at));
  return out.join(" ");
}

/**
 * Would a mark over words start..end (end not included; start == end is a place between two
 * words) run into another mark? `skip` is the mark being edited.
 */
export function clashes(marks: Mark[], start: number, end: number, skip: number) {
  return marks.some((m, k) => {
    if (k === skip) return false;
    const place = start === end,
      other = m.start === m.end;
    if (place && other) return m.start === start;
    if (place) return m.start < start && start < m.end;
    if (other) return start < m.start && m.start < end;
    return m.start < end && start < m.end;
  });
}

export const squash = (text: string) => text.trim().replace(/\s+/g, " ");
