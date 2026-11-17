import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
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

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <TooltipProvider delayDuration={400}>
      <App />
      <CopyMenu />
      <ThemedToaster />
    </TooltipProvider>
  </React.StrictMode>,
);
