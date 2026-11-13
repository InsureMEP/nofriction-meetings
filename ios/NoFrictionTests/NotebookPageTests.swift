import SwiftData
import XCTest
@testable import noFriction

// docs/design/FADELL_V2.md §3 C and §6 "The notebook page", iOS side: the
// items parsed from each recording's notes, done state kept as a hash, the
// notebook's one deck and one quiz, and the store migration.

final class NotebookItemsTests: XCTestCase {
    func testItemsFromMeetingClassAndPersonalNotes() {
        let meeting = """
        ## Summary
        Brightwater wants searchable site notes.
        ## Decisions
        - Pilot with five inspectors
        ## Action items
        - [Alex] Send the pilot agreement (Friday)
        - [owner not stated] Choose the five pilot inspectors
        """
        XCTAssertEqual(NotebookItems.texts(inNotes: meeting), ["[Alex] Send the pilot agreement (Friday)", "Choose the five pilot inspectors"])
        let lecture = """
        ## What you wrote
        - **lab report: use the template**: Follow the template.

        ## Summary
        How enzymes work.

        ## Key concepts
        - Enzymes speed up reactions

        ## Definitions
        - **Active site**: where the substrate binds

        ## Examples
        None.

        ## Announcements and deadlines
        - Lab report two is due Monday
        - Read chapter 8 before Thursday's lecture
        """
        XCTAssertEqual(NotebookItems.texts(inNotes: lecture), ["Lab report two is due Monday", "Read chapter 8 before Thursday's lecture"])
        let personal = """
        ## Summary
        Shoulder check-up.

        ## Key points
        - Ice after long runs

        ## To-dos and reminders
        - Band exercises: three sets of ten
        - Book the next visit for the end of the month
        """
        XCTAssertEqual(NotebookItems.texts(inNotes: personal), ["Band exercises: three sets of ten", "Book the next visit for the end of the month"])
    }

    func testBothHeadingFormsAndBothBulletForms() {
        // Notes synced from the Mac: bold headings and "• " bullets
        let mac = "## What you wrote\n- **typed**: detail\n\n**Summary**\nPlan agreed.\n\n**Decisions**\n• Pilot with five inspectors\n\n**Action items**\n• Alex: send the pilot agreement by Friday\n• Jonah: share the report templates"
        XCTAssertEqual(NotebookItems.texts(inNotes: mac), ["Alex: send the pilot agreement by Friday", "Jonah: share the report templates"])
        XCTAssertEqual(NotebookItems.texts(inNotes: "**Announcements and deadlines:**\n* Quiz one is next Tuesday\n1. Bring a calculator"),
                       ["Quiz one is next Tuesday", "Bring a calculator"])
        XCTAssertEqual(NotebookItems.texts(inNotes: "### TO-DOS AND REMINDERS\n- [ ] Call the clinic\n- [x] Refill the prescription"),
                       ["Call the clinic", "Refill the prescription"])
    }

    func testOnlyTheItemsHeadingCountsAndNothingIsNotAnItem() {
        XCTAssertEqual(NotebookItems.texts(inNotes: "## Summary\nNo tasks.\n\n## Decisions\n- Ship in May"), [], "no items heading")
        XCTAssertEqual(NotebookItems.texts(inNotes: "## Action items\nNone recorded."), [])
        XCTAssertEqual(NotebookItems.texts(inNotes: "## Announcements and deadlines\n- None mentioned."), [])
        XCTAssertEqual(NotebookItems.texts(inNotes: "## To-dos and reminders\n- None.\n- n/a"), [])
        XCTAssertEqual(NotebookItems.texts(inNotes: "## Action items\n- Send the deck\n\n## Sources\n- Not an item"), ["Send the deck"], "stops at the next heading")
        XCTAssertEqual(NotebookItems.texts(inNotes: "## Action items\n- old\n\n## Summary\nS\n\n## Action items\n- new"), ["new"], "the last such heading")
        XCTAssertEqual(NotebookItems.texts(inNotes: "## Action items\n- Send the deck\n- send  the DECK\n- Book a room"), ["Send the deck", "Book a room"], "each once")
        XCTAssertEqual(NotebookItems.texts(inNotes: ""), [])
        XCTAssertEqual(NotebookItems.title(for: .class), "Due")
        XCTAssertEqual(NotebookItems.title(for: .meeting), "Action items")
        XCTAssertEqual(NotebookItems.title(for: .personal), "To-dos")
    }

