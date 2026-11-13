// Notes for one recording (Recordings → Notes). First, what the user typed
// while listening ("What you wrote"), each line with its time and, once
// notes are made, what was being said then. Then the AI notes (made by
// themselves when a recording stops, or with Make notes), the "made before
// an edit" banner, (meetings) a follow-up email draft, and the Topics. The
// notes' style follows the recording's type: meeting notes, lecture notes
// (class) or personal notes.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as tauri from "../lib/tauri";
import { aiErrorClass, friendlyAiError, isNoProviderError, withAiConsent } from "../lib/ai";
import { useCapabilities, useIsPro } from "../lib/build";
import { proSuffix, showProMark } from "../lib/pro";
import { AiSetupNotice, useAiStatus } from "./AiSetupNotice";
import { notesLayout, notesStyleHint } from "../lib/recordingKind";
import { SYNCED_NOTES_MODEL, syncedNotesSections } from "../lib/syncLogic";
import {
    NOTES_DONE_EVENT,
    NOTES_PROGRESS_EVENT,
    WHAT_YOU_WROTE,
    hasWhatYouWrote,
    parseNoteDetails,
    progressDetail,
    writtenLines,
    type NotesProgress,
    type WrittenLine,
} from "../lib/notesLogic";
import { markersApi, notifyMarkersChanged } from "../lib/study";
import { MAX_TYPED_NOTE_CHARS, cleanTypedNote, clock } from "../lib/studyLogic";
import { useMarkers } from "./study/MarkerList";
import { NoteIcon, TrashIcon } from "./icons";
import { useUndoDelete } from "./UndoToast";
import { ItemCheck } from "./ItemCheck";
import { notebookApi } from "../lib/notebookPage";
import { itemFor, type RecordingItem } from "../lib/notebookPageLogic";
import { useRecordingKind } from "../hooks/useRecordingKind";
import ErrorState from "./ErrorState";
import { TopicsEditor } from "./TopicChips";
import "./MeetingNotesPanel.css";

interface SavedNotes {
    id: string;
    meeting_id: string;
    summary: string | null;
    key_topics: string | null;
    decisions: string | null;
    action_items: string | null;
    generated_at: string;
    model_used: string | null;
    stale_after_edit?: boolean;
    /** JSON `[{marker_id, detail}]`: what was being said at each typed note */
    note_details?: string | null;
}

interface FollowUpEmail {
    subject: string;
    body: string;
    to: string[];
}

type Decision = { text: string; made_by?: string | null; context?: string | null };
type ActionItem = { task: string; assignee?: string | null; due_date?: string | null };

function parseList<T>(json: string | null): T[] {
    if (!json) return [];
    try {
        const v = JSON.parse(json);
        return Array.isArray(v) ? (v as T[]) : [];
    } catch {
        return [];
    }
}

/** Error text for a failed AI action: never a raw machine prefix. */
function aiFailure(e: unknown, what: string): string {
    switch (aiErrorClass(e)) {
        case "pro_required":
            return `${what} are part of noFriction Pro.`;
        case "consent_required":
            return `${what} need your permission to send this recording to your AI endpoint. Try again and choose Allow.`;
        default:
            return friendlyAiError(e);
    }
}

interface MeetingNotesPanelProps {
    meetingId: string;
    /** Open Rewind at a moment (a typed note's time) */
    onJump?: (ms: number) => void;
}

