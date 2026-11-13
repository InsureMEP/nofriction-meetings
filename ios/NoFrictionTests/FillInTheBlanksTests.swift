import SwiftData
import XCTest
@testable import noFriction

// Fill in the blanks (docs/design/FADELL_V2.md §11), iPhone side. The pure
// parts mirror the Mac's tests ("Fill in the blanks (fill.rs)" in
// src-tauri/src/meeting_notes.rs): the same fixture, the same strings, so
// each device recognises the sections the other one wrote. The AI is always
// a mock here.

/// The Mac's `FILL` fixture
private let fillAnswer = #"""
{"missed":[
        {"at":"0:40","point":"Osmosis moves water toward the higher solute concentration."},
        {"at":"0:10","point":"The exam covers chapters three and four."},
        {"at":"59:00","point":"A time that isn't in this recording."},
        {"at":"0:10","point":"the exam covers chapters three and four."},
        "A point with no time."],
      "more":[
        {"about":"Sodium-potassium pump","detail":"It moves three sodium out and two potassium in for each ATP.","at":"0:20","from":"recording"},
        {"about":"Active transport","detail":"Transport that needs energy because it goes against the gradient.","at":null,"from":"background"},
        {"about":"sodium-potassium pump","detail":"A second entry for the same idea."},
        {"about":"","detail":"Nothing it is about."}]}
