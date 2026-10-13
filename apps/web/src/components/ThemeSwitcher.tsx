import { Check, ChevronDown, Monitor, Moon, Sun, type LucideIcon } from "lucide-react";
import { DropdownMenu as Menu } from "radix-ui";
import type { ThemeChoice } from "@/lib/theme";

const CHOICES: { value: ThemeChoice; label: string; icon: LucideIcon }[] = [
  { value: "system", label: "System", icon: Monitor },
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
];

/**
 * The app's own colours, as one compact button naming the current mode that opens a menu of three.
 * Radix gives the menu its roles, arrow keys, Escape and the focus return to the button. The
 * reading view keeps the colour theme from the reading settings.
 */
export function ThemeSwitcher({ value, onChange }: { value: ThemeChoice; onChange: (choice: ThemeChoice) => void }) {
  const current = CHOICES.find((c) => c.value === value) ?? CHOICES[0];
  const Icon = current.icon;
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger
        data-mode={current.value}
        className="inline-flex min-h-11 min-w-11 touch-manipulation items-center justify-center gap-2 rounded-md border-2 border-border bg-bg px-3 text-base font-bold text-fg hover:border-fg data-[state=open]:border-fg data-[state=open]:shadow-[inset_0_0_0_1px_var(--fg)] [&_svg]:size-5 [&_svg]:shrink-0"
      >
        <span className="sr-only">Colour mode: </span>
        <Icon aria-hidden="true" />
        {/* the narrowest phones show the icon only; the name is still read out */}
        <span className="max-[374px]:sr-only">{current.label}</span>
        <ChevronDown aria-hidden="true" className="-ml-0.5 !size-4" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content
          align="end"
          sideOffset={6}
          // never closer than 8 pixels to the screen edge, which also clears the notch of a phone held sideways
          collisionPadding={8}
          className="z-50 min-w-40 rounded-md border-2 border-fg bg-bg p-1 text-fg shadow-[0_0.5rem_1.5rem_rgba(0,0,0,0.18)]"
        >
          <Menu.RadioGroup value={value} onValueChange={(v) => onChange(v as ThemeChoice)}>
            {CHOICES.map((c) => (
              <Menu.RadioItem
                key={c.value}
                value={c.value}
                className="flex min-h-11 cursor-pointer items-center gap-2 rounded-sm px-2.5 text-base font-bold select-none data-[highlighted]:bg-surface data-[highlighted]:underline data-[highlighted]:decoration-2 data-[highlighted]:underline-offset-4 [&_svg]:size-5 [&_svg]:shrink-0"
              >
                <c.icon aria-hidden="true" />
                <span>{c.label}</span>
                {/* the chosen mode shows a tick as well as being announced as checked */}
                <Menu.ItemIndicator className="ml-auto pl-3">
                  <Check aria-hidden="true" />
                </Menu.ItemIndicator>
              </Menu.RadioItem>
            ))}
          </Menu.RadioGroup>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  );
}
