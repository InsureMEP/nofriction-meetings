// Ask (src-tauri/src/chat/live.rs): a question about the recording you are
// in. This file is what the panel says about what your AI is given, and the
// questions it offers. No AI is called from here.

/** Someone in the recording's calendar event */
export interface Who {
    name: string;
    /** The company, or the mail domain when no company is known */
    company: string | null;
    /** "organizer" when they called it */
    role: string | null;
    /** This is you */
    you: boolean;
}

/** What a question about this recording would draw on right now */
export interface Knows {
    meeting_id: string;
    title: string;
    /** "meeting" | "class" | "personal" */
    kind: string;
    notebook: string | null;
    /** Still being recorded */
    recording: boolean;
    /** Transcript lines so far */
    lines: number;
    /** Lines that say which side spoke (your microphone, or the computer's audio) */
    tagged: number;
    people: Who[];
    event: string | null;
    /** The event has a place or notes that go with a question */
    event_details: boolean;
    /** Notes were already made from this recording */
    saved_notes: boolean;
    /** Your typed notes and marks */
    notes: number;
    /** Earlier recordings it can draw on (same Notebook or people) */
    related: number;
}

const n = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

/** The others in the recording, and their companies, in a few words. */
export function peopleLine(people: Who[]): string | null {
    const others = people.filter((p) => !p.you);
    if (others.length === 0) return null;
    const companies = [...new Set(others.map((p) => p.company).filter((c): c is string => !!c))];
    const names = others.slice(0, 3).map((p) => p.name);
    const more = others.length - names.length;
    const who = names.join(", ") + (more > 0 ? ` and ${more} more` : "");
    return companies.length > 0 ? `${who} (${companies.slice(0, 2).join(", ")})` : who;
}

/** One line under the panel's title: what it knows right now. */
export function knowsLine(k: Knows): string {
    const parts: string[] = [];
    parts.push(k.lines === 0 ? "nothing said yet" : k.recording ? "what has been said so far" : "what was said");
    if (k.notes > 0) parts.push(`your ${n(k.notes, "note or mark", "notes and marks")}`);
    const people = peopleLine(k.people);
    if (people) parts.push(`who is here: ${people}`);
    if (k.related > 0) parts.push(n(k.related, "earlier recording", "earlier recordings"));
    return `Knows ${parts.join("; ")}.`;
}

/** Everything your AI is given when you ask, said plainly. */
export function givenList(k: Knows): string[] {
    const out: string[] = [];
    out.push(
        `The recording: "${k.title}", its type${k.notebook ? `, its Notebook (${k.notebook})` : ""}, when it started${k.recording ? " and that it is still running" : ""}.`,
    );
    out.push(
        k.lines === 0
            ? "The transcript, once something has been said."
            : `The transcript: the last few minutes in order, and earlier lines that match your question (${n(k.lines, "line", "lines")} so far).`,
    );
    out.push(
        k.tagged > 0
            ? "Which side spoke each line, when it could be told: your microphone, or the computer's audio (the other side of a call). Never a name: noFriction doesn't recognise voices."
            : "Not who spoke: noFriction doesn't recognise voices. On a call it can tell your microphone from the computer's audio.",
    );
    if (k.notes > 0) out.push(`Your ${n(k.notes, "typed note or mark", "typed notes and marks")} in this recording, with their times.`);
    if (k.saved_notes) out.push("The notes already made from this recording: their summary, and the decisions, action items and points that match your question.");
    if (k.event || k.event_details) {
        out.push(
            `The calendar event${k.event ? ` "${k.event}"` : ""}: its title${k.event_details ? ", its place and its notes as they are in your calendar (a meeting link or dial-in written there goes too)" : ""}.`,
        );
    }
    if (k.people.length > 0) {
        out.push("The people in the event: names, companies, who organised it, which one is you, and how many of your recordings each is in. Not their email addresses.");
    } else if (!k.event && !k.event_details) {
        out.push("No people or event: this recording isn't linked to a calendar event.");
    }
    if (k.related > 0) {
        out.push(`The saved notes of up to 3 earlier recordings in the same Notebook or with the same people, and lines from them that match your question (${n(k.related, "recording", "recordings")} to draw on).`);
    }
    return out;
}

/** Questions offered before the first one is asked. */
export function suggestions(k: Knows | null): string[] {
    if (!k) return [];
    const other = k.people.find((p) => !p.you);
    if (k.recording) {
        const out =
            k.kind === "class"
                ? ["What did I miss?", "Explain the last concept simply", "Sum up the last few minutes", "What here is likely to be on the test?"]
                : ["What did I miss?", "Sum up the last few minutes", "What was just asked of me?", "Explain the last thing they mentioned", "What should I ask next?"];
        if (other && k.related > 0) out.push(`What did we agree with ${other.name} last time?`);
        return out;
    }
    const out =
        k.kind === "class"
            ? ["What were the key ideas?", "What should I review first?", "What was I told to do before next class?"]
            : ["What was decided?", "What did I say I would do?", "What is still open?"];
    if (other) out.push(`What did ${other.name} want?`);
    return out;
}

/** The placeholder of the question field. */
export function askPlaceholder(k: Knows | null): string {
    if (!k || k.recording) return "Ask about what's being said";
    return "Ask about this recording";
}
