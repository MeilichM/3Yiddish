import { Laptop, Moon, Sun } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ICON } from "@/lib/utils";
import { setTheme, useDark, useTheme, type Theme } from "@/lib/theme";

const OPTIONS = [
  ["dark", "Dark", Moon],
  ["light", "Light", Sun],
  ["system", "System", Laptop],
] as const;

/** The header's appearance button: dark, light, or whatever the system uses. */
export function ThemeMenu() {
  const theme = useTheme();
  // The button shows the look in use, a moon or a sun, even when it follows the system.
  const Icon = useDark() ? Moon : Sun;
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button size="icon" variant="ghost" className={ICON} aria-label="Appearance">
              <Icon />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>Appearance</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="min-w-40">
        <DropdownMenuRadioGroup value={theme} onValueChange={(v) => setTheme(v as Theme)}>
          {OPTIONS.map(([key, label, ItemIcon]) => (
            <DropdownMenuRadioItem key={key} value={key}>
              <ItemIcon />
              {label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
