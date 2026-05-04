import * as TogglePrimitive from "@radix-ui/react-toggle";
import * as React from "react";
import { cn } from "@/lib/utils";

export const Toggle = React.forwardRef<
  React.ElementRef<typeof TogglePrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof TogglePrimitive.Root>
>(({ className, ...props }, ref) => (
  <TogglePrimitive.Root
    ref={ref}
    className={cn(
      "inline-flex items-center justify-center rounded-full px-3 py-1 text-xs font-medium transition-all border border-[var(--color-border)] hover:bg-[var(--color-accent)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-ring)] disabled:pointer-events-none disabled:opacity-50 data-[state=on]:bg-[var(--color-primary)] data-[state=on]:text-[var(--color-primary-foreground)] data-[state=on]:border-[var(--color-primary)]",
      className,
    )}
    {...props}
  />
));
Toggle.displayName = TogglePrimitive.Root.displayName;