    /// The page's words (the same as the Mac's).
    func testSectionTitlesAndEmptyStatesFollowTheNotebooksType() {
        // The marks section holds the ✎ marks and the questions, so it is named for them, never "Marked"
        XCTAssertEqual(NotebookItems.markedTitle(for: .class), "On the test and questions")
        XCTAssertEqual(NotebookItems.markedTitle(for: .meeting), "Follow ups and questions")
        XCTAssertEqual(NotebookItems.markedTitle(for: .personal), "To remember and questions")
        XCTAssertEqual(NotebookItems.markedEmpty(for: .class), "Nothing marked yet. While you record, mark a moment and choose On the test or Question: it shows up here.")
        XCTAssertTrue(NotebookItems.markedEmpty(for: .meeting).contains("choose Follow up or Question"))
        XCTAssertTrue(NotebookItems.markedEmpty(for: .personal).contains("choose Remember or Question"))
        // One short sentence when there is nothing to review yet
        XCTAssertEqual(NotebookItems.reviewEmpty(for: .class), "No study guides yet. Make one on a recording, and its flashcards and practice questions join the others here.")
        XCTAssertEqual(NotebookItems.reviewEmpty(for: .meeting), "No review guides yet. Make one on a recording, and its flashcards and practice questions join the others here.")
        XCTAssertEqual(NotebookItems.reviewEmpty(for: .personal), NotebookItems.reviewEmpty(for: .meeting))
        for kind in RecordingKind.allCases {
            XCTAssertEqual(NotebookItems.emptyText(for: kind, anyNotes: false), "Nothing yet. Make notes on a recording, and what it says is to be done shows up here.")
        }
        XCTAssertEqual(NotebookItems.emptyText(for: .class, anyNotes: true), "No assignments, readings or deadlines were mentioned in the notes so far.")
        XCTAssertEqual(NotebookItems.emptyText(for: .meeting, anyNotes: true), "No action items were found in the notes so far.")
        XCTAssertEqual(NotebookItems.emptyText(for: .personal, anyNotes: true), "No to-dos or reminders were mentioned in the notes so far.")
        XCTAssertEqual(NotebookPage.summary(kind: .class, count: 1, seconds: 0), "Class · 1 recording")
        XCTAssertEqual(NotebookPage.summary(kind: .meeting, count: 3, seconds: 9000), "Meeting · 3 recordings · 2 h 30 min")
    }

    /// A meeting's item is "[owner] task (due date)": the owner and the due text are read out of that form only.
    func testOwnerAndDueTextComeFromAMeetingsItems() {
        XCTAssertEqual(NotebookItems.parse("[Alex] Send the pilot agreement (Friday)"),
                       .init(text: "[Alex] Send the pilot agreement (Friday)", task: "Send the pilot agreement", assignee: "Alex", due: "Friday"))
        XCTAssertEqual(NotebookItems.parse("[owner not stated] Choose the five pilot inspectors"),
                       .init(text: "Choose the five pilot inspectors", task: "Choose the five pilot inspectors", assignee: nil, due: nil))
        XCTAssertEqual(NotebookItems.parse("[Owner Not Stated] Book the room (by June 3)"),
                       .init(text: "Book the room (by June 3)", task: "Book the room", assignee: nil, due: "by June 3"))
        // A class's deadline keeps its own parenthesis: it isn't that form
        XCTAssertEqual(NotebookItems.parse("Read chapter 8 (pages 100 to 120)"),
                       .init(text: "Read chapter 8 (pages 100 to 120)", task: "Read chapter 8 (pages 100 to 120)", assignee: nil, due: nil))
        XCTAssertEqual(NotebookItems.parse("  **Lab report two**   is due Monday ")?.text, "Lab report two is due Monday", "bold marks and spacing go")
        XCTAssertEqual(NotebookItems.parse("[x] [Dana] Call the clinic")?.assignee, "Dana")
        XCTAssertNil(NotebookItems.parse("None recorded."))
        XCTAssertNil(NotebookItems.parse("[owner not stated] None."))
        XCTAssertNil(NotebookItems.parse("  "))
        // The tick is kept for the whole item; trailing punctuation doesn't change it
        let id = UUID()
        XCTAssertEqual(NotebookItems.hash("Read chapter 8.", meetingID: id), NotebookItems.hash("read chapter 8", meetingID: id))
        XCTAssertNotEqual(NotebookItems.hash("[Alex] Send it (Friday)", meetingID: id), NotebookItems.hash("[Alex] Send it (Monday)", meetingID: id))
    }

