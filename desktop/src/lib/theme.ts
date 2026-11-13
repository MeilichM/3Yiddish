import { useSyncExternalStore } from "react";

export type Theme = "light" | "dark" | "system";

const SAVED = "3yiddish.theme";
const systemDark = window.matchMedia("(prefers-color-scheme: dark)");
const listeners = new Set<() => void>();

function load(): Theme {
  try {
    const saved = localStorage.getItem(SAVED);
    if (saved === "light" || saved === "dark" || saved === "system") return saved;
  } catch {
    // No storage: follow the system.
  }
  return "system";
}

let theme = load();

/** Puts the chosen look on the page: the "dark" class, or the system's setting when "system". */
function apply() {
  const dark = theme === "dark" || (theme === "system" && systemDark.matches);
  document.documentElement.classList.toggle("dark", dark);
}

export function initTheme() {
  apply();
  systemDark.addEventListener("change", apply);
}

export function setTheme(next: Theme) {
  theme = next;
  try {
    localStorage.setItem(SAVED, next);
  } catch {
    // Not saved; it still applies for this session.
  }
  apply();
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useTheme(): Theme {
  return useSyncExternalStore(subscribe, () => theme);
}
