// Small shared presentational pieces.
import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <kbd
      className={cn(
        "inline-flex h-[22px] min-w-[22px] items-center justify-center rounded-[6px] border border-b-2 border-border bg-card px-1.5 font-sans text-[11px] font-medium text-foreground/80",
        className,
      )}
    >
      {children}
    </kbd>
  );
}

/** System Settings–style coloured square with a white glyph. */
export function IconTile({ icon: Icon, className }: { icon: LucideIcon; className: string }) {
  return (
    <span className={cn("grid size-7 shrink-0 place-items-center rounded-[8px] shadow-[inset_0_-1px_0_rgba(0,0,0,0.12)]", className)}>
      <Icon className="size-4 text-white" strokeWidth={2.25} />
    </span>
  );
}

export function PageHeader({ eyebrow, title, description, actions }: { eyebrow?: string; title: ReactNode; description?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="mb-8 flex items-end justify-between gap-6">
      <div>
        {eyebrow && <div className="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">{eyebrow}</div>}
        <h1 className="font-display text-[44px] leading-[0.95] tracking-[-0.02em]">{title}</h1>
        {description && <p className="mt-3 max-w-xl text-sm text-muted-foreground">{description}</p>}
      </div>
      {actions}
    </header>
  );
}

export function SectionTitle({ children, action }: { children: ReactNode; action?: ReactNode }) {
  return (
    <div className="mt-10 mb-3 flex items-center justify-between">
      <h2 className="text-[13px] font-semibold tracking-tight">{children}</h2>
      {action}
    </div>
  );
}

/** Rounded card surface used across pages (hairline edge, soft lift). */
export function Panel({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("rounded-2xl border bg-card shadow-lift", className)}>{children}</div>;
}

export function EmptyState({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="px-6 py-14 text-center">
      <div className="font-display text-2xl">{title}</div>
      <p className="mt-1 text-sm text-muted-foreground">{children}</p>
    </div>
  );
}
