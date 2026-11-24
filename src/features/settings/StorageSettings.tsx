// Settings → Storage (src-tauri/src/space.rs, docs/STORAGE.md): what
// noFriction keeps on this Mac, measured from the files, and the ways to
// make it smaller. Anything that removes something asks first, in place.

import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
    KEEP_DAYS,
    dueFor,
    formatBytes,
    keepLabel,
    onSpaceChanged,
    parts,
    spaceApi,
    type Freed,
    type RecordingSpace,
    type SpaceOverview,
} from "../../lib/space";
import { openSettings } from "../../lib/navigation";
import "./StorageSettings.css";

/** How many of the largest recordings are listed before "Show all" */
const SHOWN = 8;

const day = (iso: string) => new Date(iso).toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" });

export function StorageSettings({ showToast }: { showToast: (text: string, error?: boolean) => void }) {
    const [o, setO] = useState<SpaceOverview | null>(null);
    const [failed, setFailed] = useState(false);
    /** What is running ("all", "video", "leftover", or a recording id + action) */
    const [busy, setBusy] = useState<string | null>(null);
    const [progress, setProgress] = useState<{ done: number; total: number; bytes: number } | null>(null);
    /** The action waiting for its second click */
    const [asking, setAsking] = useState<string | null>(null);
    const [keepAsk, setKeepAsk] = useState<number | null>(null);
    const [all, setAll] = useState(false);

    const load = useCallback(() => {
        spaceApi
            .overview()
            .then((v) => {
                setO(v);
                setFailed(false);
            })
            .catch(() => setFailed(true));
    }, []);

    useEffect(() => {
        load();
        return onSpaceChanged(load);
    }, [load]);

    useEffect(() => {
        let off: (() => void) | null = null;
        let disposed = false;
        listen<{ done: number; total: number; bytes: number }>("space_progress", (e) => setProgress(e.payload))
            .then((stop) => (disposed ? stop() : (off = stop)))
            .catch(() => {});
        return () => {
            disposed = true;
            off?.();
        };
    }, []);

    const run = async (id: string, what: () => Promise<Freed>, done: (f: Freed) => string) => {
        if (busy) return;
        setBusy(id);
        setAsking(null);
        setProgress(null);
        try {
            const freed = await what();
            showToast(done(freed));
        } catch (e) {
            showToast(String(e), true);
        } finally {
            setBusy(null);
            setProgress(null);
            load();
        }
    };

    const smallerDone = (f: Freed) => (f.items === 0 ? "The pictures were already small." : `${f.items} ${f.items === 1 ? "picture" : "pictures"} made smaller. ${formatBytes(f.bytes)} freed.`);
    const freedDone = (f: Freed) => (f.bytes === 0 ? "There was nothing to remove." : `${formatBytes(f.bytes)} freed.`);

    if (!o) {
        return (
            <section className="settings-section">
                <p className="section-desc">{failed ? "The storage couldn't be measured. Close Settings and open it again." : "Measuring…"}</p>
            </section>
        );
    }

    const shown = parts(o);
    const list = all ? o.recordings : o.recordings.slice(0, SHOWN);
    const due = keepAsk === null ? [] : dueFor(o.recordings, keepAsk);
    const working = busy !== null;

    /** A button that asks once more, in place, before it removes anything. */
    const confirmable = (id: string, label: string, question: string, go: () => void) =>
        asking === id ? (
            <span className="space-ask" role="group" aria-label={question}>
                <span className="space-ask__q">{question}</span>
                <button type="button" className="btn-secondary space-ask__yes" onClick={go} disabled={working}>
                    Remove
                </button>
                <button type="button" className="space-link" onClick={() => setAsking(null)}>
                    Cancel
                </button>
            </span>
        ) : (
            <button type="button" className="btn-secondary" onClick={() => setAsking(id)} disabled={working}>
                {label}
            </button>
        );

    return (
        <>
            <section className="settings-section space">
                <div className="space-total">
                    <span className="space-total__n" data-testid="space-total">{formatBytes(o.total_bytes)}</span>
                    <span className="space-total__what">used by noFriction on this Mac</span>
                    {o.free_bytes !== null && <span className="space-total__free">{formatBytes(o.free_bytes)} free on the disk</span>}
                </div>
                <div
                    className="space-bar"
                    role="img"
                    aria-label={shown.map((p) => `${p.label} ${formatBytes(p.bytes)}`).join(", ")}
                >
                    {shown.map((p) => (
                        <span key={p.id} className={`space-bar__part space-bar__part--${p.id}`} style={{ flexGrow: p.bytes }} title={`${p.label}: ${formatBytes(p.bytes)}`} />
                    ))}
                </div>

                <ul className="space-parts">
                    <li className="space-part">
                        <span className="space-dot space-dot--screens" />
                        <span className="space-part__what">
                            <span className="label-main">Screens and photos</span>
                            <span className="label-sub">
                                {o.screens_count.toLocaleString()} {o.screens_count === 1 ? "picture" : "pictures"} in your recordings.
                                {o.shrink.pictures > 0 && " The older ones were saved at full screen size."}
                            </span>
                        </span>
                        <span className="space-part__size">{formatBytes(o.screens_bytes)}</span>
                        <span className="space-part__do">
                            {busy === "all" ? (
                                <span className="space-progress" role="status">
                                    {progress ? `${progress.done} of ${progress.total} · ${formatBytes(progress.bytes)} freed` : "Starting…"}
                                </span>
                            ) : o.shrink.pictures > 0 ? (
                                <button
                                    type="button"
                                    className="btn-primary"
                                    data-testid="space-shrink"
                                    onClick={() => void run("all", () => spaceApi.makeSmaller(null), smallerDone)}
                                    disabled={working}
                                    title="Re-saves pictures larger than 1600 pixels at that size. They stay sharp enough to read, and nothing else about the recording changes."
                                >
                                    Make smaller · saves about {formatBytes(o.shrink.saves_bytes)}
                                </button>
                            ) : null}
                        </span>
                    </li>
                    {o.video_bytes > 0 && (
                        <li className="space-part">
                            <span className="space-dot space-dot--video" />
                            <span className="space-part__what">
                                <span className="label-main">Screen video</span>
                                <span className="label-sub">Recorded by earlier versions. Nothing in noFriction shows it, and it is no longer recorded.</span>
                            </span>
                            <span className="space-part__size">{formatBytes(o.video_bytes)}</span>
                            <span className="space-part__do">
                                {confirmable("video", "Remove", `Remove ${formatBytes(o.video_bytes)} of screen video? Your screens, transcripts and notes stay.`, () =>
                                    void run("video", () => spaceApi.removeVideo(null), freedDone),
                                )}
                            </span>
                        </li>
                    )}
                    {o.models_bytes > 0 && (
                        <li className="space-part">
                            <span className="space-dot space-dot--models" />
                            <span className="space-part__what">
                                <span className="label-main">Speech model</span>
                                <span className="label-sub">What turns speech into text on this Mac.</span>
                            </span>
                            <span className="space-part__size">{formatBytes(o.models_bytes)}</span>
                            <span className="space-part__do">
                                <button type="button" className="space-link" onClick={() => openSettings("transcription")}>
                                    Transcription settings
                                </button>
                            </span>
                        </li>
                    )}
                    <li className="space-part">
                        <span className="space-dot space-dot--library" />
                        <span className="space-part__what">
                            <span className="label-main">Transcripts and notes</span>
                            <span className="label-sub">Everything that was said and written, for all your recordings.</span>
                        </span>
                        <span className="space-part__size">{formatBytes(o.database_bytes)}</span>
                        <span className="space-part__do" />
                    </li>
                    {o.leftover_bytes > 0 && (
                        <li className="space-part">
                            <span className="space-dot space-dot--leftover" />
                            <span className="space-part__what">
                                <span className="label-main">Leftover files</span>
                                <span className="label-sub">Screens captured while nothing was recording, and files left behind by recordings you deleted. No recording uses them.</span>
                            </span>
                            <span className="space-part__size">{formatBytes(o.leftover_bytes)}</span>
                            <span className="space-part__do">
                                {confirmable("leftover", "Clear", `Clear ${formatBytes(o.leftover_bytes)} of leftover files?`, () =>
                                    void run("leftover", () => spaceApi.clearLeftovers(), freedDone),
                                )}
                            </span>
                        </li>
                    )}
                    {o.other_bytes > 0 && (
                        <li className="space-part">
                            <span className="space-dot space-dot--other" />
                            <span className="space-part__what">
                                <span className="label-main">Everything else</span>
                                <span className="label-sub">Lessons, logs and settings.</span>
                            </span>
                            <span className="space-part__size">{formatBytes(o.other_bytes)}</span>
                            <span className="space-part__do" />
                        </li>
                    )}
                </ul>
            </section>

            <section className="settings-section">
                <h3>From now on</h3>
                <div className="settings-row">
                    <div className="settings-label">
                        <span className="label-main">Save screens</span>
                        <span className="label-sub">
                            Smaller saves each screen at 1600 pixels, less than half the space of full size, and still easy to read. Used from your next recording.
                        </span>
                    </div>
                    <select
                        className="space-select"
                        aria-label="Save screens"
                        value={o.screen_size}
                        disabled={working}
                        onChange={(e) => {
                            const size = e.target.value === "full" ? "full" : "smaller";
                            setO({ ...o, screen_size: size });
                            spaceApi.setScreenSize(size).catch((err) => showToast(String(err), true));
                        }}
                    >
                        <option value="smaller">Smaller (recommended)</option>
                        <option value="full">Full size</option>
                    </select>
                </div>
                <div className="settings-row">
                    <div className="settings-label">
                        <span className="label-main">Keep screens for</span>
                        <span className="label-sub">
                            After this long, a recording's screens are removed. Its transcript, notes and photos from your iPhone stay. A paired iPhone removes its copies of those screens too.
                        </span>
                    </div>
                    <select
                        className="space-select"
                        aria-label="Keep screens for"
                        data-testid="space-keep"
                        value={keepAsk ?? o.keep_screens_days}
                        disabled={working}
                        onChange={(e) => {
                            const days = Number(e.target.value);
                            // Longer, or nothing to remove now: no question to ask
                            if (dueFor(o.recordings, days).length === 0) {
                                setKeepAsk(null);
                                void run("keep", () => spaceApi.setKeepScreens(days), () => `Screens are kept ${days === 0 ? "forever" : `for ${keepLabel(days)}`}.`);
                            } else {
                                setKeepAsk(days);
                            }
                        }}
                    >
                        {KEEP_DAYS.map((d) => (
                            <option key={d} value={d}>{keepLabel(d)}</option>
                        ))}
                    </select>
                </div>
                {keepAsk !== null && due.length > 0 && (
                    <div className="space-keep-ask" role="alertdialog" aria-label="Remove screens now?">
                        <p>
                            This removes the screens of {due.length} {due.length === 1 ? "recording" : "recordings"} older than {keepLabel(keepAsk)} now
                            (up to {formatBytes(due.reduce((n, r) => n + r.total_bytes, 0))}; photos from your iPhone stay), and of later ones as they reach that
                            age. It can't be undone.
                        </p>
                        <button
                            type="button"
                            className="btn-secondary space-ask__yes"
                            disabled={working}
                            onClick={() => {
                                const days = keepAsk;
                                setKeepAsk(null);
                                void run("keep", () => spaceApi.setKeepScreens(days), freedDone);
                            }}
                        >
                            Remove and keep for {keepLabel(keepAsk)}
                        </button>
                        <button type="button" className="space-link" onClick={() => setKeepAsk(null)}>
                            Cancel
                        </button>
                    </div>
                )}
            </section>

            <section className="settings-section">
                <h3>Largest recordings</h3>
                {o.recordings.length === 0 ? (
                    <p className="section-desc">No recordings yet.</p>
                ) : (
                    <ul className="space-recs">
                        {list.map((r) => (
                            <RecordingRow
                                key={r.meeting_id}
                                r={r}
                                busy={busy}
                                working={working}
                                progress={busy === `smaller:${r.meeting_id}` ? progress : null}
                                confirmable={confirmable}
                                onSmaller={() => void run(`smaller:${r.meeting_id}`, () => spaceApi.makeSmaller(r.meeting_id), smallerDone)}
                                onRemove={() => void run(`remove:${r.meeting_id}`, () => spaceApi.removeScreens(r.meeting_id), freedDone)}
                            />
                        ))}
                    </ul>
                )}
                {o.recordings.length > SHOWN && (
                    <button type="button" className="space-link space-more" onClick={() => setAll((v) => !v)}>
                        {all ? "Show fewer" : `Show all ${o.recordings.length}`}
                    </button>
                )}
                <p className="section-desc space-foot">
                    To remove a whole recording, delete it in Recordings. Making pictures smaller changes only the files on this Mac.
                </p>
            </section>
        </>
    );
}

