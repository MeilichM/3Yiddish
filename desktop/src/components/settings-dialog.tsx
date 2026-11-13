import { useEffect, useState } from "react";
import { toast } from "sonner";
import { api, message } from "@/lib/api";
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
  appVersion,
  entries,
  onRollback,
  onClearHistory,
  sharing,
  onSharing,
}: {
  /** Sharing what is transcribed; null where it is never offered (the owner's computer). */
  sharing: boolean | null;
  onSharing: (on: boolean) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  shortcut: string | null;
  onShortcut: (shortcut: string | null) => void;
  model: string | null;
  appVersion?: string;
  entries: number;
  onRollback: () => void;
  onClearHistory: () => void;
}) {
  const [autostart, setAutostart] = useState(false);
  const [fallback, setFallback] = useState("F9");
  /** The shortcut to come back to when push-to-talk is switched on again. */
  const [last, setLast] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  useEffect(() => {
    if (!open) return setConfirmClear(false);
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
          <DialogTitle>Settings</DialogTitle>
          <DialogDescription className="sr-only">3Yiddish settings</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <Group>
            <Row label="Push-to-talk" htmlFor="ptt">
              <Switch id="ptt" checked={!!shortcut} onCheckedChange={pushToTalk} />
            </Row>
            {shortcut && (
              <Row label="Shortcut">
                <ShortcutPicker shortcut={shortcut} onChange={onShortcut} onError={setError} />
              </Row>
            )}
            <Row label={windows ? "Start with Windows" : "Open at login"} htmlFor="autostart">
              <Switch id="autostart" checked={autostart} onCheckedChange={startAtLogin} />
            </Row>
          </Group>
          {error && <p className="px-1 text-xs text-destructive">{error}</p>}

          <Group>
            <Row label="Speech model">
              <span className="text-sm text-muted-foreground tabular-nums">{model ?? "None"}</span>
              {model && (
                <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={onRollback}>
                  Roll back
                </Button>
              )}
            </Row>
            {sharing != null && (
              <Row label="Share transcriptions" htmlFor="sharing">
                <Switch id="sharing" checked={sharing} onCheckedChange={onSharing} />
              </Row>
            )}
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
        </div>

        {appVersion && <p className="text-center text-xs text-muted-foreground">Version {appVersion}</p>}
      </DialogContent>
    </Dialog>
  );
}
