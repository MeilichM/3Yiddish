import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { Pill } from "@/components/pill";
import { CopyMenu } from "@/components/copy-menu";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { initTheme, useTheme } from "@/lib/theme";
import "./index.css";

// Dark, light, or the system's setting (followed live), as chosen in the header.
initTheme();

function ThemedToaster() {
  return <Toaster position="top-center" theme={useTheme()} />;
}

// The push-to-talk pill's window (src-tauri's pill.rs) shows only the pill, on a clear background.
const pill = getCurrentWindow().label === "pill";
if (pill) document.documentElement.classList.add("pill");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  pill ? (
    <React.StrictMode>
      <Pill />
    </React.StrictMode>
  ) : (
    <React.StrictMode>
      <TooltipProvider delayDuration={400}>
        <App />
        <CopyMenu />
        <ThemedToaster />
      </TooltipProvider>
    </React.StrictMode>
  ),
);
