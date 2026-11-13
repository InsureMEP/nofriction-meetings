// Recordings: one search field, the list (narrowed by Notebook), and beside
// it either the notebook's page (pick a notebook: what's due, what was
// marked, review everything) or the selected recording with its title
// block, four views (Rewind · Notes · Links · Review), Make lesson on a
// Class recording (docs/LESSONS.md §13) and Export… (docs/EXPORT.md).

import { useEffect, useState } from "react";
import {
    onExportRequest,
    onRecordingSeek,
    onRecordingView,
    requestMakeLesson,
    requestRecordingSeek,
    requestRecordingView,
    takeExport,
    takeRecordingSeek,
    takeRecordingView,
    type RecordingViewName,
} from "../../lib/navigation";
import { MeetingHistory } from "../MeetingHistory";
import { NotebookPage } from "../NotebookPage";
import { RewindGallery, type RewindSeek } from "../RewindGallery";
import { MeetingNotesPanel } from "../MeetingNotesPanel";
import { MeetingLinksPanel } from "../MeetingLinksPanel";
import { MeetingPeople } from "../People";
import { RecordingTitleBlock } from "../Notebook";
import { StudyPanel } from "../study/StudyPanel";
import { ExportSheet } from "../export/ExportSheet";
import { useRecordingKind } from "../../hooks/useRecordingKind";
import { guideTitle } from "../../lib/recordingKind";

type MeetingView = RecordingViewName;

interface RecordingsViewProps {
    onSelectMeeting: (id: string) => void;
    selectedMeetingId: string | null;
    refreshKey: number;
    isRecording: boolean;
    recordingMeetingId: string | null;
}

