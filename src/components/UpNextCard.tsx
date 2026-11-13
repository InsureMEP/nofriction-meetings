// Up next, on the idle Record screen: one card when something is starting
// (a calendar event, a notebook you usually record about now, a call app
// on the microphone), with one button that has the answer on it. No sheet:
// the type, length and notebook are already filled in.
// docs/design/FADELL_V2.md §3 A

import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { CalendarIcon, ClockIcon, MicIcon } from "./icons";
import { UP_NEXT_EVENT, upNextApi } from "../lib/upNext";
import { planFor, planSummary, recordLabel, sourceLabel, type UpNext } from "../lib/upNextLogic";
import type { StartPlan } from "../lib/recordPlan";
import "./UpNextCard.css";

/** What to record now, kept fresh while the window is open. */
export function useUpNext(): { upNext: UpNext | null; dismiss: () => void } {
    const [upNext, setUpNext] = useState<UpNext | null>(null);
    const load = useCallback(() => {
        upNextApi
            .get()
            .then(setUpNext)
            .catch(() => setUpNext(null));
    }, []);
    useEffect(() => {
        load();
        let off: (() => void) | undefined;
        let disposed = false;
        listen<UpNext | null>(UP_NEXT_EVENT, (e) => setUpNext(e.payload ?? null)).then((fn) => (disposed ? fn() : (off = fn)));
        // Coming back to the window: the hour may have changed
        window.addEventListener("focus", load);
        return () => {
            disposed = true;
            off?.();
            window.removeEventListener("focus", load);
        };
    }, [load]);
    const dismiss = useCallback(() => {
        const key = upNext?.key;
        if (!key) return;
        setUpNext(null);
        upNextApi.dismiss(key).catch(() => load());
    }, [upNext, load]);
    return { upNext, dismiss };
}

interface UpNextCardProps {
    upNext: UpNext;
    onStart: (plan: StartPlan) => Promise<void>;
    onDismiss: () => void;
}

export function UpNextCard({ upNext, onStart, onDismiss }: UpNextCardProps) {
    const [starting, setStarting] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const Icon = upNext.source === "calendar" ? CalendarIcon : upNext.source === "call" ? MicIcon : ClockIcon;

    const start = async () => {
        if (starting) return;
        setStarting(true);
        setError(null);
        try {
            await onStart(planFor(upNext));
        } catch (e) {
            setError(String(e));
            setStarting(false);
        }
    };

    return (
        <section className="upnext" aria-label="Up next">
            <header className="upnext__head">
                <Icon size={14} strokeWidth={1.75} />
                <span>{sourceLabel(upNext.source)}</span>
            </header>
            <h2 className="upnext__title">{upNext.source === "call" ? upNext.detail : upNext.title}</h2>
            {upNext.source !== "call" && <p className="upnext__detail">{upNext.detail}</p>}
            <p className="upnext__plan">{planSummary(upNext)}</p>
            {error && (
                <p className="upnext__error" role="alert">
                    Couldn't start: {error}
                </p>
            )}
            <div className="upnext__actions">
                <button type="button" className="upnext__go" onClick={() => void start()} disabled={starting}>
                    <span className="upnext__dot" aria-hidden />
                    {starting ? "Starting…" : recordLabel(upNext)}
                </button>
                <button type="button" className="upnext__later" onClick={onDismiss} disabled={starting}>
                    Not now
                </button>
            </div>
        </section>
    );
}
