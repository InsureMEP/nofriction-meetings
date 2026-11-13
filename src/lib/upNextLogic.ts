// Up next (docs/design/FADELL_V2.md §3 A): pure helpers for the one
// suggestion to record what is starting. No Tauri imports, so `npm test`
// loads it. The suggestion itself is worked out in src-tauri/src/up_next.rs.

import { kindLabel, parseKind } from "./recordingKind.ts";
import { parseChoice, type StartPlan } from "./recordPlan.ts";

export type UpNextSource = "calendar" | "routine" | "call";

export interface UpNext {
    source: UpNextSource;
    /** Identifies this suggestion for "Not now" */
    key: string;
    /** What follows "Record": a calendar event, a notebook, or "this call" */
    title: string;
    /** Why it is offered, in one line */
    detail: string;
    notebook: string | null;
    /** "meeting" | "class" | "personal" */
    recording_kind: string;
    /** "15" | "30" | "60" | "90" | "none" */
    duration: string;
}

/** The button: "Record BIO 101", "Record this call". */
export function recordLabel(s: UpNext): string {
    return `Record ${s.title.trim() || "now"}`;
}

/** The heading over the card's title: what kind of thing is starting. */
export function sourceLabel(source: UpNextSource): string {
    return source === "call" ? "On a call" : "Up next";
}

/**
 * How it will be recorded, as the card says it: "Class · 60 min · BIO 101".
 * The answers to the Record sheet's three questions, already filled in.
 */
export function planSummary(s: UpNext): string {
    const parts = [kindLabel(parseKind(s.recording_kind))];
    const d = parseChoice(s.duration);
    parts.push(d === "none" ? "Stops when it's over" : `${d} min`);
    const nb = s.notebook?.trim();
    if (nb && nb.toLowerCase() !== s.title.trim().toLowerCase()) parts.push(nb);
    return parts.join(" · ");
}

/** What starting it sends to `start_recording`. Never remembered as the
 *  user's own choice: the Record sheet's last answers stay theirs. */
export function planFor(s: UpNext): StartPlan {
    return {
        duration: parseChoice(s.duration),
        recordingKind: parseKind(s.recording_kind),
        notebook: s.notebook?.trim() || null,
        remember: false,
    };
}
