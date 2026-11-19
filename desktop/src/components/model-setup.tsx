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
  otherIsOmni,
  onOpenChange,
  onDownload,
}: {
  model: Listed | null;
  /** The model in use now is the Omnilingual-based one. */
  otherIsOmni?: boolean;
  onOpenChange: (open: boolean) => void;
  onDownload: (model: Listed) => void;
}) {
  const omni = model?.label === "Omnilingual";
  /** Only the Omnilingual-based model is a second model; a Whisper one is the main one, again. */
  const second = omni || !!otherIsOmni;
  return (
    <Dialog open={!!model} onOpenChange={onOpenChange}>
      <DialogContent className="gap-5 sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Download {model && modelName(model.label)}?</DialogTitle>
          <DialogDescription className="leading-relaxed">
            About {model && mb(model.size)}, downloaded once and checked after download.
            {second &&
              " It makes different mistakes than the model you have, so when one gets a recording wrong, try the other."}{" "}
            You can switch back any time from this menu.
            {omni && " It runs on the processor only, not on the graphics card."}
          </DialogDescription>
        </DialogHeader>
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
