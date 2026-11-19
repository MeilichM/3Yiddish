import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { toast } from "sonner";
import { api, message, type Family } from "@/lib/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { ShortcutPicker } from "@/components/shortcut-setting";
import { About } from "@/components/about";
import { modelName } from "@/components/model-setup";

const windows = navigator.userAgent.includes("Windows");

function Group({ children }: { children: React.ReactNode }) {
  return <div className="divide-y rounded-xl border">{children}</div>;
}

function Row({ label, htmlFor, children }: { label: string; htmlFor?: string; children: React.ReactNode }) {
  return (
    <div className="flex min-h-12 items-center justify-between gap-4 px-4 py-2">
      <label htmlFor={htmlFor} className="text-sm">
        {label}
      </label>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

export function SettingsDialog({
  open,
  onOpenChange,
  shortcut,
  onShortcut,
  model,
  activeModel,
  appVersion,
  entries,
  onRollback,
  onClearHistory,
  contributing,
  onContributing,
}: {
  /** The Contribute switch; null where nothing is ever sent (the owner's computer). */
  contributing: boolean | null;
  onContributing: (on: boolean) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  shortcut: string | null;
  onShortcut: (shortcut: string | null) => void;
  model: string | null;
  /** The active model's version: Settings lists the models again when it changes. */
  activeModel: string | null;
  appVersion?: string;
  entries: number;
  onRollback: (family: Family) => Promise<void>;
  onClearHistory: () => void;
}) {
  const [autostart, setAutostart] = useState(false);
  const [fallback, setFallback] = useState("F9");
  /** The shortcut to come back to when push-to-talk is switched on again. */
  const [last, setLast] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  /** Settings shows the About page instead of the settings. */
  const [about, setAbout] = useState(false);
  /** Each kind of model here (Whisper, Omnilingual), each with its own Roll back. */
  const [families, setFamilies] = useState<Family[]>([]);

  useEffect(() => {
    if (open) api.modelFamilies().then(setFamilies);
  }, [open, activeModel]);

  useEffect(() => {
    if (!open) {
      setAbout(false);
      return setConfirmClear(false);
    }
    api.autostartGet().then(setAutostart);
    api.shortcutGet().then((info) => {
      setFallback(info.default);
      setError(info.error);
    });
  }, [open]);

  useEffect(() => {
    if (shortcut) setLast(shortcut);
  }, [shortcut]);

  async function pushToTalk(on: boolean) {
    const next = on ? (last ?? fallback) : null;
    setError(null);
    try {
      await api.shortcutSet(next);
      onShortcut(next);
    } catch (e) {
      setError(message(e));
    }
  }

  async function startAtLogin(on: boolean) {
    try {
      await api.autostartSet(on);
      setAutostart(on);
    } catch (e) {
      toast.error(message(e));
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="gap-5 sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-1">
            {about && (
              <Button
                variant="ghost"
                size="icon"
                className="-ms-2 size-7"
                onClick={() => setAbout(false)}
                aria-label="Back to Settings"
              >
                <ChevronLeft />
              </Button>
            )}
            {about ? "About" : "Settings"}
          </DialogTitle>
          <DialogDescription className="sr-only">{about ? "About 3Yiddish" : "3Yiddish settings"}</DialogDescription>
        </DialogHeader>

        {about ? (
          <About appVersion={appVersion} model={model} />
        ) : (
        <div className="flex flex-col gap-3">
          <Group>
            <Row label={windows ? "Start with Windows" : "Open at login"} htmlFor="autostart">
              <Switch id="autostart" checked={autostart} onCheckedChange={startAtLogin} />
            </Row>
            <Row label="Push-to-talk" htmlFor="ptt">
              <Switch id="ptt" checked={!!shortcut} onCheckedChange={pushToTalk} />
            </Row>
            {shortcut && (
              <Row label="Shortcut">
                <ShortcutPicker shortcut={shortcut} onChange={onShortcut} onError={setError} />
              </Row>
            )}
          </Group>
          {error && <p className="px-1 text-xs text-destructive">{error}</p>}

          {contributing != null && (
            <Group>
              <div>
                <Row label="Contribute" htmlFor="contribute">
                  <Switch id="contribute" checked={contributing} onCheckedChange={onContributing} />
                </Row>
                <p className="-mt-1 px-4 pb-3 text-xs leading-relaxed text-muted-foreground">
                  Off: nothing leaves your computer. On: what you transcribe from now on, the
                  recordings with their text and technical facts (no name), is sent to 3Yiddish to
                  test and improve Yiddish speech to text; nothing from before is sent.
                </p>
              </div>
            </Group>
          )}

          {/* Each kind of model rolls back on its own, to the version of it before. */}
          {families.length > 0 && (
            <Group>
              {families.map((f) => (
                <div key={f.label} className="flex min-h-12 items-center justify-between gap-4 px-4 py-2">
                  <div className="flex min-w-0 flex-col">
                    <span className="flex items-center gap-2 text-sm">
                      {f.label}-based model
                      {f.active && (
                        <span className="rounded-full bg-primary/10 px-1.5 py-px text-[0.6875rem] font-medium">In use</span>
                      )}
                    </span>
                    <span className="text-xs text-muted-foreground tabular-nums">Version {f.version}</span>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 shrink-0 px-2 text-xs"
                    disabled={!f.older}
                    title={
                      f.older
                        ? `Go back to version ${f.older} and use ${modelName(f.label)}`
                        : "No earlier version on this computer"
                    }
                    onClick={() => onRollback(f).then(() => api.modelFamilies().then(setFamilies))}
                  >
                    Roll back
                  </Button>
                </div>
              ))}
            </Group>
          )}

          <Group>
            <Row label="History">
              <span className="text-sm text-muted-foreground tabular-nums">{entries}</span>
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2 text-xs text-destructive hover:text-destructive"
                disabled={!entries}
                onClick={() => {
                  if (!confirmClear) return setConfirmClear(true);
                  setConfirmClear(false);
                  onClearHistory();
                }}
              >
                {confirmClear ? "Click to confirm" : "Clear"}
              </Button>
            </Row>
          </Group>

          <Group>
            <button
              type="button"
              onClick={() => setAbout(true)}
              className="flex min-h-12 w-full items-center justify-between gap-4 rounded-xl px-4 py-2 text-start text-sm outline-none hover:bg-accent/50 focus-visible:bg-accent"
            >
              About
              <span className="flex items-center gap-1 text-muted-foreground">
                {appVersion && <span className="text-xs tabular-nums">Version {appVersion}</span>}
                <ChevronRight className="size-4" />
              </span>
            </button>
          </Group>
        </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
