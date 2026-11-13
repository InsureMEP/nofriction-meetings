import SwiftData
import XCTest
@testable import noFriction

// docs/design/FADELL_V2.md §3 B and §6, iOS side: typed notes (a mark of kind
// `note`), notes that read the whole recording and build on what was typed,
// the "What you wrote" section, automatic notes, and their sync and purge.
// The AI is always a mock here.

private let uuidPattern = try! NSRegularExpression(pattern: "[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}")

private func hasUUID(_ s: String) -> Bool {
    uuidPattern.firstMatch(in: s, range: NSRange(s.startIndex..., in: s)) != nil
}

/// Answers by the system prompt; records every request with its answer budget.
private final class NotesMockAI: @unchecked Sendable {
    struct Call { var messages: [ChatMessage]; var maxTokens: Int }
    private let lock = NSLock()
    private var _calls: [Call] = []
    var error: Error?
    /// What the notes request answers
    var notes = "## Summary\nCells make ATP.\n\n## Action items\n- [owner not stated] Read chapter 8"
    /// Runs before each answer (on the caller's task)
    var before: (@Sendable () async -> Void)?

    var calls: [Call] { lock.withLock { _calls } }
    var condenseCalls: [Call] { calls.filter { $0.messages[0].content.hasPrefix("You condense") } }
    var notesCalls: [Call] { calls.filter { $0.messages[0].content.contains("Answer in Markdown with exactly these sections") } }

    /// A condensed part keeps its first and last line, so the test can see what survived
    static func condensed(_ user: String) -> String {
        let body = user.components(separatedBy: "TRANSCRIPT:\n").last ?? ""
        let lines = body.split(whereSeparator: \.isNewline).map(String.init).filter { $0.hasPrefix("[") }
        guard let first = lines.first, let last = lines.last else { return "[0:00] nothing" }
        return first == last ? first : first + "\n" + last
    }

    var complete: MeetingAI.Complete {
        { messages, maxTokens, _ in
            if let before = self.before { await before() }
            self.lock.withLock { self._calls.append(Call(messages: messages, maxTokens: maxTokens)) }
            if let error = self.error { throw error }
            let system = messages[0].content
            if system.hasPrefix("You condense") { return Self.condensed(messages[1].content) }
            if system.contains("Answer in Markdown with exactly these sections") { return self.notes }
            return #"{"topics": [{"label": "Cells", "confidence": 0.9}]}"#
        }
    }
}

// MARK: - The note kind

@MainActor
final class TypedNoteModelTests: XCTestCase {
    private var container: ModelContainer!
    private var context: ModelContext { container.mainContext }

    override func setUp() async throws {
        container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        Storage.prepare()
    }

    override func tearDown() async throws { container = nil }

    private func meeting(_ lines: [String] = ["Welcome to biology.", "Cells make ATP."], kind: RecordingKind = .class) -> Meeting {
        let start = Date(timeIntervalSince1970: 1_790_000_000)
        let m = Meeting(title: "Biology", startedAt: start)
        m.endedAt = start.addingTimeInterval(600)
        m.kind = kind
        context.insert(m)
        for (i, l) in lines.enumerated() {
            let s = Segment(text: l, start: start.addingTimeInterval(Double(i) * 10), duration: 8, audioOffset: Double(i) * 10)
            context.insert(s)
            s.meeting = m
        }
        try? context.save()
        return m
    }

    @discardableResult
    private func add(_ m: Meeting, _ kind: MarkerKind, _ note: String?, at seconds: Double) -> MomentMarker {
        let k = MomentMarker(at: m.startedAt.addingTimeInterval(seconds), kind: kind, note: note)
        context.insert(k)
        k.meeting = m
        try? context.save()
        return k
    }

    func testNoteIsAKindBesideTheMarksButNotOneOfThem() {
        XCTAssertEqual(MarkerKind.note.rawValue, "note", "the stored and wire value")
        XCTAssertEqual(MarkerKind.markKinds, [.important, .question, .test], "what the chooser, the Marks list and the watch offer")
        XCTAssertEqual(MarkerKind.markKinds.map(\.symbol), ["★", "?", "✎"])
        XCTAssertEqual(Set(MarkerKind.allCases), Set(MarkerKind.markKinds + [.note]))
        XCTAssertFalse(MarkerKind.markKinds.contains(.note))
        XCTAssertEqual(MarkerKind.note.symbol, "✍")
        for kind in RecordingKind.allCases { XCTAssertEqual(MarkerKind.note.label(for: kind), "Note") }
        XCTAssertFalse(MarkerKind.note.isMark)
        XCTAssertTrue(MarkerKind.markKinds.allSatisfy(\.isMark))
        XCTAssertNil(MarkerKind.mark(rawValue: "note"))
        XCTAssertNil(MarkerKind.mark(rawValue: "garbage"))
        XCTAssertEqual(MarkerKind.mark(rawValue: "test"), .test)
        XCTAssertEqual(MarkerKind.default, .important, "one tap still marks Important")
    }

    func testLimitsAThousandCharactersOneLineForANoteAndTwoEightyForAMark() {
        XCTAssertEqual(MomentMarker.maxNoteLength, 280)
        XCTAssertEqual(MomentMarker.maxTypedNoteLength, 1000)
        let long = String(repeating: "é", count: 1200)
        XCTAssertEqual(MomentMarker.clean(long, kind: .note)?.count, 1000)
        XCTAssertEqual(MomentMarker.clean(long, kind: .test)?.count, 280)
        XCTAssertEqual(MomentMarker.clean(long)?.count, 280, "a mark unless told otherwise")
        XCTAssertEqual(MomentMarker.clean("  ask   about\n\tproof 2 \n", kind: .note), "ask about proof 2", "one line, whitespace closed up")
        XCTAssertNil(MomentMarker.clean(" \n ", kind: .note))
        let m = meeting()
        let note = add(m, .note, long, at: 5)
        XCTAssertEqual(note.note?.count, 1000)
        XCTAssertTrue(note.isTypedNote)
        XCTAssertEqual(note.label, "Note")
        note.setNote(String(repeating: "x", count: 600) + "\nmore")
        XCTAssertEqual(note.note?.count, 605, "editing keeps the note's own limit")
        let mark = add(m, .question, long, at: 6)
        XCTAssertEqual(mark.note?.count, 280)
        XCTAssertFalse(mark.isTypedNote)
    }

    func testNotesStayOutOfTheMarksListItsFilterAndItsCount() {
        let m = meeting()
        add(m, .important, nil, at: 1)
        add(m, .note, "ask about oxygen", at: 2)
        add(m, .test, "on the exam", at: 3)
        add(m, .note, "second line", at: 4)
        XCTAssertEqual(m.orderedMarkers.count, 4)
        XCTAssertEqual(m.marks.map(\.markerKind), [.important, .test], "what the Marks section lists and counts")
        XCTAssertEqual(m.typedNotes.compactMap(\.note), ["ask about oxygen", "second line"])
        for kind in MarkerKind.markKinds {
            XCTAssertFalse(m.marks.filter { $0.markerKind == kind }.contains(where: \.isTypedNote))
        }
        // The watch makes marks only: a kind it doesn't offer reads as Important, as before
        let fromWatch = WatchMarker(dictionary: [WatchMarker.Key.id: UUID().uuidString, WatchMarker.Key.kind: "note",
                                                 WatchMarker.Key.at: Date(timeIntervalSince1970: 1_790_000_000)])
        XCTAssertEqual(fromWatch?.kind, .important)
    }

    func testTypingWhileRecordingKeepsANoteStampedNow() {
        let m = meeting()
        m.endedAt = nil
        let session = RecordingSession()
        session.attach(context)
        XCTAssertNil(session.addMarker(kind: .note, note: "not recording"), "only while recording")
        session.showDemo(meeting: m, startedAt: m.startedAt, partial: "")
        let stamp = m.startedAt.addingTimeInterval(12)
        let kept = session.addMarker(kind: .note, note: "  ask about   step 3 ", at: stamp)
        XCTAssertEqual(kept?.note, "ask about step 3")
        XCTAssertEqual(kept?.at, stamp)
        XCTAssertEqual(kept?.kind, "note")
        XCTAssertNil(session.addMarker(kind: .note, note: "   "), "an empty line isn't kept")
        XCTAssertNotNil(session.addMarker(), "a mark needs no text")
        // Never debounced against a mark: a note right after one is its own record
        XCTAssertNotNil(session.addMarker(kind: .note, note: "right after", at: stamp))
        XCTAssertEqual(m.typedNotes.count, 2)
        XCTAssertEqual(m.marks.count, 1)
    }

