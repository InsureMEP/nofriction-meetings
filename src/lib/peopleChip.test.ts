// Run: node --test src/lib/peopleChip.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import { chipLabel, chipTitle, popoverPlace, shortName } from "./peopleChip.ts";

const p = (name: string | null, email = "x@acme.example") => ({ name, email });

test("the chip names who was there in a few words", () => {
    assert.equal(chipLabel([]), "");
    assert.equal(chipLabel([p("Maya Chen")]), "Maya Chen");
    assert.equal(chipLabel([p("Maya Chen"), p("Jordan Lee")]), "Maya and Jordan");
    assert.equal(chipLabel([p("Maya Chen"), p("Jordan Lee"), p("Dana Okafor")]), "Maya, Jordan +1");
    const ten = Array.from({ length: 10 }, (_, i) => p(`Person${i} Last`));
    assert.equal(chipLabel(ten), "Person0, Person1 +8");
    // No name on the invite: the name part of the address, never the whole address
    assert.equal(shortName(p(null, "anitha.b@acme.example")), "anitha.b");
    assert.equal(chipLabel([p("  ", "sam@acme.example")]), "sam");
    assert.equal(chipLabel([p(null, "sam@acme.example"), p("Maya Chen")]), "sam and Maya");
});

test("the chip says what opening it shows", () => {
    assert.equal(chipTitle([p("Maya Chen")], "Acme sync"), '1 person in "Acme sync". Show them.');
    assert.equal(chipTitle([p("A B"), p("C D")], null), "2 people in the calendar event. Show them.");
    assert.equal(chipTitle([], "Focus time"), 'The calendar event "Focus time"');
});

test("the list opens inside the recording's pane", () => {
    const pane = { left: 400, right: 1500, bottom: 1000 };
    // Room to the right: under the chip, as tall as fits
    assert.deepEqual(popoverPlace({ left: 660, bottom: 170 }, pane, 440), { left: 0, maxHeight: 812 });
    // Near the right edge: moved left so it isn't cut off
    assert.deepEqual(popoverPlace({ left: 1300, bottom: 170 }, pane, 440), { left: -252, maxHeight: 812 });
    // A pane narrower than the list: against the pane's left edge
    assert.equal(popoverPlace({ left: 520, bottom: 170 }, { left: 400, right: 800, bottom: 1000 }, 440).left, -108);
    // A short pane still gets a usable height
    assert.equal(popoverPlace({ left: 660, bottom: 900 }, pane, 440).maxHeight, 160);
});
