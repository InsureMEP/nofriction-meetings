// noFriction - live transcript, and the idle Record screen
//
// One job: make what's being said readable the instant it's said.
// Text is set as calm paragraphs in a single reading column. The words
// still being spoken appear in place, dimmed, and firm up as Whisper
// finalizes them — nothing jumps, nothing re-flows below the fold.

import { useEffect, useRef, useMemo } from "react";
import type { LiveTranscript } from "../hooks/useTranscripts";
import type { TypedNote } from "../lib/studyLogic";
import { MicIcon, NoteIcon } from "./icons";
import { PrivacyPromise } from "./PrivacyPromise";
import "./LiveTranscript.css";

interface LiveTranscriptProps {
    transcripts: LiveTranscript[];
    isRecording: boolean;
    onStartRecording?: () => void;
    /** What the user typed while listening: shown where it was written */
    notes?: TypedNote[];
    /** Shown above the Record button on the idle screen (the Up next card) */
    idleExtra?: React.ReactNode;
}

interface Paragraph {
    id: string;
    speaker: string | null;
    start: Date;
    finals: { id: string; text: string }[];
    interim: string | null;
}

/** A pause this long (or a speaker change) starts a new paragraph. */
const PARAGRAPH_GAP_MS = 6000;
/** Paragraphs are also capped so the timestamp gutter stays useful. */
const PARAGRAPH_MAX_MS = 45000;

function groupParagraphs(transcripts: LiveTranscript[]): Paragraph[] {
    const out: Paragraph[] = [];
    let cur: Paragraph | null = null;
    let lastAt = 0;

    for (const t of transcripts) {
        const at = t.timestamp.getTime();
        const newPara =
            !cur ||
            cur.speaker !== t.speaker ||
            at - lastAt > PARAGRAPH_GAP_MS ||
            at - cur.start.getTime() > PARAGRAPH_MAX_MS;

        if (newPara) {
            cur = { id: t.id, speaker: t.speaker, start: t.timestamp, finals: [], interim: null };
            out.push(cur);
        }
        if (t.isFinal) {
            cur!.finals.push({ id: t.id, text: t.text });
        } else {
            cur!.interim = t.text;
        }
        lastAt = Math.max(lastAt, at);
    }
    return out;
}

const formatTime = (date: Date) =>
    date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

type Row = { at: number; para: Paragraph; note?: undefined } | { at: number; note: TypedNote; para?: undefined };

/** What was said and what the user wrote, in the order it happened. */
export function interleave(paragraphs: Paragraph[], notes: TypedNote[]): Row[] {
    const rows: Row[] = [
        ...paragraphs.map((para) => ({ at: para.start.getTime(), para })),
        ...notes.map((note) => ({ at: new Date(note.ts).getTime() || 0, note })),
    ];
    // A stable sort: a note typed during a paragraph follows it
    return rows.sort((a, b) => a.at - b.at);
}

export function LiveTranscriptView({ transcripts, isRecording, onStartRecording, notes = [], idleExtra }: LiveTranscriptProps) {
    const containerRef = useRef<HTMLDivElement>(null);
    const followRef = useRef(true);

    const paragraphs = useMemo(() => groupParagraphs(transcripts), [transcripts]);
    const rows = useMemo(() => interleave(paragraphs, notes), [paragraphs, notes]);

    // Follow the live edge unless the reader has scrolled up. A note the
    // user just typed always comes into view.
    const noteCount = notes.length;
    const lastNotes = useRef(noteCount);
    useEffect(() => {
        const el = containerRef.current;
        const typed = noteCount > lastNotes.current;
        lastNotes.current = noteCount;
        if (typed) followRef.current = true;
        if (!el || !followRef.current) return;
        requestAnimationFrame(() => {
            el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
        });
    }, [transcripts, noteCount]);

    const handleScroll = () => {
        const el = containerRef.current;
        if (!el) return;
        followRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    };

    if (rows.length === 0) {
        return (
            <div className={`lt-empty${idleExtra && !isRecording ? " has-suggestion" : ""}`}>
                {/* With something up next, its card is the anchor: no second emblem above it */}
                {!(idleExtra && !isRecording) && (
                    <div className={`lt-empty__mark ${isRecording ? "is-listening" : ""}`}>
                        <MicIcon size={28} strokeWidth={1.5} />
                    </div>
                )}
                {/* Something is starting: that is the headline, and the promise follows it */}
                {!(idleExtra && !isRecording) && <p className="lt-empty__title">{isRecording ? "Listening" : "Ready when you are"}</p>}
                {!isRecording && idleExtra}
                {isRecording ? (
                    <p className="lt-empty__hint">Words appear here as they're spoken. Type a note below at any moment.</p>
                ) : (
                    // "Recording and transcription work offline. Nothing leaves this Mac…"
                    <PrivacyPromise />
                )}
                {!isRecording && onStartRecording && (
                    <button
                        className={`lt-empty__action${idleExtra ? " is-second" : ""}`}
                        onClick={onStartRecording}
                        type="button"
                        title="⌘N records with your last type and length"
                    >
                        {/* One red dot per screen: the suggestion's button has it */}
                        {!idleExtra && <span className="lt-empty__action-dot" aria-hidden />}
                        {idleExtra ? "Record something else" : "Record"}
                    </button>
                )}
            </div>
        );
    }

    return (
        <div ref={containerRef} className="lt" onScroll={handleScroll} aria-live="polite">
            {rows.map((row) => {
                if (row.note) {
                    const n = row.note;
                    return (
                        <section key={`note-${n.id}`} className="lt-para lt-note" aria-label="Your note">
                            <time className="lt-para__time" dateTime={n.ts}>
                                {formatTime(new Date(n.ts))}
                            </time>
                            <div className="lt-note__body">
                                <span className="lt-note__glyph" aria-hidden>
                                    <NoteIcon size={13} strokeWidth={2.2} />
                                </span>
                                <p className="lt-note__text">{n.text}</p>
                            </div>
                        </section>
                    );
                }
                const p = row.para;
                return (
                <section key={p.id} className="lt-para">
                    <time className="lt-para__time" dateTime={p.start.toISOString()}>
                        {formatTime(p.start)}
                    </time>
                    <div className="lt-para__body">
                        {p.speaker && <div className="lt-para__speaker">{p.speaker}</div>}
                        <p className="lt-para__text">
                            {p.finals.map((f) => (
                                <span key={f.id} className="lt-final">
                                    {f.text}{" "}
                                </span>
                            ))}
                            {p.interim && (
                                <span className="lt-interim">
                                    {p.interim}
                                    <span className="lt-caret" aria-hidden />
                                </span>
                            )}
                        </p>
                    </div>
                </section>
                );
            })}
        </div>
    );
}