function RecordingRow({
    r,
    busy,
    working,
    progress,
    confirmable,
    onSmaller,
    onRemove,
}: {
    r: RecordingSpace;
    busy: string | null;
    working: boolean;
    progress: { done: number; total: number; bytes: number } | null;
    confirmable: (id: string, label: string, question: string, go: () => void) => React.ReactNode;
    onSmaller: () => void;
    onRemove: () => void;
}) {
    const has = r.screens_count > 0 || r.video_bytes > 0;
    return (
        <li className="space-rec" data-testid="space-row">
            <span className="space-rec__what">
                <span className="label-main" title={r.title}>{r.title}</span>
                <span className="label-sub">
                    {day(r.started_at)}
                    {r.screens_count > 0 && ` · ${r.screens_count.toLocaleString()} ${r.screens_count === 1 ? "picture" : "pictures"}`}
                    {r.video_bytes > 0 && ` · ${formatBytes(r.video_bytes)} of screen video`}
                    {r.recording && " · recording now"}
                </span>
            </span>
            <span className="space-rec__size">{formatBytes(r.total_bytes)}</span>
            <span className="space-rec__do">
                {busy === `smaller:${r.meeting_id}` ? (
                    <span className="space-progress" role="status">{progress ? `${progress.done} of ${progress.total}` : "Starting…"}</span>
                ) : (
                    !r.recording && (
                        <>
                            {r.saves_bytes > 0 && (
                                <button type="button" className="btn-secondary" onClick={onSmaller} disabled={working} title={`Saves about ${formatBytes(r.saves_bytes)}`}>
                                    Make smaller
                                </button>
                            )}
                            {has &&
                                confirmable(
                                    `remove:${r.meeting_id}`,
                                    "Remove screens",
                                    "Remove this recording's screens? What was read from them goes too, and a paired iPhone removes its copies. The transcript, notes and iPhone photos stay.",
                                    onRemove,
                                )}
                        </>
                    )
                )}
            </span>
        </li>
    );
}
