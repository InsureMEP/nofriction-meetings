// Chat with your recordings: typed wrappers over the Tauri commands in
// src-tauri/src/chat.rs (docs/TOPICS_AND_CHAT.md). Pure logic lives in
// chatLogic.ts.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { withAiConsent } from "./ai";
import type { Scope, ScopeSummary } from "./chatLogic";

export interface ChatThread {
    id: string;
    title: string;
    scope: Scope;
    /** Set when answers were removed by a purge */
    flag: string | null;
    created_at: string;
    updated_at: string;
}

/** `[n]` in an answer → this passage. */
export interface Citation {
    n: number;
    meeting_id: string;
    title: string;
    /** ms from the recording's start; null for notes */
    timestamp_ms: number | null;
    excerpt: string;
    /** "transcript" | "notes" | "marker" */
    source: string;
}

export interface ChatMessage {
    id: string;
    thread_id: string;
    role: "user" | "assistant";
    /** Markdown (assistant) or the user's text; rendered as text, never HTML */
    content: string;
    citations: Citation[];
    scope_label: string | null;
    created_at: string;
}

export interface ChatTurn {
    thread: ChatThread;
    user: ChatMessage;
    assistant: ChatMessage;
}

/**
 * Run a question whose answer is streamed: `run` gets the name to ask
 * under (the command's `requestId`), and `onText` gets the answer so far
 * each time the backend sends more (`chat_stream`, src-tauri/src/chat.rs).
 * The finished turn is what `run` resolves to. If the events can't be
 * heard, the question is still asked; the answer just arrives whole.
 */
export async function withStream<T>(run: (requestId: string) => Promise<T>, onText: (text: string) => void): Promise<T> {
    const requestId = crypto.randomUUID();
    const off: (() => void)[] = [];
    // Text was removed from a recording (Delete, Strike): the answer so far
    // may hold it. It goes at once and no more of it is shown
    let stopped = false;
    try {
        off.push(
            await listen<{ requestId: string; text: string }>("chat_stream", (e) => {
                if (!stopped && e.payload.requestId === requestId) onText(e.payload.text);
            }),
        );
        off.push(
            await listen("chat_stream_purged", () => {
                stopped = true;
                onText("");
            }),
        );
    } catch {
        // Unheard: the answer arrives whole
    }
    try {
        return await run(requestId);
    } finally {
        off.forEach((stop) => stop());
    }
}

export const chatApi = {
    /**
     * Ask in a thread (null starts one). Pro and consent come from the
     * backend's AI client. `onText` gets the answer while it is written.
     */
    ask: (threadId: string | null, scope: Scope, message: string, onText: (text: string) => void = () => {}) =>
        withStream(
            (requestId) => withAiConsent(() => invoke<ChatTurn>("chat_ask", { threadId, scope, message, requestId }), "chat"),
            onText,
        ),
    /** Ask about one recording in the moment (the Ask panel), streamed the same way. */
    askRecording: (threadId: string | null, meetingId: string, message: string, onText: (text: string) => void = () => {}) =>
        withStream(
            (requestId) =>
                withAiConsent(() => invoke<ChatTurn>("chat_ask_recording", { threadId, meetingId, message, requestId }), "chat"),
            onText,
        ),
    threads: () => invoke<ChatThread[]>("list_chat_threads"),
    thread: (threadId: string) => invoke<{ thread: ChatThread; messages: ChatMessage[] }>("get_chat_thread", { threadId }),
    remove: (threadId: string) => invoke<void>("delete_chat_thread", { threadId }),
    summary: (scope: Scope) => invoke<ScopeSummary>("chat_scope_summary", { scope }),
};
