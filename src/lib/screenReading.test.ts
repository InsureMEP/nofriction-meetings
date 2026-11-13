// Read the screens: what the confirmation says and how a reading is shown. Run: npm test

import test from "node:test";
import assert from "node:assert/strict";
import { confirmButton, confirmScope, confirmTitle, readingParts, summaryLine, type ScreenPlan } from "./screenReading.ts";

const plan = (over: Partial<ScreenPlan>): ScreenPlan => ({
    send: Array.from({ length: 12 }, (_, i) => ({ state_id: `s${i}`, ms: i * 60_000, kind: "Slide", app_name: "Keynote" })),
    total: 40,
    already: 0,
    destination: "api.example.com",
    model: "vision-1",
    local: false,
    blocked: null,
    ...over,
});

test("the confirmation says how many screens go, and where", () => {
    assert.equal(confirmTitle(plan({})), "Send 12 screens to api.example.com?");
    assert.equal(confirmButton(plan({})), "Send 12 screens");
    const one = plan({ send: plan({}).send.slice(0, 1) });
    assert.equal(confirmTitle(one), "Send 1 screen to api.example.com?");
    assert.equal(confirmButton(one), "Send 1 screen");
    assert.equal(confirmTitle(plan({ destination: null })), "Send 12 screens to your AI?");
});

test("the confirmation says what is left out", () => {
    assert.equal(confirmScope(plan({})), "This recording has 40 screens. 28 more can be read after these: the ones at your marks and notes go first.");
    assert.equal(
        confirmScope(plan({ already: 12 })),
        "This recording has 40 screens. 12 have been read already. 16 more can be read after these: the ones at your marks and notes go first.",
    );
    assert.equal(confirmScope(plan({ total: 13, already: 1 })), "This recording has 13 screens. 1 has been read already.");
});

test("a run is summed up in a sentence", () => {
    assert.equal(summaryLine({ read: 9, skipped: 3, gone: 0 }), "Read 9 screens. 3 had nothing to keep.");
    assert.equal(summaryLine({ read: 1, skipped: 0, gone: 1 }), "Read 1 screen. 1 was removed meanwhile.");
    assert.equal(summaryLine({ read: 0, skipped: 2, gone: 0 }), "Nothing was kept. 2 had nothing to keep.");
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
