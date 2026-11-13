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
import { setTheme, useTheme, type Theme } from "@/lib/theme";

const OPTIONS = [
  ["dark", "Dark", Moon],
  ["light", "Light", Sun],
  ["system", "System", Laptop],
] as const;

/** The header's appearance button: dark, light, or whatever the system uses. */
export function ThemeMenu() {
  const theme = useTheme();
  const Icon = OPTIONS.find(([key]) => key === theme)![2];
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