    /// "Still open from last time": the first three open items, and how many more.
    func testOpenItemsPreview() {
        let id = UUID()
        let items = ["a", "b", "c", "d", "e"].map { NotebookItems.item(.init(text: $0, task: $0, assignee: nil, due: $0 == "b" ? "Friday" : nil), meetingID: id) }
        let preview = NotebookItems.preview(open: items)
        XCTAssertEqual(preview.shown.map(\.task), ["a", "b", "c"])
        XCTAssertEqual(preview.shown[1].due, "Friday")
        XCTAssertEqual(preview.more, 2)
        XCTAssertEqual(NotebookItems.preview(open: Array(items.prefix(2))), NotebookItems.Preview(shown: Array(items.prefix(2)), more: 0))
        XCTAssertEqual(NotebookItems.preview(open: []), NotebookItems.Preview())
    }

    /// The Notes view puts a tick beside the bullets under the notes' last items heading, and nowhere else.
    func testTheNotesViewKnowsWhichBulletsAreItems() {
        let notes = "## Summary\nPlan.\n\n## Decisions\n- Pilot with five inspectors\n\n## Action items\n- [Alex] Send the agreement (Friday)\n- None recorded.\n- [owner not stated] Book the room"
        let blocks = NotesMarkdown.blocks(notes)
        let items = NotebookItems.itemBlocks(blocks)
        XCTAssertEqual(items.count, 2, "the decision isn't an item, and neither is a placeholder")
        XCTAssertEqual(items.keys.sorted().compactMap { items[$0]?.task }, ["Send the agreement", "Book the room"])
        for (index, _) in items {
            guard case .bullet = blocks[index] else { return XCTFail("a tick only goes beside a bullet") }
        }
        XCTAssertEqual(NotebookItems.itemBlocks(NotesMarkdown.blocks("**Action items**\n• Alex: send the agreement")).count, 1, "the Mac's form")
        XCTAssertTrue(NotebookItems.itemBlocks(NotesMarkdown.blocks("## Summary\n- not an item")).isEmpty)
    }

    func testDoneStateHash() {
        let a = UUID(), b = UUID()
        let h = NotebookItems.hash("Read chapter 8", meetingID: a)
        XCTAssertEqual(h.count, 64)
        XCTAssertTrue(h.allSatisfy { $0.isHexDigit })
        XCTAssertEqual(NotebookItems.hash("  read   CHAPTER 8\n", meetingID: a), h, "case and spacing don't change it")
        XCTAssertNotEqual(NotebookItems.hash("Read chapter 9", meetingID: a), h)
        XCTAssertNotEqual(NotebookItems.hash("Read chapter 8", meetingID: b), h, "per recording")
        XCTAssertFalse(h.contains("chapter"))
    }

    func testReviewEverythingIsTheUnionInRecordingOrder() {
        let old = UUID(), new = UUID(), none = UUID()
        let t = Date(timeIntervalSince1970: 1_790_000_000)
        func q(_ text: String, at ms: Int? = nil) -> StudyQuiz.Item {
            .init(question: text, choices: ["a", "b"], answer: 0, explanation: "", atMs: ms)
        }
        let sources = [
            NotebookReview.Source(meetingID: new, startedAt: t.addingTimeInterval(7 * 86_400),
                                  cards: [.init(front: "what makes ATP?", back: "mitochondria."), .init(front: "Where is glycolysis?", back: "The cytoplasm")],
                                  questions: [q("What makes ATP?"), q("Where does glycolysis happen?", at: 5000)]),
            NotebookReview.Source(meetingID: none, startedAt: t.addingTimeInterval(3 * 86_400), cards: [], questions: []),
            NotebookReview.Source(meetingID: old, startedAt: t,
                                  cards: [.init(front: "What is a cell?", back: "The unit of life"), .init(front: "What makes ATP?", back: "Mitochondria")],
                                  questions: [q("What makes ATP?", at: 9000)]),
        ]
        let all = NotebookReview.everything(sources)
        XCTAssertEqual(all.cards.map(\.front), ["What is a cell?", "What makes ATP?", "Where is glycolysis?"], "oldest recording first, a repeated card once")
        XCTAssertEqual(all.questions.map(\.item.question), ["What makes ATP?", "Where does glycolysis happen?"])
        XCTAssertEqual(all.questions.map(\.meetingID), [old, new], "each question still jumps into its own recording")
        XCTAssertEqual(all.questions.first?.item.atMs, 9000)
        XCTAssertEqual(all.guides, 2, "a recording without a guide isn't counted")
        XCTAssertFalse(all.isEmpty)
        XCTAssertTrue(NotebookReview.everything([]).isEmpty)
        // The deck and the quiz run in the guide's own state
        var deck = FlashcardDeckState(count: all.cards.count)
        for _ in all.cards { deck.flip(); deck.mark(known: true) }
        XCTAssertTrue(deck.isDone)
        XCTAssertEqual(QuizRunState(count: all.questions.count).picked.count, 2)
    }

