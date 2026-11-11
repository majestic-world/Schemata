import { Dialog as RDialog } from "radix-ui";
import { X } from "lucide-react";
import type { ReactNode } from "react";

type Props = {
  open: boolean;
  onClose: () => void;
  title: string;
  width?: number;
  children: ReactNode;
  footer?: ReactNode;
};

export function Dialog({ open, onClose, title, width = 480, children, footer }: Props) {
  return (
    <RDialog.Root open={open} onOpenChange={(o) => !o && onClose()}>
      <RDialog.Portal>
        <RDialog.Overlay className="dialog-overlay" />
        <RDialog.Content className="dialog" style={{ width }} aria-describedby={undefined}>
          <div className="dialog-header">
            <RDialog.Title className="dialog-title">{title}</RDialog.Title>
            <RDialog.Close className="icon-btn" aria-label="Close"><X size={16} /></RDialog.Close>
          </div>
          <div className="dialog-body">{children}</div>
          {footer && <div className="dialog-footer">{footer}</div>}
        </RDialog.Content>
      </RDialog.Portal>
    </RDialog.Root>
  );
}
