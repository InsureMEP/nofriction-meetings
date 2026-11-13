// Record: idle, a title, the promise and one Record button, plus one card
// when something is starting (Up next). While recording, the capture bar
// (status, time left, what is captured, Mark, Capture screen), the live
// transcript with the notes the user types into it, and the Screens
// captured so far. No AI runs here.

import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { LiveTranscriptView } from "../LiveTranscript";
import { CaptureBar, Screens } from "../CaptureBar";
import { NoteComposer } from "../NoteComposer";
import { UpNextCard, useUpNext } from "../UpNextCard";
import { useRecording } from "../../hooks/useRecording";
import { useTranscripts } from "../../hooks/useTranscripts";
import { useRecordPicker } from "../RecordPicker";
import { PrivacyPromiseBadge } from "../PrivacyPromise";
import { NOTE_ADDED_EVENT, markersApi, notifyMarkersChanged } from "../../lib/study";
import { splitMarkers, type StoredMarker, type TypedNote } from "../../lib/studyLogic";

interface RecordViewProps {
    recording: ReturnType<typeof useRecording>;
    transcripts: ReturnType<typeof useTranscripts>;
}

interface TranscriptionStatus {
    connected: boolean;
    provider: string;
    error: string | null;
}

/** The notes typed in the recording in progress, in the order typed. */
function useLiveNotes(meetingId: string | null, isRecording: boolean) {
    const [notes, setNotes] = useState<TypedNote[]>([]);
    const add = useCallback(
        (row: StoredMarker) => {
            if (!meetingId || row.meeting_id !== meetingId) return;
            const [note] = splitMarkers([row]).notes;
            if (!note) return;
            setNotes((prev) => (prev.some((n) => n.id === note.id) ? prev : [...prev, note]));
            notifyMarkersChanged(meetingId);
        },
        [meetingId],
    );
    // A recording already under way (the window was reopened on it)
    useEffect(() => {
        setNotes([]);
        if (!meetingId || !isRecording) return;
        let live = true;
        markersApi
            .list(meetingId)
            .then((rows) => live && setNotes(splitMarkers(rows).notes))
            .catch(() => {});
        return () => {
            live = false;
        };
    }, [meetingId, isRecording]);
    useEffect(() => {
        let off: (() => void) | null = null;
        let disposed = false;
        listen<StoredMarker>(NOTE_ADDED_EVENT, (e) => add(e.payload)).then((fn) => (disposed ? fn() : (off = fn)));
        return () => {
            disposed = true;
            off?.();
        };
    }, [add]);
    return { notes, add };
}

export function RecordView({ recording, transcripts }: RecordViewProps) {
    const recordPicker = useRecordPicker();
    const [sttStatus, setSttStatus] = useState<TranscriptionStatus | null>(null);
    const { upNext, dismiss } = useUpNext();
    const liveNotes = useLiveNotes(recording.meetingId, recording.isRecording);

    // Transcription health: a failure must never be silent (screenshots but no
    // transcript, with no explanation)
    useEffect(() => {
        let unlisten: (() => void) | null = null;
        let disposed = false;
        listen<TranscriptionStatus>("transcription_status", (e) => setSttStatus(e.payload)).then((fn) => {
            if (disposed) fn();
            else unlisten = fn;
        });
        return () => {
            disposed = true;
            unlisten?.();
        };
    }, []);

    if (!recording.isRecording) {
        return (
            <div className="record-view is-idle">
                <LiveTranscriptView
                    isRecording={false}
                    transcripts={[]}
                    onStartRecording={recordPicker.open}
                    idleExtra={upNext && <UpNextCard upNext={upNext} onStart={recordPicker.start} onDismiss={dismiss} />}
                />
            </div>
        );
    }

    return (
        <div className="record-view is-live">
            <CaptureBar
                isRecording
                isPaused={recording.isPaused}
                onResume={() => void recording.resumeRecording()}
                sttStatus={sttStatus}
                audioWarning={recording.audioWarning}
            />
            <div className="record-view__body">
                <div className="record-view__transcript">
                    <div className="record-view__stream">
                        <LiveTranscriptView isRecording transcripts={transcripts.liveTranscripts} notes={liveNotes.notes} />
                        {transcripts.liveTranscripts.length > 0 && <PrivacyPromiseBadge />}
                    </div>
                    {/* Type while it listens: the line lands in the transcript at this moment */}
                    <NoteComposer onSaved={liveNotes.add} />
                </div>
                <aside className="record-view__screens">
                    <Screens isRecording />
                </aside>
            </div>
        </div>
    );
}
