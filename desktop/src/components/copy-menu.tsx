import { useEffect, useState } from "react";
import { Copy } from "lucide-react";

const mac = navigator.userAgent.includes("Mac");

/** The selected text: in a text field, or anywhere on the page. */
function selected(target: EventTarget | null): string {
  const el = target as HTMLInputElement | HTMLTextAreaElement | null;
  if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA") && el.selectionStart != null) {
    return el.value.slice(el.selectionStart, el.selectionEnd ?? el.selectionStart);
  }
  return window.getSelection()?.toString() ?? "";
}

/**
 * The right-click menu: only Copy, and only when there is text selected. The browser's own
 * menu (Print, More tools…) never shows: this is an app, not a web page.
 */
export function CopyMenu() {
  const [menu, setMenu] = useState<{ x: number; y: number; text: string } | null>(null);

  useEffect(() => {
    const open = (e: MouseEvent) => {
      e.preventDefault();
      const text = selected(e.target);
      // Kept inside the window: the menu is about 140 by 40 pixels.
      setMenu(
        text.trim()
          ? { x: Math.min(e.clientX, window.innerWidth - 148), y: Math.min(e.clientY, window.innerHeight - 48), text }
          : null,
      );
    };
    const close = () => setMenu(null);
    const key = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("contextmenu", open);
    window.addEventListener("pointerdown", close);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("contextmenu", open);
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", key);
    };
  }, []);

  if (!menu) return null;
  return (
    <div
      role="menu"
      style={{ left: menu.x, top: menu.y }}
      className="fixed z-50 min-w-36 rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10 animate-in fade-in-0 zoom-in-95"
      // Clicking the menu is not clicking away from it.
      onPointerDown={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        role="menuitem"
        autoFocus
        className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-none hover:bg-accent focus-visible:bg-accent [&_svg]:size-4 [&_svg]:text-muted-foreground"
        onClick={() => {
          void navigator.clipboard.writeText(menu.text);
          setMenu(null);
        }}
      >
        <Copy /> Copy
        <span className="ms-auto text-xs text-muted-foreground">{mac ? "⌘C" : "Ctrl+C"}</span>
      </button>
    </div>
  );
}
