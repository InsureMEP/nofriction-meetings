// On the screens: what the user's AI read from this recording's screens, and
// the one button that sends them (src-tauri/src/screen_reading.rs).
//
// A picture leaves this Mac only from here, and only after the confirmation
// below, which shows every picture that would go and names where. The ids
// left ticked and that destination are passed back, and the Mac sends those
// ids to that destination and nothing else.

import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { aiErrorClass, friendlyAiError, isNoProviderError, withAiConsent } from "../lib/ai";
import { useCapabilities, useIsPro } from "../lib/build";
import { proSuffix } from "../lib/pro";
import { clock } from "../lib/studyLogic";
import {
    SCREENS_PROGRESS_EVENT,
    confirmButton,
    confirmScope,
    confirmTitle,
    confirmedFor,
    nothingLeft,
    readingParts,
    summaryLine,
    tickedIds,
    trapTab,
    type ReadSummary,
    type ScreenPlan,
    type StoredReading,
} from "../lib/screenReading";
import "./ScreenReadings.css";

function failure(e: unknown): string {
    if (isNoProviderError(e)) return "Reading a screen needs an AI that can read images. Set one up under Settings → AI.";
    switch (aiErrorClass(e)) {
        case "pro_required":
            return "Reading screens is part of noFriction Pro.";
        case "consent_required":
            return "Reading screens needs your permission to send to your AI endpoint. Nothing was sent.";
        case "vision_unavailable":
            return "Reading a screen needs a model that can read images. Choose one under Settings → AI → Screens.";
        default:
            return friendlyAiError(e);
    }
}

const FOCUSABLE = 'button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex="-1"])';

interface Props {
    meetingId: string;
    /** Open Rewind at a moment (when the screen was shown) */
    onJump?: (ms: number) => void;
}

