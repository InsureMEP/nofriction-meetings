// The notebook page (docs/design/FADELL_V2.md §3 C): pure helpers. No Tauri
// imports, so `npm test` loads it. The page itself is gathered in
// src-tauri/src/notebook_page.rs.

import { parseKind, type RecordingKind } from "./recordingKind.ts";
import type { Card, QuizQuestion } from "./studyLogic.ts";

export interface NotebookRecording {
    id: string;
    title: string;
    started_at: string;
    duration_seconds: number | null;
    recording_kind: string;
    has_notes: boolean;
    has_guide: boolean;
}

/** One thing the notes say is to be done, with its tick. */
export interface NotebookItem {
    meeting_id: string;
    meeting_title: string;
    started_at: string;
    /** A hash of the item's text: what the tick is stored under */
    key: string;
    task: string;
    assignee: string | null;
    due: string | null;
    done: boolean;
}

export interface NotebookMark {
    id: string;
    meeting_id: string;
    meeting_title: string;
    started_at: string;
    kind: "test" | "question";
    /** "On the test" / "Follow up" / "Remember" / "Question" */
    label: string;
    note: string | null;
    offset_ms: number;
}

export interface NotebookOverview {
    name: string;
    recording_kind: string;
    total_seconds: number;
    recordings: NotebookRecording[];
    items: NotebookItem[];
    marked: NotebookMark[];
    cards: number;
    questions: number;
}

export interface ReviewCard extends Card {
    meeting_id: string;
    meeting_title: string;
}

export interface ReviewQuestion extends QuizQuestion {
    meeting_id: string;
    meeting_title: string;
}

export interface NotebookReview {
    cards: ReviewCard[];
    questions: ReviewQuestion[];
}

/** What the notes' list of things to do is called, by the notebook's type. */
export function itemsTitle(kind: RecordingKind): string {
    return kind === "class" ? "Due" : kind === "personal" ? "To-dos" : "Action items";
}

/** The empty items section says what fills it. */
export function itemsEmpty(kind: RecordingKind, anyNotes: boolean): string {
    if (!anyNotes) {
        return "Nothing yet. Make notes on a recording, and what it says is to be done shows up here.";
    }
    return kind === "class"
        ? "No assignments, readings or deadlines were mentioned in the notes so far."
        : kind === "personal"
          ? "No to-dos or reminders were mentioned in the notes so far."
          : "No action items were found in the notes so far.";
}

/**
 * The marks section holds the ✎ marks and the questions (not every ★), so
 * it is named for them: "On the test and questions" for a class.
 */
export function markedTitle(kind: RecordingKind): string {
    const third = kind === "class" ? "On the test" : kind === "personal" ? "To remember" : "Follow ups";
    return `${third} and questions`;
}

export function markedEmpty(kind: RecordingKind): string {
    const third = kind === "class" ? "On the test" : kind === "personal" ? "Remember" : "Follow up";
    return `Nothing marked yet. While you record, Mark a moment and choose ${third} or Question: it shows up here.`;
}

function plural(n: number, one: string, many = `${one}s`): string {
    return `${n} ${n === 1 ? one : many}`;
}

/** 17400 → "4 h 50 min"; 3000 → "50 min"; 0 → "". */
export function totalTime(seconds: number): string {
    const mins = Math.round(Math.max(0, seconds) / 60);
    if (mins === 0) return "";
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return h === 0 ? `${m} min` : m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** "Class · 6 recordings · 4 h 50 min" under the notebook's name. */
export function headline(o: Pick<NotebookOverview, "recording_kind" | "recordings" | "total_seconds">): string {
    const kind = parseKind(o.recording_kind);
    const parts = [kind === "class" ? "Class" : kind === "personal" ? "Personal" : "Meeting", plural(o.recordings.length, "recording")];
    const t = totalTime(o.total_seconds);
    if (t) parts.push(t);
    return parts.join(" · ");
}

/** Open items first (in the page's order), then the ticked ones. */
export function splitItems(items: NotebookItem[]): { open: NotebookItem[]; done: NotebookItem[] } {
    return { open: items.filter((i) => !i.done), done: items.filter((i) => i.done) };
}

/** The items after ticking or unticking one. */
export function toggleItem(items: NotebookItem[], meetingId: string, key: string, done: boolean): NotebookItem[] {
    return items.map((i) => (i.meeting_id === meetingId && i.key === key ? { ...i, done } : i));
}

/** What "Review everything" holds, or null when no guide has been made. */
export function reviewSummary(cards: number, questions: number): string | null {
    if (cards === 0 && questions === 0) return null;
    const parts: string[] = [];
    if (cards > 0) parts.push(plural(cards, "flashcard"));
    if (questions > 0) parts.push(plural(questions, "practice question"));
    return parts.join(" and ");
}

/** "Nov 11" for the line under an item or a mark. */
export function shortDate(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

/** A stored quiz question as the quiz runner takes it. */
export function asQuizQuestion(q: Omit<ReviewQuestion, "explanation"> & { explanation: string | null }): ReviewQuestion {
    return { ...q, explanation: q.explanation ?? "" };
}
