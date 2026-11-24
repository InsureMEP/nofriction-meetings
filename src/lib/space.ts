// Storage: what noFriction keeps on this Mac and how big it is
// (src-tauri/src/space.rs, docs/STORAGE.md). Typed wrappers over the
// commands, the size formatter (the same rule as the backend's) and the
// sizes the Recordings list shows.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface RecordingSpace {
    meeting_id: string;
    title: string;
    started_at: string;
    screens_bytes: number;
    screens_count: number;
    video_bytes: number;
    total_bytes: number;
    /** Roughly what making its pictures smaller frees */
    saves_bytes: number;
    /** Being recorded now: nothing can be done to it yet */
    recording: boolean;
}

export interface SpaceOverview {
    total_bytes: number;
    free_bytes: number | null;
    screens_bytes: number;
    screens_count: number;
    video_bytes: number;
    models_bytes: number;
    database_bytes: number;
    leftover_bytes: number;
    other_bytes: number;
    shrink: { pictures: number; saves_bytes: number };
    recordings: RecordingSpace[];
    screen_size: "smaller" | "full";
    keep_screens_days: number;
}

export interface Freed {
    items: number;
    bytes: number;
}

/** Days offered for "Keep screens for"; 0 = forever. */
export const KEEP_DAYS = [0, 365, 90, 30] as const;

/** A size the way Finder says it (1 MB = 1,000,000 bytes). */
export function formatBytes(bytes: number): string {
    const b = Math.max(0, bytes);
    if (b < 1e6) return `${Math.round(b / 1e3)} KB`;
    if (b < 1e8) {
        const v = b / 1e6;
        return v < 10 ? `${v.toFixed(1)} MB` : `${Math.round(v)} MB`;
    }
    if (b < 1e9) return `${Math.round(b / 1e6)} MB`;
    const v = b / 1e9;
    return v < 100 ? `${v.toFixed(1)} GB` : `${Math.round(v)} GB`;
}

/** "Forever", "1 year", "90 days". */
export function keepLabel(days: number): string {
    if (days === 0) return "Forever";
    if (days === 365) return "1 year";
    return `${days} days`;
}

/**
 * The recordings a "Keep screens for `days`" would remove screens from now:
 * older than that, with screens or video still on disk, and not being recorded.
 */
export function dueFor(recordings: RecordingSpace[], days: number, now: Date = new Date()): RecordingSpace[] {
    if (days === 0) return [];
    const cutoff = now.getTime() - days * 86_400_000;
    return recordings.filter((r) => r.total_bytes > 0 && !r.recording && new Date(r.started_at).getTime() < cutoff);
}

/** The parts of the bar, largest job first; empty parts are left out. */
export function parts(o: SpaceOverview): { id: string; label: string; bytes: number }[] {
    return [
        { id: "screens", label: "Screens and photos", bytes: o.screens_bytes },
        { id: "video", label: "Screen video", bytes: o.video_bytes },
        { id: "models", label: "Speech model", bytes: o.models_bytes },
        { id: "library", label: "Transcripts and notes", bytes: o.database_bytes },
        { id: "leftover", label: "Leftover files", bytes: o.leftover_bytes },
        { id: "other", label: "Everything else", bytes: o.other_bytes },
    ].filter((p) => p.bytes > 0);
}

export const spaceApi = {
    overview: () => invoke<SpaceOverview>("space_overview"),
    sizes: () => invoke<[string, number][]>("space_sizes"),
    makeSmaller: (meetingId: string | null) => invoke<Freed>("space_make_smaller", { meetingId }),
    removeVideo: (meetingId: string | null) => invoke<Freed>("space_remove_video", { meetingId }),
    removeScreens: (meetingId: string) => invoke<Freed>("space_remove_screens", { meetingId }),
    clearLeftovers: () => invoke<Freed>("space_clear_leftovers"),
    setScreenSize: (size: "smaller" | "full") => invoke<void>("space_set_screen_size", { size }),
    setKeepScreens: (days: number) => invoke<Freed>("space_set_keep_screens", { days }),
};

/** Run `fn` whenever the backend says sizes changed. */
export function onSpaceChanged(fn: () => void): () => void {
    let off: (() => void) | null = null;
    let disposed = false;
    listen("space_changed", fn)
        .then((stop) => (disposed ? stop() : (off = stop)))
        .catch(() => {});
    return () => {
        disposed = true;
        off?.();
    };
}

/**
 * Bytes on disk per recording id, for the Recordings list. Read again when
 * `refreshKey` changes or the backend says sizes changed. Empty until read,
 * and if it can't be read: the list works without sizes.
 */
export function useSpaceSizes(refreshKey: unknown): Map<string, number> {
    const [sizes, setSizes] = useState<Map<string, number>>(new Map());
    useEffect(() => {
        let live = true;
        const load = () =>
            spaceApi
                .sizes()
                .then((list) => live && setSizes(new Map(list ?? [])))
                .catch(() => {});
        load();
        const off = onSpaceChanged(load);
        return () => {
            live = false;
            off();
        };
    }, [refreshKey]);
    return sizes;
}
