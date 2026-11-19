import { Download, ShieldCheck } from "lucide-react";
import type { Progress as Downloading, Listed } from "@/lib/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";

/** Sizes as the terminal app gives them (a million bytes to the MB). */
export function mb(bytes: number) {
  return `${Math.round(bytes / 1_000_000)} MB`;
}

/** "our Omnilingual-based model" for a published model; a model put in by hand by its own name. */
export function modelName(label: string) {
  return ["Whisper", "Omnilingual"].includes(label) ? `our ${label}-based model` : `the ${label} model`;
}

/** First run: the speech model is not on this computer yet. */
export function ModelSetup({
  progress,
  busy,
  error,
  onDownload,
}: {
  progress: Downloading | null;
  busy: boolean;
  error: string | null;
  onDownload: () => void;
}) {
  const share = progress && progress.total ? (progress.done / progress.total) * 100 : 0;
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <div className="flex w-full max-w-md flex-col gap-5 rounded-2xl border bg-card p-6 text-card-foreground shadow-sm">
        <div className="flex flex-col gap-2 rounded-xl bg-muted/60 p-4">
          <p className="flex items-center gap-2 font-semibold">
            <ShieldCheck className="size-5 shrink-0" />
            Nothing leaves your computer.
          </p>
          <p className="text-sm text-muted-foreground">
            3Yiddish listens and writes right here, on this computer. Your recordings and what they
            say are never sent anywhere, unless you yourself switch on sharing in Settings.
          </p>
        </div>

        <div className="flex flex-col gap-1.5">
          <h2 className="text-lg font-semibold tracking-tight">One step: the speech model</h2>
          <p className="text-sm text-muted-foreground">
            To do that, 3Yiddish needs its speech model, about 575 MB, downloaded once. A second
            model can be added later from the model menu at the top.
          </p>
        </div>

        {busy && progress ? (
          <div className="flex flex-col gap-2">
            <Progress value={share} />
            <div className="flex justify-between text-xs text-muted-foreground tabular-nums">
              <span className="truncate">{progress.file}</span>
              <span>
                {mb(progress.done)} of {mb(progress.total)}
              </span>
            </div>
          </div>
        ) : (
          <Button onClick={onDownload} disabled={busy} size="lg">
            <Download /> {busy ? "Starting…" : "Download the model"}
          </Button>
        )}

        {error && <p className="text-sm text-destructive">{error}</p>}

        <p className="text-xs text-muted-foreground">It is checked after download.</p>
      </div>
    </div>
  );
}

/** The model tag's menu asks before downloading a model nobody has here yet. */
export function DownloadModelDialog({
  model,
  replaces,
  otherIsOmni,
  onOpenChange,
  onDownload,
}: {
  model: Listed | null;
  /** When this is a newer version of the model in use: the version it replaces. */
  replaces?: string;
  /** The model in use now is the Omnilingual-based one. */
  otherIsOmni?: boolean;
  onOpenChange: (open: boolean) => void;
  onDownload: (model: Listed) => void;
}) {
  const omni = model?.label === "Omnilingual";
  /** Only the Omnilingual-based model is a second model; a Whisper one is the main one, again. */
  const second = !replaces && (omni || !!otherIsOmni);
  const name = model ? modelName(model.label) : "";
  return (
    <Dialog open={!!model} onOpenChange={onOpenChange}>
      <DialogContent className="gap-5 sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{replaces ? "Download the newer model?" : `Download ${name}?`}</DialogTitle>
          <DialogDescription className="leading-relaxed">
            Downloaded once and checked after download.
            {second &&
              " It makes different mistakes than the model you have, so when one gets a recording wrong, try the other."}{" "}
            {replaces
              ? "The one you have stays on this computer; Roll back in Settings goes back to it."
              : "You can switch back any time from this menu."}
            {omni && " It runs on the processor only, not on the graphics card."}
          </DialogDescription>
        </DialogHeader>
        {model && (
          <div className="flex items-center justify-between gap-4 rounded-xl border px-4 py-3 text-sm">
            <div className="flex flex-col gap-0.5">
              <span className="font-medium">{name.charAt(0).toUpperCase() + name.slice(1)}</span>
              <span className="text-xs text-muted-foreground tabular-nums">
                Version {model.version}
                {replaces && <> · you have {replaces}</>}
              </span>
            </div>
            <span className="shrink-0 text-muted-foreground tabular-nums">{mb(model.size)}</span>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Not now
          </Button>
          <Button onClick={() => model && onDownload(model)}>
            <Download /> Download
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