export function ScreenReadings({ meetingId, onJump }: Props) {
    const [plan, setPlan] = useState<ScreenPlan | null>(null);
    const [readings, setReadings] = useState<StoredReading[]>([]);
    const [loadError, setLoadError] = useState(false);
    const [confirming, setConfirming] = useState(false);
    /** The pictures left ticked in the confirmation */
    const [ticked, setTicked] = useState<ReadonlySet<string>>(new Set());
    const [running, setRunning] = useState<{ done: number; total: number } | null>(null);
    const [stopping, setStopping] = useState(false);
    const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
    const caps = useCapabilities();
    const isPro = useIsPro();
    const opener = useRef<HTMLButtonElement>(null);
    const dialog = useRef<HTMLDivElement>(null);
    /** This view started the run under way (its result comes back to `send`) */
    const mine = useRef(false);

    // Only the latest read is shown
    const request = useRef(0);
    const load = useCallback(async (): Promise<ScreenPlan | null> => {
        const id = ++request.current;
        try {
            const [p, r] = await Promise.all([
                invoke<ScreenPlan>("plan_screen_reading", { meetingId }),
                invoke<StoredReading[]>("get_screen_readings", { meetingId }),
            ]);
            if (id !== request.current) return null;
            setPlan(p);
            setReadings(r);
            setLoadError(false);
            // A run started before this view opened: show it, with its Stop
            if (p.running && !mine.current) setRunning((cur) => cur ?? { done: 0, total: 0 });
            return p;
        } catch {
            if (id === request.current) {
                setPlan(null);
                setLoadError(true);
            }
            return null;
        }
    }, [meetingId]);

    useEffect(() => {
        mine.current = false;
        setPlan(null);
        setReadings([]);
        setLoadError(false);
        setConfirming(false);
        setTicked(new Set());
        setRunning(null);
        setStopping(false);
        setMessage(null);
        void load();
    }, [load]);

    useEffect(() => {
        let off: (() => void) | null = null;
        let disposed = false;
        listen<{ meetingId: string; done: number; total: number }>(SCREENS_PROGRESS_EVENT, (e) => {
            if (e.payload.meetingId !== meetingId) return;
            const { done, total } = e.payload;
            if (done >= total && !mine.current) {
                // A run this view didn't start has finished
                setRunning(null);
                setStopping(false);
                void load();
            } else {
                setRunning((r) => (r ? { done, total } : r));
            }
        }).then((fn) => (disposed ? fn() : (off = fn)));
        return () => {
            disposed = true;
            off?.();
        };
    }, [meetingId, load]);

    // The confirmation closes on Escape, keeps Tab inside itself, and gives
    // the keyboard back to its button
    const close = useCallback(() => {
        setConfirming(false);
        opener.current?.focus({ preventScroll: true });
    }, []);
    useEffect(() => {
        if (!confirming) return;
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                close();
                return;
            }
            if (e.key !== "Tab" || !dialog.current) return;
            const controls = Array.from(dialog.current.querySelectorAll<HTMLElement>(FOCUSABLE));
            const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
            const next = trapTab(controls, active, e.shiftKey);
            if (next) {
                e.preventDefault();
                next.focus();
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [confirming, close]);

    if (loadError) {
        return (
            <section className="mn-section sr" aria-label="On the screens">
                <h4>On the screens</h4>
                <p className="mn-error" role="alert">
                    This recording's screens couldn't be checked.{" "}
                    <button type="button" className="mn-btn" onClick={() => void load()}>
                        Try again
                    </button>
                </p>
            </section>
        );
    }
    // A recording without screens has nothing to read
    if (!plan || plan.total + plan.left_out === 0) return null;

    const open = async () => {
        setMessage(null);
        // What would go is worked out at the moment of asking, and the dialog
        // shows exactly that: it doesn't change while it is open
        const fresh = await load();
        if (!fresh) return;
        setTicked(new Set(fresh.send.map((s) => s.state_id)));
        setConfirming(true);
    };

    const toggle = (id: string) =>
        setTicked((cur) => {
            const next = new Set(cur);
            if (!next.delete(id)) next.add(id);
            return next;
        });

    const send = async () => {
        const confirmed = confirmedFor(plan, ticked);
        if (!confirmed) return;
        setConfirming(false);
        mine.current = true;
        setStopping(false);
        setRunning({ done: 0, total: confirmed.state_ids.length });
        try {
            // If Pro or consent is asked for, nothing has been sent yet; the
            // same confirmation is then passed again. A run that already sent
            // a picture comes back as a result, never as a retry.
            const summary = await withAiConsent(() => invoke<ReadSummary>("read_screens", { meetingId, confirmed }), "notes");
            const text = summary.stopped ? `${summaryLine(summary)} It stopped there: ${failure(summary.stopped)}` : summaryLine(summary);
            setMessage({ text, error: summary.stopped !== null });
        } catch (e) {
            setMessage({ text: failure(e), error: true });
        } finally {
            mine.current = false;
            setRunning(null);
            setStopping(false);
            await load();
        }
    };

    const stop = async () => {
        setStopping(true);
        try {
            await invoke<boolean>("stop_screen_reading", { meetingId });
        } catch {
            setStopping(false);
        }
    };

    const done = nothingLeft(plan);
    const proNote = proSuffix(caps?.pro_gating, isPro);
    const count = tickedIds(plan, ticked).length;

    return (
        <section className="mn-section sr" aria-label="On the screens">
            <h4>On the screens</h4>
            {readings.length === 0 ? (
                <p className="sr__empty">
                    {plan.total === 0
                        ? `This recording's screens look private (mail, messages, passwords or a terminal), so they are never sent to an AI.`
                        : done
                          ? "The screens were read and had nothing to keep."
                          : "Slides, documents, whiteboards and photos from this recording can be read by your AI and kept here as text. Nothing is sent until you choose to."}
                </p>
            ) : (
                <ol className="sr__list">
                    {readings.map((r) => {
                        const p = readingParts(r.text);
                        return (
                            <li key={r.state_id} className="sr__row">
                                <button
                                    type="button"
                                    className="mn-wrote__time"
                                    onClick={() => onJump?.(r.ms)}
                                    disabled={!onJump}
                                    title="See this screen in Rewind"
                                    aria-label={`Open Rewind at ${clock(r.ms)}`}
                                >
                                    {clock(r.ms)}
                                </button>
                                <div className="sr__body">
                                    <p className="sr__title">
                                        <span className="sr__kind">{p.kind}</span>
                                        {p.title && <strong>{p.title}</strong>}
                                    </p>
                                    {p.points.length > 0 && (
                                        <ul>
                                            {p.points.map((x, i) => (
                                                <li key={i}>{x}</li>
                                            ))}
                                        </ul>
                                    )}
                                    {p.terms.length > 0 && (
                                        <ul className="sr__terms" aria-label="Terms">
                                            {p.terms.map((x, i) => (
                                                <li key={i}>{x}</li>
                                            ))}
                                        </ul>
                                    )}
                                    {p.exact && <p className="sr__exact">{p.exact}</p>}
                                </div>
                            </li>
                        );
                    })}
                </ol>
            )}
            {message && (
                <p className={message.error ? "mn-error" : "sr__status"} role={message.error ? "alert" : "status"}>
                    {message.text}
                </p>
            )}
            {running ? (
                <div className="sr__running">
                    <p className="sr__status" role="status" aria-live="polite">
                        {stopping
                            ? "Stopping after this picture…"
                            : running.total > 0
                              ? `Reading picture ${Math.min(running.done + 1, running.total)} of ${running.total}…`
                              : "Reading the screens…"}
                    </p>
                    <button type="button" className="mn-btn" onClick={stop} disabled={stopping}>
                        Stop
                    </button>
                </div>
            ) : (
                !done &&
                plan.total > 0 && (
                    <div className="mn-actions">
                        <button ref={opener} className="mn-btn" onClick={open}>
                            {`Read the screens…${proNote}`}
                        </button>
                    </div>
                )
            )}

            {confirming && (
                <div className="modal-overlay" role="dialog" aria-modal="true" aria-labelledby="sr-confirm-title" ref={dialog}>
                    <div className="modal-content sr-confirm">
                        <div className="modal-header">
                            <h2 id="sr-confirm-title">{plan.blocked ? "The screens can't be read yet" : confirmTitle(count, plan)}</h2>
                        </div>
                        <div className="modal-body">
                            {plan.blocked ? (
                                <p className="sr-confirm__copy">{plan.blocked}</p>
                            ) : (
                                <>
                                    <p className="sr-confirm__copy">
                                        These are the pictures that would go, made smaller.{" "}
                                        {plan.model ? <strong style={{ overflowWrap: "anywhere" }}>{plan.model}</strong> : "Your AI"} reads each
                                        one, and noFriction keeps what it says as text beside the screen. Untick any you don't want sent.
                                    </p>
                                    <ul className="sr-confirm__grid" aria-label="The pictures that will be sent">
                                        {plan.send.map((s) => {
                                            const on = ticked.has(s.state_id);
                                            const what = `${s.kind} at ${clock(s.ms)}${s.app_name ? `, ${s.app_name}` : ""}`;
                                            return (
                                                <li key={s.state_id} className={on ? "sr-pick" : "sr-pick is-off"}>
                                                    <label>
                                                        <img src={convertFileSrc(s.image_path)} alt="" loading="lazy" />
                                                        <span className="sr-pick__line">
                                                            <input
                                                                type="checkbox"
                                                                checked={on}
                                                                onChange={() => toggle(s.state_id)}
                                                                aria-label={`Send the ${what}`}
                                                            />
                                                            <span className="sr-confirm__time">{clock(s.ms)}</span>
                                                            <span>{s.kind}</span>
                                                        </span>
                                                        {s.app_name && <span className="sr-confirm__app">{s.app_name}</span>}
                                                    </label>
                                                </li>
                                            );
                                        })}
                                    </ul>
                                    <p className="sr-confirm__copy">{confirmScope(plan)}</p>
                                    <p className="sr-confirm__copy">
                                        Only the ticked pictures are sent, each with a fixed instruction and nothing else from the recording,
                                        and only now.
                                        {plan.local ? " This endpoint is on your Mac or your own network." : " Its operator's privacy policy and terms apply."}
                                    </p>
                                </>
                            )}
                            <div className="ai-consent-actions">
                                <button className="btn-secondary" onClick={close} autoFocus>
                                    {plan.blocked ? "Close" : "Cancel"}
                                </button>
                                {!plan.blocked && (
                                    <button className="btn-primary" onClick={send} disabled={count === 0}>
                                        {confirmButton(count)}
                                    </button>
                                )}
                            </div>
                        </div>
                    </div>
                </div>
            )}
        </section>
    );
}
