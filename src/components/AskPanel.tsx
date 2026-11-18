// Ask: a question about the recording you are in, answered in the moment
// (src-tauri/src/chat/live.rs). It sits beside the live transcript while a
// recording runs.
//
// Nothing is asked by itself: your AI is called only when you send a
// question. The panel says what it would be given, and every answer cites
// the moments it came from.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { friendlyAiError, isNoProviderError, withAiConsent } from "../lib/ai";
import type { ChatMessage, ChatThread, ChatTurn, Citation } from "../lib/chat";
import { clockOf } from "../lib/chatLogic";
import { askPlaceholder, givenList, knowsLine, suggestions, type Knows } from "../lib/askLogic";
import { ChatMarkdown } from "./chat/ChatMarkdown";
import "./chat/RecordingsChat.css";
import "./AskPanel.css";

interface Props {
    meetingId: string;
}

/** How often what it knows is refreshed while the recording runs */
const REFRESH_MS = 20_000;

export function AskPanel({ meetingId }: Props) {
    const [knows, setKnows] = useState<Knows | null>(null);
    const [thread, setThread] = useState<ChatThread | null>(null);
    const [messages, setMessages] = useState<ChatMessage[]>([]);
    const [input, setInput] = useState("");
    const [asking, setAsking] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [showGiven, setShowGiven] = useState(false);
    /** The source a citation was pressed for, shown under its answer */
    const [shown, setShown] = useState<{ messageId: string; citation: Citation } | null>(null);
    const field = useRef<HTMLInputElement>(null);
    const end = useRef<HTMLDivElement>(null);
    /** The recording this panel shows now: an answer for another one is dropped */
    const current = useRef(meetingId);
    current.current = meetingId;

    /** A question is on its way (read by the refresh, which must not race it) */
    const busy = useRef(false);

    const refresh = useCallback(async () => {
        try {
            const k = await invoke<Knows>("chat_recording_knows", { meetingId });
            if (current.current === meetingId) setKnows(k);
        } catch {
            // The panel still works without the summary line
        }
        // An answer that arrived while this panel was away (another tab, or
        // Stop pressed mid-question) is picked up here
        if (busy.current) return;
        try {
            const d = await invoke<{ thread: ChatThread; messages: ChatMessage[] } | null>("chat_thread_for_recording", { meetingId });
            if (!d || busy.current || current.current !== meetingId) return;
            setThread(d.thread);
            setMessages((prev) => (d.messages.length > prev.length ? d.messages : prev));
        } catch {
            // Nothing to pick up
        }
    }, [meetingId]);

    // Pick the conversation about this recording up where it was left
    useEffect(() => {
        setKnows(null);
        setThread(null);
        setMessages([]);
        setInput("");
        setAsking(null);
        setError(null);
        setShown(null);
        busy.current = false;
        void refresh();
        const timer = window.setInterval(() => void refresh(), REFRESH_MS);
        return () => window.clearInterval(timer);
    }, [meetingId, refresh]);

    // A question asked from the card over another app (⌃⌥⌘A) is part of
    // this conversation: show it as soon as it is answered
    useEffect(() => {
        let off: (() => void) | null = null;
        let disposed = false;
        listen<{ meetingId: string }>("ask_answered", (e) => {
            if (e.payload.meetingId === meetingId) void refresh();
        }).then((fn) => (disposed ? fn() : (off = fn)));
        return () => {
            disposed = true;
            off?.();
        };
    }, [meetingId, refresh]);

    // Command-J puts the keyboard in the question field
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (e.metaKey && !e.shiftKey && !e.altKey && !e.ctrlKey && e.key.toLowerCase() === "j") {
                e.preventDefault();
                field.current?.focus();
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, []);

    useEffect(() => {
        // Before the first question the offered ones are read from the top
        if (messages.length === 0 && !asking) return;
        end.current?.scrollIntoView({ block: "end" });
    }, [messages, asking]);

    const send = async (text: string) => {
        const question = text.trim();
        if (!question || asking) return;
        const forMeeting = meetingId;
        setError(null);
        setShown(null);
        setInput("");
        setAsking(question);
        busy.current = true;
        try {
            const turn = await withAiConsent(
                () => invoke<ChatTurn>("chat_ask_recording", { threadId: thread?.id ?? null, meetingId: forMeeting, message: question }),
                "chat",
            );
            if (current.current !== forMeeting) return;
            setThread(turn.thread);
            setMessages((prev) => [...prev, turn.user, turn.assistant]);
        } catch (e) {
            if (current.current !== forMeeting) return;
            // The question isn't lost: it goes back into the field
            setInput(question);
            setError(
                isNoProviderError(e)
                    ? "Ask needs an AI. Set one up under Settings → AI (Apple on-device works too)."
                    : friendlyAiError(e),
            );
        } finally {
            if (current.current === forMeeting) {
                busy.current = false;
                setAsking(null);
                void refresh();
                // The next question can be typed straight away
                field.current?.focus({ preventScroll: true });
            }
        }
    };

    const offered = messages.length === 0 && !asking ? suggestions(knows) : [];

    return (
        <section className="ask" aria-label="Ask about this recording">
            <header className="ask__head">
                <h3 className="ask__title">Ask</h3>
                <button
                    type="button"
                    className="ask__given-toggle"
                    onClick={() => setShowGiven((v) => !v)}
                    aria-expanded={showGiven}
                    aria-controls="ask-given"
                >
                    {showGiven ? "Hide what it's given" : "What it's given"}
                </button>
            </header>
            <p className="ask__knows">{knows ? knowsLine(knows) : "Ask anything about what is being said."}</p>
            {showGiven && knows && (
                <div className="ask__given" id="ask-given">
                    <p>When you ask, your AI is given this, and nothing is asked by itself:</p>
                    <ul>
                        {givenList(knows).map((line, i) => (
                            <li key={i}>{line}</li>
                        ))}
                    </ul>
                    <p>Text you struck from the record is never included.</p>
                </div>
            )}

            <div className="ask__thread" role="log" aria-live="polite">
                {thread?.flag && <p className="ask__flag">{thread.flag}</p>}
                {offered.length > 0 && (
                    <div className="ask__offers">
                        {offered.map((s) => (
                            <button key={s} type="button" className="ask__offer" onClick={() => void send(s)}>
                                {s}
                            </button>
                        ))}
                    </div>
                )}
                {messages.map((m) =>
                    m.role === "user" ? (
                        <p key={m.id} className="ask__question">
                            {m.content}
                        </p>
                    ) : (
                        <div key={m.id} className="ask__answer rc-msg assistant">
                            <div className="rc-msg__bubble">
                                <ChatMarkdown
                                    content={m.content}
                                    citations={m.citations}
                                    onCite={(c) => setShown((cur) => (cur?.messageId === m.id && cur.citation.n === c.n ? null : { messageId: m.id, citation: c }))}
                                />
                            </div>
                            {shown?.messageId === m.id && (
                                <p className="ask__source">
                                    <span className="ask__source-where">
                                        {shown.citation.meeting_id === meetingId ? "" : `${shown.citation.title} · `}
                                        {shown.citation.timestamp_ms != null
                                            ? clockOf(shown.citation.timestamp_ms)
                                            : shown.citation.source === "notes"
                                              ? "notes"
                                              : ""}
                                    </span>
                                    {shown.citation.excerpt}
                                </p>
                            )}
                        </div>
                    ),
                )}
                {asking && (
                    <>
                        <p className="ask__question">{asking}</p>
                        <div className="rc-thinking" role="status">
                            <div className="rc-thinking__dots">
                                <span />
                                <span />
                                <span />
                            </div>
                            <span className="rc-thinking__label">Asking your AI…</span>
                        </div>
                    </>
                )}
                {error && (
                    <p className="ask__error" role="alert">
                        {error}
                    </p>
                )}
                <div ref={end} />
            </div>

            <form
                className="ask__input"
                onSubmit={(e) => {
                    e.preventDefault();
                    void send(input);
                }}
            >
                <input
                    ref={field}
                    type="text"
                    className="ask__field"
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    placeholder={askPlaceholder(knows)}
                    aria-label="Your question about this recording"
                    readOnly={asking !== null}
                    aria-busy={asking !== null}
                    maxLength={600}
                />
                <button type="submit" className="ask__send" disabled={!input.trim() || asking !== null} aria-label="Ask">
                    Ask
                </button>
            </form>
            <p className="ask__hint">
                <kbd>⌘J</kbd> here · <kbd>⌃⌥⌘A</kbd> in any app · sent only when you ask
            </p>
        </section>
    );
}