    /// The line leaves the field at once and comes back, ahead of anything typed since, only when it couldn't be kept.
    func testTheNoteLineNeverLosesText() {
        var draft = NoteDraft(text: "  ask about   step 3 ")
        XCTAssertEqual(draft.take(), "ask about step 3", "one line, as it will be kept")
        XCTAssertEqual(draft.text, "", "the field is free for the next line at once")
        // Kept: nothing comes back. Not kept: it is back, before what was typed since
        draft.text = "next thought"
        draft.putBack("ask about step 3")
        XCTAssertEqual(draft.text, "ask about step 3 next thought")
        var empty = NoteDraft(text: "   ")
        XCTAssertNil(empty.take(), "nothing to keep")
        XCTAssertEqual(empty.text, "")
        var alone = NoteDraft()
        alone.putBack("only line")
        XCTAssertEqual(alone.text, "only line")
        var long = NoteDraft(text: String(repeating: "b", count: 600))
        long.putBack(String(repeating: "a", count: 600))
        XCTAssertEqual(long.text.count, 1000)
        XCTAssertTrue(long.text.hasPrefix("aaa"), "the line that failed comes first")
        // A note that can't be kept (the recording already stopped) is reported, not swallowed
        let m = meeting()
        let session = RecordingSession()
        session.attach(context)
        var field = NoteDraft(text: "typed as it stopped")
        let line = field.take()!
        if session.addMarker(kind: .note, note: line) == nil { field.putBack(line) }
        XCTAssertEqual(field.text, "typed as it stopped")
        XCTAssertTrue(m.typedNotes.isEmpty)
    }

    /// A typed note belongs to the recording and the moment it was typed in.
    func testALineStillInTheFieldIsKeptOnTheRecordingThatIsStopping() async {
        let m = meeting()
        m.endedAt = nil
        let session = RecordingSession()
        session.attach(context)
        session.showDemo(meeting: m, startedAt: m.startedAt, partial: "")
        // Return: the recording and the moment are taken there and then
        let stamp = m.startedAt.addingTimeInterval(42)
        XCTAssertEqual(session.addMarker(kind: .note, note: "typed at 0:42", at: stamp)?.meeting?.id, m.id)
        // Stop with a line still in the field: it is kept on this recording before it ends
        var field = NoteDraft(text: "typed just before Stop")
        session.beforeStop = {
            guard let line = field.take() else { return }
            if session.addMarker(kind: .note, note: line) == nil { field.putBack(line) }
        }
        await session.stop(byUser: true)
        XCTAssertEqual(session.phase, .idle)
        XCTAssertEqual(m.typedNotes.compactMap(\.note), ["typed at 0:42", "typed just before Stop"])
        XCTAssertEqual(m.typedNotes.first?.at, stamp)
        XCTAssertEqual(field.text, "", "nothing left behind")
        XCTAssertEqual(session.lastStopped?.meetingID, m.id)
        XCTAssertEqual(session.lastStopped?.byUser, true)
        // After Stop there is no recording to keep it on: the line stays in the field instead of vanishing
        var late = NoteDraft(text: "typed after Stop")
        if let line = late.take(), session.addMarker(kind: .note, note: line) == nil { late.putBack(line) }
        XCTAssertEqual(late.text, "typed after Stop")
        XCTAssertEqual(m.typedNotes.count, 2)
    }

    func testPromptsAndExportsLabelThemNote() {
        let m = meeting()
        add(m, .test, "on the exam", at: 12)
        add(m, .note, "ask about oxygen", at: 15)
        let input = StudyInput(meeting: m)
        XCTAssertEqual(input.marksBlock(), "[0:12] ✎ On the test: on the exam\n[0:15] ✍ Note: ask about oxygen")
        XCTAssertEqual(input.marksBlock(typedNotes: false), "[0:12] ✎ On the test: on the exam")
        XCTAssertEqual(input.typed.map(\.note), ["ask about oxygen"])
        // A long note is clipped in the marks block (the notes prompt lists it whole)
        add(m, .note, String(repeating: "w", count: 900), at: 20)
        let clipped = StudyInput(meeting: m).marksBlock().components(separatedBy: "\n").last ?? ""
        XCTAssertTrue(clipped.hasSuffix("…"))
        XCTAssertLessThanOrEqual(clipped.count, "[0:20] ✍ Note: ".count + 280)
        // The review guide's Markdown
        let guide = StudyExport.guideMarkdown(title: "Bio", kind: .class, when: "", summary: nil, terms: nil, cards: nil, quiz: nil, asks: nil,
                                              marks: [.init(ms: 15_000, kind: .note, note: "ask about oxygen")])
        XCTAssertTrue(guide.contains("- 0:15 ✍ Note: ask about oxygen"))
        // Share as Text: where it was written, after the line being spoken then
        let text = MeetingExport.markdown(m)
        let line = text.range(of: "Cells make ATP.")
        let note = text.range(of: "✍ Note: ask about oxygen")
        XCTAssertNotNil(line)
        XCTAssertNotNil(note)
        if let line, let note { XCTAssertLessThan(line.lowerBound, note.lowerBound) }
        // Chat reads it
        XCTAssertTrue(ChatSource(meeting: m).marks.contains { $0.label == "Note" && $0.note == "ask about oxygen" })
    }

    func testEditingAndDeletingANoteKeepsTheNotesSectionInStep() {
        let m = meeting()
        let first = add(m, .note, "ask about oxygen", at: 5)
        let second = add(m, .note, "lab report", at: 9)
        m.aiNotes = "## What you wrote\n- **ask about oxygen**: Oxygen is the final acceptor.\n- **lab report**: Due Monday.\n\n## Summary\nCells."
        TypedNoteStore.edit(first, text: "  ask about   O2 ", context: context)
        XCTAssertEqual(first.note, "ask about O2")
        XCTAssertEqual(m.aiNotes, "## What you wrote\n- **ask about O2**: Oxygen is the final acceptor.\n- **lab report**: Due Monday.\n\n## Summary\nCells.")
        TypedNoteStore.edit(first, text: "   ", context: context)
        XCTAssertEqual(first.note, "ask about O2", "an empty edit changes nothing")
        TypedNoteStore.delete(second, context: context)
        XCTAssertEqual(m.typedNotes.compactMap(\.note), ["ask about O2"])
        XCTAssertEqual(m.aiNotes, "## What you wrote\n- **ask about O2**: Oxygen is the final acceptor.\n\n## Summary\nCells.")
        XCTAssertFalse(m.aiNotes?.contains("lab report") ?? true, "a deleted line doesn't stay quoted in the notes")
        TypedNoteStore.delete(first, context: context)
        XCTAssertEqual(m.aiNotes, "## Summary\nCells.", "the section goes with its last line")
        XCTAssertEqual(try context.fetchCount(FetchDescriptor<MomentMarker>()), 0)
        // A mark's note is edited the old way and never touches the notes
        let mark = add(m, .test, "exam", at: 3)
        TypedNoteStore.edit(mark, text: "changed", context: context)
        XCTAssertEqual(mark.note, "exam")
    }

    func testANoteGoesWithItsRecording() throws {
        let m = meeting()
        add(m, .note, "private line", at: 5)
        context.delete(m)
        try context.save()
        XCTAssertEqual(try context.fetchCount(FetchDescriptor<MomentMarker>()), 0)
    }
}

// MARK: - "What you wrote"

final class WhatYouWroteTests: XCTestCase {
    func testParsesTheIPhoneForm() throws {
        let md = "## What you wrote\n- **ask about oxygen**: Oxygen is the final electron acceptor.\n- **lab report: use the template**: Follow the template.\n\n## Summary\nCells.\n\n## Key concepts\n- ATP"
        let p = try XCTUnwrap(WhatYouWrote.parse(md))
        XCTAssertEqual(p.entries, [.init(line: "ask about oxygen", detail: "Oxygen is the final electron acceptor."),
                                   .init(line: "lab report: use the template", detail: "Follow the template.")])
        XCTAssertEqual(p.rest, "## Summary\nCells.\n\n## Key concepts\n- ATP")
    }

