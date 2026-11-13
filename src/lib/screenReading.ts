// Read the screens (src-tauri/src/screen_reading.rs): the one click that
// sends pictures of the user's screens to their AI. This file holds what
// the confirmation and the list say; the component is ScreenReadings.tsx.

/**
 * `{meetingId, done, total}` while screens are being read, then
 * `{meetingId, finished: true}` once, however the run ended.
 */
export const SCREENS_PROGRESS_EVENT = "screens_progress";

export interface ScreensProgress {
    meetingId: string;
    done?: number;
    total?: number;
    finished?: boolean;
}

/** One picture a click would send */
export interface PlanItem {
    state_id: string;
    ms: number;
    /** "Slide", "Document", "Photo"…: which prompt of the library it gets */
    kind: string;
    app_name: string | null;
    /** The picture's file, shown in the confirmation */
    image_path: string;
    /** A capture of a whole display: it shows everything that was on it */
    whole_display: boolean;
    /** A screenshot from the iPhone */
    iphone_screen: boolean;
}

/** What Read the screens would do right now (looking sends nothing) */
export interface ScreenPlan {
    send: PlanItem[];
    /** Pictures of this recording that may be read (private ones aren't counted) */
    total: number;
    /** Of those, how many the AI has already looked at */
    already: number;
    /** Of those, how many a later click can send */
    more: number;
    /** Pictures that are never sent because they look private */
    left_out: number;
    /** Pictures the AI answered about with nothing to keep (they count as read) */
    skipped: number;
    /** The host the pictures would go to */
    destination: string | null;
    model: string | null;
    /** The endpoint is on this Mac or the local network */
    local: boolean;
    /** Why it can't run now; null when it can */
    blocked: string | null;
    /** A run for this recording is under way */
    running: boolean;
}

export interface StoredReading {
    state_id: string;
    ms: number;
    text: string;
}

export interface ReadSummary {
    read: number;
    skipped: number;
    /** Not sent, or not kept: removed, changed or read meanwhile */
    gone: number;
    /** The answer couldn't be used; still unread */
    failed: number;
    /** Why the run ended early (an AI error or a sentence); null when it didn't */
    stopped: string | null;
    /** The time of the screen it stopped at, when it stopped at one */
    stopped_at_ms: number | null;
    /** The user stopped it */
    cancelled: boolean;
}

/**
 * The error is the app's own "needs consent" or "needs Pro" answer, not text
 * from an endpoint that happens to contain those words: the backend's gate
 * errors start with the marker. Only these may lead to asking and calling
 * again, because calling again sends a picture again.
 */
export function isOwnGateError(e: unknown): boolean {
    const s = e instanceof Error ? e.message : String(e);
    return s.startsWith("CONSENT_REQUIRED:") || s.startsWith("PRO_REQUIRED");
}

/** What the confirmation's button passes back: exactly what the user saw. */
export interface Confirmed {
    state_ids: string[];
    destination: string;
    model: string;
}

const pictures = (n: number) => `${n} ${n === 1 ? "picture" : "pictures"}`;

/** The pictures the user left ticked, in the order shown. */
export function tickedIds(plan: ScreenPlan, ticked: ReadonlySet<string>): string[] {
    return plan.send.filter((s) => ticked.has(s.state_id)).map((s) => s.state_id);
}

/**
 * What to pass back for the ticked pictures. Null when nothing is ticked or
 * the plan names no destination: then there is nothing to confirm.
 */
export function confirmedFor(plan: ScreenPlan, ticked: ReadonlySet<string>): Confirmed | null {
    const state_ids = tickedIds(plan, ticked);
    if (state_ids.length === 0 || plan.blocked || !plan.destination || !plan.model) return null;
    return { state_ids, destination: plan.destination, model: plan.model };
}

/** The confirmation's title: how many, and where to. */
export function confirmTitle(count: number, plan: ScreenPlan): string {
    const where = plan.destination ?? "your AI";
    return count === 0 ? `Send pictures to ${where}?` : `Send ${pictures(count)} to ${where}?`;
}

