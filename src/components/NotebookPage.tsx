// The notebook page (Recordings → pick a notebook): everything its
// recordings hold, in one place. What's due (or the action items, or the
// to-dos) with a tick each, every ✎ and ? mark one click from its moment,
// and Review everything: the flashcards and practice questions of every
// study guide in the notebook, together. Nothing here calls the AI; the
// page is read from the notes, marks and guides that exist.
// docs/design/FADELL_V2.md §3 C

import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { notebookApi } from "../lib/notebookPage";
import {
    headline,
    itemsEmpty,
    itemsTitle,
    markedEmpty,
    markedTitle,
    reviewSummary,
    shortDate,
    splitItems,
    toggleItem,
    type NotebookItem,
    type NotebookMark,
    type NotebookOverview,
    type NotebookReview,
    type ReviewQuestion,
} from "../lib/notebookPageLogic";
import { NOTES_DONE_EVENT } from "../lib/notesLogic";
import { MARKERS_CHANGED_EVENT } from "../lib/study";
import { clock } from "../lib/studyLogic";
import { parseKind, type RecordingKind } from "../lib/recordingKind";
import { requestChatAboutNotebook } from "../lib/navigation";
import { useRecordPicker } from "./RecordPicker";
import { FlashcardDeck } from "./study/FlashcardDeck";
import { QuizRunner } from "./study/QuizRunner";
import { MarkerGlyph } from "./study/MarkerBits";
import ErrorState from "./ErrorState";
import { CheckIcon, NotebookIcon } from "./icons";
import "./MeetingNotesPanel.css";
import "./NotebookPage.css";

interface NotebookPageProps {
    notebook: string;
    /** Bumped when recordings change (a stop, a delete, an edit) */
    refreshKey: number;
    isRecording: boolean;
    /** Open a recording on its Notes */
    onOpenNotes: (meetingId: string) => void;
    /** Open a recording in Rewind at a moment */
    onOpenAt: (meetingId: string, ms: number) => void;
}

