// Notes for a recording: pure helpers for the Notes view. No Tauri imports,
// so `npm test` loads it. docs/design/FADELL_V2.md §3 B and §6.

import type { SyncedNotesSection } from "./syncLogic.ts";
import type { TypedNote } from "./studyLogic.ts";

/** Events from src-tauri/src/meeting_notes.rs */
export const NOTES_PROGRESS_EVENT = "notes_progress";
export const NOTES_DONE_EVENT = "notes_done";

export interface NotesProgress {
    meetingId: string;
    label: string;
    done: number;
    total: number;
}

/** The heading the user's typed lines appear under, on every platform. */
export const WHAT_YOU_WROTE = "What you wrote";

/**
 * `meeting_notes.note_details`: what the AI wrote under each typed note, by
 * the note's id. Anything unreadable is no details (the lines still show).
 */
export function parseNoteDetails(json: string | null | undefined): Map<string, string> {
    const out = new Map<string, string>();
    if (!json) return out;
    try {
        const v: unknown = JSON.parse(json);
        if (!Array.isArray(v)) return out;
        for (const d of v) {
            if (d && typeof d === "object") {
                const { marker_id, detail } = d as { marker_id?: unknown; detail?: unknown };
                if (typeof marker_id === "string" && typeof detail === "string" && detail.trim()) {
                    out.set(marker_id, detail.trim());
                }
            }
        }
    } catch {
        /* stored by an older or newer version: show the lines without details */
    }
    return out;
}

export interface WrittenLine {
    note: TypedNote;
    /** What was being said then, written by the AI; null until notes are made */
    detail: string | null;
}

/** The user's lines in the order typed, each with its detail when there is one. */
export function writtenLines(notes: TypedNote[], details: Map<string, string>): WrittenLine[] {
    return notes.map((note) => ({ note, detail: details.get(note.id) ?? null }));
}

/** Notes that came from the iPhone already carry the section in their Markdown. */
export function hasWhatYouWrote(sections: SyncedNotesSection[]): boolean {
    return sections.some((s) => (s.heading ?? "").trim().toLowerCase() === WHAT_YOU_WROTE.toLowerCase());
}

/** "Reading the recording (2 of 5)…" → shown under "Making notes…"; the
 *  generic first label says nothing more than the heading does. */
export function progressDetail(label: string | null | undefined): string | null {
    const l = (label ?? "").trim();
    return l && l !== "Making notes…" && l !== "Done" ? l : null;
}
