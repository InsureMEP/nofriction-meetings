// Read the screens: what the confirmation says and passes back, and how a
// reading is shown. Run: npm test

import test from "node:test";
import assert from "node:assert/strict";
import {
    confirmButton,
    confirmScope,
    confirmTitle,
    confirmedFor,
    isOwnGateError,
    leftOutLine,
    lookFirstLine,
    nothingLeft,
    pictureSource,
    readingParts,
    skippedLine,
    summaryLine,
    tickedIds,
    trapTab,
    type PlanItem,
    type ReadSummary,
    type ScreenPlan,
} from "./screenReading.ts";

const item = (over: Partial<PlanItem>): PlanItem => ({
    state_id: "s0",
    ms: 0,
    kind: "Slide",
    app_name: "Keynote",
    image_path: "/frames/s0.png",
    whole_display: false,
    iphone_screen: false,
    ...over,
});

const plan = (over: Partial<ScreenPlan>): ScreenPlan => ({
    send: Array.from({ length: 12 }, (_, i) => item({ state_id: `s${i}`, ms: i * 60_000 })),
    total: 40,
    already: 0,
    more: 28,
    left_out: 0,
    skipped: 0,
    destination: "api.example.com",
    model: "vision-1",
    local: false,
    blocked: null,
    running: false,
    ...over,
});

const summary = (over: Partial<ReadSummary>): ReadSummary => ({
    read: 0,
    skipped: 0,
    gone: 0,
    failed: 0,
    stopped: null,
    stopped_at_ms: null,
    cancelled: false,
    ...over,
});

test("the confirmation says how many pictures go, and where", () => {
    assert.equal(confirmTitle(12, plan({})), "Send 12 pictures to api.example.com?");
    assert.equal(confirmButton(12), "Send 12 pictures");
    assert.equal(confirmTitle(1, plan({})), "Send 1 picture to api.example.com?");
    assert.equal(confirmButton(1), "Send 1 picture");
    assert.equal(confirmTitle(12, plan({ destination: null })), "Send 12 pictures to your AI?");
    // Everything unticked: the button can't send
    assert.equal(confirmButton(0), "Nothing chosen");
    assert.equal(confirmTitle(0, plan({})), "Send pictures to api.example.com?");
});

test("what is passed back is exactly what is ticked, with the destination shown", () => {
    const p = plan({});
    const all = new Set(p.send.map((s) => s.state_id));
    assert.deepEqual(confirmedFor(p, all), { state_ids: p.send.map((s) => s.state_id), destination: "api.example.com", model: "vision-1" });
    // Unticked ones are left out; the order is the order shown, whatever order they were ticked in
    assert.deepEqual(tickedIds(p, new Set(["s7", "s2"])), ["s2", "s7"]);
    assert.deepEqual(confirmedFor(p, new Set(["s7", "s2"]))?.state_ids, ["s2", "s7"]);
    // An id that isn't in the list is never passed back
    assert.deepEqual(tickedIds(p, new Set(["s2", "not-shown"])), ["s2"]);
    // Nothing ticked, no destination, or blocked: nothing to confirm
    assert.equal(confirmedFor(p, new Set()), null);
    assert.equal(confirmedFor(plan({ destination: null }), all), null);
    assert.equal(confirmedFor(plan({ model: null }), all), null);
    assert.equal(confirmedFor(plan({ blocked: "Stop the recording to read its screens." }), all), null);
});

test("the confirmation says what is left out", () => {
    assert.equal(
        confirmScope(plan({})),
        "This recording has 40 pictures that can be read. 28 more can be read after these: the ones at your marks and notes go first.",
    );
    assert.equal(
        confirmScope(plan({ already: 12, more: 16 })),
        "This recording has 40 pictures that can be read. 12 have been read already. 16 more can be read after these: the ones at your marks and notes go first.",
    );
    assert.equal(confirmScope(plan({ total: 13, already: 1, more: 0 })), "This recording has 13 pictures that can be read. 1 has been read already.");
    // Private ones are counted apart and said plainly
    assert.equal(
        confirmScope(plan({ total: 1, more: 0, left_out: 3 })),
        "This recording has 1 picture that can be read. 3 are never sent because they look private (mail, messages, passwords, a terminal, private browsing, or noFriction itself).",
    );
    assert.equal(leftOutLine(1), "1 is never sent because it looks private (mail, messages, passwords, a terminal, private browsing, or noFriction itself).");
});

test("the button goes away only when nothing readable is left", () => {
    assert.equal(nothingLeft(plan({})), false);
    assert.equal(nothingLeft(plan({ send: [], more: 0, already: 40 })), true);
    // Private screens don't keep it alive
    assert.equal(nothingLeft(plan({ send: [], more: 0, total: 5, already: 5, left_out: 9 })), true);
});