    /// "Start from": a recording that has a guide keeps itself and every later one.
    func testReviewCanStartFromAnyRecording() {
        let first = UUID(), second = UUID(), third = UUID(), none = UUID()
        let t = Date(timeIntervalSince1970: 1_790_000_000)
        func source(_ id: UUID, _ title: String, day: Double, card: String?, question: String?) -> NotebookReview.Source {
            NotebookReview.Source(meetingID: id, title: title, startedAt: t.addingTimeInterval(day * 86_400),
                                  cards: card.map { [.init(front: $0, back: "answer to \($0)")] } ?? [],
                                  questions: question.map { [.init(question: $0, choices: ["a", "b"], answer: 0, explanation: "", atMs: nil)] } ?? [])
        }
        let sources = [source(third, "Lecture 3", day: 14, card: "c3", question: "q3"), source(none, "No guide", day: 10, card: nil, question: nil),
                       source(first, "Lecture 1", day: 0, card: "c1", question: "q1"), source(second, "Lecture 2", day: 7, card: "c2", question: nil)]
        // What the picker lists: the recordings that have a guide, oldest first
        XCTAssertEqual(NotebookReview.guides(sources).map(\.title), ["Lecture 1", "Lecture 2", "Lecture 3"])
        // Nothing chosen, the first one, or one that isn't there: everything
        for start in [nil, first, none, UUID()] as [UUID?] {
            let all = NotebookReview.everything(sources, from: start)
            XCTAssertEqual(all.cards.map(\.front), ["c1", "c2", "c3"])
            XCTAssertEqual(all.questions.map(\.item.question), ["q1", "q3"])
            XCTAssertEqual(all.guides, 3)
        }
        // From the second: that recording and every later one
        let later = NotebookReview.everything(sources, from: second)
        XCTAssertEqual(later.cards.map(\.front), ["c2", "c3"])
        XCTAssertEqual(later.questions.map(\.meetingID), [third])
        XCTAssertEqual(later.guides, 2)
        XCTAssertEqual(NotebookReview.guides(sources, from: second).map(\.title), ["Lecture 2", "Lecture 3"])
        let last = NotebookReview.everything(sources, from: third)
        XCTAssertEqual(last.cards.map(\.front), ["c3"])
        XCTAssertEqual(last.questions.map(\.item.question), ["q3"])
        // A card two guides share comes from the first guide that is kept
        let shared = sources + [source(UUID(), "Lecture 4", day: 21, card: "c1", question: nil)]
        XCTAssertEqual(NotebookReview.everything(shared).cards.map(\.front), ["c1", "c2", "c3"])
        XCTAssertEqual(NotebookReview.everything(shared, from: second).cards.map(\.front), ["c2", "c3", "c1"], "still there when its first guide is left out")
    }
}

@MainActor
final class NotebookPageStoreTests: XCTestCase {
    private var container: ModelContainer!
    private var context: ModelContext { container.mainContext }

    override func setUp() async throws {
        container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        Storage.prepare()
    }

    override func tearDown() async throws { container = nil }