export function RecordingsView({ onSelectMeeting, selectedMeetingId, refreshKey, isRecording, recordingMeetingId }: RecordingsViewProps) {
    const [meetingView, setMeetingView] = useState<MeetingView>("rewind");
    // Bumped when a recording's type or notebook is edited, so the list and its filter refresh
    const [metaEdits, setMetaEdits] = useState(0);
    // Links → "first said at 12:03", Review → "Jump to this moment", Chat citations
    // and search results show the moment in Rewind
    const [seek, setSeek] = useState<RewindSeek | null>(null);
    const [exporting, setExporting] = useState<string | null>(null);
    // The notebook the list is narrowed to, and whether its page is showing
    // (picking a notebook opens the page; opening a recording puts it away)
    const [notebook, setNotebook] = useState<string | null>(null);
    const [pageOpen, setPageOpen] = useState(false);
    const showPage = notebook !== null && pageOpen;
    const kind = useRecordingKind(selectedMeetingId);
    const jumpTo = (meetingId: string, ms: number) => {
        setSeek({ meetingId, ms, n: Date.now() });
        setMeetingView("rewind");
    };

    // A recording asked for on one of its views (after Stop: Notes)
    useEffect(() => {
        const pending = takeRecordingView(selectedMeetingId);
        if (pending) setMeetingView(pending);
        return onRecordingView((id, view) => {
            if (id === selectedMeetingId) {
                takeRecordingView(selectedMeetingId);
                setMeetingView(view);
            }
        });
    }, [selectedMeetingId]);

    useEffect(() => {
        const pending = takeRecordingSeek(selectedMeetingId);
        if (pending) jumpTo(pending.meetingId, pending.ms);
        return onRecordingSeek((req) => {
            if (req.meetingId === selectedMeetingId) {
                takeRecordingSeek(selectedMeetingId);
                jumpTo(req.meetingId, req.ms);
            }
        });
    }, [selectedMeetingId]);

    // Opening the recording that is still being recorded shows it live
    const live = isRecording && selectedMeetingId !== null && selectedMeetingId === recordingMeetingId;

    // File → Export Recording… (⌘E): the selected recording, once it's stopped
    useEffect(() => {
        const open = () => {
            if (selectedMeetingId && !live) setExporting(selectedMeetingId);
        };
        if (takeExport()) open();
        return onExportRequest(() => {
            takeExport();
            open();
        });
    }, [selectedMeetingId, live]);

    const tabs: { id: MeetingView; label: string }[] = [
        { id: "rewind", label: "Rewind" },
        { id: "notes", label: "Notes" },
        { id: "links", label: "Links" },
        { id: "review", label: guideTitle(kind) },
    ];

    const openRecording = (id: string) => {
        setPageOpen(false);
        onSelectMeeting(id);
    };

    return (
        <div className={`recordings-view ${selectedMeetingId || showPage ? "has-selection" : ""}`}>
            <MeetingHistory
                onSelectMeeting={openRecording}
                selectedMeetingId={showPage ? null : selectedMeetingId}
                refreshKey={refreshKey + metaEdits}
                compact={!!selectedMeetingId || showPage}
                notebook={notebook}
                onNotebook={(n) => {
                    setNotebook(n);
                    setPageOpen(n !== null);
                }}
                onOpenAt={(id, ms) => {
                    openRecording(id);
                    jumpTo(id, ms);
                }}
            />
            {showPage && notebook && (
                <NotebookPage
                    notebook={notebook}
                    refreshKey={refreshKey + metaEdits}
                    isRecording={isRecording}
                    onOpenNotes={(id) => {
                        requestRecordingView(id, "notes");
                        openRecording(id);
                    }}
                    onOpenAt={(id, ms) => {
                        requestRecordingSeek(id, ms);
                        openRecording(id);
                    }}
                />
            )}
            {!showPage && selectedMeetingId && (
                <div className="recording-detail">
                    {notebook && (
                        <button type="button" className="nb-back" onClick={() => setPageOpen(true)} title={`Back to ${notebook}`}>
                            ‹ {notebook}
                        </button>
                    )}
                    <RecordingTitleBlock meetingId={selectedMeetingId} refreshKey={refreshKey} onSaved={() => setMetaEdits((n) => n + 1)} />
                    <MeetingPeople meetingId={selectedMeetingId} />
                    <div className="deck-tabs" role="tablist" aria-label="Recording view">
                        {tabs.map((t) => (
                            <button
                                key={t.id}
                                role="tab"
                                aria-selected={meetingView === t.id}
                                className={`deck-tab ${meetingView === t.id ? "active" : ""}`}
                                onClick={() => setMeetingView(t.id)}
                            >
                                {t.label}
                            </button>
                        ))}
                        <span className="deck-tabs__spacer" />
                        {kind === "class" && (
                            // Make lesson (docs/LESSONS.md §13): opens Lessons and makes an
                            // interactive lesson from this lecture's transcript
                            <button
                                className="deck-tabs__export deck-tabs__make-lesson"
                                onClick={() => requestMakeLesson(selectedMeetingId)}
                                disabled={live}
                                title={
                                    live
                                        ? "Stop the recording to make a lesson from it"
                                        : "Turn this lecture into an interactive lesson in Lessons"
                                }
                            >
                                Make lesson
                            </button>
                        )}
                        <button
                            className="deck-tabs__export"
                            onClick={() => setExporting(selectedMeetingId)}
                            disabled={live}
                            title={live ? "Stop the recording to export it" : "Export this recording with its notes, screens and photos"}
                        >
                            Export…
                        </button>
                    </div>
                    <div className="recording-detail__body">
                        {meetingView === "rewind" ? (
                            <RewindGallery meetingId={selectedMeetingId} isRecording={live} seek={seek} />
                        ) : meetingView === "notes" ? (
                            <MeetingNotesPanel meetingId={selectedMeetingId} onJump={(ms) => jumpTo(selectedMeetingId, ms)} />
                        ) : meetingView === "links" ? (
                            <MeetingLinksPanel meetingId={selectedMeetingId} onJump={(ms) => jumpTo(selectedMeetingId, ms)} />
                        ) : (
                            <StudyPanel meetingId={selectedMeetingId} onJump={(ms) => jumpTo(selectedMeetingId, ms)} />
                        )}
                    </div>
                </div>
            )}
            {exporting && <ExportSheet meetingId={exporting} onClose={() => setExporting(null)} />}
        </div>
    );
}
