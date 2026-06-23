import { useEffect, useRef } from "react";

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/// Shared modal accessibility for dialogs/drawers (audit A1/A2/A3):
/// - Escape closes
/// - focus is trapped inside the container while open
/// - focus moves into the container on open and is restored to the previously
///   focused element on close
///
/// Returns a ref to attach to the modal container element.
export function useModalA11y<T extends HTMLElement>(open: boolean, onClose: () => void) {
  const ref = useRef<T | null>(null);
  const restoreTo = useRef<HTMLElement | null>(null);
  // Keep the latest onClose without retriggering the focus effect on each render
  // (callers often pass a fresh closure as their draft state changes).
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // Focus management + restore — keyed only to `open` so typing inside the modal
  // never steals focus back to the first control.
  useEffect(() => {
    if (!open) return;
    const container = ref.current;
    restoreTo.current = (document.activeElement as HTMLElement) ?? null;

    const focusables = container
      ? Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE))
      : [];
    if (focusables.length > 0) {
      focusables[0].focus();
    } else if (container) {
      container.setAttribute("tabindex", "-1");
      container.focus();
    }

    return () => {
      restoreTo.current?.focus?.();
    };
  }, [open]);

  // Escape-to-close + Tab focus-trap.
  useEffect(() => {
    if (!open) return;
    const container = ref.current;

    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key !== "Tab" || !container) return;
      const items = Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
        (el) => el.offsetParent !== null || el === document.activeElement,
      );
      if (items.length === 0) {
        e.preventDefault();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement as HTMLElement;
      if (e.shiftKey && (active === first || !container.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [open]);

  return ref;
}