    func testParsesTheMacForm() throws {
        // What the Mac sends: the section, a blank line, then bold headings and "• " bullets
        let md = "## What you wrote\n- **typed line**: detail one\n\n**Summary**\nPlan agreed.\n\n**Action items**\n• Alex: send the agreement"
        let p = try XCTUnwrap(WhatYouWrote.parse(md))
        XCTAssertEqual(p.entries, [.init(line: "typed line", detail: "detail one")])
        XCTAssertEqual(p.rest, "**Summary**\nPlan agreed.\n\n**Action items**\n• Alex: send the agreement")
        // Both heading forms and both bullet forms, anywhere in the notes
        let other = "**Summary**\nPlan.\n\n**What you wrote:**\n• **a**: one\n• **b** — two\n* c: three\n  carried on\n1. **d**\n\n## Decisions\nNone recorded."
        let q = try XCTUnwrap(WhatYouWrote.parse(other))
        XCTAssertEqual(q.entries, [.init(line: "a", detail: "one"), .init(line: "b", detail: "two"),
                                   .init(line: "c", detail: "three carried on"), .init(line: "d", detail: "")])
        XCTAssertEqual(q.rest, "**Summary**\nPlan.\n\n## Decisions\nNone recorded.")
        XCTAssertNil(WhatYouWrote.parse("## Summary\nNothing typed.\n\n## Action items\n- [owner not stated] x"))
        XCTAssertNil(WhatYouWrote.parse("I wrote ## What you wrote in a sentence"))
        XCTAssertEqual(WhatYouWrote.parse("## what you wrote")?.entries, [])
    }

    func testALineKeepsItsOwnColonsAndAsterisks() {
        XCTAssertEqual(WhatYouWrote.entry("**ratio 3:1 **important****: The ratio was three to one."),
                       .init(line: "ratio 3:1 **important**", detail: "The ratio was three to one."))
        XCTAssertEqual(WhatYouWrote.entry("**why?**: Because **oxygen** is needed."), .init(line: "why?", detail: "Because **oxygen** is needed."))
        XCTAssertEqual(WhatYouWrote.entry("**alone**"), .init(line: "alone", detail: ""))
        XCTAssertEqual(WhatYouWrote.entry("no bold: just text"), .init(line: "no bold", detail: "just text"))
    }

    func testStoredNotesListEveryTypedNoteAsTypedAndFirst() {
        // The model tidied the spelling and put the section last
        let raw = "## Summary\nCells.\n\n## What you wrote\n- **Ask about oxygen.**: It is the final acceptor.\n- **Lab report: use the template**: Follow it.\n"
        let stored = WhatYouWrote.finalize(raw, typed: ["ask abt oxygen", "lab report  use **the** template"])
        XCTAssertEqual(stored, "## What you wrote\n- **ask abt oxygen**: It is the final acceptor.\n- **lab report use the template**: Follow it.\n\n## Summary\nCells.",
                       "in the order typed, one line each, whitespace closed up, ** removed from the user's line")
        // One bullet missing: matched by its words; the other line is still listed, with nothing under it
        let short = "## What you wrote\n- **Second Line!**: about two\n\n## Summary\nS"
        XCTAssertEqual(WhatYouWrote.finalize(short, typed: ["first line", "second line"]),
                       "## What you wrote\n- **first line**\n- **second line**: about two\n\n## Summary\nS")
        // No section in the answer: every typed note is listed all the same
        XCTAssertEqual(WhatYouWrote.finalize("  ## Summary\nS\n", typed: ["a", "b"]), "## What you wrote\n- **a**\n- **b**\n\n## Summary\nS")
        // Nothing typed: a section the model made up goes; notes without one stay as they are
        XCTAssertEqual(WhatYouWrote.finalize("## What you wrote\n- **x**: y\n\n## Summary\nS", typed: []), "## Summary\nS")
        XCTAssertEqual(WhatYouWrote.finalize("  ## Summary\nS\n", typed: []), "## Summary\nS")
        // What is stored parses back to the same lines, and is the Mac's own format
        XCTAssertEqual(WhatYouWrote.parse(stored)?.entries.map(\.line), ["ask abt oxygen", "lab report use the template"])
        XCTAssertEqual(WhatYouWrote.section(typed: ["typed line", "no detail"], details: ["what  was\nsaid", ""]),
                       "## What you wrote\n- **typed line**: what was said\n- **no detail**")
        XCTAssertEqual(WhatYouWrote.section(typed: [], details: []), "")
    }

    /// A sentence stays with the note it was written for, by identity. When
    /// the typed notes change while the notes are being written, a deleted
    /// note's sentence is dropped and never lands under another line.
    func testADetailStaysWithTheNoteItWasWrittenFor() {
        let a = UUID(), b = UUID(), c = UUID()
        let raw = "## What you wrote\n- **ask abt oxygen**: Oxygen binds to hemoglobin.\n- **lab report**: Due Monday.\n\n## Summary\nS"
        // Nothing changed: by position, as typed
        XCTAssertEqual(WhatYouWrote.finalize(raw, asked: [a, b], now: [(a, "ask abt oxygen"), (b, "lab report")]), raw)
        // b was deleted and c arrived from the other device meanwhile (still two notes): c gets nothing of b's
        XCTAssertEqual(WhatYouWrote.finalize(raw, asked: [a, b], now: [(a, "ask abt oxygen"), (c, "bring calculator")]),
                       "## What you wrote\n- **ask abt oxygen**: Oxygen binds to hemoglobin.\n- **bring calculator**\n\n## Summary\nS")
        // b was edited: its sentence stays with it (it is about that moment)
        XCTAssertEqual(WhatYouWrote.finalize(raw, asked: [a, b], now: [(a, "ask abt oxygen"), (b, "lab report two")]),
                       "## What you wrote\n- **ask abt oxygen**: Oxygen binds to hemoglobin.\n- **lab report two**: Due Monday.\n\n## Summary\nS")
        // b was deleted: its sentence goes with it
        XCTAssertEqual(WhatYouWrote.finalize(raw, asked: [a, b], now: [(a, "ask abt oxygen")]),
                       "## What you wrote\n- **ask abt oxygen**: Oxygen binds to hemoglobin.\n\n## Summary\nS")
        // c arrived and sorts first: the others keep their own
        XCTAssertEqual(WhatYouWrote.finalize(raw, asked: [a, b], now: [(c, "bring calculator"), (a, "ask abt oxygen"), (b, "lab report")]),
                       "## What you wrote\n- **bring calculator**\n- **ask abt oxygen**: Oxygen binds to hemoglobin.\n- **lab report**: Due Monday.\n\n## Summary\nS")
        // The model wrote one bullet too few: by their words, among the notes it was asked about
        let short = "## What you wrote\n- **lab report**: Due Monday.\n\n## Summary\nS"
        XCTAssertEqual(WhatYouWrote.finalize(short, asked: [a, b], now: [(c, "lab report"), (b, "lab report")]),
                       "## What you wrote\n- **lab report**\n- **lab report**: Due Monday.\n\n## Summary\nS")
        // Every note was deleted: no section
        XCTAssertEqual(WhatYouWrote.finalize(raw, asked: [a, b], now: []), "## Summary\nS")
    }

    /// Notes from the other device carry the section; the lines also arrive as typed notes. Merged by line.
    func testTheSectionIsMergedWithTheTypedNotesByLineAndNothingIsHidden() {
        let bullets = ["**a**: one", "**b**: two", "**c**: three"]
        // Every typed note has its row; a bullet that quotes it is its detail
        let all = WhatYouWrote.merge(bullets, typed: ["a", "b", "c"])
        XCTAssertEqual(all.bullet, [0, 1, 2])
        XCTAssertEqual(all.details, ["one", "two", "three"])
        XCTAssertEqual(all.unmatched, [], "a matched bullet leaves the section: no line is shown twice")
        // A bullet no typed note matches (its note hasn't crossed yet) stays visible; a note no bullet matches still has its row
        let some = WhatYouWrote.merge(bullets, typed: ["b", "typed after the notes"])
        XCTAssertEqual(some.bullet, [1, nil])
        XCTAssertEqual(some.details, ["two", ""])
        XCTAssertEqual(some.unmatched, [.init(line: "a", detail: "one"), .init(line: "c", detail: "three")])
        // No typed notes here at all: the section as written
        XCTAssertEqual(WhatYouWrote.merge(bullets, typed: []).unmatched.map(\.line), ["a", "b", "c"])
        // Each note at most once, in the order typed
        XCTAssertEqual(WhatYouWrote.merge(["**a**: one", "**a**: again"], typed: ["a", "a", "nope"]).bullet, [0, 1, nil])
        // The longest typed line a bullet starts with
        let longest = WhatYouWrote.merge(["**lab: report**: due Monday"], typed: ["lab", "lab: report"])
        XCTAssertEqual(longest.bullet, [nil, 0])
        XCTAssertEqual(longest.details, ["", "due Monday"])
        // A bare bullet is the line itself; bold marks and spacing don't matter
        XCTAssertEqual(WhatYouWrote.merge(["ask  about   oxygen", "**why**"], typed: ["why", "ask about **oxygen**"]).bullet, [1, 0])
        // Another case or punctuation, or a line struck in part, still finds its note
        XCTAssertEqual(WhatYouWrote.merge(["**Ask about oxygen.**: d"], typed: ["ask about oxygen"]).details, ["d"])
        let struck = WhatYouWrote.merge(["**ask about [stricken from the record]**: They need the [stricken from the record] by Friday."], typed: ["ask about Azure credits"])
        XCTAssertEqual(struck.bullet, [0])
        XCTAssertEqual(struck.unmatched, [])
        XCTAssertEqual(WhatYouWrote.merge(["**[stricken from the record] is due**: d"], typed: ["something else entirely"]).bullet, [nil])
    }

