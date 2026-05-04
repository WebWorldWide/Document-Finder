import * as React from "react";
import { cn } from "@/lib/utils";

export interface BadgeProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: "default" | "secondary" | "outline" | "source";
  source?: string;
}

export function Badge({ className, variant = "default", source, style, ...props }: BadgeProps) {
  const variantClass = {
    default: "bg-[var(--color-primary)] text-[var(--color-primary-foreground)]",
    secondary: "bg-[var(--color-secondary)] text-[var(--color-secondary-foreground)]",
    outline: "border border-[var(--color-border)]",
    source: "",
  }[variant];

  const sourceStyle =
    variant === "source" && source
      ? {
          ...style,
          backgroundColor: `color-mix(in oklab, var(--color-source-${source.replace(
            /-/g,
            "_",
          )}, oklch(0.7 0.1 270)) 20%, transparent)`,
          color: `var(--color-source-${source.replace(/-/g, "_")}, oklch(0.85 0.1 270))`,
          borderColor: `color-mix(in oklab, var(--color-source-${source.replace(
            /-/g,
            "_",
          )}, oklch(0.7 0.1 270)) 50%, transparent)`,
          borderWidth: "1px",
          borderStyle: "solid",
        }
      : style;

  return (
    <div
      className={cn(
        "inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors",
        variantClass,
        className,
      )}
      style={sourceStyle}
      {...props}
    />
  );
}
