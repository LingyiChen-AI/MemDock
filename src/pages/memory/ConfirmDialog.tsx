import * as React from "react";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  body: string;
  confirmLabel: string;
  cancelLabel: string;
  /** Confirming discards something; styled as a destructive action. */
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/** Two-button modal (shadcn AlertDialog look): cancel is the default focus. */
export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel,
  cancelLabel,
  destructive,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  // Radix also reports "closed" after the action button; that must not count as a cancel.
  const confirmed = React.useRef(false);
  React.useEffect(() => {
    if (open) confirmed.current = false;
  }, [open]);
  return (
    <AlertDialog
      open={open}
      onOpenChange={(o) => {
        if (o) return;
        if (confirmed.current) confirmed.current = false;
        else onCancel();
      }}
    >
        <AlertDialogContent>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription className="mt-2 whitespace-pre-line">{body}</AlertDialogDescription>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button variant="outline" className="h-7 cursor-pointer px-3 text-xs">
                {cancelLabel}
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                variant={destructive ? "destructive" : "default"}
                className="h-7 cursor-pointer px-3 text-xs font-semibold"
                onClick={() => {
                  confirmed.current = true;
                  onConfirm();
                }}
              >
                {confirmLabel}
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
    </AlertDialog>
  );
}
