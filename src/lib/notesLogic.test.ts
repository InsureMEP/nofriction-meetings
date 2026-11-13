// The Notes view: what the user wrote and what the AI wrote under it. Run: npm test

import test from "node:test";
import assert from "node:assert/strict";
import { hasWhatYouWrote, mergeSyncedWritten, parseNoteDetails, progressDetail, writtenLines } from "./notesLogic.ts";
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

test("the other device's section is merged with the typed notes, and nothing written is hidden", () => {
    const notes = [note("a", "pump 3 na out", 21_000), note("b", "2 k in", 25_000), note("c", "ask about **ATP**", 30_000)];
    // Both bullet forms; one bullet with no typed note here yet; note c has no bullet
    const md = "## What you wrote\n- **pump 3 na out**: Three sodium ions leave.\n- **2 k in**\n- **quiz friday**: It covers chapter 4.\n\n## Summary\nTransport.";
    const m = mergeSyncedWritten(notes, syncedNotesSections(md));
    assert.deepEqual(m.lines.map((l) => [l.note.id, l.detail]), [["a", "Three sodium ions leave."], ["b", null], ["c", null]]);
    // The unmatched line stays readable in the notes; the matched ones aren't shown twice
    assert.deepEqual(m.sections.map((s) => [s.heading, s.bullets]), [["What you wrote", ["quiz friday: It covers chapter 4."]], ["Summary", []]]);

    // Everything matched: the section is gone, the rest of the notes stay
    const all = mergeSyncedWritten(notes.slice(0, 2), syncedNotesSections("**What you wrote**\n• pump 3 na out: Three sodium ions leave.\n• 2 k in: Two potassium.\n\n**Summary**\nTransport."));
    assert.deepEqual(all.lines.map((l) => l.detail), ["Three sodium ions leave.", "Two potassium."]);
    assert.deepEqual(all.sections.map((s) => s.heading), ["Summary"]);

    // A line that starts another is matched to the longer one; each note takes one bullet
    const twins = [note("x", "pump", 1_000), note("y", "pump: sodium", 2_000), note("z", "pump", 3_000)];
    const t = mergeSyncedWritten(twins, syncedNotesSections("## What you wrote\n- **pump: sodium**: Out of the cell.\n- **pump**: First.\n- **pump**: Second."));
    assert.deepEqual(t.lines.map((l) => l.detail), ["First.", "Out of the cell.", "Second."]);
    assert.deepEqual(t.sections, []);

    // Bold marks in a typed line are quoted without them
    const bold = mergeSyncedWritten([notes[2]], syncedNotesSections("## What you wrote\n- **ask about ATP**: The pump spends one."));
    assert.deepEqual(bold.lines.map((l) => l.detail), ["The pump spends one."]);

    // No section: every typed note is still a row; no typed notes: the section is left alone
    assert.deepEqual(mergeSyncedWritten(notes, syncedNotesSections("## Summary\nTransport.")).lines.map((l) => l.detail), [null, null, null]);
    const untouched = syncedNotesSections(md);
    assert.deepEqual(mergeSyncedWritten([], untouched).sections, untouched);
});

test("the progress line adds something or stays out of the way", () => {
    assert.equal(progressDetail("Reading the recording (2 of 5)…"), "Reading the recording (2 of 5)…");
    assert.equal(progressDetail("Making notes…"), null);
    assert.equal(progressDetail("Done"), null);
    assert.equal(progressDetail(null), null);
    assert.equal(progressDetail("  "), null);
});
