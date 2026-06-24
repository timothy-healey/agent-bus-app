import "@testing-library/jest-dom/vitest";

// --- React Flow (@xyflow/react) jsdom shims ---------------------------------
// React Flow measures the DOM via ResizeObserver + DOMMatrix, neither of which
// jsdom implements. These minimal stubs let the canvas mount in vitest
// (component tests assert orchestration, not pixel layout).
if (typeof globalThis.ResizeObserver === "undefined") {
  class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver = ResizeObserver as unknown as typeof globalThis.ResizeObserver;
}

if (typeof (globalThis as { DOMMatrixReadOnly?: unknown }).DOMMatrixReadOnly === "undefined") {
  class DOMMatrixReadOnly {
    m22 = 1;
    constructor(_t?: string) {}
  }
  (globalThis as { DOMMatrixReadOnly?: unknown }).DOMMatrixReadOnly = DOMMatrixReadOnly;
}
