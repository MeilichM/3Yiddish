import { useEffect, useRef, useState } from "react";
import { TriangleAlert } from "lucide-react";
import { api, message } from "@/lib/api";
import { accelerator, heldModifiers, isModifier, keyCode, keyName, parts, takesTypingKey } from "@/lib/shortcut";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/** A shortcut as key caps; long combinations wrap instead of squeezing. */
export function Keys({ shortcut, className }: { shortcut: string; className?: string }) {
  return (
    <span className={cn("inline-flex flex-wrap items-center justify-end gap-1", className)}>
      {parts(shortcut).map((p, i) => (
        <kbd
          key={i}
          className="min-w-5 rounded bg-muted px-1.5 py-0.5 text-center font-sans text-[11px] font-medium whitespace-nowrap"
        >
          {p}
        </kbd>
      ))}
    </span>
  );
}

/**
 * The push-to-talk key: click, press any key or combination, and it is in use at once. The
 * current shortcut is let go while choosing, so pressing it is just a key press.
 */
export function ShortcutPicker({
  shortcut,
  onChange,
  onError,
}: {
  shortcut: string;
  onChange: (shortcut: string) => void;
  onError: (error: string | null) => void;
}) {
  const [capturing, setCapturing] = useState(false);
  const [held, setHeld] = useState<string[]>([]);
  const paused = useRef(false);

  function pause(on: boolean) {
    if (paused.current === on) return;
    paused.current = on;
    void api.shortcutPause(on);
  }
  useEffect(() => () => pause(false), []);

  async function use(next: string) {
    onError(null);
    try {
      await api.shortcutSet(next);
      onChange(next);
    } catch (e) {
      onError(message(e));
    }
  }

  useEffect(() => {
    if (!capturing) return;
    const stop = () => {
      setCapturing(false);
      setHeld([]);
      pause(false);
    };
    const down = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopImmediatePropagation();
      const code = keyCode(e);
      if (code === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) return stop();
      const modifiers = heldModifiers(e);
      if (isModifier(code) || !code) return setHeld(modifiers);
      stop();
      void use(accelerator(modifiers, code));
    };
    const up = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopImmediatePropagation();
      setHeld(heldModifiers(e));
    };
    const blur = () => stop();
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
      window.removeEventListener("blur", blur);
    };
  }, [capturing]);

  const warn = !capturing && takesTypingKey(shortcut);

  return (
    <div className="flex items-center gap-2">
      {warn && (
        <Tooltip>
          <TooltipTrigger asChild>
            <TriangleAlert className="size-4 text-amber-500" aria-label="Warning" />
          </TooltipTrigger>
          <TooltipContent className="max-w-56">This key won't type in other programs while 3Yiddish runs.</TooltipContent>
        </Tooltip>
      )}
      <button
        type="button"
        onClick={() => {
          onError(null);
          setCapturing(true);
          pause(true);
        }}
        className={cn(
          "flex h-8 max-w-56 min-w-24 items-center justify-center rounded-md border px-2 text-xs transition-colors",
          capturing ? "border-ring ring-[3px] ring-ring/30" : "hover:bg-accent",
        )}
        aria-label="Change the push-to-talk shortcut"
      >
        {capturing ? (
          <span className="text-muted-foreground">
            {held.length ? `${held.map(keyName).join(" + ")} + …` : "Press keys…"}
          </span>
        ) : (
          <Keys shortcut={shortcut} />
        )}
      </button>
    </div>
  );
}
