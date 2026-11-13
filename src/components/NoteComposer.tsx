// The note field under the live transcript: type while it listens. Enter
// keeps the line, stamped with this moment, and it appears in the
// transcript where it was written (docs/design/FADELL_V2.md §3 B). A note
// is stored beside the marks (kind `note`), so Rewind, Notes, Export, Chat
// and sync all see it. No AI runs here.

import { useEffect, useRef, useState } from "react";
import { NoteIcon } from "./icons";
import { markersApi } from "../lib/study";
import { MAX_TYPED_NOTE_CHARS, cleanTypedNote, type StoredMarker } from "../lib/studyLogic";
import "./NoteComposer.css";

/** Show how much room is left once the line is this close to the limit. */
const COUNT_FROM = MAX_TYPED_NOTE_CHARS - 100;

interface NoteComposerProps {
    /** The recording in progress: the line is kept on it even if Stop is pressed right after Return */
    meetingId: string | null;
    onSaved: (row: StoredMarker) => void;
}

export function NoteComposer({ meetingId, onSaved }: NoteComposerProps) {
    const [text, setText] = useState("");
    const [error, setError] = useState<string | null>(null);
    // Lines on their way to being kept
    const [saving, setSaving] = useState(0);
    const inputRef = useRef<HTMLInputElement>(null);

    // The cursor is ready when the recording starts: just type
    useEffect(() => {
        inputRef.current?.focus({ preventScroll: true });
    }, []);

    // The line leaves the field the moment Return is pressed, so the next
    // one can be typed while it is being kept. If it can't be kept it comes
    // back, ahead of whatever was typed since.
    const save = async () => {
        const line = cleanTypedNote(text);
        if (!line) {
            if (text.trim()) setError(`Keep a note under ${MAX_TYPED_NOTE_CHARS} characters.`);
            return;
        }
        // The moment and the recording are taken now, not when the save arrives
        const at = new Date();
        setText("");
        setError(null);
        setSaving((n) => n + 1);
        try {
            onSaved(await markersApi.addNote(line, meetingId, at));
        } catch (e) {
            setText((typed) => (typed.trim() ? `${line} ${typed}` : line));
            setError(`Couldn't keep that note: ${String(e)}`);
        } finally {
            setSaving((n) => n - 1);
        }
    };

    const length = [...text].length;

    return (
        <div className="note-composer-wrap">
            {error && (
                <p className="note-composer__error" role="alert">
                    {error}
                </p>
            )}
            <div className="note-composer" aria-busy={saving > 0}>
                <span className="note-composer__glyph" aria-hidden>
                    <NoteIcon size={15} strokeWidth={2} />
                </span>
                <input
                    ref={inputRef}
                    className="note-composer__input"
                    value={text}
                    onChange={(e) => {
                        setText(e.target.value);
                        if (error) setError(null);
                    }}
                    onKeyDown={(e) => {
                        if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                            e.preventDefault();
                            void save();
                        } else if (e.key === "Escape") {
                            (e.target as HTMLInputElement).blur();
                        }
                    }}
                    placeholder="Type a note"
                    aria-label="Type a note. Return keeps it at this moment."
                    maxLength={MAX_TYPED_NOTE_CHARS}
                    autoComplete="off"
                    spellCheck
                />
                {length > COUNT_FROM ? (
                    <span className="note-composer__hint" aria-live="polite">
                        {MAX_TYPED_NOTE_CHARS - length} left
                    </span>
                ) : (
                    // Said only while there is something to keep
                    text.trim() && (
                        <span className="note-composer__hint" aria-hidden>
                            <kbd className="note-composer__key">Return</kbd> keeps it at this moment
                        </span>
                    )
                )}
            </div>
        </div>
    );
}