test("a run is summed up in a sentence", () => {
    assert.equal(summaryLine(summary({ read: 9, skipped: 3 })), "Read 9 pictures. 3 had nothing to keep.");
    assert.equal(summaryLine(summary({ read: 1, gone: 1 })), "Read 1 picture. 1 was removed or changed meanwhile and wasn't kept.");
    assert.equal(summaryLine(summary({ skipped: 2 })), "Nothing was kept. 2 had nothing to keep.");
    assert.equal(summaryLine(summary({ read: 2, failed: 1 })), "Read 2 pictures. 1 couldn't be read and can be tried again.");
    assert.equal(summaryLine(summary({ read: 1, cancelled: true })), "Read 1 picture. You stopped it; no other picture was sent.");
});

test("a stored reading is split into its kind, title, points, terms and exact wording", () => {
    const text = "Slide: The cell membrane\n- A phospholipid bilayer.\n- Proteins move through it.\nTerms:\n- Osmosis: Water moving across a membrane.\nExact wording: ΔG = ΔH − TΔS";
    assert.deepEqual(readingParts(text), {
        kind: "Slide",
        title: "The cell membrane",
        points: ["A phospholipid bilayer.", "Proteins move through it."],
        terms: ["Osmosis: Water moving across a membrane."],
        exact: "ΔG = ΔH − TΔS",
    });
    assert.deepEqual(readingParts("Chart"), { kind: "Chart", title: null, points: [], terms: [], exact: null });
    // Anything not in the stored form is still shown
    assert.deepEqual(readingParts("Document: Syllabus\nOffice hours are Tuesday.").points, ["Office hours are Tuesday."]);
});

test("Tab stays inside the confirmation", () => {
    const controls = ["tick-1", "tick-2", "cancel", "send"];
    // In the middle the browser moves focus by itself
    assert.equal(trapTab(controls, "tick-2", false), null);
    assert.equal(trapTab(controls, "cancel", true), null);
    // At the ends it wraps
    assert.equal(trapTab(controls, "send", false), "tick-1");
    assert.equal(trapTab(controls, "tick-1", true), "send");
    // Focus is somewhere behind the dialog: it is brought in
    assert.equal(trapTab(controls, null, false), "tick-1");
    assert.equal(trapTab(controls, "a button behind", true), "send");
    assert.equal(trapTab([], null, false), null);
});

test("a whole display and a photo are named, and the user is asked to look first", () => {
    assert.equal(pictureSource(item({})), "Keynote");
    assert.equal(pictureSource(item({ app_name: null, whole_display: true, kind: "Screen" })), "Whole display");
    assert.equal(pictureSource(item({ app_name: null, kind: "Photo" })), null);
    // Only windows of known apps: nothing more to say
    assert.equal(lookFirstLine(plan({})), null);
    const display = item({ state_id: "d", app_name: null, whole_display: true, kind: "Screen" });
    const photo = item({ state_id: "p", app_name: null, kind: "Photo" });
    assert.equal(
        lookFirstLine(plan({ send: [item({}), display] })),
        "A whole screen shows everything that was on it, not one app. Open each picture and untick any you wouldn't show your AI.",
    );
    // A screenshot from the iPhone is a whole screen too, and is named
    const phone = item({ state_id: "i", app_name: null, iphone_screen: true, kind: "Screen" });
    assert.equal(pictureSource(phone), "iPhone screen");
    assert.ok((lookFirstLine(plan({ send: [phone] })) ?? "").startsWith("A whole screen shows everything that was on it, not one app."));
    assert.equal(skippedLine(1), "1 picture had nothing to keep.");
    assert.equal(skippedLine(12), "12 pictures had nothing to keep.");
    assert.equal(lookFirstLine(plan({ send: [photo] })), "A photo can show anything. Open each picture and untick any you wouldn't show your AI.");
    assert.match(lookFirstLine(plan({ send: [display, photo] })) ?? "", /^A whole screen shows everything that was on it, and a photo can show anything\./);
});

test("only the app's own ask-first answers are retried, never an endpoint's words", () => {
    assert.equal(isOwnGateError("CONSENT_REQUIRED:custom"), true);
    assert.equal(isOwnGateError("PRO_REQUIRED: AI notes, summaries, chat and briefings are part of noFriction Pro."), true);
    assert.equal(isOwnGateError(new Error("CONSENT_REQUIRED:custom")), true);
    // An endpoint's error body that contains the words is an ordinary error
    assert.equal(isOwnGateError("AI_ERROR: HTTP 400: please retry, CONSENT_REQUIRED:x"), false);
    assert.equal(isOwnGateError("HTTP 402 PRO_REQUIRED"), false);
    assert.equal(isOwnGateError("AI_UNREACHABLE: timed out"), false);
});