    func testRemovingAndReplacingALine() {
        let md = "## What you wrote\n- **a**: one\n- **b**: two\n- **c**: three\n\n## Summary\nS"
        XCTAssertEqual(WhatYouWrote.removing(typedIndex: 1, typed: ["a", "b", "c"], from: md), "## What you wrote\n- **a**: one\n- **c**: three\n\n## Summary\nS")
        XCTAssertEqual(WhatYouWrote.replacing(typedIndex: 2, with: "see", typed: ["a", "b", "c"], in: md),
                       "## What you wrote\n- **a**: one\n- **b**: two\n- **see**: three\n\n## Summary\nS")
        XCTAssertEqual(WhatYouWrote.removing(typedIndex: 0, typed: ["new note"], from: md), md, "a line the notes never had")
        XCTAssertEqual(WhatYouWrote.removing(typedIndex: 0, typed: ["a"], from: "## Summary\nS"), "## Summary\nS")
        // A bullet no note here matches is never dropped by an edit or a delete of another line
        XCTAssertEqual(WhatYouWrote.removing(typedIndex: 0, typed: ["a", "b"], from: md), "## What you wrote\n- **b**: two\n- **c**: three\n\n## Summary\nS")
        XCTAssertEqual(WhatYouWrote.replacing(typedIndex: 0, with: "A!", typed: ["a"], in: md),
                       "## What you wrote\n- **A!**: one\n- **b**: two\n- **c**: three\n\n## Summary\nS")
        // The last line goes: so does the section
        XCTAssertEqual(WhatYouWrote.removing(typedIndex: 0, typed: ["a"], from: "## What you wrote\n- **a**: one\n\n## Summary\nS"), "## Summary\nS")
    }
}

// MARK: - Notes generation

final class NotesPromptTests: XCTestCase {
    private func input(kind: RecordingKind = .class, typed: Bool = true) -> NotesInput {
        var marks: [StudyInput.Mark] = [.init(ms: 12_000, kind: .test, note: "on the exam"), .init(ms: 40_000, kind: .important, note: nil)]
        if typed {
            marks += [.init(ms: 30_000, kind: .note, note: "ask about oxygen"), .init(ms: 65_000, kind: .note, note: "lab report: use the template")]
        }
        let study = StudyInput(title: "Lecture 7", recordingKind: kind, courseName: "BIO 101", durationMs: 120_000,
                               lines: [.init(ms: 0, text: "Welcome to biology."),
                                       .init(ms: 10_000, text: "the secret answer is \(RedactionText.placeholder)"),
                                       .init(ms: 60_000, text: "Lab report two is due Monday.")],
                               marks: marks.sorted { $0.ms < $1.ms })
        return NotesInput(study: study, when: "Wednesday, November 11, 2026 at 9:00 AM", attendees: ["Dana Whitfield (Brightwater)"],
                          inviteNotes: "Bring the syllabus")
    }

    func testUserMessageHasTheMarksTheNumberedNotesAndTheWholeTranscript() {
        let i = input()
        let message = MeetingAI.notesUserMessage(i, condensed: false, body: i.study.transcriptLines.joined(separator: "\n"))
        XCTAssertTrue(message.hasPrefix("Class: Lecture 7\nWhen: Wednesday, November 11, 2026 at 9:00 AM\nNotebook: BIO 101\nAttendees: Dana Whitfield (Brightwater)\nInvite notes: Bring the syllabus\nLength: 2:00\n"))
        // Marks in the review guide's format, without the typed notes
        XCTAssertTrue(message.contains("\nSTUDENT MARKS:\n[0:12] ✎ On the test: on the exam\n[0:40] ★ Important\n\n"))
        // Typed notes numbered 1..n with their times
        XCTAssertTrue(message.contains("\nTYPED NOTES:\n1. [0:30] ask about oxygen\n2. [1:05] lab report: use the template\n\n"))
        // The transcript with times; stricken text stays the placeholder
        XCTAssertTrue(message.hasSuffix("TRANSCRIPT:\n[0:00] Welcome to biology.\n[0:10] the secret answer is [stricken from the record]\n[1:00] Lab report two is due Monday."))
        XCTAssertFalse(hasUUID(message), "ids never go in a prompt")
        XCTAssertFalse(message.contains("⟦"))
        // A meeting says MARKS; nothing typed, no typed block
        let plain = MeetingAI.notesUserMessage(input(kind: .meeting, typed: false), condensed: true, body: "[0:00] x")
        XCTAssertTrue(plain.contains("\nMARKS:\n[0:12] ✎ Follow up: on the exam\n"))
        XCTAssertFalse(plain.contains("TYPED NOTES"))
        XCTAssertTrue(plain.hasSuffix("NOTES (condensed from the transcript, with times):\n[0:00] x"))
        // No marks at all
        let none = NotesInput(study: StudyInput(title: "x", durationMs: 0, lines: [.init(ms: 0, text: "hi")], marks: []))
        XCTAssertTrue(MeetingAI.notesUserMessage(none, condensed: false, body: "b").contains("MARKS:\n(none)\n\nTRANSCRIPT:\nb"))
    }

    func testSystemPromptKeepsTheTypesSectionsAndAddsWhatYouWroteOnlyWhenTyped() {
        for kind in RecordingKind.allCases {
            let base = MeetingAI.notesSystem(for: kind)
            let typed = MeetingAI.notesSystem(for: kind, typedNotes: true)
            let plain = MeetingAI.notesSystem(for: kind, typedNotes: false)
            XCTAssertTrue(typed.hasPrefix(base), "\(kind): the sections for the type are exactly as they were")
            XCTAssertTrue(plain.hasPrefix(base))
            XCTAssertTrue(typed.contains("\n## What you wrote\n"))
            XCTAssertTrue(typed.contains("- **the line exactly as it was typed**: one or two sentences of what was being said at that time"))
            XCTAssertTrue(typed.contains("Start your answer with this section"))
            XCTAssertFalse(plain.contains("What you wrote"))
            for s in [typed, plain] {
                XCTAssertTrue(s.contains("[stricken from the record]"))
                XCTAssertTrue(s.contains("Cover what was marked"))
            }
            XCTAssertTrue(typed.contains("Cover what was marked and typed first"))
        }
        XCTAssertTrue(MeetingAI.notesSystem(for: .class, typedNotes: true).contains("(STUDENT MARKS) and typed their own lines (TYPED NOTES)"))
        XCTAssertFalse(MeetingAI.notesSystem(for: .meeting, typedNotes: true).localizedCaseInsensitiveContains("student"))
        XCTAssertFalse(MeetingAI.notesSystem(for: .personal, typedNotes: true).localizedCaseInsensitiveContains("student"))
    }

    func testAnswerBudgetGrowsWithTheTypedNotesAndStaysInsideTheWindow() {
        XCTAssertEqual(MeetingAI.notesMaxTokens(typedNotes: 0, contextTokens: 32_768), 1500, "as before with nothing typed")
        XCTAssertEqual(MeetingAI.notesMaxTokens(typedNotes: 5, contextTokens: 32_768), 1900)
        XCTAssertEqual(MeetingAI.notesMaxTokens(typedNotes: 500, contextTokens: 32_768), 4700)
        XCTAssertEqual(MeetingAI.notesMaxTokens(typedNotes: 5, contextTokens: 4096), 1228)
    }

