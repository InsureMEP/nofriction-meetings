// Ask: what the panel says your AI is given, and the questions it offers. Run: npm test

import test from "node:test";
import assert from "node:assert/strict";
import { askPlaceholder, givenList, knowsLine, otherSide, peopleLine, suggestions, type Knows } from "./askLogic.ts";

const knows = (over: Partial<Knows>): Knows => ({
    meeting_id: "m2",
    title: "Acme sync",
    kind: "meeting",
    notebook: "Acme",
    recording: true,
    lines: 124,
    tagged: 80,
    people: [
        { name: "Casey", company: null, role: null, you: true },
        { name: "Ana Ruiz", company: "Acme Corp", role: "organizer", you: false },
    ],
    event: "Acme renewal call",
    event_details: true,
    saved_notes: true,
    notes: 3,
    related: 1,
    ...over,
});

test("one line says what it knows right now", () => {
    assert.equal(
        knowsLine(knows({})),
        "Knows what has been said so far; your 3 notes and marks; who is here: Ana Ruiz (Acme Corp); 1 earlier recording.",
    );
    assert.equal(knowsLine(knows({ lines: 0, notes: 0, people: [], related: 0 })), "Knows nothing said yet.");
    assert.equal(knowsLine(knows({ recording: false, notes: 1, people: [], related: 2 })), "Knows what was said; your 1 note or mark; 2 earlier recordings.");
});

test("the people line names the others and their companies, never you", () => {
    assert.equal(peopleLine(knows({}).people), "Ana Ruiz (Acme Corp)");
    const four = ["Ana", "Bo", "Cy", "Di"].map((name) => ({ name, company: name === "Di" ? "Zed Ltd" : "Acme Corp", role: null, you: false }));
    assert.equal(peopleLine(four), "Ana, Bo, Cy and 1 more (Acme Corp, Zed Ltd)");
    assert.equal(peopleLine([{ name: "Casey", company: null, role: null, you: true }]), null);
    assert.equal(peopleLine([{ name: "Sam", company: null, role: null, you: false }]), "Sam");
});

test("what your AI is given is said plainly, with what it is not", () => {
    const g = givenList(knows({})).join("\n");
    assert.match(g, /The recording: "Acme sync", its type, its Notebook \(Acme\), when it started and that it is still running\./);
    assert.match(g, /the last few minutes in order, and earlier lines that match your question \(124 lines so far\)/);
    // A call with one other person: they may be named for the other side, and it says how
    assert.match(g, /The event has only you and Ana Ruiz, so your AI is told the computer's audio is most likely Ana Ruiz, and an answer that names them for it says "\(likely\)"\. noFriction doesn't recognise voices\./);
    assert.equal(otherSide(knows({}))?.name, "Ana Ruiz");
    // Three people, nobody known to be you, or no side tags: never a name
    const bo = { name: "Bo Chen", company: "Acme Corp", role: null, you: false };
    const three = knows({ people: [...knows({}).people, bo] });
    assert.equal(otherSide(three), null);
    assert.match(givenList(three).join("\n"), /Never a name: noFriction doesn't recognise voices\./);
    assert.equal(otherSide(knows({ people: [bo, knows({}).people[1]] })), null);
    assert.equal(otherSide(knows({ tagged: 0 })), null);
    assert.equal(otherSide(knows({ people: [knows({}).people[0], { ...bo, name: " " }] })), null);
    assert.equal(otherSide(knows({ people: [knows({}).people[0], { ...bo, name: "bo@acme.example" }] })), null);
    assert.match(g, /Your 3 typed notes and marks in this recording/);
    assert.match(g, /The calendar event "Acme renewal call": its title, its place and its notes as they are in your calendar \(a meeting link or dial-in written there goes too\)\./);
    assert.match(g, /The people in the event: names, companies, who organised it, which one is you, and how many of your recordings each is in\. Not their email addresses\./);
    assert.match(g, /The notes already made from this recording: their summary/);
    // Nothing is promised that the app doesn't hold
    assert.doesNotMatch(g, /your own notes about/);
    assert.match(g, /up to 3 earlier recordings in the same Notebook or with the same people/);
    // Nothing is claimed that isn't there
    const bare = givenList(knows({ lines: 0, tagged: 0, notes: 0, people: [], event: null, event_details: false, saved_notes: false, related: 0, notebook: null, recording: false })).join("\n");
    assert.match(bare, /The transcript, once something has been said\./);
    assert.match(bare, /No people or event: this recording isn't linked to a calendar event\./);
    assert.doesNotMatch(bare, /earlier recordings|typed notes|still running|Notebook|notes already made/);
    // An event with a place and notes but no attendee list is still said
    const solo = givenList(knows({ people: [], event: null, event_details: true })).join("\n");
    assert.match(solo, /The calendar event: its title, its place and its notes/);
    assert.doesNotMatch(solo, /isn't linked/);
});

test("the questions offered fit the moment and the type", () => {
    const live = suggestions(knows({}));
    assert.equal(live[0], "What did I miss?");
    assert.ok(live.includes("What was just asked of me?"));
    assert.equal(live[live.length - 1], "What did we agree with Ana Ruiz last time?");
    assert.ok(!suggestions(knows({ related: 0 })).some((s) => s.includes("last time")));
    const cls = suggestions(knows({ kind: "class", people: [] }));
    assert.ok(cls.includes("Explain the last concept simply") && cls.includes("What here is likely to be on the test?"));
    const after = suggestions(knows({ recording: false }));
    assert.deepEqual(after, ["What was decided?", "What did I say I would do?", "What is still open?", "What did Ana Ruiz want?"]);
    assert.deepEqual(suggestions(null), []);
    assert.equal(askPlaceholder(knows({})), "Ask about what's being said");
    assert.equal(askPlaceholder(knows({ recording: false })), "Ask about this recording");
    assert.equal(askPlaceholder(null), "Ask about what's being said");
});
