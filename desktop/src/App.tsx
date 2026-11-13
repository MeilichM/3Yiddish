import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ChevronDown, FileAudio, Mic, RefreshCw, Settings } from "lucide-react";
import { toast } from "sonner";
import { api, message, type Compute, type Progress, type Status } from "@/lib/api";
import { Composer } from "@/components/composer";
import { EntryView, type Entry } from "@/components/entry";
import { ModelSetup } from "@/components/model-setup";
import { SettingsDialog } from "@/components/settings-dialog";
import { Keys } from "@/components/shortcut-setting";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Progress as Bar } from "@/components/ui/progress";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

const SAVED = "3yiddish.entries";
const BARS = 48;

function loadEntries(): Entry[] {
  try {
    const saved: Entry[] = JSON.parse(localStorage.getItem(SAVED) ?? "[]");
    // A transcription cut off by closing the app is offered again.
    return saved.map((e) =>
      e.state === "transcribing" ? { ...e, state: "error", error: "Interrupted." } : e,
    );
  } catch {
    return [];
  }
}

/** Space starts a take unless the person is typing somewhere, or a dialog or menu is open. */
function typing(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  if (document.querySelector('[role="dialog"], [role="menu"]')) return true;
  return !!el && (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName));
}

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [entries, setEntries] = useState<Entry[]>(loadEntries);
  const [recording, setRecording] = useState(false);
  const [levels, setLevels] = useState<number[]>(() => Array(BARS).fill(0));
  const [seconds, setSeconds] = useState(0);
  const [pulling, setPulling] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [pullError, setPullError] = useState<string | null>(null);
  const [newer, setNewer] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [shortcut, setShortcut] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [checking, setChecking] = useState(false);
  /** The take going now came from the push-to-talk shortcut. */
  const [ptt, setPtt] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const busy = useRef(false); // a start/stop is on its way

  const ready = !!status?.model;

  const refresh = useCallback(async () => {
    const s = await api.status();
    setStatus(s);
    return s;
  }, []);

  useEffect(() => {
    refresh().then((s) => {
      if (s.model) api.warm().catch((e) => toast.error(message(e)));
      api.newerModel().then((v) => setNewer(s.model ? v : null));
    });
    const off = api.onModelProgress(setProgress);
    return () => void off.then((f) => f());
  }, [refresh]);

  useEffect(() => {
    api.shortcutGet().then((info) => {
      setShortcut(info.shortcut);
      if (info.error) toast.error(info.error);
    });
    const off = api.onPtt((e) => {
      switch (e.state) {
        case "recording":
          setLevels(Array(BARS).fill(0));
          setSeconds(0);
          setPtt(true);
          setRecording(true);
          break;
        case "cancelled":
          setPtt(false);
          setRecording(false);
          if (document.visibilityState === "visible") toast(e.reason);
          break;
        case "transcribing":
          setPtt(false);
          setRecording(false);
          setEntries((all) => [...all, { clip: e.clip, recorded: true, at: Date.now(), state: "transcribing" }]);
          break;
        case "done":
          update(e.id, { state: "done", text: e.transcript.text, ms: e.transcript.ms, compute: e.transcript.compute });
          break;
        case "error":
          update(e.id, { state: "error", error: e.error });
          break;
      }
    });
    return () => void off.then((f) => f());
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(SAVED, JSON.stringify(entries.slice(-200)));
    } catch {
      /* storage full or unavailable: history just is not kept */
    }
  }, [entries]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [entries.length, recording]);

  const update = (id: string, patch: Partial<Entry>) =>
    setEntries((all) => all.map((e) => (e.clip.id === id ? { ...e, ...patch } : e)));

  /** Transcribe in order, one at a time (the engine does one at a time anyway). */
  const transcribe = useCallback((entry: Entry) => {
    update(entry.clip.id, { state: "transcribing", error: undefined });
    queue.current = queue.current.then(async () => {
      try {
        const t = await api.transcribe(entry.clip.path);
        update(entry.clip.id, { state: "done", text: t.text, ms: t.ms, compute: t.compute });
        setStatus((s) => (s ? { ...s, compute: t.compute } : s));
      } catch (e) {
        update(entry.clip.id, { state: "error", error: message(e) });
      }
    });
  }, []);

  const add = useCallback(
    (clip: Entry["clip"], recorded: boolean) => {
      const entry: Entry = { clip, recorded, at: Date.now(), state: "transcribing" };
      setEntries((all) => [...all, entry]);
      transcribe(entry);
    },
    [transcribe],
  );

  // ---- Recording ----
  const start = useCallback(async () => {
    if (busy.current || recording || !ready) return;
    busy.current = true;
    try {
      await api.recordStart();
      setLevels(Array(BARS).fill(0));
      setSeconds(0);
      setRecording(true);
    } catch (e) {
      toast.error(message(e));
    } finally {
      busy.current = false;
    }
  }, [recording, ready]);

  const stop = useCallback(async () => {
    if (busy.current || !recording) return;
    busy.current = true;
    setRecording(false);
    setPtt(false);
    try {
      add(await api.recordStop(), true);
    } catch (e) {
      toast(message(e));
    } finally {
      busy.current = false;
    }
  }, [recording, add]);

  const cancel = useCallback(() => {
    if (!recording) return;
    setRecording(false);
    setPtt(false);
    void api.recordCancel();
  }, [recording]);

  useEffect(() => {
    if (!recording) return;
    const timer = setInterval(async () => {
      const l = await api.recordLevel();
      if (l.failed) {
        toast.error(l.failed);
        cancel();
        return;
      }
      setSeconds(l.seconds);
      setLevels((prev) => [...prev.slice(1), l.level]);
    }, 60);
    return () => clearInterval(timer);
  }, [recording, cancel]);

  // Hold Space to talk; Esc cancels.
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.code === "Space" && !e.repeat && !typing(e.target)) {
        e.preventDefault();
        void start();
      } else if (e.code === "Escape") cancel();
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === "Space" && !typing(e.target)) {
        e.preventDefault();
        void stop();
      }
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, [start, stop, cancel]);

  // ---- Files: drag and drop, + button, paste ----
  const importFiles = useCallback(
    async (files: File[]) => {
      for (const file of files) {
        try {
          add(await api.importFile(file), false);
        } catch (e) {
          toast.error(`${file.name}: ${message(e)}`);
        }
      }
    },
    [add],
  );

  useEffect(() => {
    const off = getCurrentWebview().onDragDropEvent(async (e) => {
      if (e.payload.type === "enter" || e.payload.type === "over") setDragging(true);
      else if (e.payload.type === "leave") setDragging(false);
      else if (e.payload.type === "drop") {
        setDragging(false);
        if (!ready) return;
        for (const path of e.payload.paths) {
          try {
            add(await api.importPath(path), false);
          } catch (err) {
            toast.error(`${path.split(/[\\/]/).pop()}: ${message(err)}`);
          }
        }
      }
    });
    return () => void off.then((f) => f());
  }, [add, ready]);

  useEffect(() => {
    const paste = (e: ClipboardEvent) => {
      const files = Array.from(e.clipboardData?.files ?? []);
      if (files.length && ready) {
        e.preventDefault();
        void importFiles(files);
      }
    };
    window.addEventListener("paste", paste);
    return () => window.removeEventListener("paste", paste);
  }, [importFiles, ready]);

  // ---- Model ----
  async function pull() {
    setPulling(true);
    setPullError(null);
    setProgress(null);
    try {
      const version = await api.pullModel();
      setNewer(null);
      await refresh();
      toast.success(`Speech model ${version} is ready`);
      api.warm().catch((e) => toast.error(message(e)));
    } catch (e) {
      setPullError(message(e));
      if (ready) toast.error(message(e));
    } finally {
      setPulling(false);
    }
  }

  /** The update button: download the newer model when there is one, else look for one. */
  async function checkForUpdates() {
    if (pulling || checking) return;
    if (newer) return pull();
    setChecking(true);
    try {
      const v = await api.newerModel();
      if (v) {
        setNewer(v);
        await pull();
      } else toast.success("You have the latest speech model.");
    } finally {
      setChecking(false);
    }
  }

  async function rollback() {
    try {
      const version = await api.rollbackModel();
      await refresh();
      toast.success(`Back to model ${version}`);
    } catch (e) {
      toast.error(message(e));
    }
  }

  async function setCompute(mode: string) {
    if (!mode || mode === status?.compute) return;
    try {
      await api.setCompute(mode as Compute);
      setStatus((s) => (s ? { ...s, compute: mode as Compute } : s));
      api.warm().catch((e) => toast.error(message(e)));
    } catch (e) {
      toast.error(message(e));
    }
  }

  function remove(entry: Entry) {
    setEntries((all) => all.filter((e) => e.clip.id !== entry.clip.id));
    void api.forgetAudio(entry.clip.path); // only the app's own copies are deleted
  }

  function clearHistory() {
    for (const e of entries) void api.forgetAudio(e.clip.path);
    setEntries([]);
  }

  return (
    <div className="relative flex h-screen flex-col bg-background text-foreground">
      <header className="flex items-center justify-between gap-3 border-b px-4 py-2.5">
        <div className="flex items-center gap-2">
          <span className="text-base font-semibold tracking-tight">3Yiddish</span>
          {status?.model && (
            <Badge variant="secondary" className="font-normal tabular-nums">
              Model {status.model}
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-1">
          {status && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm" className="h-8 gap-1 px-2 text-xs font-medium text-muted-foreground">
                  {status.compute.toUpperCase()}
                  <ChevronDown className="size-3.5 opacity-60" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="min-w-32">
                <DropdownMenuRadioGroup value={status.compute} onValueChange={setCompute}>
                  <DropdownMenuRadioItem value="gpu" disabled={!status.gpuAvailable} title={status.gpuName ?? undefined}>
                    GPU
                  </DropdownMenuRadioItem>
                  <DropdownMenuRadioItem value="cpu">CPU</DropdownMenuRadioItem>
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          {ready && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="icon"
                  variant="ghost"
                  className="relative size-8"
                  onClick={checkForUpdates}
                  disabled={pulling || checking}
                  aria-label="Check for updates"
                >
                  <RefreshCw className={pulling || checking ? "animate-spin" : ""} />
                  {newer && !pulling && (
                    <span className="absolute top-1.5 right-1.5 size-2 rounded-full bg-red-500 ring-2 ring-background" />
                  )}
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {pulling
                  ? "Downloading the new model…"
                  : newer
                    ? `New speech model ${newer}: click to download`
                    : "Check for updates"}
              </TooltipContent>
            </Tooltip>
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button size="icon" variant="ghost" className="size-8" onClick={() => setSettingsOpen(true)} aria-label="Settings">
                <Settings />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Settings</TooltipContent>
          </Tooltip>
        </div>
      </header>

      {pulling && ready && progress && (
        <div className="flex items-center gap-3 border-b px-4 py-2 text-xs text-muted-foreground">
          <span className="shrink-0">Downloading the new model</span>
          <Bar value={progress.total ? (progress.done / progress.total) * 100 : 0} className="flex-1" />
        </div>
      )}

      {status && !ready ? (
        <ModelSetup progress={progress} busy={pulling} error={pullError} onDownload={pull} />
      ) : (
        <main className="flex-1 overflow-y-auto">
          <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-4 py-6">
            {entries.length === 0 && !recording && (
              <div className="flex flex-col items-center gap-3 py-24 text-center">
                <div className="flex size-12 items-center justify-center rounded-2xl border bg-card">
                  <Mic className="size-5" />
                </div>
                <h1 className="text-xl font-semibold tracking-tight">Say something in Yiddish</h1>
                <p className="max-w-sm text-sm text-muted-foreground">
                  Hold <kbd className="rounded border bg-muted px-1.5 py-0.5 text-xs">Space</kbd> or
                  click the microphone, speak, and the text appears here. Click it to copy. Everything
                  stays on this computer.
                </p>
                {shortcut && (
                  <p className="flex flex-wrap items-center justify-center gap-1.5 text-sm text-muted-foreground">
                    In any other program, hold <Keys shortcut={shortcut} /> and the Yiddish is typed there.
                  </p>
                )}
              </div>
            )}
            {entries.map((entry) => (
              <EntryView
                key={entry.clip.id}
                entry={entry}
                onRetry={() => transcribe(entry)}
                onRemove={() => remove(entry)}
              />
            ))}
            <div ref={bottom} />
          </div>
        </main>
      )}

      <footer className="mx-auto w-full max-w-3xl px-4 pb-4">
        <Composer
          recording={recording}
          pushToTalk={ptt}
          levels={levels}
          seconds={seconds}
          disabled={!ready}
          onStart={start}
          onStop={stop}
          onCancel={cancel}
          onFiles={importFiles}
        />
      </footer>

      <SettingsDialog
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        shortcut={shortcut}
        onShortcut={setShortcut}
        model={status?.model ?? null}
        appVersion={status?.appVersion}
        entries={entries.length}
        onRollback={rollback}
        onClearHistory={clearHistory}
      />

      {dragging && ready && (
        <div className="pointer-events-none absolute inset-2 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary/60 bg-background/80 backdrop-blur-sm">
          <div className="flex flex-col items-center gap-2 text-sm">
            <FileAudio className="size-8" />
            Drop audio to transcribe it
          </div>
        </div>
      )}
    </div>
  );
}