    func testManyLongNotesAreFittedNotDropped() {
        let lines = (1...30).map { "\($0). [0:\(String(format: "%02d", $0))] " + String(repeating: "word ", count: 150) }
        let fitted = MeetingAI.fitLines(lines, budget: 1200)
        XCTAssertLessThanOrEqual(fitted.reduce(0) { $0 + $1.count + 1 }, 1200 + 20)
        XCTAssertTrue(fitted.first?.hasPrefix("1. [0:01] word") ?? false)
        XCTAssertTrue(fitted.last?.hasPrefix("(and ") ?? false, "what didn't fit is counted, not silently cut")
        XCTAssertEqual(MeetingAI.fitLines(["a", "b"], budget: 100), ["a", "b"])
        // Spread: the start, the middle and the end are all still there
        let many = (0..<100).map { "[\($0):00] line \($0)" }
        let spread = MeetingAI.spread(many, budget: 200)
        XCTAssertLessThanOrEqual(spread.reduce(0) { $0 + $1.count + 1 }, 200)
        XCTAssertEqual(spread.first, many.first)
        XCTAssertEqual(spread.last, many.last)
        XCTAssertGreaterThan(spread.count, 2)
        XCTAssertEqual(MeetingAI.spread(many, budget: 100_000), many)
    }
}

final class NotesGenerationTests: XCTestCase {
    private func input(lines: Int, typed: Int = 2) -> NotesInput {
        var marks: [StudyInput.Mark] = [.init(ms: 60_000, kind: .test, note: "phases of mitosis")]
        for n in 0..<typed { marks.append(.init(ms: 30_000 + n * 600_000, kind: .note, note: "typed note number \(n + 1)")) }
        return NotesInput(study: StudyInput(title: "Biology", recordingKind: .class, courseName: "BIO 101", durationMs: lines * 6000,
                                            lines: (0..<lines).map { .init(ms: $0 * 6000, text: "sentence \($0) about the cell cycle and mitosis phases") },
                                            marks: marks.sorted { $0.ms < $1.ms }))
    }

    func testAShortRecordingIsOneRequestWithTheWholeTranscript() async throws {
        let ai = NotesMockAI()
        let raw = try await MeetingAI.notes(input(lines: 3), contextTokens: 32_768, complete: ai.complete)
        XCTAssertEqual(raw, ai.notes)
        XCTAssertEqual(ai.calls.count, 1)
        let user = ai.calls[0].messages[1].content
        XCTAssertTrue(user.contains("\nTRANSCRIPT:\n[0:00] sentence 0"))
        XCTAssertTrue(user.contains("[0:12] sentence 2"))
        XCTAssertTrue(user.contains("STUDENT MARKS:\n[1:00] ✎ On the test: phases of mitosis"))
        XCTAssertTrue(user.contains("TYPED NOTES:\n1. [0:30] typed note number 1\n2. [10:30] typed note number 2"))
        XCTAssertTrue(ai.calls[0].messages[0].content.contains("## What you wrote"))
        XCTAssertEqual(ai.calls[0].maxTokens, 1660)
    }

    /// An hour of lecture on the 4K on-device window: read in parts, never trimmed by the client.
    func testALongRecordingIsCondensedInPartsAndNothingIsCutOff() async throws {
        let ai = NotesMockAI()
        let i = input(lines: 600)
        let window = AppleOnDevice.contextTokens
        XCTAssertGreaterThan(i.study.transcriptLines.joined(separator: "\n").count, ContextFit.budgetChars(contextTokens: window, maxTokens: 1228),
                             "the transcript alone doesn't fit the window")
        _ = try await MeetingAI.notes(i, contextTokens: window, complete: ai.complete)

        XCTAssertGreaterThanOrEqual(ai.condenseCalls.count, 2, "the review guide's map step ran")
        XCTAssertEqual(ai.notesCalls.count, 1)
        // No request exceeds what the client can send without trimming
        for call in ai.calls {
            let chars = call.messages.reduce(0) { $0 + $1.content.count }
            XCTAssertLessThanOrEqual(chars, ContextFit.budgetChars(contextTokens: window, maxTokens: call.maxTokens), "fits beside its answer")
            XCTAssertEqual(ContextFit.fit(call.messages, contextTokens: window, maxTokens: call.maxTokens), call.messages, "the client trims nothing")
            XCTAssertFalse(call.messages.contains { $0.content.contains("middle of transcript trimmed") })
        }
        // Every part of the transcript was read: the parts cover it from the first line to the last
        let read = ai.condenseCalls.map { $0.messages[1].content }.joined(separator: "\n")
        for n in [0, 150, 299, 300, 450, 599] {
            XCTAssertTrue(read.contains("sentence \(n) about"), "line \(n) was never sent")
        }
        // The notes are written from the condensed material: the start, the middle and the tail are there
        let final = try XCTUnwrap(ai.notesCalls.first).messages[1].content
        XCTAssertTrue(final.contains("NOTES (condensed from the transcript, with times):\n"))
        XCTAssertFalse(final.contains("\nTRANSCRIPT:\n"))
        XCTAssertTrue(final.contains("[0:00] sentence 0 about"), "the start")
        XCTAssertTrue(final.contains("[59:54] sentence 599 about"), "the tail of the transcript is represented")
        let middle = (150...450).contains { final.contains("sentence \($0) about") }
        XCTAssertTrue(middle, "the middle is represented")
        // The marks and the typed notes are still in the final request
        XCTAssertTrue(final.contains("[1:00] ✎ On the test: phases of mitosis"))
        XCTAssertTrue(final.contains("1. [0:30] typed note number 1\n2. [10:30] typed note number 2"))
    }

    func testNoTranscriptMakesNoRequestAndAccessErrorsStopTheRun() async {
        let ai = NotesMockAI()
        let empty = NotesInput(study: StudyInput(title: "x", durationMs: 0, lines: [.init(ms: 0, text: RedactionText.placeholder)], marks: []))
        do { _ = try await MeetingAI.notes(empty, contextTokens: 32_768, complete: ai.complete); XCTFail("should throw") } catch {
            XCTAssertEqual(error as? MeetingAI.NotesFailure, .noTranscript)
        }
        XCTAssertTrue(ai.calls.isEmpty)
        ai.error = AIError.consentRequired("example.com")
        do { _ = try await MeetingAI.notes(input(lines: 600), contextTokens: 4096, complete: ai.complete); XCTFail("should throw") } catch {
            XCTAssertEqual(error as? AIError, .consentRequired("example.com"))
        }
        XCTAssertEqual(ai.calls.count, 1, "the first refused request ends it")
    }
}

// MARK: - Saving, automatic notes, redaction

@MainActor
final class NotesStoreTests: XCTestCase {
    private var container: ModelContainer!
    private var context: ModelContext { container.mainContext }
    private let local = AIEndpoint(provider: .custom, baseURL: URL(string: "http://127.0.0.1:11434/v1"), apiKey: nil, model: "m",
                                   contextTokens: 32_768, consentGranted: false)
    private let publicNoConsent = AIEndpoint(provider: .custom, baseURL: URL(string: "https://ai.example.com/v1"), apiKey: nil, model: "m",
                                             contextTokens: 32_768, consentGranted: false)

    override func setUp() async throws {
        container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        Storage.prepare()
        UserDefaults.standard.removeObject(forKey: AutoNotes.settingKey)
    }

    override func tearDown() async throws {
        UserDefaults.standard.removeObject(forKey: AutoNotes.settingKey)
        container = nil
    }

    private func meeting(_ lines: [String], minutes: Double = 10, notes: String? = nil) -> Meeting {
        let start = Date(timeIntervalSince1970: 1_790_000_000)
        let m = Meeting(title: "Test", startedAt: start)
        m.endedAt = start.addingTimeInterval(minutes * 60)
        m.kind = .class
        m.aiNotes = notes
        context.insert(m)
        for (i, line) in lines.enumerated() {
            let toks = RedactionText.tokens(line)
            let base = Double(i) * 10
            let timings = toks.enumerated().map { j, t in
                WordTiming(location: t.range.location, length: t.range.length, start: base + Double(j) * 0.5, end: base + Double(j) * 0.5 + 0.4)
            }
            let s = Segment(text: line, start: start.addingTimeInterval(base), duration: 8, audioOffset: base, wordTimings: timings)
            context.insert(s)
            s.meeting = m
        }
        try? context.save()
        return m
    }

    @discardableResult
    private func add(_ m: Meeting, _ kind: MarkerKind, _ note: String?, at seconds: Double) -> MomentMarker {
        let k = MomentMarker(at: m.startedAt.addingTimeInterval(seconds), kind: kind, note: note)
        context.insert(k)
        k.meeting = m
        try? context.save()
        return k
    }

