// Read the screens (src-tauri/src/screen_reading.rs): the one click that
// sends pictures of the user's screens to their AI. This file holds what
// the confirmation and the list say; the component is ScreenReadings.tsx.

/** `{meetingId, done, total}` while screens are being read */
export const SCREENS_PROGRESS_EVENT = "screens_progress";

/** One screen a click would send */
export interface PlanItem {
    state_id: string;
    ms: number;
    /** "Slide", "Document"…: which prompt of the library it gets */
    kind: string;
    app_name: string | null;
}

/** What Read the screens would do right now (looking sends nothing) */
export interface ScreenPlan {
    send: PlanItem[];
    /** Screens of this recording that have a picture */
    total: number;
    /** Of those, how many the AI has already looked at */
    already: number;
    /** The host the pictures would go to */
    destination: string | null;
    model: string | null;
    /** The endpoint is on this Mac or the local network */
    local: boolean;
    /** Why it can't run now; null when it can */
    blocked: string | null;
}

export interface StoredReading {
    state_id: string;
    ms: number;
    text: string;
}

export interface ReadSummary {
    read: number;
    skipped: number;
    gone: number;
}

const screens = (n: number) => `${n} ${n === 1 ? "screen" : "screens"}`;

/** The confirmation's title: how many, and where to. */
export function confirmTitle(plan: ScreenPlan): string {
    const where = plan.destination ?? "your AI";
    return `Send ${screens(plan.send.length)} to ${where}?`;
}

/** The button that sends them: it says the number again. */
export function confirmButton(plan: ScreenPlan): string {
    return `Send ${screens(plan.send.length)}`;
}

/** Under the title: what is left out, so the number makes sense. */
export function confirmScope(plan: ScreenPlan): string {
    const rest = plan.total - plan.already - plan.send.length;
    const parts: string[] = [`This recording has ${screens(plan.total)}.`];
    if (plan.already > 0) parts.push(`${plan.already} ${plan.already === 1 ? "has" : "have"} been read already.`);
    if (rest > 0) parts.push(`${rest} more can be read after these: the ones at your marks and notes go first.`);
    return parts.join(" ");
}

/** What a run did, in a sentence. */
export function summaryLine(s: ReadSummary): string {
    const parts: string[] = [s.read > 0 ? `Read ${screens(s.read)}.` : "Nothing was kept."];
    if (s.skipped > 0) parts.push(`${s.skipped} had nothing to keep.`);
    if (s.gone > 0) parts.push(`${s.gone} ${s.gone === 1 ? "was" : "were"} removed meanwhile.`);
    return parts.join(" ");
}

/** A stored reading, split for display: "Slide: Title", then its lines. */
export interface ReadingParts {
    /** "Slide" */
    kind: string;
    /** "The cell membrane", when the screen had a title */
    title: string | null;
    points: string[];
    terms: string[];
    /** Wording kept exactly: a formula, a few lines of code */
    exact: string | null;
}

export function readingParts(text: string): ReadingParts {
    const lines = text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
    const first = lines.shift() ?? "";
    const colon = first.indexOf(":");
    const parts: ReadingParts = {
        kind: (colon === -1 ? first : first.slice(0, colon)).trim(),
        title: colon === -1 ? null : first.slice(colon + 1).trim() || null,
        points: [],
        terms: [],
        exact: null,
    };
    let inTerms = false;
    for (const line of lines) {
        if (line === "Terms:") {
            inTerms = true;
        } else if (line.startsWith("Exact wording:")) {
            parts.exact = line.slice("Exact wording:".length).trim() || null;
            inTerms = false;
        } else if (line.startsWith("- ")) {
            (inTerms ? parts.terms : parts.points).push(line.slice(2).trim());
        } else {
            // A line that isn't in the stored form is still shown
            parts.points.push(line);
        }
    }
    return parts;
}
