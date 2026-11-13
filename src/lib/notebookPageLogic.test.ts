// The notebook page: its words, the items list and the ticks. Run: npm test

import test from "node:test";
import assert from "node:assert/strict";
import {
    asQuizQuestion,
    headline,
    itemFor,
    itemsEmpty,
    itemsTitle,
    markedEmpty,
    markedTitle,
    openItemsPreview,
    reviewFrom,
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
    assert.equal(markedTitle("class"), "On the test and questions");
    assert.equal(markedTitle("meeting"), "Follow ups and questions");
    assert.equal(markedTitle("personal"), "To remember and questions");
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

test("a note's action item finds its tick by its text, spacing aside", () => {
    const items = [{ key: "k1", task: "Read chapter 7", done: true }, { key: "k2", task: "Quiz Wednesday", done: false }];
    assert.equal(itemFor(items, "  Read   chapter 7 ")?.key, "k1");
    assert.equal(itemFor(items, "Quiz Wednesday")?.done, false);
    assert.equal(itemFor(items, "Something the notes no longer say"), null);
    assert.equal(itemFor([], "Read chapter 7"), null);
});

test("the suggestion shows the first open items and counts the rest", () => {
    const items = [item("a"), item("b", true), item("c"), item("d"), item("e"), item("f")];
    const p = openItemsPreview(items);
    assert.deepEqual(p.shown.map((i) => i.key), ["a", "c", "d"]);
    assert.equal(p.more, 2);
    assert.deepEqual(openItemsPreview([item("x", true)]), { shown: [], more: 0 });
    assert.equal(openItemsPreview(items, 10).more, 0);
});

test("a review can start from any recording and keeps everything after it", () => {
    const from = (id: string) => ({ meeting_id: id, meeting_title: id.toUpperCase() });
    const review = {
        recordings: [
            { id: "l1", title: "L1", started_at: "2026-10-30T09:00:00Z" },
            { id: "l2", title: "L2", started_at: "2026-11-06T09:00:00Z" },
            { id: "l3", title: "L3", started_at: "2026-11-11T09:00:00Z" },
        ],
        cards: [{ front: "a", back: "1", ...from("l1") }, { front: "b", back: "2", ...from("l2") }, { front: "c", back: "3", ...from("l3") }],
        questions: [{ question: "q", choices: ["x", "y"], answer: 0, explanation: "", at_ms: null, ...from("l3") }],
    };
    const since = reviewFrom(review, "l2");
    assert.deepEqual(since.cards.map((c) => c.front), ["b", "c"]);
    assert.equal(since.questions.length, 1);
    assert.deepEqual(since.recordings.map((r) => r.id), ["l2", "l3"]);
    // The first recording, none, or one that isn't there: everything
    assert.equal(reviewFrom(review, "l1"), review);
    assert.equal(reviewFrom(review, null), review);
    assert.equal(reviewFrom(review, "gone"), review);
});
