// On the screens: what the user's AI read from this recording's screens, and
// the one button that sends them (src-tauri/src/screen_reading.rs).
//
// A picture of a screen leaves this Mac only from here, and only after the
// confirmation below, which says how many screens go and where. The number
// confirmed is passed back, and the Mac refuses to send a different one.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
    readingParts,
    summaryLine,
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
            return "Reading screens needs your permission to send to your AI endpoint. Try again and choose Allow.";
        case "vision_unavailable":
            return "Reading a screen needs a model that can read images. Choose one under Settings → AI → Screens.";
        default:
            return friendlyAiError(e);
    }
}

interface Props {
    meetingId: string;
    /** Open Rewind at a moment (when the screen was shown) */
    onJump?: (ms: number) => void;
}

export function ScreenReadings({ meetingId, onJump }: Props) {
    const [plan, setPlan] = useState<ScreenPlan | null>(null);
    const [readings, setReadings] = useState<StoredReading[]>([]);
    const [confirming, setConfirming] = useState(false);
    const [running, setRunning] = useState<{ done: number; total: number } | null>(null);
    const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
    const caps = useCapabilities();
    const isPro = useIsPro();
    const opener = useRef<HTMLButtonElement>(null);

    // Only the latest read is shown
    const request = useRef(0);
    const load = useCallback(async () => {
        const mine = ++request.current;
        try {
            const [p, r] = await Promise.all([
                invoke<ScreenPlan>("plan_screen_reading", { meetingId }),
                invoke<StoredReading[]>("get_screen_readings", { meetingId }),
            ]);
            if (mine !== request.current) return;
            setPlan(p);
            setReadings(r);
        } catch {
            // Without a plan there is nothing to offer; the notes above still show
            if (mine === request.current) setPlan(null);
        }
    }, [meetingId]);

    useEffect(() => {
        setPlan(null);
        setReadings([]);
        setConfirming(false);
        setRunning(null);
        setMessage(null);
        void load();
    }, [load]);

    useEffect(() => {
        let off: (() => void) | null = null;
        let disposed = false;
        listen<{ meetingId: string; done: number; total: number }>(SCREENS_PROGRESS_EVENT, (e) => {
            if (e.payload.meetingId === meetingId) setRunning((r) => (r ? { done: e.payload.done, total: e.payload.total } : r));
        }).then((fn) => (disposed ? fn() : (off = fn)));
        return () => {
            disposed = true;
            off?.();
        };
    }, [meetingId]);

    // The confirmation closes on Escape and gives the keyboard back to its button
    const close = useCallback(() => {
        setConfirming(false);
        opener.current?.focus({ preventScroll: true });
    }, []);
    useEffect(() => {
        if (!confirming) return;
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") close();
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [confirming, close]);

    // A recording without screens has nothing to read
    if (!plan || plan.total === 0) return null;

    const open = async () => {
        setMessage(null);
        // What would go is worked out again at the moment of asking
        await load();
        setConfirming(true);
    };

    const send = async () => {
        const expect = plan.send.length;
        setConfirming(false);
        setRunning({ done: 0, total: expect });
        try {
            const summary = await withAiConsent(() => invoke<ReadSummary>("read_screens", { meetingId, expect }), "notes");
            setMessage({ text: summaryLine(summary), error: false });
        } catch (e) {
            setMessage({ text: failure(e), error: true });
        } finally {
            setRunning(null);
            await load();
        }
    };

    const allRead = plan.send.length === 0 && plan.already >= plan.total;
    const proNote = proSuffix(caps?.pro_gating, isPro);

    return (
        <section className="mn-section sr" aria-label="On the screens">
            <h4>On the screens</h4>
            {readings.length === 0 ? (
                <p className="sr__empty">
                    {allRead
                        ? "The screens were read and had nothing to keep."
                        : "Slides, documents and whiteboards that were on screen can be read by your AI and kept here as text. Nothing is sent until you choose to."}
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
                <p className="sr__status" role="status" aria-live="polite">
                    Reading screen {Math.min(running.done + 1, running.total)} of {running.total}…
                </p>
            ) : (
                !allRead && (
                    <div className="mn-actions">
                        <button ref={opener} className="mn-btn" onClick={open}>
                            {`Read the screens…${proNote}`}
                        </button>
                    </div>
                )
            )}

            {confirming && (
                <div className="modal-overlay" role="dialog" aria-modal="true" aria-labelledby="sr-confirm-title">
                    <div className="modal-content sr-confirm">
                        <div className="modal-header">
                            <h2 id="sr-confirm-title">{plan.blocked ? "The screens can't be read yet" : confirmTitle(plan)}</h2>
                        </div>
                        <div className="modal-body">
                            {plan.blocked ? (
                                <p className="sr-confirm__copy">{plan.blocked}</p>
                            ) : (
                                <>
                                    <p className="sr-confirm__copy">
                                        Each one is a picture of what was on your screen at that moment, made smaller.{" "}
                                        {plan.model ? <strong style={{ overflowWrap: "anywhere" }}>{plan.model}</strong> : "Your AI"} reads it, and
                                        noFriction keeps what it says as text beside the screen. {confirmScope(plan)}
                                    </p>
                                    <ul className="sr-confirm__list" aria-label="The screens that will be sent">
                                        {plan.send.map((s) => (
                                            <li key={s.state_id}>
                                                <span className="sr-confirm__time">{clock(s.ms)}</span>
                                                <span>{s.kind}</span>
                                                {s.app_name && <span className="sr-confirm__app">{s.app_name}</span>}
                                            </li>
                                        ))}
                                    </ul>
                                    <p className="sr-confirm__copy">
                                        Only these pictures are sent, with what was being said then, and only now. A screen is never sent
                                        unless you press the button below.
                                        {plan.local ? " This endpoint is on your Mac or your own network." : " Its operator's privacy policy and terms apply."}
                                    </p>
                                </>
                            )}
                            <div className="ai-consent-actions">
                                <button className="btn-secondary" onClick={close} autoFocus>
                                    {plan.blocked ? "Close" : "Cancel"}
                                </button>
                                {!plan.blocked && (
                                    <button className="btn-primary" onClick={send} disabled={plan.send.length === 0}>
                                        {confirmButton(plan)}
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