"""#

/// What that answer becomes in a Class's notes (60 seconds long), on either device
private let fillSections = """
    ## What you missed
    - [0:10] The exam covers chapters three and four.
    - [0:40] Osmosis moves water toward the higher solute concentration.
    - A time that isn't in this recording.
    - A point with no time.

    ## More detail
    - **Sodium-potassium pump**: It moves three sodium out and two potassium in for each ATP. [0:20]
    - **Active transport**: Transport that needs energy because it goes against the gradient. (background, not said in the recording)
    """

private let uuidPattern = try! NSRegularExpression(pattern: "[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}")

private func hasUUID(_ s: String) -> Bool {
    uuidPattern.firstMatch(in: s, range: NSRange(s.startIndex..., in: s)) != nil
}

private func count(_ needle: String, in text: String) -> Int {
    text.components(separatedBy: needle).count - 1
}

/// Answers by the system prompt; records every request. The fill request
/// gets `answers` in turn (the last one again after that).
private final class FillMockAI: @unchecked Sendable {
    struct Call { var messages: [ChatMessage]; var maxTokens: Int; var temperature: Double }
    private let lock = NSLock()
    private var _calls: [Call] = []
    private var answers: [String]
    /// Every request fails with this
    var error: Error?
    /// The first fill requests fail with these, in turn
    var failures: [Error] = []
    /// What a Make notes request answers
    var notes = "## Summary\nCells make ATP."
    /// Runs before each answer (on the caller's task)
    var before: (@Sendable () async -> Void)?

    init(_ answers: [String] = [fillAnswer]) { self.answers = answers }

    var calls: [Call] { lock.withLock { _calls } }
    var condenseCalls: [Call] { calls.filter { $0.messages[0].content.hasPrefix("You condense") } }
    var fillCalls: [Call] { calls.filter { $0.messages[0].content.contains("NOTES SO FAR is what they already have") } }
    var sent: String { calls.flatMap(\.messages).map(\.content).joined(separator: "\n") }

    /// A condensed part keeps its first and last line, so the test can see what survived
    static func condensed(_ user: String) -> String {
        let body = user.components(separatedBy: "TRANSCRIPT:\n").last ?? ""
        let lines = body.split(whereSeparator: \.isNewline).map(String.init).filter { $0.hasPrefix("[") }
        guard let first = lines.first, let last = lines.last else { return "[0:00] nothing" }
        return first == last ? first : first + "\n" + last
    }

    var complete: MeetingAI.Complete {
        { messages, maxTokens, temperature in
            if let before = self.before { await before() }
            self.lock.withLock { self._calls.append(Call(messages: messages, maxTokens: maxTokens, temperature: temperature)) }
            if let error = self.error { throw error }
            let system = messages[0].content
            if system.hasPrefix("You condense") { return Self.condensed(messages[1].content) }
            if system.contains("NOTES SO FAR is what they already have") {
                let failure: Error? = self.lock.withLock { self.failures.isEmpty ? nil : self.failures.removeFirst() }
                if let failure { throw failure }
                return self.lock.withLock { self.answers.count > 1 ? self.answers.removeFirst() : (self.answers.first ?? "") }
            }
            if system.contains("Answer in Markdown with exactly these sections") { return self.notes }
            return #"{"topics": [{"label": "Cells", "confidence": 0.9}]}"#
        }
    }
}

// MARK: - The prompt

final class FillPromptTests: XCTestCase {
    private let kinds: [RecordingKind] = [.class, .meeting, .personal]

    /// The Mac's `fill_prompts_follow_the_type_and_only_a_class_may_get_background`.
    func testThePromptsFollowTheTypeAndOnlyAClassMayGetBackground() {
        let lecture = FillInTheBlanks.basePrompt(for: .class, notebook: "BIO 101")
        XCTAssertTrue(lecture.contains("\"BIO 101\"") && lecture.contains("\"from\": \"background\""))
        XCTAssertTrue(lecture.contains("Never present background as something that was said"))
        // A class may get what a textbook says, and it must be marked
        XCTAssertTrue(lecture.contains("you may add what a standard textbook says about it"))
        XCTAssertTrue(lecture.contains("Mark that entry \"from\": \"background\" and set \"at\" to null."))
        for kind in [RecordingKind.meeting, .personal] {
            let p = FillInTheBlanks.basePrompt(for: kind, notebook: nil)
            XCTAssertTrue(p.contains("Use only what was said. Do not add outside knowledge, advice or opinions."), "\(kind)")
            XCTAssertFalse(p.contains("textbook"), "\(kind)")
            XCTAssertFalse(p.contains("background"), "\(kind)")
        }
        // Every type is told the user's own lines are theirs, and never to invent
        for kind in kinds {
            let p = FillInTheBlanks.basePrompt(for: kind, notebook: nil)
            XCTAssertTrue(p.contains("Never rewrite or repeat the lines"), "\(kind)")
            XCTAssertTrue(p.hasSuffix("- Do NOT invent anything that was said."), "\(kind)")
            XCTAssertTrue(p.contains("only things actually said, each with the time of the line it comes from"), "\(kind)")
            // The same JSON as the Mac
            XCTAssertTrue(p.contains(FillInTheBlanks.shape), "\(kind)")
            XCTAssertTrue(p.contains("at most 12.") && p.contains("at most 8,"), "\(kind)")
        }
        XCTAssertTrue(FillInTheBlanks.shape.contains(#""missed": [{"at": "m:ss", "point": "one thing that was said and NOTES SO FAR don't have"}],"#))
        XCTAssertTrue(FillInTheBlanks.shape.contains(#""more": [{"about": "a key idea or term from NOTES SO FAR", "detail": "two to four sentences that explain it further", "at": "m:ss", "from": "recording"}]"#))
        // The notebook is named when there is one
        XCTAssertTrue(lecture.hasPrefix("You are helping a student complete their notes from a class (\"BIO 101\"). NOTES SO FAR is what they already have: "
                                        + "the notes made from the recording and the lines they typed themselves. Below it is everything that was said."))
        XCTAssertTrue(FillInTheBlanks.basePrompt(for: .class, notebook: "  ").hasPrefix("You are helping a student complete their notes from a class. NOTES SO FAR"))
        XCTAssertTrue(FillInTheBlanks.basePrompt(for: .personal, notebook: "Health")
            .hasPrefix("You are helping someone complete their notes from a personal recording (\"Health\"): a conversation, appointment, talk or idea."))
        XCTAssertTrue(FillInTheBlanks.basePrompt(for: .meeting, notebook: "Acme").hasPrefix("You are helping someone complete their notes from a meeting. NOTES SO FAR"))
    }

    /// The Mac's `the_fill_prompt_shows_what_the_user_already_has_and_never_an_id`.
    func testTheRequestShowsWhatTheUserAlreadyHasThenTheTranscript() {
        let input = NotesInput(study: StudyInput(
            title: "Membranes", recordingKind: .class, courseName: "BIO 101", durationMs: 60_000,
            lines: [.init(ms: 0, text: "Cells have membranes."), .init(ms: 20_000, text: "The pump moves sodium out.")],
            marks: [.init(ms: 20_000, kind: .note, note: "pump: 3 Na out")]))
        // The type's prompt, then the reading rules the notes prompt uses
        let system = MeetingAI.fillSystem(for: .class, notebook: "BIO 101")
        XCTAssertEqual(system, FillInTheBlanks.basePrompt(for: .class, notebook: "BIO 101") + "\n" + MeetingAI.readingRules(for: .class))
        XCTAssertTrue(system.contains("Text shown as [stricken from the record] was removed by the student: never guess at or mention what it said."))
        for kind in kinds {
            XCTAssertTrue(MeetingAI.notesSystem(for: kind, typedNotes: true).contains(MeetingAI.readingRules(for: kind)), "the notes prompt's own rules")
            XCTAssertTrue(MeetingAI.fillSystem(for: kind, notebook: nil).hasSuffix(MeetingAI.readingRules(for: kind)))
        }
        // The header, NOTES SO FAR, the transcript
        let have = FillInTheBlanks.haveBlock(notes: "## Summary\nA lecture on membranes.", typed: input.typed)
        let user = MeetingAI.fillUserMessage(input, have: have, condensed: false, body: "[0:00] Cells have membranes.")
        XCTAssertEqual(user, """
            Class: Membranes
            Notebook: BIO 101
            Length: 1:00

            NOTES SO FAR:
            ## Summary
            A lecture on membranes.

            Typed by the user while listening:
            - [0:20] pump: 3 Na out

            TRANSCRIPT:
            [0:00] Cells have membranes.

            JSON RESPONSE:
            """)
        XCTAssertFalse(hasUUID(system + user), "ids never go in a prompt")
        // Nothing yet: it says so instead of leaving the block empty
        XCTAssertTrue(MeetingAI.fillUserMessage(input, have: " ", condensed: true, body: "x")
            .contains("NOTES SO FAR:\n(none)\n\nNOTES (condensed from the whole transcript, with times):\nx\n\nJSON RESPONSE:"))
    }

    func testNotesSoFarLeaveOutAnEarlierFillAndAreCapped() {
        let notes = "## What you wrote\n- **pump: 3 Na out**: The pump moves sodium.\n\n## Summary\nMembranes.\n\n"
            + "## What you missed\n- [0:10] The exam covers chapter three.\n\n## More detail\n- **Osmosis**: water moves. [0:40]"
        let typed: [StudyInput.Mark] = [.init(ms: 20_000, kind: .note, note: "pump: 3 Na out"), .init(ms: 30_000, kind: .note, note: "ask about osmosis")]
        // A refill doesn't feed itself; a typed line the notes already list isn't repeated; one they don't list is shown
        XCTAssertEqual(FillInTheBlanks.haveBlock(notes: notes, typed: typed),
                       "## What you wrote\n- **pump: 3 Na out**: The pump moves sodium.\n\n## Summary\nMembranes.\n\n"
                       + "Typed by the user while listening:\n- [0:30] ask about osmosis")
        XCTAssertEqual(FillInTheBlanks.haveBlock(notes: nil, typed: []), "")
        XCTAssertEqual(FillInTheBlanks.haveBlock(notes: "## What you missed\n- only what an earlier fill wrote", typed: []), "")
        XCTAssertEqual(FillInTheBlanks.haveBlock(notes: nil, typed: typed),
                       "Typed by the user while listening:\n- [0:20] pump: 3 Na out\n- [0:30] ask about osmosis")
        // 6,000 characters at most
        let long = "## Summary\n" + String(repeating: "word ", count: 2000)
        let capped = FillInTheBlanks.haveBlock(notes: long, typed: [])
        XCTAssertTrue(capped.hasPrefix("## Summary\nword word") && capped.hasSuffix("\n(more notes follow)"))
        XCTAssertEqual(capped.count, 6_000 + "\n(more notes follow)".count)
        // Fewer on a small model window: the recording keeps most of the room, and the request still fits
        let system = MeetingAI.fillSystem(for: .class, notebook: nil)
        XCTAssertEqual(FillInTheBlanks.haveCap(contextTokens: 32_768, maxTokens: 2500, system: system), 6_000)
        let small = FillInTheBlanks.haveCap(contextTokens: AppleOnDevice.contextTokens, maxTokens: 1228, system: system)
        XCTAssertTrue((300...3_072).contains(small), "never more than the Mac's quarter of the window: \(small)")
        XCTAssertLessThan(small * 3, ContextFit.budgetChars(contextTokens: AppleOnDevice.contextTokens, maxTokens: 1228) - system.count + 1, "a third of the room at most")
        XCTAssertEqual(MeetingAI.fillMaxTokens(contextTokens: 32_768), 2500)
        XCTAssertEqual(MeetingAI.fillMaxTokens(contextTokens: 4096), 1228)
    }
}

// MARK: - The answer

final class FillParseTests: XCTestCase {
    /// The Mac's `fill_answers_are_parsed_tolerantly_and_background_stays_in_class`.
    func testAnswersAreParsedTolerantlyAndBackgroundStaysInAClass() throws {
        let lecture = try FillInTheBlanks.parse(fillAnswer, kind: .class, durationMs: 60_000)
        // In the order said, the repeat dropped, the time outside the recording dropped, the ones without a time last
        XCTAssertEqual(lecture.missed, [
            .init(atMs: 10_000, text: "The exam covers chapters three and four."),
            .init(atMs: 40_000, text: "Osmosis moves water toward the higher solute concentration."),
            .init(atMs: nil, text: "A time that isn't in this recording."),
            .init(atMs: nil, text: "A point with no time."),
        ])
        // The repeat and the one about nothing are dropped
        XCTAssertEqual(lecture.more, [
            .init(about: "Sodium-potassium pump", detail: "It moves three sodium out and two potassium in for each ATP.", atMs: 20_000, background: false),
            .init(about: "Active transport", detail: "Transport that needs energy because it goes against the gradient.", atMs: nil, background: true),
        ])
        // A meeting and a personal recording never get what wasn't said
        for kind in [RecordingKind.meeting, .personal] {
            let other = try FillInTheBlanks.parse(fillAnswer, kind: kind, durationMs: 60_000)
            XCTAssertEqual(other.missed, lecture.missed, "\(kind)")
            XCTAssertEqual(other.more.map(\.about), ["Sodium-potassium pump"], "\(kind)")
            XCTAssertFalse(other.more[0].background, "\(kind)")
        }
        // Nothing missing is an answer; no lists at all, not an object, or no JSON, is not
        XCTAssertTrue(try FillInTheBlanks.parse(#"{"missed":[],"more":[]}"#, kind: .class, durationMs: 0).isEmpty)
        XCTAssertTrue(try FillInTheBlanks.parse(#"{"more":[]}"#, kind: .class, durationMs: 0).isEmpty)
        XCTAssertThrowsError(try FillInTheBlanks.parse(#"{"summary":"notes again"}"#, kind: .class, durationMs: 0)) {
            XCTAssertEqual($0 as? FillInTheBlanks.ParseFailure, .noLists)
        }
        XCTAssertThrowsError(try FillInTheBlanks.parse("[1,2]", kind: .class, durationMs: 0)) {
            XCTAssertEqual($0 as? FillInTheBlanks.ParseFailure, .notAnObject)
        }
        XCTAssertThrowsError(try FillInTheBlanks.parse("I can't help with that.", kind: .class, durationMs: 0)) {
            XCTAssertEqual($0 as? StudyParse.Failure, .noJSON)
        }
        // Sizes are capped
        let points = (0..<40).map { #"{"at":"0:\#(String(format: "%02d", $0))","point":"point \#($0)"}"# }.joined(separator: ",")
        let ideas = (0..<30).map { #"{"about":"idea \#($0)","detail":"detail \#($0)"}"# }.joined(separator: ",")
        let many = try FillInTheBlanks.parse("{\"missed\":[\(points)],\"more\":[\(ideas)]}", kind: .class, durationMs: 0)
        XCTAssertEqual(many.missed.count, FillInTheBlanks.maxMissed)
        XCTAssertEqual(many.missed.count, 12)
        XCTAssertEqual(many.missed.first?.text, "point 0")
        XCTAssertEqual(many.more.count, FillInTheBlanks.maxMore)
        XCTAssertEqual(many.more.count, 8)
        let big = try FillInTheBlanks.parse(
            "{\"missed\":[{\"point\":\"\(String(repeating: "p", count: 900))\"}],\"more\":[{\"about\":\"\(String(repeating: "a", count: 900))\",\"detail\":\"\(String(repeating: "d", count: 900))\"}]}",
            kind: .class, durationMs: 0)
        XCTAssertEqual(big.missed[0].text.count, 400)
        XCTAssertEqual(big.more[0].about.count, 160)
        XCTAssertEqual(big.more[0].detail.count, 700)
        XCTAssertTrue(big.more[0].detail.hasSuffix("…"))
    }

    func testOtherShapesAModelWrites() throws {
        // A think block, a code fence, a trailing comma, other names for the fields, a time in seconds
        let raw = """
            <think>What did the notes miss?</think>
            ```json
            {"missed": [{"time": "[0:05]", "text": "- The quiz is on Friday."}, {"at": 12, "point": "Bring a calculator."}, 7, true, null,],
             "more": [{"term": "Osmosis", "explanation": "Water moves across the membrane.", "time": "0:06", "source": "recording"},
                      "not an entry", {"concept": "Diffusion", "text": "Down the gradient."}],}
            ```
            """
        let got = try FillInTheBlanks.parse(raw, kind: .class, durationMs: 60_000)
        XCTAssertEqual(got.missed, [
            .init(atMs: 5_000, text: "The quiz is on Friday."),
            .init(atMs: 12_000, text: "Bring a calculator."),
            .init(atMs: nil, text: "7"),
        ])
        // An entry that says neither where it comes from nor when it was said is not taken as said
        XCTAssertEqual(got.more, [
            .init(about: "Osmosis", detail: "Water moves across the membrane.", atMs: 6_000, background: false),
            .init(about: "Diffusion", detail: "Down the gradient.", atMs: nil, background: true),
        ])
        XCTAssertEqual(try FillInTheBlanks.parse(raw, kind: .meeting, durationMs: 60_000).more.map(\.about), ["Osmosis"])
        // A list that isn't a list holds nothing; the answer still counts
        XCTAssertTrue(try FillInTheBlanks.parse(#"{"missed":"none","more":null}"#, kind: .class, durationMs: 0).isEmpty)
    }

    func testATimeIsInsideTheRecordingOrDropped() throws {
        func at(_ value: String, duration: Int = 60_000) throws -> Int? {
            try FillInTheBlanks.parse("{\"missed\":[{\"at\":\(value),\"point\":\"x\"}]}", kind: .class, durationMs: duration).missed.first?.atMs
        }
        XCTAssertEqual(try at("\"0:59\""), 59_000)
        XCTAssertEqual(try at("\"1:04\""), 64_000, "a few seconds past the end is the transcriber's last line")
        XCTAssertNil(try at("\"1:06\""))
        XCTAssertNil(try at("\"-1:05\""))
        XCTAssertNil(try at("-3"))
        XCTAssertNil(try at("true"))
        XCTAssertNil(try at("null"))
        XCTAssertNil(try at("\"later\""))
        XCTAssertEqual(try at("\"59:00\"", duration: 0), 3_540_000, "an unknown length drops nothing")
        // A number no recording has: dropped, and the app doesn't stop
        XCTAssertNil(try at("1e30"))
        XCTAssertNil(try at("\"1e30\""))
        XCTAssertNil(try at("\"99999999999999999999999:00\"", duration: 0))
    }

    /// The Mac's `sly` answers: whatever isn't recognisably from the recording
    /// is background. It never passes as said, however the answer words it.
    func testWhatIsNotRecognisablyFromTheRecordingIsBackground() throws {
        let sly = #"""
            {"missed":[],"more":[
            {"about":"A","detail":"d","from":"textbook","at":"0:10"},
            {"about":"B","detail":"d","from":"outside knowledge"},
            {"about":"C","detail":"d","background":"true","from":"recording","at":"0:10"},
            {"about":"D","detail":"d","source":"background"},
            {"about":"E","detail":"d"},
            {"about":"F","detail":"d","at":"0:30"},
            {"about":"G","detail":"d","from":"Transcript","at":"0:40"},
            {"about":"H","detail":"d","from":"recording","background":false}]}
            """#
        let lecture = try FillInTheBlanks.parse(sly, kind: .class, durationMs: 60_000)
        XCTAssertEqual(lecture.more.map(\.about), ["A", "B", "C", "D", "E", "F", "G", "H"])
        XCTAssertEqual(lecture.more.map(\.background), [true, true, true, true, true, false, false, false])
        // Background never has a time, even when the answer gave one
        XCTAssertEqual(lecture.more.map(\.atMs), [nil, nil, nil, nil, nil, 30_000, 40_000, nil])
        for kind in [RecordingKind.meeting, .personal] {
            XCTAssertEqual(try FillInTheBlanks.parse(sly, kind: kind, durationMs: 60_000).more.map(\.about), ["F", "G", "H"], "\(kind)")
        }
        // Other ways of marking it
        let marked = try FillInTheBlanks.parse(
            #"{"more":[{"about":"A","detail":"d","at":"0:10","from":" Background knowledge"},{"about":"B","detail":"d","at":"0:20","background":true},{"about":"C","detail":"d","at":"0:30","background":"yes"},{"about":"D","detail":"d","at":"0:40","from":null,"source":"background"},{"about":"E","detail":"d","at":"0:50","background":"no","from":"the lecture"},{"about":"F","detail":"d","at":"0:55","from":"lecture"}]}"#,
            kind: .class, durationMs: 60_000)
        XCTAssertEqual(marked.more.map(\.background), [true, true, true, false, true, false])
        XCTAssertEqual(marked.more.map(\.atMs), [nil, nil, nil, 40_000, nil, 55_000])
    }
}

// MARK: - The two sections

final class FillSectionsTests: XCTestCase {
    /// The Mac's `filled_sections_cross_to_the_iphone_under_the_shared_headings`:
    /// character for character what `fill::markdown` writes.
    func testTheTwoSectionsReadAsTheMacWritesThem() throws {
        XCTAssertEqual(FillInTheBlanks.missedHeading, "What you missed")
        XCTAssertEqual(FillInTheBlanks.moreHeading, "More detail")
        XCTAssertEqual(FillInTheBlanks.backgroundNote, "background, not said in the recording")
        let filled = try FillInTheBlanks.parse(fillAnswer, kind: .class, durationMs: 60_000)
        let md = try XCTUnwrap(FillInTheBlanks.markdown(filled))
        XCTAssertEqual(md, fillSections)
        // The Mac test's own assertions
        XCTAssertTrue(md.hasPrefix("## What you missed\n- [0:10] The exam covers chapters three and four.\n"), md)
        XCTAssertTrue(md.contains("\n- A point with no time.\n\n## More detail\n"), md)
        XCTAssertTrue(md.contains("- **Sodium-potassium pump**: It moves three sodium out and two potassium in for each ATP. [0:20]"), md)
        XCTAssertTrue(md.hasSuffix("against the gradient. (background, not said in the recording)"), md)
        XCTAssertNil(FillInTheBlanks.markdown(.init()))
        // One list only; an hour's time; one line each; no bold marks inside the bold
        XCTAssertEqual(FillInTheBlanks.markdown(.init(missed: [.init(atMs: 3_725_000, text: " two\n lines ")])), "## What you missed\n- [1:02:05] two lines")
        XCTAssertEqual(FillInTheBlanks.markdown(.init(more: [.init(about: "**Bold**  idea", detail: "d\te", atMs: nil)])), "## More detail\n- **Bold idea**: d e")
    }

    /// The Mac's `the_two_sections_are_taken_out_of_notes_text_cleanly`.
    func testTheSectionsAreTakenOutOfNotesCleanlyWhereverTheySit() {
        let md = "## What you wrote\n- **pump**: three sodium out\n\n## Summary\nMembranes.\n\n## What you missed\n- [0:10] The exam covers chapter three.\n\n## Key concepts\n- Osmosis\n\n## More detail\n- **Osmosis**: water moves. [0:40]\n"
        XCTAssertEqual(FillInTheBlanks.strip(md), "## What you wrote\n- **pump**: three sodium out\n\n## Summary\nMembranes.\n\n## Key concepts\n- Osmosis")
        // Nothing of ours: unchanged (but for trailing blank lines)
        XCTAssertEqual(FillInTheBlanks.strip("## Summary\nMembranes.\n\n"), "## Summary\nMembranes.")
        // Whatever the case of the heading
        XCTAssertEqual(FillInTheBlanks.strip("Intro\n## what you missed\n- x\n## MORE DETAIL ##\n- y"), "Intro")
        XCTAssertEqual(FillInTheBlanks.strip(""), "")
        // A bold line is a heading too: it ends a section of ours, and can be one
        XCTAssertEqual(FillInTheBlanks.strip("## What you missed\n- x\n**Key concepts**\n- Osmosis\n**More detail**\n- y"), "**Key concepts**\n- Osmosis")
        // The words inside a bullet or a paragraph are not a heading
        let prose = "## Summary\nSee ## What you missed below.\n- **More detail** on osmosis: later"
        XCTAssertEqual(FillInTheBlanks.strip(prose), prose)
        XCTAssertFalse(FillInTheBlanks.has(prose))
        // First, with Windows line ends, and as a deeper heading
        XCTAssertEqual(FillInTheBlanks.strip("## More detail\r\n- **x**: y\r\n\r\n## Summary\r\nS"), "## Summary\nS")
        XCTAssertEqual(FillInTheBlanks.strip("## Summary\nS\n\n### What you missed\n- x\n\n"), "## Summary\nS")
        // Only those two headings: a longer title is someone else's
        for other in ["## What you missed out on\n- x", "## Summary\nWhat you missed\n- x", "**More details**\n- x"] {
            XCTAssertEqual(FillInTheBlanks.strip(other), other)
            XCTAssertFalse(FillInTheBlanks.has(other), other)
        }
        XCTAssertTrue(FillInTheBlanks.has(md))
        XCTAssertTrue(FillInTheBlanks.has("**Summary**\nCells divide.\n\n## More detail\n- **x**: y"), "also when the notes came from the Mac")
        // What every removal calls: exactly as they were when there is nothing to remove
        XCTAssertEqual(FillInTheBlanks.removed(from: "## Summary\nMembranes.\n\n"), "## Summary\nMembranes.\n\n")
        XCTAssertEqual(FillInTheBlanks.removed(from: md), FillInTheBlanks.strip(md))
        // The button says which it is
        XCTAssertEqual(FillInTheBlanks.buttonTitle(notes: "## Summary\nS"), "Fill in the blanks")
        XCTAssertEqual(FillInTheBlanks.buttonTitle(notes: md), "Fill again")
        XCTAssertEqual(FillInTheBlanks.buttonTitle(notes: "**Summary**\nCells divide.\n\n## What you missed\n- [0:10] x"), "Fill again")
        XCTAssertEqual(FillInTheBlanks.buttonTitle(notes: nil), "Fill in the blanks")
    }

    func testARefillReplacesTheSectionsAndLeavesTheRestAlone() throws {
        let notes = "## What you wrote\n- **pump**: three sodium out\n\n## Summary\nMembranes.\n\n## Announcements and deadlines\n- Read chapter 3 by Friday"
        let first = FillInTheBlanks.replacing(in: notes, with: "## What you missed\n- a")
        XCTAssertEqual(first, notes + "\n\n## What you missed\n- a")
        let second = FillInTheBlanks.replacing(in: first, with: "## What you missed\n- b\n\n## More detail\n- **x**: y")
        XCTAssertEqual(second, notes + "\n\n## What you missed\n- b\n\n## More detail\n- **x**: y")
        XCTAssertEqual(count("## What you missed", in: second), 1)
        // Wherever the earlier ones sat (notes from the Mac, edited by hand): the new ones go at the end
        let middle = "## What you missed\n- old\n\n## Summary\nMembranes.\n\n## More detail\n- **old**: old\n\n## Key concepts\n- Osmosis"
        XCTAssertEqual(FillInTheBlanks.replacing(in: middle, with: "## More detail\n- **x**: y"),
                       "## Summary\nMembranes.\n\n## Key concepts\n- Osmosis\n\n## More detail\n- **x**: y")
        XCTAssertEqual(FillInTheBlanks.replacing(in: second, with: nil), notes)
        XCTAssertEqual(FillInTheBlanks.replacing(in: "", with: "## More detail\n- **x**: y"), "## More detail\n- **x**: y")
        // "What you wrote" and the items are as they were
        XCTAssertEqual(WhatYouWrote.parse(second)?.bullets, WhatYouWrote.parse(notes)?.bullets)
        XCTAssertEqual(WhatYouWrote.parse(second)?.bullets, ["**pump**: three sodium out"])
        XCTAssertEqual(NotebookItems.texts(inNotes: second), ["Read chapter 3 by Friday"])
    }

    /// The notes view shows them with its own renderer: a heading, bullets,
    /// the time as written and the idea in bold.
    func testTheSectionsRenderThroughTheNotesView() throws {
        let blocks = NotesMarkdown.blocks(fillSections)
        XCTAssertEqual(blocks, [
            .heading("What you missed", level: 2),
            .bullet("[0:10] The exam covers chapters three and four.", indent: 0),
            .bullet("[0:40] Osmosis moves water toward the higher solute concentration.", indent: 0),
            .bullet("A time that isn't in this recording.", indent: 0),
            .bullet("A point with no time.", indent: 0),
            .heading("More detail", level: 2),
            .bullet("**Sodium-potassium pump**: It moves three sodium out and two potassium in for each ATP. [0:20]", indent: 0),
            .bullet("**Active transport**: Transport that needs energy because it goes against the gradient. (background, not said in the recording)", indent: 0),
        ])
        // "[0:10]" stays text: not a link, nothing swallowed
        let timed = NotesMarkdown.inline("[0:10] The exam covers chapters three and four.")
        XCTAssertEqual(String(timed.characters), "[0:10] The exam covers chapters three and four.")
        XCTAssertTrue(timed.runs.allSatisfy { $0.link == nil })
        // The idea is bold, the rest plain, the time at the end as written
        let more = NotesMarkdown.inline("**Sodium-potassium pump**: It moves three sodium out and two potassium in for each ATP. [0:20]")
        XCTAssertEqual(String(more.characters), "Sodium-potassium pump: It moves three sodium out and two potassium in for each ATP. [0:20]")
        let bold = more.runs.filter { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) ?? false }
        XCTAssertEqual(bold.map { String(more[$0.range].characters) }, ["Sodium-potassium pump"])
        let background = NotesMarkdown.inline("**Active transport**: Transport that needs energy. (background, not said in the recording)")
        XCTAssertTrue(String(background.characters).hasSuffix("(background, not said in the recording)"))
        // The items the notes found keep their ticks: the new bullets are never items
        let notes = "## Summary\nS\n\n## Action items\n- [Alex] Send the deck (Friday)"
        XCTAssertEqual(NotebookItems.texts(inNotes: notes + "\n\n" + fillSections), NotebookItems.texts(inNotes: notes))
        XCTAssertEqual(NotebookItems.texts(inNotes: notes + "\n\n" + fillSections).count, 1)
    }
}

// MARK: - The requests

final class FillGenerationTests: XCTestCase {
    private func input(lines: Int, kind: RecordingKind = .class) -> NotesInput {
        let marks: [StudyInput.Mark] = [.init(ms: 30_000, kind: .note, note: "typed note number 1"),
                                        .init(ms: 60_000, kind: .test, note: "phases of mitosis")]
        return NotesInput(study: StudyInput(title: "Biology", recordingKind: kind, courseName: "BIO 101", durationMs: max(60_000, lines * 6000),
                                            lines: (0..<lines).map { .init(ms: $0 * 6000, text: "sentence \($0) about the cell cycle and mitosis phases") },
                                            marks: marks))
    }

    func testAShortRecordingIsOneRequestWithTheWholeTranscript() async throws {
        let ai = FillMockAI()
        let filled = try await MeetingAI.fill(input(lines: 3), notes: "## Summary\nThe cell cycle.", contextTokens: 32_768, complete: ai.complete)
        XCTAssertEqual(filled, try FillInTheBlanks.parse(fillAnswer, kind: .class, durationMs: 60_000))
        XCTAssertEqual(ai.calls.count, 1)
        let call = ai.calls[0]
        XCTAssertEqual(call.messages.map(\.role), ["system", "user"])
        XCTAssertEqual(call.messages[0].content, MeetingAI.fillSystem(for: .class, notebook: "BIO 101"))
        let user = call.messages[1].content
        XCTAssertTrue(user.contains("\nNOTES SO FAR:\n## Summary\nThe cell cycle.\n\nTyped by the user while listening:\n- [0:30] typed note number 1\n\nTRANSCRIPT:\n[0:00] sentence 0"), user)
        XCTAssertTrue(user.hasSuffix("[0:12] sentence 2 about the cell cycle and mitosis phases\n\nJSON RESPONSE:"), user)
        XCTAssertEqual(call.maxTokens, 2500)
        XCTAssertEqual(call.temperature, 0.3)
        // A meeting's answer never keeps background, whatever the model wrote
        let meeting = try await MeetingAI.fill(input(lines: 3, kind: .meeting), notes: "## Summary\nS", contextTokens: 32_768, complete: FillMockAI().complete)
        XCTAssertFalse(meeting.more.contains(where: \.background))
    }

    /// An hour on the 4K on-device window: read in parts, never trimmed by the client.
    func testALongRecordingIsCondensedInPartsAndNothingIsCutOff() async throws {
        let ai = FillMockAI()
        let i = input(lines: 600)
        let window = AppleOnDevice.contextTokens
        let notes = "## Summary\n" + String(repeating: "The lecture covered the cell cycle. ", count: 200)
        XCTAssertGreaterThan(i.study.transcriptLines.joined(separator: "\n").count, ContextFit.budgetChars(contextTokens: window, maxTokens: 1228),
                             "the transcript alone doesn't fit the window")
        let labels = Labels()
        _ = try await MeetingAI.fill(i, notes: notes, contextTokens: window, complete: ai.complete) { await labels.add($0) }

        XCTAssertGreaterThanOrEqual(ai.condenseCalls.count, 2, "the review guide's map step ran")
        XCTAssertEqual(ai.fillCalls.count, 1)
        // No request exceeds what the client can send without trimming
        for call in ai.calls {
            let chars = call.messages.reduce(0) { $0 + $1.content.count }
            XCTAssertLessThanOrEqual(chars, ContextFit.budgetChars(contextTokens: window, maxTokens: call.maxTokens), "fits beside its answer")
            XCTAssertEqual(ContextFit.fit(call.messages, contextTokens: window, maxTokens: call.maxTokens), call.messages, "the client trims nothing")
            XCTAssertFalse(call.messages.contains { $0.content.contains("middle of transcript trimmed") })
        }
        // Every part of the transcript was read
        let read = ai.condenseCalls.map { $0.messages[1].content }.joined(separator: "\n")
        for n in [0, 150, 299, 300, 450, 599] {
            XCTAssertTrue(read.contains("sentence \(n) about"), "line \(n) was never sent")
        }
        // The pass works from the condensed material: the start and the tail are there
        let final = try XCTUnwrap(ai.fillCalls.first).messages[1].content
        XCTAssertTrue(final.contains("NOTES (condensed from the whole transcript, with times):\n"))
        XCTAssertFalse(final.contains("\nTRANSCRIPT:\n"))
        XCTAssertTrue(final.contains("[0:00] sentence 0 about"), "the start")
        XCTAssertTrue(final.contains("[59:54] sentence 599 about"), "the tail of the transcript is represented")
        // The notes took their share of a small window and say so; the line the user typed kept its place
        XCTAssertTrue(final.contains("NOTES SO FAR:\n## Summary\nThe lecture covered the cell cycle."))
        XCTAssertTrue(final.contains("\n(more notes follow)\n\nTyped by the user while listening:\n- [0:30] typed note number 1\n\nNOTES (condensed"), final)
        // The progress line says which run it is, all the way
        let said = await labels.all
        XCTAssertTrue(said.contains { $0.hasPrefix("Filling in the blanks… Reading the lecture (1 of ") }, "\(said)")
        XCTAssertEqual(said.last, "Filling in the blanks…")
        XCTAssertTrue(said.allSatisfy { $0.hasPrefix("Filling in the blanks…") })
    }

    private actor Labels {
        private(set) var all: [String] = []
        func add(_ label: String) { all.append(label) }
    }

    func testAnUnusableAnswerIsAskedForOnceMoreThenReportedWithoutQuotingIt() async throws {
        // Asked once more, saying why in fixed words
        let ai = FillMockAI(["I can't help with that secret-sounding thing.", fillAnswer])
        let filled = try await MeetingAI.fill(input(lines: 3), notes: "## Summary\nS", contextTokens: 32_768, complete: ai.complete)
        XCTAssertEqual(filled.missed.count, 4)
        XCTAssertEqual(ai.calls.count, 2)
        let first = ai.calls[0].messages[1].content, second = ai.calls[1].messages[1].content
        XCTAssertEqual(second, first + "\n\nYour previous answer couldn't be used (The answer had no JSON in it). "
                       + "Answer again with only the JSON object in the shape described, and nothing else.")
        XCTAssertFalse(second.contains("secret-sounding"), "the answer is never sent back")
        XCTAssertEqual(ai.calls.map(\.temperature), [0.3, 0.4])
        // Twice: reported, without a word of the answer
        let bad = FillMockAI(["no json here: tangerine"])
        do {
            _ = try await MeetingAI.fill(input(lines: 3), notes: "## Summary\nS", contextTokens: 32_768, complete: bad.complete)
            XCTFail("should throw")
        } catch {
            XCTAssertEqual(error as? FillInTheBlanks.Failure, .unusable)
            XCTAssertFalse(error.localizedDescription.contains("tangerine"))
            XCTAssertTrue(error.localizedDescription.hasPrefix("Couldn't fill in the blanks"))
        }
        XCTAssertEqual(bad.calls.count, 2)
        // An answer with neither list is one of those
        let notes = FillMockAI([#"{"summary":"notes again"}"#])
        do {
            _ = try await MeetingAI.fill(input(lines: 3), notes: "## Summary\nS", contextTokens: 32_768, complete: notes.complete)
            XCTFail("should throw")
        } catch { XCTAssertEqual(error as? FillInTheBlanks.Failure, .unusable) }
        XCTAssertTrue(notes.calls[1].messages[1].content.contains("(The answer had neither list in it)"))
        // The model cut off or refused: asked once more too
        let cut = FillMockAI()
        cut.failures = [AIError.truncated]
        _ = try await MeetingAI.fill(input(lines: 3), notes: "## Summary\nS", contextTokens: 32_768, complete: cut.complete)
        XCTAssertEqual(cut.calls.count, 2)
    }

    func testNoTranscriptMakesNoRequestAndAccessErrorsStopTheRun() async {
        let ai = FillMockAI()
        let empty = NotesInput(study: StudyInput(title: "x", durationMs: 0, lines: [.init(ms: 0, text: RedactionText.placeholder)], marks: []))
        do { _ = try await MeetingAI.fill(empty, notes: "## Summary\nS", contextTokens: 32_768, complete: ai.complete); XCTFail("should throw") } catch {
            XCTAssertEqual(error as? MeetingAI.NotesFailure, .noTranscript)
        }
        XCTAssertTrue(ai.calls.isEmpty)
        ai.error = AIError.consentRequired("example.com")
        do { _ = try await MeetingAI.fill(input(lines: 3), notes: "## Summary\nS", contextTokens: 32_768, complete: ai.complete); XCTFail("should throw") } catch {
            XCTAssertEqual(error as? AIError, .consentRequired("example.com"))
        }
        XCTAssertEqual(ai.calls.count, 1, "the first refused request ends it")
    }
}

// MARK: - Saving, and what removes it

@MainActor
final class FillStoreTests: XCTestCase {
    private var container: ModelContainer!
    private var context: ModelContext { container.mainContext }
    private var dir: URL!
    private let key = SyncCrypto.tokenKey(secret: Data((0..<32).map { UInt8($0) }))
    private let local = AIEndpoint(provider: .custom, baseURL: URL(string: "http://127.0.0.1:11434/v1"), apiKey: nil, model: "m",
                                   contextTokens: 32_768, consentGranted: false)

    override func setUp() async throws {
        container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        Storage.prepare()
        dir = FileManager.default.temporaryDirectory.appending(path: "nf-fill-\(UUID().uuidString)")
        SyncLedger.directory = dir
        UserDefaults.standard.removeObject(forKey: AutoNotes.settingKey)
    }

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: dir)
        UserDefaults.standard.removeObject(forKey: AutoNotes.settingKey)
        container = nil
    }

    private var engine: SyncEngine { SyncEngine(context: context, tokenKey: key) }
    private func newState() -> SyncMacState {
        SyncMacState(macID: "00000000-0000-4000-8000-0000000000ee", name: "Mac", fingerprint: String(repeating: "0", count: 64), hosts: [], port: 1)
    }

    /// A ten-minute class: a line every ten seconds
    private func meeting(_ lines: [String] = ["Cells have membranes.", "The exam covers chapters three and four.", "The pump moves three sodium out and two potassium in."],
                         notes: String? = "## Summary\nMembranes.", kind: RecordingKind = .class) -> Meeting {
        let start = Date(timeIntervalSince1970: 1_790_000_000)
        let m = Meeting(title: "Test", startedAt: start)
        m.endedAt = start.addingTimeInterval(600)
        m.kind = kind
        m.aiNotes = notes
        m.aiNotesAt = notes == nil ? nil : start.addingTimeInterval(900)
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

    private func fill(_ m: Meeting, _ ai: FillMockAI, recording: UUID? = nil) async throws -> FillInTheBlanks.Filled {
        try await NotesStore.fill(m, contextTokens: 32_768, complete: ai.complete, context: context, recording: recording)
    }

    private func failure(_ m: Meeting, _ ai: FillMockAI, recording: UUID? = nil) async -> FillInTheBlanks.Failure? {
        do { _ = try await fill(m, ai, recording: recording); return nil } catch { return error as? FillInTheBlanks.Failure }
    }

    // MARK: Saving

    func testATapAddsTheTwoSectionsAtTheEndAndLeavesTheUsersLinesAlone() async throws {
        let notes = "## What you wrote\n- **pump: 3 Na out**: The pump moves sodium out.\n\n## Summary\nMembranes.\n\n## Announcements and deadlines\n- Read chapter 3 by Friday"
        let m = meeting(notes: notes)
        let typed = add(m, .note, "pump: 3 Na out", at: 21)
        let mark = add(m, .test, "on the exam", at: 12)
        let madeAt = m.aiNotesAt
        XCTAssertEqual(FillInTheBlanks.buttonTitle(notes: m.aiNotes), "Fill in the blanks")
        let ai = FillMockAI()
        let filled = try await fill(m, ai)
        XCTAssertEqual(filled.missed.count, 4)
        XCTAssertEqual(filled.more.count, 2)
        // At the end, in the form both devices write
        XCTAssertEqual(m.aiNotes, notes + "\n\n" + fillSections)
        XCTAssertEqual(FillInTheBlanks.buttonTitle(notes: m.aiNotes), "Fill again")
        // They are still the notes that were made then
        XCTAssertEqual(m.aiNotesAt, madeAt)
        XCTAssertFalse(m.aiNotesStale)
        // What the user typed is never rewritten: not the note, not its line in the notes
        XCTAssertEqual(typed.note, "pump: 3 Na out")
        XCTAssertEqual(m.typedNotes.map(\.id), [typed.id])
        XCTAssertEqual(WhatYouWrote.parse(m.aiNotes ?? "")?.bullets, ["**pump: 3 Na out**: The pump moves sodium out."])
        XCTAssertEqual(NotebookItems.texts(inNotes: m.aiNotes ?? ""), ["Read chapter 3 by Friday"])
        // One request for a short recording: the notes, then the whole transcript with times
        XCTAssertEqual(ai.calls.count, 1, "nothing but the pass itself: no topics, no notes")
        let user = ai.calls[0].messages[1].content
        XCTAssertTrue(user.contains("NOTES SO FAR:\n## What you wrote\n- **pump: 3 Na out**: The pump moves sodium out.\n\n## Summary"), user)
        XCTAssertFalse(user.contains("Typed by the user while listening"), "the notes already list the line")
        XCTAssertTrue(user.contains("\nTRANSCRIPT:\n[0:00] Cells have membranes.\n[0:10] The exam covers chapters three and four.\n[0:20] The pump moves three sodium out and two potassium in.\n"), user)
        for id in [typed.id, mark.id, m.id] { XCTAssertFalse(ai.sent.localizedCaseInsensitiveContains(id.uuidString)) }
        XCTAssertFalse(hasUUID(ai.sent), "ids never go in a prompt")
    }

    func testFillAgainReplacesTheSectionsAndNeverFeedsOnItself() async throws {
        let m = meeting(["Cells have membranes.", "The pump moves sodium out."])
        _ = try await fill(m, FillMockAI())
        XCTAssertEqual(m.aiNotes, "## Summary\nMembranes.\n\n" + fillSections)
        let again = FillMockAI([#"{"missed":[{"at":"0:10","point":"The pump uses ATP."}],"more":[]}"#])
        _ = try await fill(m, again)
        XCTAssertEqual(m.aiNotes, "## Summary\nMembranes.\n\n## What you missed\n- [0:10] The pump uses ATP.", "replaced, not added to")
        let user = again.calls[0].messages[1].content
        XCTAssertTrue(user.contains("NOTES SO FAR:\n## Summary\nMembranes.\n\nTRANSCRIPT:\n"), user)
        for earlier in ["What you missed", "More detail", "Osmosis moves water", "Active transport"] {
            XCTAssertFalse(again.sent.contains(earlier), "what the first pass wrote isn't shown to the second: \(earlier)")
        }
        // Nothing to add: nothing is written, and the caller is told so
        let nothing = FillMockAI([#"{"missed":[],"more":[]}"#])
        let empty = try await fill(m, nothing)
        XCTAssertTrue(empty.isEmpty)
        XCTAssertEqual(m.aiNotes, "## Summary\nMembranes.\n\n## What you missed\n- [0:10] The pump uses ATP.")
        XCTAssertTrue(FillInTheBlanks.nothingToAdd.contains("already cover the recording"))
        // What VoiceOver is told when a pass ends
        XCTAssertEqual(FillInTheBlanks.announcement(empty), FillInTheBlanks.nothingToAdd)
        XCTAssertEqual(FillInTheBlanks.announcement(.init(missed: [.init(atMs: nil, text: "x")])), "What you missed was added at the end of the notes.")
        XCTAssertEqual(FillInTheBlanks.announcement(try FillInTheBlanks.parse(fillAnswer, kind: .class, durationMs: 60_000)),
                       "What you missed and More detail were added at the end of the notes.")
        // A meeting keeps only what was said
        let minutes = meeting(kind: .meeting)
        _ = try await fill(minutes, FillMockAI())
        XCTAssertFalse(minutes.aiNotes?.contains("background") ?? true)
        XCTAssertTrue(minutes.aiNotes?.hasSuffix("- **Sodium-potassium pump**: It moves three sodium out and two potassium in for each ATP. [0:20]") ?? false)
    }

    func testNothingIsAskedWithoutNotesWithoutATranscriptOrWhileThatRecordingRuns() async throws {
        let ai = FillMockAI()
        let m = meeting(notes: nil)
        var why = await failure(m, ai)
        XCTAssertEqual(why, .needsNotes)
        m.aiNotes = " \n"
        why = await failure(m, ai)
        XCTAssertEqual(why, .needsNotes)
        // The recording that is still running: never, like Make notes on the Mac
        m.aiNotes = "## Summary\nMembranes."
        why = await failure(m, ai, recording: m.id)
        XCTAssertEqual(why, .stillRecording)
        // Notes that came from the Mac for a recording with no transcript here
        let bare = meeting([], notes: "## Summary\nS")
        do { _ = try await fill(bare, ai); XCTFail("should throw") } catch {
            XCTAssertEqual(error as? MeetingAI.NotesFailure, .noTranscript)
        }
        // A Delete in its undo window (it may still be put back): asked again once it is settled
        let pending = try RedactionEngine.delete(.lines([m.orderedSegments[1]]), meeting: m, context: context)
        why = await failure(m, ai)
        XCTAssertEqual(why, .pendingEdit)
        XCTAssertTrue(ai.calls.isEmpty, "nothing was sent")
        try RedactionEngine.undo(pending, context: context)
        XCTAssertEqual(m.aiNotes, "## Summary\nMembranes.")
        XCTAssertEqual(bare.aiNotes, "## Summary\nS")
        // Another recording running is no reason: the tap is the user's own
        _ = try await fill(m, ai, recording: UUID())
        XCTAssertTrue(FillInTheBlanks.has(m.aiNotes ?? ""))
    }

    func testNothingIsSavedWhenWhatThePassReadChangedMeanwhile() async throws {
        let context = self.context
        // A line it read is edited while the AI is writing
        let m = meeting(["The launch code is tangerine.", "Marcus owns it."])
        let seg = m.orderedSegments[0]
        var ai = FillMockAI()
        ai.before = { await MainActor.run { seg.text = "The launch code is." } }
        var why = await failure(m, ai)
        XCTAssertEqual(why, .transcriptChanged)
        XCTAssertEqual(m.aiNotes, "## Summary\nMembranes.")

        // The notes are made again while the AI is writing: the answer was for the old ones
        let n = meeting()
        ai = FillMockAI()
        ai.before = { await MainActor.run { n.aiNotes = "## Summary\nMade again." } }
        why = await failure(n, ai)
        XCTAssertEqual(why, .notesChanged)
        XCTAssertEqual(n.aiNotes, "## Summary\nMade again.")

        // A typed note is edited meanwhile. These notes don't quote it, so only its own check can tell
        let t = meeting()
        let line = add(t, .note, "ask about the pump", at: 5)
        ai = FillMockAI()
        ai.before = { await MainActor.run { TypedNoteStore.edit(line, text: "ask about osmosis", context: context) } }
        why = await failure(t, ai)
        XCTAssertEqual(why, .notesChanged)
        XCTAssertEqual(t.aiNotes, "## Summary\nMembranes.")
        XCTAssertEqual(line.note, "ask about osmosis")

        // One is deleted meanwhile
        let d = meeting()
        let gone = add(d, .note, "ask about the pump", at: 5)
        ai = FillMockAI()
        ai.before = { await MainActor.run { TypedNoteStore.delete(gone, context: context) } }
        why = await failure(d, ai)
        XCTAssertEqual(why, .notesChanged)
        XCTAssertEqual(d.aiNotes, "## Summary\nMembranes.")

        // One arrives meanwhile (from the other device)
        let a = meeting()
        ai = FillMockAI()
        ai.before = {
            await MainActor.run {
                guard a.typedNotes.isEmpty else { return }
                let k = MomentMarker(at: a.startedAt.addingTimeInterval(7), kind: .note, note: "a line from the Mac")
                context.insert(k)
                k.meeting = a
                try? context.save()
            }
        }
        why = await failure(a, ai)
        XCTAssertEqual(why, .notesChanged)
        XCTAssertEqual(a.aiNotes, "## Summary\nMembranes.")

        // The recording is deleted meanwhile: nothing is written anywhere
        let x = meeting()
        ai = FillMockAI()
        ai.before = { await MainActor.run { context.delete(x); try? context.save() } }
        why = await failure(x, ai)
        XCTAssertEqual(why, .transcriptChanged)
        XCTAssertEqual(try context.fetchCount(FetchDescriptor<Meeting>()), 5)
    }

    /// The save and its checks are one synchronous step, like the notes' own.
    func testTheSaveIsOneStepWithItsChecks() async throws {
        let m = meeting()
        func basis() -> NotesStore.FillBasis {
            NotesStore.FillBasis(read: StudyInput(meeting: m, screenPrefix: true).transcriptLines, notes: m.aiNotes ?? "", typed: [])
        }
        let before = basis()
        let filled = FillInTheBlanks.Filled(missed: [.init(atMs: 10_000, text: "A point.")])
        // A Delete in its undo window: refused, also for a pass made from what is left
        let pending = try RedactionEngine.delete(.lines([m.orderedSegments[1]]), meeting: m, context: context)
        XCTAssertThrowsError(try NotesStore.saveFill(filled, basis: before, to: m, context: context)) {
            XCTAssertEqual($0 as? FillInTheBlanks.Failure, .transcriptChanged)
        }
        XCTAssertThrowsError(try NotesStore.saveFill(filled, basis: basis(), to: m, context: context)) {
            XCTAssertEqual($0 as? FillInTheBlanks.Failure, .transcriptChanged)
        }
        XCTAssertEqual(m.aiNotes, "## Summary\nMembranes.")
        // Put back: everything it read is there again
        try RedactionEngine.undo(pending, context: context)
        XCTAssertEqual(try NotesStore.saveFill(filled, basis: before, to: m, context: context), "## Summary\nMembranes.\n\n## What you missed\n- [0:10] A point.")
        XCTAssertEqual(m.aiNotes, "## Summary\nMembranes.\n\n## What you missed\n- [0:10] A point.")
        // The same answer again: the notes are no longer the ones it was given
        XCTAssertThrowsError(try NotesStore.saveFill(filled, basis: before, to: m, context: context)) {
            XCTAssertEqual($0 as? FillInTheBlanks.Failure, .notesChanged)
        }
        // A typed note the pass never saw
        let now = basis()
        let late = add(m, .note, "typed later", at: 4)
        XCTAssertThrowsError(try NotesStore.saveFill(filled, basis: now, to: m, context: context)) {
            XCTAssertEqual($0 as? FillInTheBlanks.Failure, .notesChanged)
        }
        var with = now
        with.typed = [.init(id: late.id, text: "typed later")]
        // Nothing found: nothing is written
        XCTAssertNil(try NotesStore.saveFill(.init(), basis: with, to: m, context: context))
        XCTAssertEqual(m.aiNotes, "## Summary\nMembranes.\n\n## What you missed\n- [0:10] A point.")
        // The same id with other words is a changed note
        var edited = with
        edited.typed = [.init(id: late.id, text: "typed earlier")]
        XCTAssertThrowsError(try NotesStore.saveFill(filled, basis: edited, to: m, context: context))
        // A line that was read is gone for good
        let second = try RedactionEngine.delete(.lines([m.orderedSegments[0]]), meeting: m, context: context)
        try await RedactionEngine.commit(second, context: context)
        var stale = with
        stale.notes = m.aiNotes ?? ""
        XCTAssertThrowsError(try NotesStore.saveFill(filled, basis: stale, to: m, context: context)) {
            XCTAssertEqual($0 as? FillInTheBlanks.Failure, .transcriptChanged)
        }
        // A recording that was deleted gets nothing
        context.delete(m)
        try context.save()
        XCTAssertThrowsError(try NotesStore.saveFill(filled, basis: with, to: m, context: context)) {
            XCTAssertEqual($0 as? FillInTheBlanks.Failure, .transcriptChanged)
        }
    }

    // MARK: It never outlives what it was made from

    func testTheSectionsGoWhenATypedNoteIsEditedOrDeleted() async throws {
        let base = "## What you wrote\n- **ask about oxygen**: It is the final acceptor.\n- **lab report**: Due Monday.\n\n## Summary\nS"
        let m = meeting(notes: base + "\n\n" + fillSections)
        let first = add(m, .note, "ask about oxygen", at: 4)
        let second = add(m, .note, "lab report", at: 8)
        // The same words again change nothing
        TypedNoteStore.edit(first, text: " ask about   oxygen ", context: context)
        XCTAssertEqual(m.aiNotes, base + "\n\n" + fillSections)
        // Edited: its own line follows, and both sections are gone
        TypedNoteStore.edit(first, text: "ask about O2", context: context)
        XCTAssertEqual(m.aiNotes, "## What you wrote\n- **ask about O2**: It is the final acceptor.\n- **lab report**: Due Monday.\n\n## Summary\nS")
        // Deleted
        m.aiNotes = (m.aiNotes ?? "") + "\n\n" + fillSections
        TypedNoteStore.delete(second, context: context)
        XCTAssertEqual(m.aiNotes, "## What you wrote\n- **ask about O2**: It is the final acceptor.\n\n## Summary\nS")
        // Notes that never quoted the line (made before it arrived): the sections go all the same
        let n = meeting(notes: "## Summary\nS\n\n" + fillSections)
        let k = add(n, .note, "a line", at: 3)
        TypedNoteStore.edit(k, text: "another line", context: context)
        XCTAssertEqual(n.aiNotes, "## Summary\nS")
        n.aiNotes = "## Summary\nS\n\n" + fillSections
        TypedNoteStore.delete(k, context: context)
        XCTAssertEqual(n.aiNotes, "## Summary\nS")
        // After a real pass, too
        let p = meeting()
        let typed = add(p, .note, "pump: 3 Na out", at: 21)
        _ = try await fill(p, FillMockAI())
        XCTAssertTrue(FillInTheBlanks.has(p.aiNotes ?? ""))
        TypedNoteStore.edit(typed, text: "pump: 2 K in", context: context)
        XCTAssertEqual(p.aiNotes, "## Summary\nMembranes.")
        // A mark's note is not a typed note: the sections stay
        let star = add(p, .important, "remember", at: 9)
        p.aiNotes = "## Summary\nMembranes.\n\n" + fillSections
        TypedNoteStore.edit(star, text: "changed", context: context)
        XCTAssertTrue(FillInTheBlanks.has(p.aiNotes ?? ""))
    }

    /// The two sync paths: a typed note edited or deleted on the Mac.
    func testTheSectionsGoWhenTheMacEditsOrDeletesATypedNote() async throws {
        var state = newState()
        let m = meeting(notes: nil)
        let rec = SyncIDs.wire(m.id)
        let id = "00000000-0000-4000-8000-000000000031"
        let at = SyncIDs.ms(m.startedAt) + 4000
        _ = await engine.apply([.mark(MarkItem(id: id, rec: rec, at: at, kind: "note", note: "ask about oxygen", created: 1, modified: 2))], state: &state)
        XCTAssertEqual(m.typedNotes.first?.note, "ask about oxygen")
        // Notes that arrived from the Mac with its two sections: recognised here
        let notes = "## What you wrote\n- **ask about oxygen**: It is the final acceptor.\n\n## Summary\nS"
        let made = SyncIDs.ms(m.startedAt) + 700_000
        let arrived = await engine.apply([.notes(NotesItem(rec: rec, md: notes + "\n\n" + fillSections, made: made, stale: false, modified: 3))], state: &state)
        XCTAssertEqual(arrived.applied, 1)
        XCTAssertEqual(FillInTheBlanks.buttonTitle(notes: m.aiNotes), "Fill again")
        // Edited on the Mac
        _ = await engine.apply([.mark(MarkItem(id: id, rec: rec, at: at, kind: "note", note: "ask about O2", created: 1, modified: 4))], state: &state)
        XCTAssertEqual(m.aiNotes, "## What you wrote\n- **ask about O2**: It is the final acceptor.\n\n## Summary\nS")
        // Deleted on the Mac
        m.aiNotes = (m.aiNotes ?? "") + "\n\n" + fillSections
        let gone = await engine.apply([.gone(GoneItem(entity: "mark", id: id, rec: rec))], state: &state)
        XCTAssertEqual(gone.applied, 1)
        XCTAssertTrue(m.typedNotes.isEmpty)
        XCTAssertEqual(m.aiNotes, "## Summary\nS")
    }

    func testDeleteOrStrikeOfTranscriptTextRemovesTheSectionsOutright() async throws {
        let notes = "## Summary\nDecision on azure credits by Friday.\n\n" + fillSections
        // Strike: the words in the notes are struck, the two sections go whole
        let s = meeting(["We need the Azure credits by Friday.", "Marcus owns it."], notes: notes)
        try await RedactionEngine.strike(.words(s.orderedSegments[0], 3...4), reason: nil, meeting: s, context: context)
        XCTAssertEqual(s.aiNotes, "## Summary\nDecision on [stricken from the record] by Friday.")
        XCTAssertTrue(s.aiNotesStale)
        // Delete: whatever the words were (none of these is anywhere in the sections)
        let d = meeting(["We need the Azure credits by Friday.", "Marcus owns it."], notes: notes)
        let pending = try RedactionEngine.delete(.lines([d.orderedSegments[1]]), meeting: d, context: context)
        try await RedactionEngine.commit(pending, context: context)
        XCTAssertEqual(d.aiNotes, "## Summary\nDecision on azure credits by Friday.")
        XCTAssertTrue(d.aiNotesStale)
        // A Delete the app was killed in the middle of: finished at the next launch, the sections with it
        let k = meeting(["We need the Azure credits by Friday.", "Marcus owns it."], notes: notes)
        _ = try RedactionEngine.delete(.lines([k.orderedSegments[0]]), meeting: k, context: context)
        await RedactionEngine.recover(context: context)
        XCTAssertEqual(k.aiNotes, "## Summary\nDecision on azure credits by Friday.")
        XCTAssertTrue(k.redactions.isEmpty)
        // The Mac deleted a line
        var state = newState()
        let y = meeting(["We need the Azure credits by Friday.", "Marcus owns it."], notes: notes)
        engine.assignLineIDs()
        let line = try XCTUnwrap(y.orderedSegments[1].syncID)
        let report = await engine.apply([.gone(GoneItem(entity: "line", id: SyncIDs.wire(line), rec: SyncIDs.wire(y.id)))], state: &state)
        XCTAssertEqual(report.applied, 1, "\(report)")
        XCTAssertEqual(y.aiNotes, "## Summary\nDecision on azure credits by Friday.")
        // A photo is not transcript text: striking one leaves them
        let p = meeting(notes: notes)
        let name = "fill-test-\(UUID().uuidString).jpg"
        try Data([0xFF, 0xD8, 0xFF]).write(to: Storage.snapshots.appending(path: name))
        let photo = Snapshot(fileName: name, takenAt: p.startedAt.addingTimeInterval(5))
        context.insert(photo)
        photo.meeting = p
        try context.save()
        try await RedactionEngine.strike(.screens([photo]), reason: nil, meeting: p, context: context)
        XCTAssertEqual(p.aiNotes, notes)
        XCTAssertFalse(FileManager.default.fileExists(atPath: Storage.snapshots.appending(path: name).path(percentEncoded: false)))
    }

    func testNotesMadeAgainDontHaveThemAndNothingAutomaticFillsThemIn() async throws {
        let m = meeting(notes: "## Summary\nOld.\n\n" + fillSections)
        let ai = FillMockAI()
        ai.notes = "## Summary\nNew."
        try await NotesStore.make(m, contextTokens: 32_768, complete: ai.complete, context: context)
        XCTAssertEqual(m.aiNotes, "## Summary\nNew.")
        XCTAssertFalse(ai.sent.contains("What you missed"), "the old notes aren't in the notes prompt")
        XCTAssertTrue(ai.fillCalls.isEmpty, "Make again doesn't fill in the blanks")
        // A model can't put the sections into fresh notes either
        let read = StudyInput(meeting: m, screenPrefix: true).transcriptLines
        XCTAssertEqual(try NotesStore.save("## Summary\nS\n\n## More detail\n- **x**: y", madeFrom: read, to: m, context: context), "## Summary\nS")
        // Automatic notes after Stop: the notes, the topics, and no pass of this kind
        let auto = AutoNotes()
        let fresh = meeting(["Cells make ATP.", "Lab report two is due Monday."], notes: nil)
        fresh.endedAt = fresh.startedAt.addingTimeInterval(7 * 60)
        let quiet = FillMockAI()
        let task = try XCTUnwrap(auto.recordingStopped(fresh, isPro: true, endpoint: local, context: context, complete: quiet.complete))
        await task.value
        XCTAssertEqual(fresh.aiNotes, quiet.notes)
        XCTAssertTrue(quiet.fillCalls.isEmpty)
        XCTAssertFalse(quiet.sent.contains("NOTES SO FAR"))
        XCTAssertEqual(FillInTheBlanks.buttonTitle(notes: fresh.aiNotes), "Fill in the blanks")
    }

    /// The sections are part of the notes: they cross to the Mac as Markdown, under the shared headings.
    func testFilledNotesGoToTheMacAgain() async throws {
        var state = newState()
        let m = meeting()
        engine.assignLineIDs()
        engine.confirmSent(engine.changes(for: state), retry: [], state: &state)
        XCTAssertTrue(engine.changes(for: state).isEmpty)
        _ = try await fill(m, FillMockAI())
        let sent = engine.changes(for: state).compactMap { item -> String? in if case .notes(let n) = item { return n.md } else { return nil } }
        XCTAssertEqual(sent, ["## Summary\nMembranes.\n\n" + fillSections])
    }
}
