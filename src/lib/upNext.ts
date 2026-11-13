// Up next: typed wrappers over src-tauri/src/up_next.rs. Pure helpers are
// in upNextLogic.ts.

import { invoke } from "@tauri-apps/api/core";
import type { UpNext } from "./upNextLogic";

/** The suggestion changed (payload: `UpNext` or null) */
export const UP_NEXT_EVENT = "up_next";
/** Start the suggested recording (the tray's first item, Start by itself) */
export const UP_NEXT_START_EVENT = "up_next_start";

/** Settings keys (Settings → Recording) */
export const REMIND_SETTING = "remind_when_something_starts";
export const AUTO_START_SETTING = "auto_start_recording";

export const upNextApi = {
    get: () => invoke<UpNext | null>("get_up_next"),
    /** "Not now": hidden until tomorrow (a call: until the microphone is released) */
    dismiss: (key: string) => invoke<void>("dismiss_up_next", { key }),
};