export function MeetingNotesPanel({ meetingId, onJump }: MeetingNotesPanelProps) {
    const [notes, setNotes] = useState<SavedNotes | null>(null);
    const [loading, setLoading] = useState(true);
    const [loadError, setLoadError] = useState<string | null>(null);
    const [generating, setGenerating] = useState(false);
    // A run going in the background (automatic notes after Stop): what it is doing
    const [running, setRunning] = useState<string | null>(null);
    // What the user typed while listening
    const { notes: typed, reload: reloadTyped } = useMarkers(meetingId);
    // The ticks on what the notes say is to be done (the notebook's page shows the same ones)
    const [items, setItems] = useState<RecordingItem[]>([]);
    const [actionError, setActionError] = useState<string | null>(null);
    const [needsAi, setNeedsAi] = useState(false);
    const [email, setEmail] = useState<FollowUpEmail | null>(null);
    const [drafting, setDrafting] = useState(false);
    const { configured } = useAiStatus();
    const caps = useCapabilities();
    const isPro = useIsPro();
    const kind = useRecordingKind(meetingId);
    // A provider was just connected in Settings: clear the earlier failure
    useEffect(() => {
        if (configured) setNeedsAi(false);
    }, [configured]);

    const load = useCallback(async () => {
        setLoading(true);
        setLoadError(null);
        try {
            setNotes(await invoke<SavedNotes | null>("get_meeting_notes", { meetingId }));
            // Without ticks the list still shows; it just has no boxes
            setItems(await notebookApi.recordingItems(meetingId).catch(() => []));
        } catch (e) {
            setLoadError(String(e));
        } finally {
            setLoading(false);
        }
    }, [meetingId]);

    const tick = (item: RecordingItem, done: boolean) => {
        // The tick lands at once; if it can't be saved it is put back
        const set = (v: boolean) => setItems((all) => all.map((i) => (i.key === item.key ? { ...i, done: v } : i)));
        set(done);
        notebookApi.setDone(meetingId, item.key, done).catch((e) => {
            set(!done);
            setActionError(`Couldn't save that tick: ${String(e)}`);
        });
    };

    useEffect(() => {
        setNotes(null);
        setEmail(null);
        setActionError(null);
        setNeedsAi(false);
        setRunning(null);
        load();
        // Notes may be being made right now (the recording just stopped)
        let live = true;
        invoke<string | null>("get_notes_status", { meetingId })
            .then((label) => live && setRunning(label ?? null))
            .catch(() => {});
        return () => {
            live = false;
        };
    }, [load, meetingId]);

    // The run reports what it is doing, and when it ends the notes are read again
    useEffect(() => {
        const offs: (() => void)[] = [];
        let disposed = false;
        const add = (off: () => void) => (disposed ? off() : offs.push(off));
        listen<NotesProgress>(NOTES_PROGRESS_EVENT, (e) => {
            if (e.payload.meetingId === meetingId) setRunning(e.payload.label);
        }).then(add);
        listen<{ meetingId: string; ok: boolean }>(NOTES_DONE_EVENT, (e) => {
            if (e.payload.meetingId !== meetingId) return;
            setRunning(null);
            void load();
        }).then(add);
        return () => {
            disposed = true;
            offs.forEach((o) => o());
        };
    }, [meetingId, load]);

    const generate = async () => {
        setGenerating(true);
        setActionError(null);
        setNeedsAi(false);
        try {
            // The notes prompt follows the recording's type (recording_kind.rs)
            await tauri.generateMeetingReport(meetingId);
            await load();
        } catch (e) {
            if (isNoProviderError(e)) setNeedsAi(true);
            else setActionError(aiFailure(e, "Notes"));
        } finally {
            setGenerating(false);
        }
    };

    const draftEmail = async () => {
        setDrafting(true);
        setActionError(null);
        setNeedsAi(false);
        try {
            setEmail(await withAiConsent(() => invoke<FollowUpEmail>("draft_followup_email", { meetingId }), "follow_up"));
        } catch (e) {
            if (isNoProviderError(e)) setNeedsAi(true);
            else setActionError(aiFailure(e, "Follow-up emails"));
        } finally {
            setDrafting(false);
        }
    };

    if (loading && !notes) {
        return (
            <div className="mn-panel mn-center">
                <div className="loading-spinner" />
            </div>
        );
    }

    if (loadError) {
        return (
            <div className="mn-panel">
                <ErrorState title="Couldn't load the notes" message="Try again in a moment." onRetry={load} />
            </div>
        );
    }

    const proNote = proSuffix(caps?.pro_gating, isPro);
    const showSetup = configured === false || needsAi;
    const busy = generating || running !== null;
    const busyDetail = progressDetail(running);
    // Notes made on the iPhone already carry "What you wrote" in their text
    const synced = notes?.model_used === SYNCED_NOTES_MODEL;
    const lines =
        synced && hasWhatYouWrote(syncedNotesSections(notes?.summary ?? ""))
            ? []
            : writtenLines(typed, parseNoteDetails(notes?.note_details));

    const actions = (
        <div className="mn-actions">
            <button className="mn-btn primary" onClick={generate} disabled={busy || showSetup}>
                {busy ? "Making notes…" : notes ? "Make again" : "Make notes"}
            </button>
            {kind === "meeting" && (
                <button className="mn-btn" onClick={draftEmail} disabled={drafting || showSetup}>
                    {drafting ? "Drafting…" : `Follow-up email${proNote}`}
                </button>
            )}
        </div>
    );

    return (
        <div className="mn-panel">
            {showSetup && <AiSetupNotice feature={kind === "meeting" ? "Notes and follow-up emails" : "Notes"} />}
            {actionError && (
                <p className="mn-error" role="alert">
                    {actionError}
                </p>
            )}

            {lines.length > 0 && (
                <WhatYouWrote
                    lines={lines}
                    onJump={onJump}
                    onChanged={() => {
                        notifyMarkersChanged(meetingId);
                        reloadTyped();
                    }}
                />
            )}

            {busy && (
                <div className="mn-busy" role="status" aria-live="polite">
                    <div className="loading-spinner" aria-hidden />
                    <div>
                        <p className="mn-busy__title">Making notes…</p>
                        <p className="mn-busy__detail">
                            {busyDetail ??
                                (lines.length > 0
                                    ? "Reading the whole recording, starting from what you wrote."
                                    : "Reading the whole recording.")}
                        </p>
                    </div>
                </div>
            )}

            {!notes ? (
                !busy && (
                    <div className="mn-empty">
                        <h3>{lines.length > 0 ? "Finish these notes" : "No notes yet"}</h3>
                        <p>
                            {lines.length > 0
                                ? "Make notes reads the whole recording and builds on what you wrote: under each line, what was being said then."
                                : "Notes are made from the whole recording by the AI you set up, on their own when a recording of six minutes or more stops (Settings → AI)."}{" "}
                            {notesStyleHint(kind)}
                            {showProMark(caps?.pro_gating, isPro) ? " Part of noFriction Pro." : ""}
                        </p>
                        {actions}
                    </div>
                )
            ) : (
                <div className="mn-notes">
                    {notes.stale_after_edit && (
                        <div className="rd-stale" role="status">
                            <span>These notes were made before an edit.</span>
                            <button className="rd-btn" onClick={generate} disabled={busy || showSetup}>
                                {busy ? "Making notes…" : "Make again"}
                            </button>
                        </div>
                    )}
                    <NotesBody notes={notes} items={items} onTick={tick} />
                    <p className="mn-meta">
                        Made {new Date(notes.generated_at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}
                    </p>
                    {actions}
                </div>
            )}
            <TopicsEditor meetingId={meetingId} />

            {email && <FollowUpSheet email={email} onClose={() => setEmail(null)} />}
        </div>
    );
}

/**
 * The user's own lines, in the order typed: the time (click to hear that
 * moment), the line (click to change it), and under it what was being
 * said, once notes are made. Theirs to edit and delete; the AI never
 * rewrites a line.
 */
function WhatYouWrote({
    lines,
    onJump,
    onChanged,
}: {
    lines: WrittenLine[];
    onJump?: (ms: number) => void;
    onChanged: () => void;
}) {
    const [error, setError] = useState<string | null>(null);
    // A line being deleted: gone at once, Undo for a few seconds (the one
    // safety model for every delete in the app)
    const [hidden, setHidden] = useState<string | null>(null);
    const undoDelete = useUndoDelete((e) => setError(`Couldn't delete that note: ${String(e)}`));
    const run = async (fn: () => Promise<unknown>) => {
        setError(null);
        try {
            await fn();
            onChanged();
        } catch (e) {
            setError(String(e));
        }
    };
    const remove = (line: WrittenLine) => {
        setError(null);
        setHidden(line.note.id);
        undoDelete.start(
            "Deleted your note",
            async () => {
                // It stays hidden until the list is read again without it
                await markersApi.remove(line.note.id);
                onChanged();
            },
            () => setHidden(null),
        );
    };
    const shown = lines.filter((l) => l.note.id !== hidden);
    if (shown.length === 0) return <>{undoDelete.toast}</>;
    return (
        <section className="mn-wrote" aria-label={WHAT_YOU_WROTE}>
            <h4 className="mn-wrote__title">
                <NoteIcon size={13} strokeWidth={2.2} /> {WHAT_YOU_WROTE}
            </h4>
            {error && (
                <p className="mn-error" role="alert">
                    {error}
                </p>
            )}
            <ol className="mn-wrote__list">
                {shown.map((l) => (
                    <WrittenRow key={l.note.id} line={l} onJump={onJump} run={run} onDelete={() => remove(l)} />
                ))}
            </ol>
            {undoDelete.toast}
        </section>
    );
}

function WrittenRow({
    line,
    onJump,
    run,
    onDelete,
}: {
    line: WrittenLine;
    onJump?: (ms: number) => void;
    run: (fn: () => Promise<unknown>) => void;
    onDelete: () => void;
}) {
    const { note, detail } = line;
    const [editing, setEditing] = useState(false);
    const [text, setText] = useState(note.text);
    const textRef = useRef<HTMLButtonElement>(null);
    const wasEditing = useRef(false);
    useEffect(() => setText(note.text), [note.text]);
    // Leaving the field puts the keyboard back on the line it edited
    useEffect(() => {
        if (wasEditing.current && !editing) textRef.current?.focus({ preventScroll: true });
        wasEditing.current = editing;
    }, [editing]);
    const save = () => {
        setEditing(false);
        const cleaned = cleanTypedNote(text);
        // An emptied line is left as it was: deleting is the bin's job
        if (!cleaned) {
            setText(note.text);
            return;
        }
        if (cleaned !== note.text) run(() => markersApi.setNoteText(note.id, cleaned));
    };
    return (
        <li className="mn-wrote__row">
            <button
                type="button"
                className="mn-wrote__time"
                onClick={() => onJump?.(note.offset_ms)}
                disabled={!onJump}
                title="Hear this moment in Rewind"
                aria-label={`Open Rewind at ${clock(note.offset_ms)}`}
            >
                {clock(note.offset_ms)}
            </button>
            <div className="mn-wrote__body">
                {editing ? (
                    <input
                        className="mn-input mn-wrote__input"
                        autoFocus
                        value={text}
                        maxLength={MAX_TYPED_NOTE_CHARS}
                        aria-label="Your note"
                        onChange={(e) => setText(e.target.value)}
                        onBlur={save}
                        onKeyDown={(e) => {
                            if (e.key === "Enter" && !e.nativeEvent.isComposing) save();
                            if (e.key === "Escape") {
                                setText(note.text);
                                setEditing(false);
                            }
                        }}
                    />
                ) : (
                    <button ref={textRef} type="button" className="mn-wrote__text" onClick={() => setEditing(true)} title="Change what you wrote">
                        {note.text}
                    </button>
                )}
                {detail && <p className="mn-wrote__detail">{detail}</p>}
            </div>
            <button
                type="button"
                className="mn-wrote__delete"
                onClick={onDelete}
                title="Delete this note"
                aria-label={`Delete your note at ${clock(note.offset_ms)}`}
            >
                <TrashIcon size={13} />
            </button>
        </li>
    );
}

/** The layout follows the prompt that wrote the notes (`model_used`), so
 *  notes written before a type change keep their headings until regenerated. */
type Ticks = { items: RecordingItem[]; onTick: (item: RecordingItem, done: boolean) => void };

/**
 * What the notes say is to be done. Each line has the same tick as on its
 * notebook's page; an item the ticks don't know (older notes) is a plain
 * line.
 */
function ActionItems({ actions, detail, items, onTick }: { actions: ActionItem[]; detail: (a: ActionItem) => string } & Ticks) {
    return (
        <ul className="mn-items">
            {actions.map((a, i) => {
                const item = itemFor(items, a.task);
                const more = detail(a);
                return (
                    <li key={i} className={`mn-item${item?.done ? " is-done" : ""}${item ? "" : " is-plain"}`}>
                        {item && <ItemCheck done={item.done} label={a.task} onToggle={(next) => onTick(item, next)} />}
                        <span className="mn-item__text">
                            {a.task}
                            {more && <span className="mn-muted"> — {more}</span>}
                        </span>
                    </li>
                );
            })}
        </ul>
    );
}

function NotesBody({ notes, items, onTick }: { notes: SavedNotes } & Ticks) {
    if (notes.model_used === SYNCED_NOTES_MODEL) {
        return <SyncedNotesBody markdown={notes.summary ?? ""} />;
    }
    const topics = parseList<string>(notes.key_topics);
    const decisions = parseList<Decision>(notes.decisions);
    const actions = parseList<ActionItem>(notes.action_items);
    const layout = notesLayout(notes.model_used);
    const ticks = { items, onTick };
    if (layout === "lecture") {
        return <LectureNotesBody summary={notes.summary} concepts={topics} definitions={decisions} announcements={actions} ticks={ticks} />;
    }
    if (layout === "personal") {
        return <PersonalNotesBody summary={notes.summary} points={topics} todos={actions} ticks={ticks} />;
    }
    return (
        <>
            <section className="mn-section">
                <h4>Summary</h4>
                <p>{notes.summary?.trim() || "No summary."}</p>
            </section>
            {topics.length > 0 && (
                <section className="mn-section">
                    <h4>Key topics</h4>
                    <ul>
                        {topics.map((t, i) => (
                            <li key={i}>{t}</li>
                        ))}
                    </ul>
                </section>
            )}
            <section className="mn-section">
                <h4>Decisions</h4>
                {decisions.length === 0 ? (
                    <p className="mn-muted">None recorded.</p>
                ) : (
                    <ul>
                        {decisions.map((d, i) => (
                            <li key={i}>
                                {d.text}
                                {d.made_by && <span className="mn-muted"> — {d.made_by}</span>}
                            </li>
                        ))}
                    </ul>
                )}
            </section>
            <section className="mn-section">
                <h4>Action items</h4>
                {actions.length === 0 ? (
                    <p className="mn-muted">None recorded.</p>
                ) : (
                    <ActionItems
                        actions={actions}
                        detail={(a) => `${a.assignee || "owner not stated"}${a.due_date ? ` · ${a.due_date}` : ""}`}
                        {...ticks}
                    />
                )}
            </section>
        </>
    );
}

/** Notes made on the iPhone and synced here (docs/SYNC.md): its Markdown,
 *  bold headings and bullets. */
function SyncedNotesBody({ markdown }: { markdown: string }) {
    const sections = syncedNotesSections(markdown);
    if (sections.length === 0) return <p className="mn-muted">No summary.</p>;
    return (
        <>
            {sections.map((s, i) => (
                <section className="mn-section" key={i}>
                    {s.heading && <h4>{s.heading}</h4>}
                    {s.paragraphs.map((t, j) => (
                        <p key={j}>{t}</p>
                    ))}
                    {s.bullets.length > 0 && (
                        <ul>
                            {s.bullets.map((t, j) => (
                                <li key={j}>{t}</li>
                            ))}
                        </ul>
                    )}
                </section>
            ))}
        </>
    );
}

/** Same stored shape, framed for a class: concepts, definitions with examples, announcements. */
function LectureNotesBody({
    summary,
    concepts,
    definitions,
    announcements,
    ticks,
}: {
    summary: string | null;
    concepts: string[];
    definitions: Decision[];
    announcements: ActionItem[];
    ticks: Ticks;
}) {
    return (
        <>
            <section className="mn-section">
                <h4>Lecture summary</h4>
                <p>{summary?.trim() || "No summary."}</p>
            </section>
            <section className="mn-section">
                <h4>Key concepts</h4>
                {concepts.length === 0 ? (
                    <p className="mn-muted">None recorded.</p>
                ) : (
                    <ul>
                        {concepts.map((t, i) => (
                            <li key={i}>{t}</li>
                        ))}
                    </ul>
                )}
            </section>
            <section className="mn-section">
                <h4>Definitions and examples</h4>
                {definitions.length === 0 ? (
                    <p className="mn-muted">None recorded.</p>
                ) : (
                    <ul>
                        {definitions.map((d, i) => (
                            <li key={i}>
                                {d.text}
                                {d.context && <span className="mn-muted"> — e.g. {d.context}</span>}
                            </li>
                        ))}
                    </ul>
                )}
            </section>
            <section className="mn-section">
                <h4>Announcements and deadlines</h4>
                {announcements.length === 0 ? (
                    <p className="mn-muted">None mentioned.</p>
                ) : (
                    <ActionItems actions={announcements} detail={(a) => a.due_date ?? ""} {...ticks} />
                )}
            </section>
        </>
    );
}

/** Same stored shape, framed for a personal recording: key points, to-dos and reminders. */
function PersonalNotesBody({ summary, points, todos, ticks }: { summary: string | null; points: string[]; todos: ActionItem[]; ticks: Ticks }) {
    return (
        <>
            <section className="mn-section">
                <h4>Summary</h4>
                <p>{summary?.trim() || "No summary."}</p>
            </section>
            <section className="mn-section">
                <h4>Key points</h4>
                {points.length === 0 ? (
                    <p className="mn-muted">None recorded.</p>
                ) : (
                    <ul>
                        {points.map((t, i) => (
                            <li key={i}>{t}</li>
                        ))}
                    </ul>
                )}
            </section>
            <section className="mn-section">
                <h4>To-dos and reminders</h4>
                {todos.length === 0 ? (
                    <p className="mn-muted">None mentioned.</p>
                ) : (
                    <ActionItems actions={todos} detail={(a) => a.due_date ?? ""} {...ticks} />
                )}
            </section>
        </>
    );
}

function FollowUpSheet({ email, onClose }: { email: FollowUpEmail; onClose: () => void }) {
    const [subject, setSubject] = useState(email.subject);
    const [body, setBody] = useState(email.body);
    const [copied, setCopied] = useState(false);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [onClose]);

    const copy = async () => {
        try {
            await navigator.clipboard.writeText(`Subject: ${subject}\n\n${body}`);
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
        } catch {
            setError("Couldn't copy. Select the text and press ⌘C.");
        }
    };

    const openInMail = () => {
        const to = email.to.map(encodeURIComponent).join(",");
        const url = `mailto:${to}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
        openUrl(url).catch(() => setError("Couldn't open Mail. Copy the email instead."));
    };

    return (
        <div className="modal-overlay mn-sheet-overlay" role="dialog" aria-modal="true" aria-labelledby="mn-sheet-title" onClick={onClose}>
            <div className="mn-sheet" onClick={(e) => e.stopPropagation()}>
                <h3 id="mn-sheet-title">Follow-up email</h3>
                <p className="mn-muted">
                    Drafted by your AI from the transcript. Check it before sending.
                    {email.to.length > 0 ? ` To: ${email.to.join(", ")}` : ""}
                </p>
                <label className="mn-label" htmlFor="mn-subject">Subject</label>
                <input id="mn-subject" className="mn-input" value={subject} onChange={(e) => setSubject(e.target.value)} />
                <label className="mn-label" htmlFor="mn-body">Message</label>
                <textarea id="mn-body" className="mn-textarea" value={body} onChange={(e) => setBody(e.target.value)} rows={14} />
                {error && <p className="mn-error">{error}</p>}
                <div className="mn-actions end">
                    <button className="mn-btn" onClick={onClose}>Close</button>
                    <button className="mn-btn" onClick={copy}>{copied ? "Copied" : "Copy"}</button>
                    <button className="mn-btn primary" onClick={openInMail}>Open in Mail</button>
                </div>
            </div>
        </div>
    );
}
