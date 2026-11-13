// The notebook page: its words, the items list and the ticks. Run: npm test

import test from "node:test";
import assert from "node:assert/strict";
import {
    asQuizQuestion,
    headline,
    itemsEmpty,
    itemsTitle,
    markedEmpty,
    reviewSummary,
    shortDate,
    splitItems,
    toggleItem,
    totalTime,
    type NotebookItem,
} from "./notebookPageLogic.ts";

const item = (key: string, done = false, meeting = "m1"): NotebookItem => ({
    meeting_id: meeting,
    meeting_title: "Lecture",
    started_at: "2026-11-11T09:00:00Z",
    key,
    task: `task ${key}`,
    assignee: null,
    due: null,
    done,
});

test("the items section is named for what the notebook is", () => {
    assert.equal(itemsTitle("class"), "Due");
    assert.equal(itemsTitle("meeting"), "Action items");
    assert.equal(itemsTitle("personal"), "To-dos");
});

test("empty sections say what fills them", () => {
    assert.match(itemsEmpty("class", false), /Make notes on a recording/);
    assert.match(itemsEmpty("class", true), /assignments, readings or deadlines/);
    assert.match(itemsEmpty("meeting", true), /action items/);
    assert.match(itemsEmpty("personal", true), /to-dos or reminders/);
    assert.match(markedEmpty("class"), /On the test or Question/);
    assert.match(markedEmpty("meeting"), /Follow up or Question/);
    assert.match(markedEmpty("personal"), /Remember or Question/);
});

test("the headline counts recordings and time", () => {
    const recs = (n: number) => Array.from({ length: n }, (_, i) => ({ id: String(i), title: "", started_at: "", duration_seconds: 0, recording_kind: "class", has_notes: false, has_guide: false }));
    assert.equal(headline({ recording_kind: "class", recordings: recs(6), total_seconds: 17_400 }), "Class · 6 recordings · 4 h 50 min");
    assert.equal(headline({ recording_kind: "meeting", recordings: recs(1), total_seconds: 0 }), "Meeting · 1 recording");
    assert.equal(headline({ recording_kind: "personal", recordings: recs(2), total_seconds: 7200 }), "Personal · 2 recordings · 2 h");
    assert.equal(totalTime(3000), "50 min");
    assert.equal(totalTime(29), "");
    assert.equal(totalTime(-5), "");
});

test("open items come first, and a tick moves exactly one item", () => {
    const items = [item("a"), item("b", true), item("c"), item("a", false, "m2")];
    const { open, done } = splitItems(items);
    assert.deepEqual(open.map((i) => `${i.meeting_id}:${i.key}`), ["m1:a", "m1:c", "m2:a"]);
    assert.deepEqual(done.map((i) => i.key), ["b"]);
    // The same text in another recording is another item
    const ticked = toggleItem(items, "m1", "a", true);
    assert.deepEqual(ticked.map((i) => i.done), [true, true, false, false]);
    assert.equal(items[0].done, false, "the original list is untouched");
    assert.deepEqual(toggleItem(ticked, "m1", "b", false).map((i) => i.done), [true, false, false, false]);
    assert.deepEqual(toggleItem(items, "nope", "a", true), items);
});

test("review everything says what it holds, or nothing when there are no guides", () => {
    assert.equal(reviewSummary(86, 42), "86 flashcards and 42 practice questions");
    assert.equal(reviewSummary(1, 0), "1 flashcard");
    assert.equal(reviewSummary(0, 1), "1 practice question");
    assert.equal(reviewSummary(0, 0), null);
});

test("dates and stored quiz questions", () => {
    assert.equal(shortDate("2026-11-11T09:00:00Z"), "Nov 11");
    assert.equal(shortDate("not a date"), "");
    const q = asQuizQuestion({ question: "Q", choices: ["a", "b"], answer: 1, explanation: null, at_ms: null, meeting_id: "m1", meeting_title: "L" });
    assert.equal(q.explanation, "");
    assert.equal(q.meeting_id, "m1");
});