export function NotebookPage({ notebook, refreshKey, isRecording, onOpenNotes, onOpenAt }: NotebookPageProps) {
    const [page, setPage] = useState<NotebookOverview | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [tickError, setTickError] = useState<string | null>(null);
    const [review, setReview] = useState<"cards" | "quiz" | null>(null);
    const [showDone, setShowDone] = useState(false);
    const recordPicker = useRecordPicker();

    const load = useCallback(() => {
        setError(null);
        notebookApi
            .overview(notebook)
            .then(setPage)
            .catch((e) => setError(String(e)));
    }, [notebook]);

    useEffect(() => {
        setPage(null);
        setReview(null);
        setShowDone(false);
        setTickError(null);
        load();
    }, [load, refreshKey]);

    // Notes finished or marks changed somewhere: the page is read again
    useEffect(() => {
        const offs: (() => void)[] = [];
        let disposed = false;
        listen(NOTES_DONE_EVENT, () => load()).then((off) => (disposed ? off() : offs.push(off)));
        window.addEventListener(MARKERS_CHANGED_EVENT, load);
        return () => {
            disposed = true;
            offs.forEach((o) => o());
            window.removeEventListener(MARKERS_CHANGED_EVENT, load);
        };
    }, [load]);

    if (error) {
        return (
            <div className="nb-page">
                <ErrorState title="Couldn't open this notebook" message="Your recordings are safe on this Mac. Try again." onRetry={load} />
            </div>
        );
    }
    if (!page) {
        return (
            <div className="nb-page nb-page--center" aria-busy="true">
                <div className="loading-spinner" />
            </div>
        );
    }

    const kind = parseKind(page.recording_kind);
    const { open, done } = splitItems(page.items);
    const anyNotes = page.recordings.some((r) => r.has_notes);
    const reviewText = reviewSummary(page.cards, page.questions);

    const tick = (item: NotebookItem, next: boolean) => {
        // The tick lands at once; if it can't be saved it is put back
        setTickError(null);
        setPage((p) => (p ? { ...p, items: toggleItem(p.items, item.meeting_id, item.key, next) } : p));
        notebookApi.setDone(item.meeting_id, item.key, next).catch((e) => {
            setPage((p) => (p ? { ...p, items: toggleItem(p.items, item.meeting_id, item.key, !next) } : p));
            setTickError(`Couldn't save that tick: ${String(e)}`);
        });
    };

    // Review everything leads the side column once there is something to
    // review; until then it follows what was marked
    const reviewCard = (
                <section className="nb-card" aria-labelledby="nb-review-title">
                    <h3 id="nb-review-title" className="nb-card__title">Review everything</h3>
                    {reviewText ? (
                        <>
                            <p className="nb-review__text">
                                {reviewText} from this notebook's {kind === "class" ? "study" : "review"} guides, as one.
                            </p>
                            <div className="nb-review__actions">
                                {page.cards > 0 && (
                                    <button type="button" className="mn-btn primary" onClick={() => setReview("cards")}>
                                        Flashcards
                                    </button>
                                )}
                                {page.questions > 0 && (
                                    <button type="button" className={`mn-btn${page.cards > 0 ? "" : " primary"}`} onClick={() => setReview("quiz")}>
                                        Practice quiz
                                    </button>
                                )}
                            </div>
                        </>
                    ) : (
                        <p className="nb-empty">
                            No {kind === "class" ? "study" : "review"} guides yet. Make one on a recording, and its flashcards and practice
                            questions join the others here.
                        </p>
                    )}
                </section>
    );

    return (
        <div className="nb-page">
            <header className="nb-head">
                <div className="nb-head__text">
                    <h2 className="nb-head__name">
                        <NotebookIcon size={20} strokeWidth={1.75} />
                        <span>{page.name}</span>
                    </h2>
                    <p className="nb-head__meta">{headline(page)}</p>
                </div>
                <div className="nb-head__actions">
                    <button type="button" className="mn-btn" onClick={() => requestChatAboutNotebook(page.name)} title={`Ask about everything recorded in ${page.name}`}>
                        Ask
                    </button>
                    <button
                        type="button"
                        className="mn-btn primary nb-record"
                        disabled={isRecording}
                        title={isRecording ? "A recording is already running" : `Record in ${page.name} with your usual length`}
                        onClick={() =>
                            void recordPicker.start({ notebook: page.name, recordingKind: kind, remember: false }).catch((e) => setTickError(`Couldn't start: ${String(e)}`))
                        }
                    >
                        <span className="nb-record__dot" aria-hidden />
                        Record
                    </button>
                </div>
            </header>

            {tickError && (
                <p className="mn-error" role="alert">
                    {tickError}
                </p>
            )}

            <div className="nb-grid">
                <section className="nb-card nb-card--items" aria-labelledby="nb-items-title">
                    <h3 id="nb-items-title" className="nb-card__title">
                        {itemsTitle(kind)}
                        {open.length > 0 && <span className="nb-card__count">{open.length}</span>}
                    </h3>
                    {page.items.length === 0 ? (
                        <p className="nb-empty">{itemsEmpty(kind, anyNotes)}</p>
                    ) : (
                        <>
                            {open.length === 0 && <p className="nb-empty">All done.</p>}
                            <ul className="nb-items">
                                {open.map((i) => (
                                    <ItemRow key={`${i.meeting_id}:${i.key}`} item={i} onTick={tick} onOpenNotes={onOpenNotes} />
                                ))}
                            </ul>
                            {done.length > 0 && (
                                <>
                                    <button type="button" className="nb-done-toggle" aria-expanded={showDone} onClick={() => setShowDone((s) => !s)}>
                                        {showDone ? "Hide" : "Show"} {done.length} done
                                    </button>
                                    {showDone && (
                                        <ul className="nb-items is-done">
                                            {done.map((i) => (
                                                <ItemRow key={`${i.meeting_id}:${i.key}`} item={i} onTick={tick} onOpenNotes={onOpenNotes} />
                                            ))}
                                        </ul>
                                    )}
                                </>
                            )}
                        </>
                    )}
                </section>

                <div className="nb-side">
                    {reviewText ? reviewCard : null}
                    <section className="nb-card" aria-labelledby="nb-marked-title">
                        <h3 id="nb-marked-title" className="nb-card__title">
                            {markedTitle(kind)}
                            {page.marked.length > 0 && <span className="nb-card__count">{page.marked.length}</span>}
                        </h3>
                        {page.marked.length === 0 ? (
                            <p className="nb-empty">{markedEmpty(kind)}</p>
                        ) : (
                            <ul className="nb-marks">
                                {page.marked.map((m) => (
                                    <MarkRow key={m.id} mark={m} onOpenAt={onOpenAt} />
                                ))}
                            </ul>
                        )}
                    </section>
                    {reviewText ? null : reviewCard}
                </div>
            </div>

            {review && (
                <ReviewSheet
                    notebook={page.name}
                    kind={kind}
                    mode={review}
                    onClose={() => setReview(null)}
                    onOpenAt={(id, ms) => {
                        setReview(null);
                        onOpenAt(id, ms);
                    }}
                />
            )}
        </div>
    );
}

