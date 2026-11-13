import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { cn } from "@/lib/utils";

/** Windows draws no title bar of its own here (lib.rs turns it off), so the header has its buttons. */
export const windows = navigator.userAgent.includes("Windows");

/**
 * Minimize, maximize / restore and close, drawn like Windows 11's own: 46px wide, the full
 * height of the top row, with the system's icon font (Segoe Fluent Icons, or MDL2 Assets on
 * Windows 10). Close goes through the window's close request, so the tray behavior holds.
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    const check = () => void win.isMaximized().then(setMaximized);
    check();
    const off = win.onResized(check);
    return () => void off.then((f) => f());
  }, []);

  const win = getCurrentWindow();
  return (
    <div className="ms-2 flex h-full self-stretch">
      <Control label="Minimize" glyph={"\uE921"} onClick={() => void win.minimize()} />
      <Control
        label={maximized ? "Restore" : "Maximize"}
        glyph={maximized ? "\uE923" : "\uE922"}
        onClick={() => void win.toggleMaximize()}
      />
      <Control label="Close" glyph={"\uE8BB"} close onClick={() => void win.close()} />
    </div>
  );
}

function Control({ label, glyph, close, onClick }: { label: string; glyph: string; close?: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      tabIndex={-1}
      onClick={onClick}
      className={cn(
        "flex h-full w-[46px] items-center justify-center text-[10px] text-foreground/80 transition-colors",
        "[font-family:'Segoe_Fluent_Icons','Segoe_MDL2_Assets']",
        close ? "hover:bg-[#c42b1c] hover:text-white active:bg-[#c42b1c]/80" : "hover:bg-foreground/10 active:bg-foreground/5",
      )}
    >
      {glyph}
    </button>
  );
}