    private func meeting(_ title: String, daysAgo: Double = 0, notebook: String? = "BIO 101", kind: RecordingKind = .class,
                         lines: [String] = ["Cells make ATP."], notes: String? = nil) -> Meeting {
        let start = Date(timeIntervalSince1970: 1_790_000_000 - daysAgo * 86_400)
        let m = Meeting(title: title, startedAt: start)
        m.endedAt = start.addingTimeInterval(3000)
        m.courseName = notebook
        m.kind = kind
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

    func testDoneStateIsKeptAsAHashAndGoesWithTheRecording() throws {
        let m = meeting("Lecture 6", notes: "## Announcements and deadlines\n- Lab report two is due Monday\n- Read chapter 8")
        let other = meeting("Lecture 7", notes: "## Announcements and deadlines\n- Read chapter 8")
        let items = NotebookItems.items(of: m)
        XCTAssertEqual(items.map(\.text), ["Lab report two is due Monday", "Read chapter 8"])
        XCTAssertFalse(NotebookItems.isDone(items[1], in: m))
        NotebookItems.setDone(true, item: items[1], meeting: m, context: context)
        NotebookItems.setDone(true, item: items[1], meeting: m, context: context)
        XCTAssertTrue(NotebookItems.isDone(items[1], in: m))
        XCTAssertFalse(NotebookItems.isDone(items[0], in: m))
        // The same words in another recording are another item
        let same = try XCTUnwrap(NotebookItems.items(of: other).first)
        XCTAssertNotEqual(same.hash, items[1].hash)
        XCTAssertFalse(NotebookItems.isDone(same, in: other))
        // One row, a hash and a date: the item's text isn't stored a second time
        let rows = try context.fetch(FetchDescriptor<NotebookItemDone>())
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].itemHash, NotebookItems.hash("Read chapter 8", meetingID: m.id))
        XCTAssertEqual(rows[0].itemHash.count, 64)
        XCTAssertFalse(rows[0].itemHash.localizedCaseInsensitiveContains("chapter"))
        // The tick in the recording's Notes is the same one: the Notes view reads the same item from the same store
        let inNotes = NotebookItems.itemBlocks(NotesMarkdown.blocks(m.aiNotes ?? "")).values.map { NotebookItems.item($0, meetingID: m.id) }
        XCTAssertEqual(Set(inNotes.map(\.hash)), Set(items.map(\.hash)))
        XCTAssertEqual(inNotes.filter { NotebookItems.isDone($0, in: m) }.map(\.text), ["Read chapter 8"], "ticked on the page, ticked in Notes")
        if let other = inNotes.first(where: { $0.text == "Lab report two is due Monday" }) {
            NotebookItems.setDone(true, item: other, meeting: m, context: context)   // ticked in Notes…
            XCTAssertTrue(NotebookItems.isDone(items[0], in: m), "…ticked on the page")
            XCTAssertTrue(NotebookItems.open(in: [m]).isEmpty, "nothing is open any more")
            NotebookItems.setDone(false, item: other, meeting: m, context: context)
        }
        XCTAssertEqual(NotebookItems.open(in: [m]).map(\.text), ["Lab report two is due Monday"], "what is still open")
        // Unchecked: the row goes
        NotebookItems.setDone(false, item: items[1], meeting: m, context: context)
        XCTAssertEqual(try context.fetchCount(FetchDescriptor<NotebookItemDone>()), 0)
        // Deleted with its recording
        NotebookItems.setDone(true, item: items[0], meeting: m, context: context)
        context.delete(m)
        try context.save()
        XCTAssertEqual(try context.fetchCount(FetchDescriptor<NotebookItemDone>()), 0)
    }

    func testDoneMarksOfItemsThatLeftTheNotesAreRemoved() async throws {
        let m = meeting("Sync", kind: .meeting, lines: ["Send the Zenith Labs contract today."],
                        notes: "## Action items\n- [owner not stated] Send the Zenith Labs contract\n- [owner not stated] Book the room")
        for item in NotebookItems.items(of: m) { NotebookItems.setDone(true, item: item, meeting: m, context: context) }
        XCTAssertEqual(m.doneItems.count, 2)
        // A strike rewrites the first item in the notes: its hash (of the old words) goes
        try await RedactionEngine.strike(.words(m.orderedSegments[0], 2...3), reason: nil, meeting: m, context: context)
        XCTAssertEqual(NotebookItems.items(of: m).map(\.text), ["Send the [stricken from the record] contract", "Book the room"])
        XCTAssertEqual(try context.fetch(FetchDescriptor<NotebookItemDone>()).map(\.itemHash), [NotebookItems.hash("Book the room", meetingID: m.id)])
        // Notes made again without the item: gone too
        m.aiNotes = "## Action items\nNone recorded."
        NotebookItems.pruneDone(m, context: context)
        try context.save()
        XCTAssertEqual(try context.fetchCount(FetchDescriptor<NotebookItemDone>()), 0)
    }

    func testThePageReadsTheNotebookIgnoringCase() throws {
        let l5 = meeting("Lecture 5", daysAgo: 12, notebook: "bio 101", kind: .meeting)
        let l6 = meeting("Lecture 6", daysAgo: 5, notes: "## Announcements and deadlines\n- Lab report two is due Monday")
        _ = meeting("Standup", daysAgo: 1, notebook: "Acme project", kind: .meeting)
        let all = try context.fetch(FetchDescriptor<Meeting>(sortBy: [SortDescriptor(\.startedAt, order: .reverse)]))
        let page = all.filter { Notebook.matches($0.courseName, filter: "BIO 101") }
        XCTAssertEqual(page.map(\.title), ["Lecture 6", "Lecture 5"], "newest first")
        XCTAssertEqual(NotebookPage.usualKind(page), .class, "the type of its latest recording")
        XCTAssertEqual(NotebookPage.usualKind([l5]), .meeting)
        XCTAssertEqual(NotebookPage.usualKind([]), .meeting)
        // Marked: ✎ and ? only, never ★ or a typed note
        for (kind, note) in [(MarkerKind.important, nil as String?), (.test, "inhibitors"), (.question, "why heat?"), (.note, "typed line")] {
            let k = MomentMarker(at: l6.startedAt.addingTimeInterval(30), kind: kind, note: note)
            context.insert(k); k.meeting = l6
        }
        try context.save()
        let marked = page.flatMap { m in m.marks.filter { $0.markerKind == .test || $0.markerKind == .question } }
        XCTAssertEqual(Set(marked.compactMap(\.note)), ["inhibitors", "why heat?"])
        // Review everything from the stored guides
        let cards = #"{"cards":[{"front":"What does an enzyme do?","back":"Speeds up a reaction"}]}"#
        let quiz = #"{"questions":[{"question":"Where does the substrate bind?","choices":["Active site","Nucleus"],"answer":0,"explanation":"","at_ms":20000}]}"#
        try StudyStore.save([(.flashcards, cards), (.quiz, quiz)], fingerprint: StudyInput(meeting: l6).fingerprint, meeting: l6, context: context)
        let everything = NotebookReview.everything(page.map(NotebookReview.source))
        XCTAssertEqual(everything.cards.map(\.front), ["What does an enzyme do?"])
        XCTAssertEqual(everything.questions.map(\.meetingID), [l6.id])
        XCTAssertEqual(everything.questions.first?.item.atMs, 20_000)
        XCTAssertEqual(everything.guides, 1)
        XCTAssertEqual(page.flatMap { NotebookItems.items(of: $0) }.map(\.text), ["Lab report two is due Monday"])
    }

    /// A store from before v2 opens with the new model (a new entity and a
    /// new relationship; no schema version), and the new parts work on it.
    func testStoreFromBeforeV2Migrates() throws {
        let dir = FileManager.default.temporaryDirectory.appending(path: "nf-migrate-v2-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appending(path: "old.store")
        let started = Date(timeIntervalSince1970: 1_790_000_000)
        try autoreleasepool {
            let old = try ModelContainer(for: Schema(PreKindSchema.models), configurations: ModelConfiguration(url: url))
            let ctx = ModelContext(old)
            let lecture = PreKindSchema.Meeting(title: "Lecture 3", startedAt: started)
            lecture.courseName = "BIO 101"
            lecture.aiNotes = "## Announcements and deadlines\n- Read chapter 3"
            ctx.insert(lecture)
            let mark = PreKindSchema.MomentMarker(at: started.addingTimeInterval(30), kind: "test")
            ctx.insert(mark)
            mark.meeting = lecture
            try ctx.save()
        }
        let container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(url: url))
        let context = container.mainContext
        let m = try XCTUnwrap(try context.fetch(FetchDescriptor<Meeting>()).first)
        XCTAssertEqual(m.title, "Lecture 3")
        XCTAssertEqual(m.marks.map(\.kind), ["test"], "old marks are still marks")
        XCTAssertTrue(m.typedNotes.isEmpty)
        XCTAssertTrue(m.doneItems.isEmpty)
        // A typed note and a done mark on the migrated store
        let note = MomentMarker(at: started.addingTimeInterval(40), kind: .note, note: "typed after the update")
        context.insert(note); note.meeting = m
        let item = try XCTUnwrap(NotebookItems.items(of: m).first)
        NotebookItems.setDone(true, item: item, meeting: m, context: context)
        try context.save()
        let again = ModelContext(container)
        XCTAssertEqual(try again.fetchCount(FetchDescriptor<NotebookItemDone>()), 1)
        XCTAssertEqual(try again.fetch(FetchDescriptor<MomentMarker>(predicate: #Predicate { $0.kind == "note" })).first?.note, "typed after the update")
    }
}
