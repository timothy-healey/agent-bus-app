/// E2E alias for `@tauri-apps/api/event` (S5). Mirrors the slice of the Tauri
/// event API the app uses: `listen(event, cb)` returning an `UnlistenFn`, plus the
/// `Event<T>` envelope (`{ payload }`) the callbacks read. Backed by the in-memory
/// event bus in `backend.ts`, which `window.__E2E__.emit` and the command handlers
/// fire into.
import { busListen } from "./backend";

export type UnlistenFn = () => void;

export interface Event<T> {
  event: string;
  id: number;
  payload: T;
}

export type EventCallback<T> = (event: Event<T>) => void;

let EVENT_ID = 1;

export async function listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  const unsub = busListen(event, (payload) => {
    handler({ event, id: EVENT_ID++, payload: payload as T });
  });
  return unsub;
}

/// `emit` is exported for completeness (the app emits only from the backend, but
/// the real module exposes it). Routes through the same bus.
export async function emit(event: string, payload?: unknown): Promise<void> {
  const { busEmit } = await import("./backend");
  busEmit(event, payload);
}
