/// Mock of `@tauri-apps/plugin-dialog` for the E2E build (S5). The headless
/// new-project flow types its paths rather than using the native folder picker, so
/// `open` is never reached in the specs — but the import must resolve. Returning
/// null mirrors a cancelled picker (the wizard treats null as "no selection").
export async function open(_opts?: unknown): Promise<string | null> {
  return null;
}
