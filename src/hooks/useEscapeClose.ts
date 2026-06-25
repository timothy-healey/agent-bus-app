import { useEffect, useRef } from "react";

/// Escape-to-close for a full-page view (B1/G10). Unlike `useModalA11y` this does
/// NOT trap focus or move focus on mount — a full-page authoring view is the
/// primary surface, not a modal overlay, so focus should stay where the user puts
/// it and Tab should reach the whole page. Escape still offers a quick exit.
///
/// `enabled` gates the listener; `onEscape` is kept in a ref so a fresh closure
/// (e.g. one that reads live draft state) never re-binds the listener.
export function useEscapeClose(enabled: boolean, onEscape: () => void) {
  const onEscapeRef = useRef(onEscape);
  onEscapeRef.current = onEscape;

  useEffect(() => {
    if (!enabled) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.stopPropagation();
        onEscapeRef.current();
      }
    }
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [enabled]);
}