    func testMakeSavesNotesBuiltOnWhatWasTypedWithNoIdsInThePrompt() async throws {
        let strike = UUID()
        let m = meeting(["Welcome to biology.", "the secret answer is \(RedactionText.markerToken(strike))", "Oxygen is the final acceptor."])
        let mark = add(m, .test, "on the exam", at: 12)
        let note = add(m, .note, "ask abt oxygen", at: 21)
        let ai = NotesMockAI()
        ai.notes = "## What you wrote\n- **Ask about oxygen**: Oxygen is the final electron acceptor.\n\n## Summary\nCells.\n\n## Announcements and deadlines\n- Lab report two is due Monday"
        let text = try await NotesStore.make(m, contextTokens: 32_768, complete: ai.complete, context: context)
        XCTAssertEqual(m.aiNotes, text)
        XCTAssertTrue(text.hasPrefix("## What you wrote\n- **ask abt oxygen**: Oxygen is the final electron acceptor.\n\n## Summary"),
                      "the user's line as typed, with the model's sentence")
        XCTAssertNotNil(m.aiNotesAt)
        XCTAssertFalse(m.aiNotesStale)
        let sent = ai.calls.flatMap(\.messages).map(\.content).joined(separator: "\n")
        XCTAssertTrue(sent.contains("[0:10] the secret answer is [stricken from the record]"))
        XCTAssertTrue(sent.contains("1. [0:21] ask abt oxygen"))
        XCTAssertTrue(sent.contains("[0:12] ✎ On the test: on the exam"))
        for id in [strike, mark.id, note.id, m.id] { XCTAssertFalse(sent.localizedCaseInsensitiveContains(id.uuidString)) }
        XCTAssertFalse(hasUUID(sent), "ids never go in a prompt")
        XCTAssertFalse(sent.contains("⟦"))
    }

    func testNotesMadeBeforeAnEditAreNotSaved() async throws {
        let m = meeting(["We need the Azure credits by Friday.", "Marcus owns it."], notes: "old notes")
        let seg = m.orderedSegments[0]
        let ai = NotesMockAI()
        // The transcript is edited while the model is answering
        ai.before = { await MainActor.run { seg.text = "We need the credits." } }
        do {
            try await NotesStore.make(m, contextTokens: 32_768, complete: ai.complete, context: context)
            XCTFail("should throw")
        } catch {
            XCTAssertEqual(error as? MeetingAI.NotesFailure, .transcriptChanged)
        }
        XCTAssertEqual(m.aiNotes, "old notes", "nothing made from the earlier transcript is kept")
    }

    /// The same cases as the Mac's `lost_lines` test.
    func testOnlyLinesThatWereReadAndAreGoneCountAsLost() {
        let read = ["[0:00] a", "[0:05] b", "[0:05] b"]
        XCTAssertEqual(MeetingAI.lostLines(read: read, now: read), 0)
        XCTAssertEqual(MeetingAI.lostLines(read: read, now: ["[0:00] a", "[0:05] b", "[0:05] b", "[0:09] late line"]), 0, "added lines are fine")
        XCTAssertEqual(MeetingAI.lostLines(read: read, now: ["[0:00] a", "[0:05] b"]), 1, "one of the two identical lines went")
        XCTAssertEqual(MeetingAI.lostLines(read: read, now: ["[0:00] a", "[0:05] [stricken from the record]", "[0:05] b"]), 1)
        XCTAssertEqual(MeetingAI.lostLines(read: read, now: []), 3)
        XCTAssertEqual(MeetingAI.lostLines(read: [], now: read), 0)
    }

    /// The recognizer's last line lands while the notes are being made: they are still saved.
    func testALineThatArrivesWhileNotesAreBeingMadeDoesNotStopTheSave() async throws {
        let m = meeting(["We need the Azure credits by Friday.", "Marcus owns it."])
        let ai = NotesMockAI()
        let context = self.context
        ai.before = {
            await MainActor.run {
                guard m.segments.count == 2 else { return }
                let late = Segment(text: "And that's all for today.", start: m.startedAt.addingTimeInterval(25), duration: 2)
                context.insert(late)
                late.meeting = m
                // A line in the middle too (what was playing on screen, transcribed afterwards)
                let screen = Segment(text: "Welcome to the webinar.", start: m.startedAt.addingTimeInterval(5), duration: 2)
                screen.source = Snapshot.Source.screen
                context.insert(screen)
                screen.meeting = m
                try? context.save()
            }
        }
        let text = try await NotesStore.make(m, contextTokens: 32_768, complete: ai.complete, context: context)
        XCTAssertEqual(m.segments.count, 4, "the late lines arrived")
        XCTAssertEqual(m.aiNotes, text, "added lines are no reason to throw the notes away")
        XCTAssertEqual(m.aiNotes, ai.notes)
    }

    /// The save and its check are one synchronous step: every line the notes were made from is still there, and no Delete is waiting.
    func testSaveIsRefusedWhenALineIsGoneOrADeleteIsWaiting() async throws {
        let m = meeting(["We need the Azure credits by Friday.", "Marcus owns it.", "That's all."])
        let read = StudyInput(meeting: m, screenPrefix: true).transcriptLines
        XCTAssertEqual(try NotesStore.save("## Summary\nS", madeFrom: read, to: m, context: context), "## Summary\nS")
        XCTAssertEqual(m.aiNotes, "## Summary\nS")
        // A Delete in its undo window: the line is already out of the transcript
        let pending = try RedactionEngine.delete(.lines([m.orderedSegments[1]]), meeting: m, context: context)
        XCTAssertThrowsError(try NotesStore.save("## Summary\nmade before", madeFrom: read, to: m, context: context)) {
            XCTAssertEqual($0 as? MeetingAI.NotesFailure, .transcriptChanged)
        }
        // Notes made from what is left are refused too while that Delete is waiting (it may still be undone)
        let after = StudyInput(meeting: m, screenPrefix: true).transcriptLines
        XCTAssertEqual(after.count, 2)
        XCTAssertThrowsError(try NotesStore.save("## Summary\nmade during", madeFrom: after, to: m, context: context))
        let ai = NotesMockAI()
        do { try await NotesStore.make(m, contextTokens: 32_768, complete: ai.complete, context: context); XCTFail("a Delete is waiting") } catch {
            XCTAssertEqual(error as? MeetingAI.NotesFailure, .transcriptChanged)
        }
        XCTAssertEqual(m.aiNotes, "## Summary\nS", "nothing was written")
        // The Delete is done: notes made from what is left are kept; ones made before it never are
        try await RedactionEngine.commit(pending, context: context)
        XCTAssertThrowsError(try NotesStore.save("## Summary\nmade before", madeFrom: read, to: m, context: context))
        XCTAssertEqual(try NotesStore.save("## Summary\nmade after", madeFrom: after, to: m, context: context), "## Summary\nmade after")
        // A recording that was deleted meanwhile gets nothing
        context.delete(m)
        try context.save()
        XCTAssertThrowsError(try NotesStore.save("## Summary\nlate", madeFrom: after, to: m, context: context))
    }

    /// Nothing automatic calls the AI while a recording runs.
    func testAutomaticNotesWaitWhileTheNextRecordingRuns() async throws {
        final class Flag: @unchecked Sendable { var recording = true }
        let flag = Flag()
        let auto = AutoNotes()
        let ai = NotesMockAI()
        let m = meeting(["Cells make ATP.", "Lab report two is due Monday."], minutes: 7)
        let task = try XCTUnwrap(auto.recordingStopped(m, isPro: true, endpoint: local, context: context, complete: ai.complete,
                                                       poll: .milliseconds(20), isRecording: { flag.recording }))
        XCTAssertTrue(auto.isWorking(m))
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertTrue(ai.calls.isEmpty, "the next recording is running: nothing is asked")
        XCTAssertNil(m.aiNotes)
        flag.recording = false
        await task.value
        XCTAssertEqual(m.aiNotes, ai.notes, "made once it stopped")
        // A recording that starts in the middle of a run holds the next request back
        let long = NotesMockAI()
        let big = meeting((0..<400).map { "sentence \($0) about the cell cycle and mitosis phases in some detail" }, minutes: 50)
        let small = AIEndpoint(provider: .custom, baseURL: URL(string: "http://127.0.0.1:11434/v1"), apiKey: nil, model: "m",
                               contextTokens: 4096, consentGranted: false)
        long.before = { if long.calls.count == 1 { flag.recording = true } }
        let second = try XCTUnwrap(auto.recordingStopped(big, isPro: true, endpoint: small, context: context, complete: long.complete,
                                                         poll: .milliseconds(20), isRecording: { flag.recording }))
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(long.calls.count, 2, "the request already on its way finishes; the next one waits")
        XCTAssertNil(big.aiNotes)
        flag.recording = false
        await second.value
        XCTAssertNotNil(big.aiNotes)
        XCTAssertGreaterThan(long.calls.count, 2)
    }

