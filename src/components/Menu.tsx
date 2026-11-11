import { ContextMenu } from "radix-ui";
import type { ReactNode } from "react";

export type MenuItem = { label: string; onSelect: () => void; danger?: boolean; disabled?: boolean } | "separator";

export function Menu({ items, children, onOpenChange }: { items: MenuItem[]; children: ReactNode; onOpenChange?: (open: boolean) => void }) {
  return (
    <ContextMenu.Root onOpenChange={onOpenChange}>
      <ContextMenu.Trigger asChild>{children}</ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content className="menu">
          {items.map((item, index) =>
            item === "separator" ? (
              <ContextMenu.Separator key={index} className="menu-sep" />
            ) : (
              <ContextMenu.Item key={item.label} className={`menu-item${item.danger ? " danger" : ""}`} disabled={item.disabled} onSelect={item.onSelect}>
                {item.label}
              </ContextMenu.Item>
            ),
          )}
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}
