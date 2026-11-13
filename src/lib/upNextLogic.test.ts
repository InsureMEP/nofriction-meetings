// Up next: the card's words and the plan it starts. Run: npm test

import test from "node:test";
import assert from "node:assert/strict";
import { planFor, planSummary, recordLabel, sourceLabel, type UpNext } from "./upNextLogic.ts";

const routine: UpNext = {
    source: "routine",
    key: "routine:bio 101",
    title: "BIO 101",
    detail: "You usually record this on Wednesdays around 9:00 AM",
    notebook: "BIO 101",
    recording_kind: "class",
    duration: "60",
};
const calendar: UpNext = {
    source: "calendar",
    key: "calendar:evt-1:0",
    title: "Acme project sync",
    detail: "On your calendar until 10:30 AM",
    notebook: "Acme project",
    recording_kind: "meeting",
    duration: "none",
};
const call: UpNext = { source: "call", key: "call:zoom", title: "this call", detail: "Zoom is using the microphone", notebook: null, recording_kind: "meeting", duration: "none" };

test("the button has the answer on it", () => {
    assert.equal(recordLabel(routine), "Record BIO 101");
    assert.equal(recordLabel(calendar), "Record Acme project sync");
    assert.equal(recordLabel(call), "Record this call");
    assert.equal(recordLabel({ ...call, title: "  " }), "Record now");
    assert.equal(sourceLabel("routine"), "Up next");
    assert.equal(sourceLabel("calendar"), "Up next");
    assert.equal(sourceLabel("call"), "On a call");
});

test("the plan line says the three answers, and the notebook only when it adds something", () => {
    assert.equal(planSummary(routine), "Class · 60 min", "the notebook is the title already");
    assert.equal(planSummary(calendar), "Meeting · Stops when it's over · Acme project");
    assert.equal(planSummary(call), "Meeting · Stops when it's over");
    // Anything unreadable falls back to a meeting with no limit
    assert.equal(planSummary({ ...call, recording_kind: "seminar", duration: "45" }), "Meeting · Stops when it's over");
});

test("starting it sends the suggestion's answers and never remembers them", () => {
    assert.deepEqual(planFor(routine), { duration: "60", recordingKind: "class", notebook: "BIO 101", remember: false });
    assert.deepEqual(planFor(call), { duration: "none", recordingKind: "meeting", notebook: null, remember: false });
    assert.deepEqual(planFor({ ...routine, notebook: "  ", duration: "banana" }), { duration: "none", recordingKind: "class", notebook: null, remember: false });
});