/** The button that sends them: it says the number again. */
export function confirmButton(count: number): string {
    return count === 0 ? "Nothing chosen" : `Send ${pictures(count)}`;
}

/** Under the title: what is left out, so the number makes sense. */
export function confirmScope(plan: ScreenPlan): string {
    const parts: string[] = [`This recording has ${pictures(plan.total)} that can be read.`];
    if (plan.already > 0) parts.push(`${plan.already} ${plan.already === 1 ? "has" : "have"} been read already.`);
    if (plan.more > 0) parts.push(`${plan.more} more can be read after these: the ones at your marks and notes go first.`);
    if (plan.left_out > 0) parts.push(leftOutLine(plan.left_out));
    return parts.join(" ");
}

/** Where a picture came from, for its tile: the app, or what it is. */
export function pictureSource(item: PlanItem): string | null {
    if (item.app_name) return item.app_name;
    if (item.whole_display) return "Whole display";
    if (item.iphone_screen) return "iPhone screen";
    return null;
}

/**
 * Said when the list has pictures noFriction can't judge by their app: a
 * whole display shows everything that was on it, and a photo anything.
 */
export function lookFirstLine(plan: ScreenPlan): string | null {
    // A whole display or an iPhone screen: everything that was on it
    const screens = plan.send.filter((s) => s.whole_display || s.iphone_screen).length;
    const photos = plan.send.filter((s) => s.kind === "Photo").length;
    if (screens === 0 && photos === 0) return null;
    const what =
        screens > 0 && photos > 0
            ? "A whole screen shows everything that was on it, and a photo can show anything"
            : screens > 0
              ? "A whole screen shows everything that was on it, not one app"
              : "A photo can show anything";
    return `${what}. Open each picture and untick any you wouldn't show your AI.`;
}

/** Pictures that are never sent, and why. */
export function leftOutLine(n: number): string {
    return `${n} ${n === 1 ? "is" : "are"} never sent because ${n === 1 ? "it looks" : "they look"} private (mail, messages, passwords, a terminal, private browsing, or noFriction itself).`;
}

/**
 * Pictures the AI kept nothing from, and the offer to send them again: a
 * model that couldn't read them says "nothing to keep" too.
 */
export function skippedLine(n: number): string {
    return `${n} ${n === 1 ? "picture" : "pictures"} had nothing to keep.`;
}

/** Nothing is left for another click. */
export function nothingLeft(plan: ScreenPlan): boolean {
    return plan.send.length === 0 && plan.more === 0;
}

/** What a run did, in a sentence or two. */
export function summaryLine(s: ReadSummary): string {
    const parts: string[] = [s.read > 0 ? `Read ${pictures(s.read)}.` : "Nothing was kept."];
    if (s.skipped > 0) parts.push(`${s.skipped} had nothing to keep.`);
    if (s.failed > 0) parts.push(`${s.failed} couldn't be read and can be tried again.`);
    if (s.gone > 0) parts.push(`${s.gone} ${s.gone === 1 ? "was" : "were"} removed or changed meanwhile and ${s.gone === 1 ? "wasn't" : "weren't"} kept.`);
    if (s.cancelled) parts.push("You stopped it; no other picture was sent.");
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

/**
 * Keep Tab inside a dialog: the element to focus next, or null to let the
 * browser move focus by itself. `focusable` is the dialog's controls in
 * order; `active` is the one that has focus (or null when focus is outside).
 */
export function trapTab<T>(focusable: readonly T[], active: T | null, backwards: boolean): T | null {
    if (focusable.length === 0) return null;
    const i = active === null ? -1 : focusable.indexOf(active);
    if (i === -1) return backwards ? focusable[focusable.length - 1] : focusable[0];
    if (backwards && i === 0) return focusable[focusable.length - 1];
    if (!backwards && i === focusable.length - 1) return focusable[0];
    return null;
}