    func testAutomaticNotesGatingRule() {
        var f = AutoNotesRule.Facts(settingOn: true, isPro: true, aiReady: true, consentGranted: true, seconds: 6 * 60,
                                    typedNotes: 0, marks: 0, hasTranscript: true, hasNotes: false, stillTranscribing: false)
        XCTAssertTrue(AutoNotesRule.shouldRun(f), "six minutes with everything in place")
        // Each condition on its own stops it
        for change: (inout AutoNotesRule.Facts) -> Void in [
            { $0.settingOn = false }, { $0.isPro = false }, { $0.aiReady = false }, { $0.consentGranted = false },
            { $0.hasTranscript = false }, { $0.hasNotes = true }, { $0.seconds = 6 * 60 - 1 },
        ] {
            var g = f
            change(&g)
            XCTAssertFalse(AutoNotesRule.shouldRun(g), "\(g)")
        }
        // A transcript that is still arriving (app audio from screen capture) is waited for, not a reason to skip
        var arriving = f
        arriving.stillTranscribing = true
        XCTAssertTrue(AutoNotesRule.shouldRun(arriving))
        arriving.hasTranscript = false
        XCTAssertTrue(AutoNotesRule.shouldRun(arriving), "its lines are on their way")
        // A short recording the user engaged with: from one minute, with a typed note or a mark
        f.seconds = 60
        XCTAssertFalse(AutoNotesRule.shouldRun(f))
        f.typedNotes = 1
        XCTAssertTrue(AutoNotesRule.shouldRun(f))
        f.typedNotes = 0; f.marks = 1
        XCTAssertTrue(AutoNotesRule.shouldRun(f))
        f.seconds = 59
        XCTAssertFalse(AutoNotesRule.shouldRun(f), "under a minute never")
        XCTAssertFalse(AutoNotesRule.shouldRun(AutoNotesRule.Facts()), "nothing set up")
    }

    func testFactsComeFromTheRecordingTheStoreAndTheEndpoint() {
        let m = meeting(["Cells make ATP."], minutes: 3)
        add(m, .note, "a line", at: 5)
        add(m, .important, nil, at: 6)
        let on = AutoNotes.facts(m, isPro: true, endpoint: local)
        XCTAssertEqual(on, AutoNotesRule.Facts(settingOn: true, isPro: true, aiReady: true, consentGranted: true, seconds: 180,
                                               typedNotes: 1, marks: 1, hasTranscript: true, hasNotes: false, stillTranscribing: false))
        XCTAssertFalse(AutoNotes.facts(m, isPro: true, endpoint: publicNoConsent).consentGranted, "a public endpoint the user never allowed")
        XCTAssertFalse(AutoNotes.facts(m, isPro: true, endpoint: nil).aiReady)
        UserDefaults.standard.set(false, forKey: AutoNotes.settingKey)
        XCTAssertFalse(AutoNotes.facts(m, isPro: true, endpoint: local).settingOn)
    }

    func testARecordingThatStopsGetsNotesByItselfOnlyWhenNothingMustBeAsked() async throws {
        let auto = AutoNotes()
        let ai = NotesMockAI()
        let m = meeting(["Cells make ATP.", "Lab report two is due Monday."], minutes: 7)
        // Not Pro, no AI, or no consent: nothing runs, and nothing is sent
        XCTAssertNil(auto.recordingStopped(m, isPro: false, endpoint: local, context: context, complete: ai.complete))
        XCTAssertNil(auto.recordingStopped(m, isPro: true, endpoint: nil, context: context, complete: ai.complete))
        XCTAssertNil(auto.recordingStopped(m, isPro: true, endpoint: publicNoConsent, context: context, complete: ai.complete))
        XCTAssertTrue(ai.calls.isEmpty)
        XCTAssertNil(m.aiNotes)
        // Everything in place
        let task = try XCTUnwrap(auto.recordingStopped(m, isPro: true, endpoint: local, context: context, complete: ai.complete))
        XCTAssertTrue(auto.isWorking(m), "Making notes… shows while it runs")
        XCTAssertNil(auto.recordingStopped(m, isPro: true, endpoint: local, context: context, complete: ai.complete), "never twice at once")
        await task.value
        XCTAssertFalse(auto.isWorking(m))
        XCTAssertEqual(m.aiNotes, ai.notes)
        XCTAssertEqual(ai.notesCalls.count, 1)
        // It has notes now: not again
        XCTAssertNil(auto.recordingStopped(m, isPro: true, endpoint: local, context: context, complete: ai.complete))
        // A failure is silent: no notes, and it is free to be asked for by hand
        let failing = NotesMockAI()
        failing.error = AIError.unreachable("offline")
        let other = meeting(["Something else was said here."], minutes: 7)
        let failed = try XCTUnwrap(auto.recordingStopped(other, isPro: true, endpoint: local, context: context, complete: failing.complete))
        await failed.value
        XCTAssertNil(other.aiNotes)
        XCTAssertFalse(auto.isWorking(other))
    }

    /// "Making notes…" shows from the moment of Stop, and the notes start
    /// only once the transcript has settled.
    func testAutomaticNotesWaitForATranscriptThatIsStillArriving() async throws {
        let auto = AutoNotes()
        let ai = NotesMockAI()
        let m = meeting(["Cells make ATP."], minutes: 7)
        // App audio from screen capture is still being transcribed
        m.screenAudioJSON = "[]"
        let task = try XCTUnwrap(auto.recordingStopped(m, isPro: true, endpoint: local, context: context, complete: ai.complete, poll: .milliseconds(20)))
        XCTAssertTrue(auto.isWorking(m), "from the moment of Stop")
        try await Task.sleep(for: .milliseconds(150))
        XCTAssertTrue(ai.calls.isEmpty, "nothing is asked while lines are still arriving")
        XCTAssertTrue(auto.isWorking(m))
        // Its last line lands; the transcript is whole
        let late = Segment(text: "On screen: the last slide.", start: m.startedAt.addingTimeInterval(30), duration: 2)
        context.insert(late)
        late.meeting = m
        m.screenAudioJSON = nil
        try context.save()
        await task.value
        XCTAssertFalse(auto.isWorking(m))
        XCTAssertEqual(m.aiNotes, ai.notes)
        XCTAssertTrue(ai.notesCalls.first?.messages[1].content.contains("the last slide") ?? false, "the notes read the line that came last")
        // Make notes pressed during the wait: that run is the one
        let other = meeting(["Something else was said."], minutes: 7)
        other.screenAudioJSON = "[]"
        let waiting = try XCTUnwrap(auto.recordingStopped(other, isPro: true, endpoint: local, context: context, complete: ai.complete, poll: .milliseconds(20)))
        other.aiNotes = "made by hand"
        other.screenAudioJSON = nil
        await waiting.value
        XCTAssertEqual(other.aiNotes, "made by hand")
    }

    /// The "What you wrote" section is part of the notes: a strike rewrites it too.
    func testStrikingAWordRewritesWhatYouWroteInTheNotes() async throws {
        let m = meeting(["We need the Azure credits by Friday.", "Marcus owns it."],
                        notes: "## What you wrote\n- **ask about Azure credits**: They need the Azure credits by Friday.\n\n## Summary\nDecision on azure credits by Friday.")
        let typed = add(m, .note, "ask about Azure credits", at: 3)
        let seg = m.orderedSegments[0]
        try await RedactionEngine.strike(.words(seg, 3...4), reason: nil, meeting: m, context: context)
        XCTAssertEqual(m.aiNotes, "## What you wrote\n- **ask about [stricken from the record]**: They need the [stricken from the record] by Friday.\n\n## Summary\nDecision on [stricken from the record] by Friday.")
        XCTAssertFalse(m.aiNotes?.localizedCaseInsensitiveContains("azure") ?? true)
        XCTAssertTrue(m.aiNotesStale)
        // Still one entry for the one line the user typed
        let parsed = try XCTUnwrap(WhatYouWrote.parse(m.aiNotes ?? ""))
        XCTAssertEqual(parsed.entries.count, 1)
        let merged = WhatYouWrote.merge(parsed.bullets, typed: m.typedNotes.compactMap(\.note))
        XCTAssertEqual(merged.bullet, [0], "the struck bullet is still this note's")
        XCTAssertEqual(merged.unmatched, [], "so the line isn't shown twice")
        // A word edit leaves the user's own line alone (docs/design/FADELL_V2.md §6)
        XCTAssertEqual(typed.note, "ask about Azure credits")
        // A Delete closes the words up there as well
        let d = meeting(["Send the Zenith Labs contract today."],
                        notes: "## What you wrote\n- **zenith labs contract**: Send the Zenith Labs contract today.\n\n## Summary\nS")
        let pending = try RedactionEngine.delete(.words(d.orderedSegments[0], 2...3), meeting: d, context: context)
        try await RedactionEngine.commit(pending, context: context)
        XCTAssertFalse(d.aiNotes?.localizedCaseInsensitiveContains("zenith") ?? true)
        XCTAssertTrue(d.aiNotes?.hasPrefix("## What you wrote\n- ** contract**: Send the contract today.") ?? false, d.aiNotes ?? "")
    }
}

