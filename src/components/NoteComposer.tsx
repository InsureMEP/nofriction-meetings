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

export function NoteComposer({ onSaved }: { onSaved: (row: StoredMarker) => void }) {
    const [text, setText] = useState("");
    const [error, setError] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);
    const inputRef = useRef<HTMLInputElement>(null);

    // The cursor is ready when the recording starts: just type
    useEffect(() => {
        inputRef.current?.focus({ preventScroll: true });
    }, []);

    const save = async () => {
        if (saving) return;
        const line = cleanTypedNote(text);
        if (!line) {
            if (text.trim()) setError(`Keep a note under ${MAX_TYPED_NOTE_CHARS} characters.`);
            return;
        }
        setSaving(true);
        setError(null);
        try {
            onSaved(await markersApi.addNote(line));
            setText("");
        } catch (e) {
            // The text stays in the field so nothing typed is lost
            setError(`Couldn't keep that note: ${String(e)}`);
        } finally {
            setSaving(false);
            inputRef.current?.focus({ preventScroll: true });
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
            <div className="note-composer">
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
                    aria-label="Type a note. Enter keeps it at this moment."
                    maxLength={MAX_TYPED_NOTE_CHARS}
                    autoComplete="off"
                    spellCheck
                />
                {length > COUNT_FROM ? (
                    <span className="note-composer__hint" aria-live="polite">
                        {MAX_TYPED_NOTE_CHARS - length} left
                    </span>
                ) : (
                    <span className="note-composer__hint" aria-hidden>
                        {text.trim() ? "Enter keeps it at this moment" : "Stamped with the moment you press Enter"}
                    </span>
                )}
            </div>
        </div>
    );
}
