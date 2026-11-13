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

const isWhatYouWrote = (s: SyncedNotesSection) => (s.heading ?? "").trim().toLowerCase() === WHAT_YOU_WROTE.toLowerCase();

/** Notes that came from the other device carry the section in their Markdown. */
export function hasWhatYouWrote(sections: SyncedNotesSection[]): boolean {
    return sections.some(isWhatYouWrote);
}

/** A typed line as the notes' Markdown quotes it: one line, no bold marks. */
const quoted = (text: string) => text.replace(/\*\*/g, "").split(/\s+/).filter(Boolean).join(" ");

/**
 * Notes made on the other device carry "What you wrote" in their Markdown,
 * one bullet per typed line: "line: what was being said" or just "line".
 * The lines also arrive here as typed notes, so the two are merged:
 *
 * - every typed note is a row (with Rewind, edit and delete), with the
 *   detail of the bullet that quotes it, if one does;
 * - a bullet that was matched leaves the section;
 * - a bullet no typed note matches (the note hasn't crossed yet, or the
 *   other device is older) stays in the section, so nothing the user wrote
 *   is hidden, and a typed note no bullet matches still has its row.
 *
 * A bullet matches the longest typed line it equals or starts with
 * (followed by ":"), each note at most once, in the order typed.
 */
export function mergeSyncedWritten(notes: TypedNote[], sections: SyncedNotesSection[]): { lines: WrittenLine[]; sections: SyncedNotesSection[] } {
    const details = new Map<string, string | null>();
    const rest = sections
        .map((section) => {
            if (!isWhatYouWrote(section)) return section;
            const bullets = section.bullets.filter((bullet) => {
                const b = quoted(bullet);
                let best: TypedNote | null = null;
                for (const n of notes) {
                    const q = quoted(n.text);
                    if (!q || details.has(n.id)) continue;
                    if ((b === q || b.startsWith(`${q}:`)) && (!best || q.length > quoted(best.text).length)) best = n;
                }
                if (!best) return true;
                details.set(best.id, b.slice(quoted(best.text).length).replace(/^:\s*/, "").trim() || null);
                return false;
            });
            return { ...section, bullets };
        })
        .filter((section) => !isWhatYouWrote(section) || section.bullets.length > 0 || section.paragraphs.length > 0);
    return { lines: notes.map((note) => ({ note, detail: details.get(note.id) ?? null })), sections: rest };
}

/** The first words of every label a Fill in the blanks run reports
 *  (meeting_notes.rs `run::FILLING`). */
export const FILLING = "Filling in the blanks…";

/** Whether the run a label belongs to is filling in the blanks, not making notes. */
export function isFillRun(label: string | null | undefined): boolean {
    return (label ?? "").trim().startsWith(FILLING);
}

/** "Reading the recording (2 of 5)…" → shown under "Making notes…" (or
 *  "Filling in the blanks…"); the generic first label says nothing more
 *  than the heading does. */
export function progressDetail(label: string | null | undefined): string | null {
    let l = (label ?? "").trim();
    if (l.startsWith(FILLING)) l = l.slice(FILLING.length).trim();
    return l && l !== "Making notes…" && l !== "Done" ? l : null;
}

/** The headings of what Fill in the blanks writes, the same on every
 *  platform (meeting_notes/fill.rs). */
export const WHAT_YOU_MISSED = "What you missed";
export const MORE_DETAIL = "More detail";

/** Something that was said and the notes didn't have. */
export interface MissedPoint {
    /** When it was said, from the start of the recording */
    atMs: number | null;
    text: string;
}

/** One of the notes' key ideas, explained further. */
export interface MoreDetail {
    about: string;
    detail: string;
    atMs: number | null;
    /** Not said in the recording: added by the AI (a Class only) */
    background: boolean;
}

export interface Filled {
    missed: MissedPoint[];
    more: MoreDetail[];
}

const moment = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : null);
const words = (v: unknown): string => (typeof v === "string" ? v.trim() : "");

/**
 * `meeting_notes.filled`: what Fill in the blanks wrote for these notes.
 * Null when nothing was filled in or it can't be read; an object with two
 * empty lists when the pass ran and found the notes complete.
 */
export function parseFilled(json: string | null | undefined): Filled | null {
    if (!json) return null;
    let v: unknown;
    try {
        v = JSON.parse(json);
    } catch {
        return null;
    }
    if (!v || typeof v !== "object" || Array.isArray(v)) return null;
    const o = v as { missed?: unknown; more?: unknown };
    const list = (x: unknown): Record<string, unknown>[] =>
        Array.isArray(x) ? x.filter((e): e is Record<string, unknown> => !!e && typeof e === "object" && !Array.isArray(e)) : [];
    return {
        missed: list(o.missed)
            .map((e) => ({ atMs: moment(e.at_ms), text: words(e.text) }))
            .filter((m) => m.text),
        more: list(o.more)
            .map((e) => ({ about: words(e.about), detail: words(e.detail), atMs: moment(e.at_ms), background: e.background === true }))
            .filter((m) => m.about && m.detail),
    };
}

const isFilledSection = (s: SyncedNotesSection) => {
    const h = (s.heading ?? "").trim().toLowerCase();
    return h === WHAT_YOU_MISSED.toLowerCase() || h === MORE_DETAIL.toLowerCase();
};

/** Notes that came from the other device carry the two sections in their Markdown. */
export function hasFilledSections(sections: SyncedNotesSection[]): boolean {
    return sections.some(isFilledSection);
}

/** Those notes without the two sections (this Mac filled in the blanks again and shows its own). */
export function withoutFilledSections(sections: SyncedNotesSection[]): SyncedNotesSection[] {
    return sections.filter((s) => !isFilledSection(s));
}