// MARK: - Sync

@MainActor
final class TypedNoteSyncTests: XCTestCase {
    private var container: ModelContainer!
    private var context: ModelContext { container.mainContext }
    private var dir: URL!
    private let key = SyncCrypto.tokenKey(secret: Data((0..<32).map { UInt8($0) }))

    override func setUp() async throws {
        container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        dir = FileManager.default.temporaryDirectory.appending(path: "nf-sync-notes-\(UUID().uuidString)")
        SyncLedger.directory = dir
    }

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: dir)
        container = nil
    }

    private var engine: SyncEngine { SyncEngine(context: context, tokenKey: key) }
    private func newState() -> SyncMacState {
        SyncMacState(macID: "00000000-0000-4000-8000-0000000000ee", name: "Mac", fingerprint: String(repeating: "0", count: 64), hosts: [], port: 1)
    }

    private func recording() -> Meeting {
        let m = Meeting(title: "Lecture", startedAt: Date(timeIntervalSince1970: 1_760_000_000))
        context.insert(m)
        let s = Segment(text: "Cells make ATP.", start: m.startedAt, duration: 3)
        context.insert(s)
        s.meeting = m
        try? context.save()
        return m
    }

    func testALocalNoteTravelsAsAMarkItemWithItsWholeText() throws {
        var state = newState()
        let m = recording()
        let text = String(repeating: "typed words ", count: 80).trimmingCharacters(in: .whitespaces)   // 959 characters
        XCTAssertGreaterThan(text.count, 280)
        let note = MomentMarker(at: m.startedAt.addingTimeInterval(4), kind: .note, note: text)
        context.insert(note); note.meeting = m
        try context.save()
        engine.assignLineIDs()
        let changes = engine.changes(for: state)
        let marks = changes.compactMap { item -> MarkItem? in if case .mark(let k) = item { return k } else { return nil } }
        XCTAssertEqual(marks.count, 1)
        XCTAssertEqual(marks[0].kind, "note", "the wire value")
        XCTAssertEqual(marks[0].note, text, "the full text, not a mark's 280 characters")
        XCTAssertEqual(marks[0].id, SyncIDs.wire(note.id))
        // No protocol change: the same mark item, byte for byte through the wire
        let bytes = SyncMessage.batch(phase: .changes, items: changes, last: true, upto: nil).encode()
        guard case .batch(_, let back, _, _) = try SyncMessage.decode(bytes) else { return XCTFail("not a batch") }
        XCTAssertEqual(back, changes)
        engine.confirmSent(changes, retry: [], state: &state)
        XCTAssertTrue(engine.changes(for: state).isEmpty)
        // Deleted here: a gone mark, like any mark
        context.delete(note)
        try context.save()
        XCTAssertEqual(engine.removals(for: state), [.gone(GoneItem(entity: "mark", id: SyncIDs.wire(note.id), rec: SyncIDs.wire(m.id)))])
    }

    /// A typed note's detail never outlives the note, its bullet follows an edit, and the changed notes go to the Mac again.
    func testEditingOrDeletingANoteHereSendsTheNotesAgain() throws {
        var state = newState()
        let m = recording()
        let first = MomentMarker(at: m.startedAt.addingTimeInterval(4), kind: .note, note: "ask about oxygen")
        let second = MomentMarker(at: m.startedAt.addingTimeInterval(8), kind: .note, note: "lab report")
        for k in [first, second] { context.insert(k); k.meeting = m }
        m.aiNotes = "## What you wrote\n- **ask about oxygen**: It is the final acceptor.\n- **lab report**: Due Monday.\n\n## Summary\nS"
        m.aiNotesAt = m.startedAt
        try context.save()
        engine.assignLineIDs()
        engine.confirmSent(engine.changes(for: state), retry: [], state: &state)
        XCTAssertTrue(engine.changes(for: state).isEmpty)
        func kinds(_ items: [SyncItem]) -> [String] {
            items.map { item in
                switch item {
                case .notes(let n): return "notes:" + n.md
                case .mark(let k): return "mark:" + (k.note ?? "")
                case .gone(let g): return "gone:" + g.entity
                default: return "other"
                }
            }
        }
        // Edited: its bullet's line follows, and both the note and the notes go again
        TypedNoteStore.edit(first, text: "ask about O2", context: context)
        XCTAssertEqual(Set(kinds(engine.changes(for: state))),
                       ["mark:ask about O2", "notes:## What you wrote\n- **ask about O2**: It is the final acceptor.\n- **lab report**: Due Monday.\n\n## Summary\nS"])
        engine.confirmSent(engine.changes(for: state), retry: [], state: &state)
        // Deleted: what the notes wrote under it leaves in the same step; a gone for the note, and the notes again
        TypedNoteStore.delete(second, context: context)
        XCTAssertEqual(m.aiNotes, "## What you wrote\n- **ask about O2**: It is the final acceptor.\n\n## Summary\nS")
        XCTAssertEqual(kinds(engine.removals(for: state)), ["gone:mark"])
        XCTAssertEqual(kinds(engine.changes(for: state)), ["notes:## What you wrote\n- **ask about O2**: It is the final acceptor.\n\n## Summary\nS"])
    }

    func testAMacNoteArrivesAsANoteAndStaysOutOfTheMarks() async throws {
        var state = newState()
        let m = recording()
        let rec = SyncIDs.wire(m.id)
        let text = String(repeating: "n", count: 900)
        let id = "00000000-0000-4000-8000-000000000021"
        let r = await engine.apply([
            .mark(MarkItem(id: id, rec: rec, at: SyncIDs.ms(m.startedAt) + 2000, kind: "note", note: text, created: 1, modified: 2)),
            .mark(MarkItem(id: "00000000-0000-4000-8000-000000000022", rec: rec, at: SyncIDs.ms(m.startedAt) + 1000, kind: "test", note: "exam", created: 1, modified: 2)),
            .mark(MarkItem(id: "00000000-0000-4000-8000-000000000023", rec: rec, at: 1, kind: "from-the-future", note: nil, created: 1, modified: 2)),
        ], state: &state)
        XCTAssertEqual(r.applied, 2, "\(r)")
        XCTAssertEqual(r.skipped, 1, "a kind this version doesn't know is skipped, as 1.1.0 skips a note")
        let note = try XCTUnwrap(m.typedNotes.first)
        XCTAssertEqual(note.kind, "note")
        XCTAssertEqual(note.note, text, "whole, not cut to a mark's 280 characters")
        XCTAssertEqual(SyncIDs.wire(note.id), id)
        XCTAssertEqual(m.marks.map(\.note), ["exam"])
        XCTAssertTrue(engine.changes(for: state).filter { if case .mark = $0 { return true } else { return false } }.isEmpty, "nothing echoes back")
        // The Mac edits it, then deletes it (a time-range Delete there removes the notes inside)
        _ = await engine.apply([.mark(MarkItem(id: id, rec: rec, at: SyncIDs.ms(m.startedAt) + 2000, kind: "note", note: "shorter now", created: 1, modified: 3))], state: &state)
        XCTAssertEqual(m.typedNotes.first?.note, "shorter now")
        m.aiNotes = "## What you wrote\n- **shorter now**: What was said.\n\n## Summary\nS"
        // Edited again on the Mac: the bullet that quotes it follows here
        _ = await engine.apply([.mark(MarkItem(id: id, rec: rec, at: SyncIDs.ms(m.startedAt) + 2000, kind: "note", note: "even shorter", created: 1, modified: 4))], state: &state)
        XCTAssertEqual(m.typedNotes.first?.note, "even shorter")
        XCTAssertEqual(m.aiNotes, "## What you wrote\n- **even shorter**: What was said.\n\n## Summary\nS")
        let gone = await engine.apply([.gone(GoneItem(entity: "mark", id: id, rec: rec))], state: &state)
        XCTAssertEqual(gone.applied, 1)
        XCTAssertTrue(m.typedNotes.isEmpty)
        XCTAssertEqual(m.aiNotes, "## Summary\nS", "what the notes wrote under it leaves in the same step")
        XCTAssertEqual(m.marks.count, 1)
    }
}
