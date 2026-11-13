// Moment markers + review/study guides: typed wrappers over the Tauri commands in
// src-tauri/src/markers.rs and src-tauri/src/study.rs (docs/STUDY_TOOLS.md).
// Pure logic lives in studyLogic.ts.

import { invoke } from "@tauri-apps/api/core";
import type { Marker, MarkerKind, StoredMarker, StudyPart } from "./studyLogic";

export interface StoredMaterial {
    kind: StudyPart;
    /** Validated by the backend; check shapes again before use (studyLogic) */
    data: unknown;
    created_at: string;
    /** Made from a transcript that reads differently now */
    stale: boolean;
}

export interface StudyGuide {
    meeting_id: string;
    title: string;
    /** "meeting" | "class" | "personal": "Study guide" for a class, "Review guide" otherwise */
    recording_kind: string;
    started_at: string;
    duration_ms: number;
    has_transcript: boolean;
    materials: Partial<Record<StudyPart, StoredMaterial>>;
    /** Marks and typed notes (split them with `splitMarkers`) */
    markers: StoredMarker[];
}

export interface GenerateResult {
    saved: StudyPart[];
    failed: { kind: StudyPart; error: string }[];
    guide: StudyGuide;
}

export interface StudyProgress {
    meeting_id: string;
    done: number;
    total: number;
    label: string;
}

/** Events from the backend */
export const MARKER_ADDED_EVENT = "marker_added";
export const MARKER_FAILED_EVENT = "marker_failed";
/** A typed note was saved in the recording in progress (payload: the row) */
export const NOTE_ADDED_EVENT = "note_added";
export const STUDY_PROGRESS_EVENT = "study_progress";

/** Fired in the window after any marker change, so open views refresh. */
export const MARKERS_CHANGED_EVENT = "nf:markers-changed";
export function notifyMarkersChanged(meetingId: string): void {
    window.dispatchEvent(new CustomEvent(MARKERS_CHANGED_EVENT, { detail: { meetingId } }));
}

export const markersApi = {
    /** Mark now in the meeting being recorded (★ unless a kind is given) */
    markNow: (kind?: MarkerKind, note?: string) => invoke<Marker>("mark_moment", { kind: kind ?? null, note: note ?? null }),
    /** Mark a moment of a recorded meeting (ms from its start) */
    add: (meetingId: string, offsetMs: number, kind?: MarkerKind, note?: string) =>
        invoke<Marker>("add_marker", { meetingId, offsetMs: Math.max(0, Math.round(offsetMs)), kind: kind ?? null, note: note ?? null }),
    setKind: (id: string, kind: MarkerKind) => invoke<Marker>("update_marker", { id, kind, note: null, clearNote: false }),
    setNote: (id: string, note: string) =>
        note.trim()
            ? invoke<Marker>("update_marker", { id, kind: null, note, clearNote: false })
            : invoke<Marker>("update_marker", { id, kind: null, note: null, clearNote: true }),
    remove: (id: string) => invoke<void>("delete_marker", { id }),
    /** Marks and typed notes, in time order */
    list: (meetingId: string) => invoke<StoredMarker[]>("list_markers", { meetingId }),
    /** Keep a typed line in the recording it was typed in (`meetingId`),
     *  stamped with the moment Return was pressed (`at`). Kept even if that
     *  recording stopped before this call arrived. */
    addNote: (text: string, meetingId: string | null, at: Date) =>
        invoke<StoredMarker>("add_note", { text, meetingId, at: at.toISOString() }),
    /** Add a typed note at a moment of a recorded meeting (ms from its start) */
    addNoteAt: (meetingId: string, offsetMs: number, text: string) =>
        invoke<StoredMarker>("add_marker", { meetingId, offsetMs: Math.max(0, Math.round(offsetMs)), kind: "note", note: text }),
    /** Change a typed note's text (it can't be emptied: remove it instead) */
    setNoteText: (id: string, text: string) => invoke<StoredMarker>("update_marker", { id, kind: null, note: text, clearNote: false }),
};

export const studyApi = {
    get: (meetingId: string) => invoke<StudyGuide>("get_study_guide", { meetingId }),
    generate: (meetingId: string, kinds?: StudyPart[]) =>
        invoke<GenerateResult>("generate_study_guide", { meetingId, kinds: kinds ?? null }),
    /** Save dialog; resolves to the path, or null when cancelled */
    exportFlashcards: (meetingId: string) => invoke<string | null>("export_study_flashcards", { meetingId }),
    exportGuide: (meetingId: string) => invoke<string | null>("export_study_guide", { meetingId }),
};
