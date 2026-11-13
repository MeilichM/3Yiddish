/**
 * Push-to-talk shortcuts. They are stored in the global-shortcut plugin's format: modifiers
 * then one key, joined with "+", the key named by KeyboardEvent.code ("Control+Shift+KeyD",
 * "F9", "Alt+Space").
 */
const MODIFIER_CODES: Record<string, string> = {
  ControlLeft: "Control",
  ControlRight: "Control",
  ShiftLeft: "Shift",
  ShiftRight: "Shift",
  AltLeft: "Alt",
  AltRight: "Alt",
  MetaLeft: "Super",
  MetaRight: "Super",
  OSLeft: "Super",
  OSRight: "Super",
};
const ORDER = ["Control", "Alt", "Shift", "Super"];

const mac = navigator.userAgent.includes("Mac");

/** The modifiers held during a key event, in a fixed order. */
export function heldModifiers(e: KeyboardEvent): string[] {
  const held = new Set<string>();
  if (e.ctrlKey) held.add("Control");
  if (e.altKey) held.add("Alt");
  if (e.shiftKey) held.add("Shift");
  if (e.metaKey) held.add("Super");
  return ORDER.filter((m) => held.has(m));
}

export function isModifier(code: string) {
  return code in MODIFIER_CODES;
}

const FROM_KEY: Record<string, string> = {
  " ": "Space",
  Control: "ControlLeft",
  Shift: "ShiftLeft",
  Alt: "AltLeft",
  AltGraph: "AltRight",
  Meta: "MetaLeft",
  OS: "MetaLeft",
};

/**
 * The physical key of an event. Key presses from remappers, macro keyboards and remote
 * desktops can come without one; then the key's name stands in ("y" is KeyY, "7" Digit7).
 */
export function keyCode(e: KeyboardEvent): string {
  if (e.code && e.code !== "Unidentified") return e.code;
  const k = e.key;
  if (/^[a-z]$/i.test(k)) return `Key${k.toUpperCase()}`;
  if (/^\d$/.test(k)) return `Digit${k}`;
  return FROM_KEY[k] ?? k;
}

export function accelerator(modifiers: string[], code: string) {
  return [...modifiers, code].join("+");
}

const NAMES: Record<string, string> = {
  Control: mac ? "⌃" : "Ctrl",
  Alt: mac ? "⌥" : "Alt",
  Shift: mac ? "⇧" : "Shift",
  Super: mac ? "⌘" : "Win",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  PageUp: "Page Up",
  PageDown: "Page Down",
  CapsLock: "Caps Lock",
  ScrollLock: "Scroll Lock",
  NumLock: "Num Lock",
  PrintScreen: "Print Screen",
  ContextMenu: "Menu",
  NumpadAdd: "Num +",
  NumpadSubtract: "Num -",
  NumpadMultiply: "Num *",
  NumpadDivide: "Num /",
  NumpadDecimal: "Num .",
  NumpadEnter: "Num Enter",
  MediaPlayPause: "Play/Pause",
  AudioVolumeMute: "Mute",
  AudioVolumeUp: "Volume Up",
  AudioVolumeDown: "Volume Down",
};

/** One part of a shortcut as people call it: "KeyD" is D, "Digit1" is 1. */
export function keyName(part: string) {
  if (NAMES[part]) return NAMES[part];
  if (/^Key[A-Z]$/.test(part)) return part.slice(3);
  if (/^Digit\d$/.test(part)) return part.slice(5);
  if (/^Numpad\d$/.test(part)) return `Num ${part.slice(6)}`;
  return part;
}

/** "Control+Shift+KeyD" as its parts' names: ["Ctrl", "Shift", "D"]. */
export function parts(shortcut: string) {
  return shortcut.split("+").map(keyName);
}

/**
 * A shortcut without modifiers on a key used for typing takes that key away from every other
 * program while 3Yiddish runs.
 */
export function takesTypingKey(shortcut: string) {
  const all = shortcut.split("+");
  if (all.length > 1) return false;
  return /^(Key[A-Z]|Digit\d|Space|Enter|Tab|Backspace|Delete|Minus|Equal|Bracket|Semicolon|Quote|Comma|Period|Slash|Backslash|Backquote|Arrow)/.test(
    all[0],
  );
}
