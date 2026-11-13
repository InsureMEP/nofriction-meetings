// The Notes view: what the user wrote and what the AI wrote under it. Run: npm test

import test from "node:test";
import assert from "node:assert/strict";
import { hasWhatYouWrote, parseNoteDetails, progressDetail, writtenLines } from "./notesLogic.ts";
import { syncedNotesSections } from "./syncLogic.ts";
import type { TypedNote } from "./studyLogic.ts";

const note = (id: string, text: string, offset_ms: number): TypedNote => ({ id, meeting_id: "m1", ts: "2026-11-11T09:00:00Z", text, created_at: "2026-11-11T09:00:00Z", offset_ms });

test("note details are read by note id, and anything unreadable is no details", () => {
    const d = parseNoteDetails('[{"marker_id": "a", "detail": " Three sodium out. "}, {"marker_id": "b", "detail": "  "}, {"marker_id": 7, "detail": "x"}, "junk", null]');
    assert.deepEqual([...d.entries()], [["a", "Three sodium out."]]);
    assert.equal(parseNoteDetails(null).size, 0);
    assert.equal(parseNoteDetails("").size, 0);
    assert.equal(parseNoteDetails("not json").size, 0);
    assert.equal(parseNoteDetails('{"marker_id": "a"}').size, 0);
});

test("the user's lines keep their order, each with its detail when there is one", () => {
    const notes = [note("a", "pump 3 na out", 21_000), note("b", "2 k in", 25_000)];
    const lines = writtenLines(notes, parseNoteDetails('[{"marker_id": "b", "detail": "Two potassium in."}]'));
    assert.deepEqual(lines.map((l) => [l.note.text, l.detail]), [["pump 3 na out", null], ["2 k in", "Two potassium in."]]);
    assert.deepEqual(writtenLines([], new Map()), []);
});

test("notes from the iPhone that carry the section are recognized in both heading forms", () => {
    assert.equal(hasWhatYouWrote(syncedNotesSections("## What you wrote\n- **pump**: three sodium out\n\n## Summary\nTransport.")), true);
    assert.equal(hasWhatYouWrote(syncedNotesSections("**What you wrote**\n• pump: three sodium out")), true);
    assert.equal(hasWhatYouWrote(syncedNotesSections("## Summary\nWhat you wrote was fine.")), false);
    assert.equal(hasWhatYouWrote([]), false);
});

test("the progress line adds something or stays out of the way", () => {
    assert.equal(progressDetail("Reading the recording (2 of 5)…"), "Reading the recording (2 of 5)…");
    assert.equal(progressDetail("Making notes…"), null);
    assert.equal(progressDetail("Done"), null);
    assert.equal(progressDetail(null), null);
    assert.equal(progressDetail("  "), null);
});
