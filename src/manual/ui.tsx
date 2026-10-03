import { forwardRef, useRef, type CSSProperties, type ReactNode } from "react";
import { Toaster } from "sonner";
import { Button, type ButtonProps } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectGroup,
  SelectItem,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

export const ActionButton = forwardRef<HTMLButtonElement, ButtonProps>(
  function ActionButton({ variant = "default", className, ...props }, ref) {
    return (
      <Button
        ref={ref}
        variant={variant}
        className={cn("mg-button", `mg-button--${variant}`, className)}
        {...props}
      />
    );
  },
);
export function Modal({
  title,
  description,
  children,
  footer,
  onClose,
  busy = false,
  wide = false,
  className,
}: {
  title: string;
  description: ReactNode;
  children?: ReactNode;
  footer: ReactNode;
  onClose: () => void;
  busy?: boolean;
  wide?: boolean;
  className?: string;
}) {
  const returnTarget = useRef(document.activeElement);
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent
        className={cn("mg-dialog", wide && "mg-dialog--wide", className)}
        overlayClassName="mg-overlay"
        onCloseAutoFocus={(event) => {
          const previous = returnTarget.current;
          const target =
            previous instanceof HTMLElement &&
            previous.isConnected &&
            !previous.closest("[hidden]") &&
            !previous.matches(":disabled")
              ? previous
              : document.getElementById("manual-content");
          if (target) {
            event.preventDefault();
            target.focus({ preventScroll: true });
          }
        }}
      >
        <DialogHeader className="mg-dialog-header">
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <div className="mg-dialog-body">{children}</div>
        <DialogFooter className="mg-dialog-footer">{footer}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
export function Field({
  id,
  label,
  hint,
  children,
  wide,
}: {
  id: string;
  label: string;
  hint?: string;
  children: ReactNode;
  wide?: boolean;
}) {
  return (
    <div className={cn("mg-field", wide && "mg-field--wide")}>
      <Label htmlFor={id}>{label}</Label>
      {children}
      {hint && <p className="mg-hint">{hint}</p>}
    </div>
  );
}
export function Choice({
  value,
  onChange,
  label,
  id,
  options,
  disabled,
}: {
  value: string;
  onChange: (value: string) => void;
  label?: string;
  id?: string;
  disabled?: boolean;
  options: { value: string; label: string; disabled?: boolean }[];
}) {
  return (
    <Select value={value} onValueChange={onChange} disabled={disabled}>
      <SelectTrigger id={id} aria-label={label} className="mg-select">
        <SelectValue />
      </SelectTrigger>
      <SelectContent className="mg-select-content">
        <SelectGroup>
          {options.map((option) => (
            <SelectItem
              key={option.value}
              value={option.value}
              disabled={option.disabled}
            >
              {option.label}
            </SelectItem>
          ))}
        </SelectGroup>
      </SelectContent>
    </Select>
  );
}
export function Notifications() {
  return (
    <Toaster
      className="mg-toasts"
      position="top-center"
      theme="system"
      style={
        {
          "--success-bg": "var(--mg-success-bg)",
          "--success-border": "var(--mg-success-bg)",
          "--success-text": "var(--mg-success)",
          "--error-bg": "var(--mg-error-bg)",
          "--error-border": "var(--mg-error-bg)",
          "--error-text": "var(--mg-error)",
        } as CSSProperties
      }
      richColors
      closeButton
      duration={4000}
      visibleToasts={3}
      offset={56}
      toastOptions={{ className: "mg-toast" }}
    />
  );
}