function ItemRow({
    item,
    onTick,
    onOpenNotes,
}: {
    item: NotebookItem;
    onTick: (item: NotebookItem, done: boolean) => void;
    onOpenNotes: (meetingId: string) => void;
}) {
    return (
        <li className={`nb-item${item.done ? " is-done" : ""}`}>
            <button
                type="button"
                role="checkbox"
                aria-checked={item.done}
                className="nb-item__check"
                onClick={() => onTick(item, !item.done)}
                aria-label={item.task}
                title={item.done ? "Mark as not done" : "Mark as done"}
            >
                {item.done && <CheckIcon size={12} strokeWidth={3} />}
            </button>
            <div className="nb-item__body">
                <p className="nb-item__task">
                    {item.task}
                    {item.due && <span className="nb-item__due">{item.due}</span>}
                </p>
                <p className="nb-item__from">
                    {item.assignee && <span>{item.assignee} · </span>}
                    <button type="button" className="nb-link" onClick={() => onOpenNotes(item.meeting_id)} title="Open this recording's notes">
                        {item.meeting_title}
                    </button>
                    <span> · {shortDate(item.started_at)}</span>
                </p>
            </div>
        </li>
    );
}

function MarkRow({ mark, onOpenAt }: { mark: NotebookMark; onOpenAt: (meetingId: string, ms: number) => void }) {
    return (
        <li>
            <button
                type="button"
                className={`nb-mark is-${mark.kind}`}
                onClick={() => onOpenAt(mark.meeting_id, mark.offset_ms)}
                title={`Open ${mark.meeting_title} at ${clock(mark.offset_ms)}`}
            >
                <span className="nb-mark__kind">
                    <MarkerGlyph kind={mark.kind} size={13} />
                    {mark.label}
                </span>
                <span className="nb-mark__note">{mark.note ?? <span className="nb-mark__none">No note</span>}</span>
                <span className="nb-mark__from">
                    {mark.meeting_title} · {shortDate(mark.started_at)} · {clock(mark.offset_ms)}
                </span>
            </button>
        </li>
    );
}

/** Review everything: one deck or one quiz from every guide in the notebook. */
function ReviewSheet({
    notebook,
    kind,
    mode,
    onClose,
    onOpenAt,
}: {
    notebook: string;
    kind: RecordingKind;
    mode: "cards" | "quiz";
    onClose: () => void;
    onOpenAt: (meetingId: string, ms: number) => void;
}) {
    const [data, setData] = useState<NotebookReview | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [tab, setTab] = useState(mode);

    useEffect(() => {
        let live = true;
        notebookApi
            .review(notebook)
            .then((r) => live && setData(r))
            .catch((e) => live && setError(String(e)));
        return () => {
            live = false;
        };
    }, [notebook]);

    useEffect(() => {
        const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [onClose]);

    const both = !!data && data.cards.length > 0 && data.questions.length > 0;

    return (
        <div className="modal-overlay nb-sheet-overlay" role="dialog" aria-modal="true" aria-labelledby="nb-sheet-title" onClick={onClose}>
            <div className="nb-sheet" onClick={(e) => e.stopPropagation()}>
                <header className="nb-sheet__head">
                    <div>
                        <h3 id="nb-sheet-title">Review everything</h3>
                        <p className="nb-sheet__sub">
                            {notebook} · every {kind === "class" ? "study" : "review"} guide, oldest recording first
                        </p>
                    </div>
                    <button type="button" className="mn-btn" onClick={onClose}>
                        Close
                    </button>
                </header>
                {both && (
                    <div className="deck-tabs nb-sheet__tabs" role="tablist" aria-label="Review">
                        <button role="tab" aria-selected={tab === "cards"} className={`deck-tab ${tab === "cards" ? "active" : ""}`} onClick={() => setTab("cards")}>
                            Flashcards ({data.cards.length})
                        </button>
                        <button role="tab" aria-selected={tab === "quiz"} className={`deck-tab ${tab === "quiz" ? "active" : ""}`} onClick={() => setTab("quiz")}>
                            Practice quiz ({data.questions.length})
                        </button>
                    </div>
                )}
                <div className="nb-sheet__body">
                    {error && <ErrorState title="Couldn't open the review" message="Try again in a moment." />}
                    {!data && !error && (
                        <div className="nb-page--center">
                            <div className="loading-spinner" />
                        </div>
                    )}
                    {data &&
                        (tab === "quiz" && data.questions.length > 0 ? (
                            <QuizRunner<ReviewQuestion> questions={data.questions} onJump={(ms, q) => onOpenAt(q.meeting_id, ms)} />
                        ) : data.cards.length > 0 ? (
                            <FlashcardDeck cards={data.cards} />
                        ) : data.questions.length > 0 ? (
                            <QuizRunner<ReviewQuestion> questions={data.questions} onJump={(ms, q) => onOpenAt(q.meeting_id, ms)} />
                        ) : (
                            <p className="nb-empty">The guides in this notebook were removed. Make one again on a recording.</p>
                        ))}
                </div>
            </div>
        </div>
    );
}
