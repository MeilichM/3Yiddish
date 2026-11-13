import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { initTheme, useTheme } from "@/lib/theme";
import "./index.css";

// Dark, light, or the system's setting (followed live), as chosen in the header.
initTheme();

// No browser context menu or reload shortcuts: this is an app, not a web page.
window.addEventListener("contextmenu", (e) => {
  const el = e.target as HTMLElement;
  if (!el.closest("input, textarea, [dir='rtl']")) e.preventDefault();
});

function ThemedToaster() {
  return <Toaster position="top-center" theme={useTheme()} />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <TooltipProvider delayDuration={400}>
      <App />
      <ThemedToaster />
    </TooltipProvider>
  </React.StrictMode>,
);
