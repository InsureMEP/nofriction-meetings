import Foundation

// Fill in the blanks (docs/design/FADELL_V2.md §11; the Mac half is
// `src-tauri/src/meeting_notes/fill.rs`).
//
// One button on a recording's Notes. It reads the whole recording again,
// beside the notes that already exist and the lines the user typed, and adds
// two sections at the end of the notes' Markdown:
//   - What you missed: things that were said and aren't in the notes, each
//     with the time it was said.
//   - More detail: the notes' key ideas explained further, from what was
//     said. For a Class only, the AI may also add what a textbook would say,
//     and every such entry is marked as background, never as something that
//     was said.
//
// Rules:
//   - It runs only on the user's tap, with the same gates as Make notes (Pro,
//     an AI that is set up, consent). Never by itself, never from
//     `AutoNotes`, never for the recording that is still running, and not
//     while a Delete is waiting in its undo window.
//   - The user's own lines are never rewritten. Typed notes are only shown to
//     the AI so it knows what is already covered.
//   - The result has no storage of its own: it lives inside `aiNotes`, and
//     is written by `NotesStore.saveFill`, one synchronous step with its
//     checks, like the notes themselves.
//   - It never outlives what it was made from. Delete or Strike of transcript
//     text removes both sections outright (they paraphrase the recording, so
//     word matching can't clean them); changing or deleting a typed note
//     removes them; notes that are made again don't have them. Every one of
//     those goes through `FillInTheBlanks.removed(from:)`.
//   - The answer is untrusted: sizes are capped, a time outside the recording
//     is dropped, whatever isn't recognisably from the recording counts as
//     background, and background is refused outside a Class.
//   - The two sections read the same on both platforms (`fill::markdown` on
//     the Mac): notes sync as Markdown, and each device must recognise the
//     other's sections.
//   - Nothing here is logged: not the transcript, not the answer.

enum FillInTheBlanks {
    // MARK: Shared with the Mac (the same on every platform)

    static let missedHeading = "What you missed"
    static let moreHeading = "More detail"
    /// How an entry that wasn't said is marked
    static let backgroundNote = "background, not said in the recording"

    /// How many missed points and explanations are asked for, and kept
    static let maxMissed = 12
    static let maxMore = 8
    static let maxPoint = 400
    static let maxAbout = 160
    static let maxMoreDetail = 700
    /// The notes the user already has are shown to the AI up to this many
    /// characters, fewer on a small model window (`haveCap`)
    static let maxHaveChars = 6_000
    static let minHaveChars = 800
    /// Typed lines the notes don't list yet: how many are shown, and how much of each
    static let maxPromptNotes = 40
    static let promptNoteChars = 240
    /// At most this many entries of a list are looked at
    private static let maxRead = 60

    // MARK: Words the user sees

    /// The progress line, in the place Make notes shows its own
    static let working = "Filling in the blanks…"
    /// An answer with both lists empty: nothing is added
    static let nothingToAdd = "Nothing to add: the notes already cover the recording."
    /// What VoiceOver says when a pass ends: which sections were written, or that none was
    static func announcement(_ filled: Filled) -> String {
        let names = [filled.missed.isEmpty ? nil : missedHeading, filled.more.isEmpty ? nil : moreHeading].compactMap { $0 }
        guard !names.isEmpty else { return nothingToAdd }
        return names.joined(separator: " and ") + (names.count == 1 ? " was" : " were") + " added at the end of the notes."
    }

    /// **Fill in the blanks**, or **Fill again** once the notes have either
    /// section (also when they arrived from the Mac that way)
    static func buttonTitle(notes: String?) -> String {
        (notes.map(has) ?? false) ? "Fill again" : "Fill in the blanks"
    }

    enum Failure: LocalizedError, Equatable {
        case needsNotes
        case stillRecording
        case pendingEdit
        case transcriptChanged
        case notesChanged
        case unusable

        var errorDescription: String? {
            switch self {
            case .needsNotes: "Make the notes first, then fill in the blanks."
            case .stillRecording: "Stop the recording to fill in the blanks."
            case .pendingEdit: "An edit to this transcript is still in its 5-second undo window. Try again in a moment."
            case .transcriptChanged: "The transcript changed while this was being written (it was edited), so nothing was added. Fill in the blanks again."
            case .notesChanged: "The notes changed while this was being written, so nothing was added. Fill in the blanks again."
            // Never quotes the answer
            case .unusable: "Couldn't fill in the blanks: the AI's answer wasn't usable (asked twice). Try again, or choose a larger model in Settings."
            }
        }
    }

    /// Why an answer couldn't be used. Shown to the model when it is asked
    /// once more; never contains the answer.
    enum ParseFailure: LocalizedError, Equatable {
        case notAnObject
        case noLists

        var errorDescription: String? {
            switch self {
            case .notAnObject: "The answer wasn't a JSON object"
            case .noLists: "The answer had neither list in it"
            }
        }
    }

    // MARK: What a pass found

    /// Something that was said and the notes don't have.
    struct Missed: Equatable, Sendable {
        /// When it was said, from the start of the recording
        var atMs: Int?
        var text: String
    }

    /// One of the notes' key ideas, explained further.
    struct More: Equatable, Sendable {
        var about: String
        var detail: String
        var atMs: Int?
        /// Not said in the recording: what a textbook says, added by the AI (Class only)
        var background = false
    }

    struct Filled: Equatable, Sendable {
        var missed: [Missed] = []
        var more: [More] = []

        /// The notes already cover the recording
        var isEmpty: Bool { missed.isEmpty && more.isEmpty }
    }

    // MARK: The prompt

    /// The JSON the model is asked for (the Mac's, word for word)
    static let shape = #"""
        Your output MUST be valid JSON with this exact schema:
        {
          "missed": [{"at": "m:ss", "point": "one thing that was said and NOTES SO FAR don't have"}],
          "more": [{"about": "a key idea or term from NOTES SO FAR", "detail": "two to four sentences that explain it further", "at": "m:ss", "from": "recording"}]
        }
        """#

    /// What the pass is asked to do, by recording type (the Mac's
    /// `fill::base_prompt`, word for word). Only a Class may get background,
    /// and it must be marked.
    static func basePrompt(for kind: RecordingKind, notebook: String?) -> String {
        func named(_ what: String) -> String {
            Notebook.normalize(notebook).map { "\(what) (\"\($0)\")" } ?? what
        }
        switch kind {
        case .class:
            return #"""
                You are helping a student complete their notes from \#(named("a class")). NOTES SO FAR is what they already have: the notes made from the recording and the lines they typed themselves. Below it is everything that was said. Find what the notes are missing, and explain the key ideas further so the student can study from them.

                \#(shape)

                Rules:
                - missed: only things actually said, each with the time of the line it comes from. Look for definitions, the steps of a method, formulas, examples, corrections, what the instructor said matters or will be on a test, and answers to students' questions. Skip anything NOTES SO FAR already says. In the order they were said, at most \#(maxMissed).
                - more: take the key concepts and terms in NOTES SO FAR, at most \#(maxMore), and explain each one further from what the instructor said: "from": "recording", with the time.
                - If the recording does not explain a concept well enough to study from, you may add what a standard textbook says about it: a definition, the formula, a simple example, in one to three sentences. Mark that entry "from": "background" and set "at" to null. Background must be well-established knowledge. Never present background as something that was said in class.
                - Never rewrite or repeat the lines the student typed.
                - Write plain sentences a student can study from.
                - If there is nothing for a list, return an empty array.
                - Do NOT invent anything that was said.
                """#
        case .meeting:
            return #"""
                You are helping someone complete their notes from a meeting. NOTES SO FAR is what they already have: the notes made from the recording and the lines they typed themselves. Below it is everything that was said. Find what the notes are missing, and say more about the key topics.

                \#(shape)

                Rules:
                - missed: only things actually said, each with the time of the line it comes from. Look for decisions, numbers, dates, names, commitments, objections, open questions and the reasons given. Skip anything NOTES SO FAR already says. In the order they were said, at most \#(maxMissed).
                - more: take the key topics in NOTES SO FAR, at most \#(maxMore), and say what was said about each that the notes leave out: the reasoning, the alternatives that were discussed, who raised what. Every entry is "from": "recording", with the time.
                - Use only what was said. Do not add outside knowledge, advice or opinions.
                - Never rewrite or repeat the lines the user typed.
                - If there is nothing for a list, return an empty array.
                - Do NOT invent anything that was said.
                """#
        case .personal:
            return #"""
                You are helping someone complete their notes from \#(named("a personal recording")): a conversation, appointment, talk or idea. NOTES SO FAR is what they already have: the notes made from the recording and the lines they typed themselves. Below it is everything that was said. Find what the notes are missing, and say more about the key points.

                \#(shape)

                Rules:
                - missed: only things actually said, each with the time of the line it comes from. Look for facts, numbers, names, dates, instructions and things to do. Skip anything NOTES SO FAR already says. In the order they were said, at most \#(maxMissed).
                - more: take the key points in NOTES SO FAR, at most \#(maxMore), and say what was said about each that the notes leave out. Every entry is "from": "recording", with the time.
                - Use only what was said. Do not add outside knowledge, advice or opinions.
                - Never rewrite or repeat the lines the user typed.
                - If there is nothing for a list, return an empty array.
                - Do NOT invent anything that was said.
                """#
        }
    }

    /// Header of the user's typed lines the notes don't list (the Mac's words)
    static let typedHeader = "Typed by the user while listening"

    /// NOTES SO FAR: the notes as they are, without what an earlier pass
    /// wrote (a refill must not feed itself), then any line the user typed
    /// that the notes' "What you wrote" doesn't list (one that arrived from
    /// the other device after the notes were made). About `budget`
    /// characters at most: long notes are cut and say so, and the user's
    /// own lines keep their place. "" when there is nothing.
    static func haveBlock(notes: String?, typed: [StudyInput.Mark], budget: Int = maxHaveChars) -> String {
        let made = strip(notes ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let lines = typed.compactMap { t -> (ms: Int, text: String)? in
            let text = (t.note ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : (t.ms, text)
        }
        // For each typed line: the bullet of "What you wrote" that quotes it, if one does
        let quoted: [Int?] = WhatYouWrote.parse(made).map { WhatYouWrote.merge($0.bullets, typed: lines.map(\.text)).bullet }
            ?? Array(repeating: nil, count: lines.count)
        let unlisted = lines.enumerated().filter { quoted[$0.offset] == nil }.prefix(maxPromptNotes).map {
            "- [\(StudyParse.clock($0.element.ms))] \(StudyInput.clip($0.element.text, to: promptNoteChars))"
        }
        // Whole lines, in at most half the room
        let own = unlisted.isEmpty ? "" : "\(typedHeader):\n" + MeetingAI.fitLines(Array(unlisted), budget: max(60, budget / 2)).joined(separator: "\n")
        let room = max(0, budget - (own.isEmpty ? 0 : own.count + 2))
        let shown = made.count > room ? String(made.prefix(room)) + "\n(more notes follow)" : made
        return [shown, own].filter { !$0.isEmpty }.joined(separator: "\n\n")
    }

    /// How much of NOTES SO FAR a model with this window is shown: the Mac's
    /// cap (`have_cap`: 6,000 characters, about a quarter of a small window),
    /// and never more than a third of what fits beside the prompt and the
    /// answer, so the recording itself keeps most of the room and the client
    /// never has to trim the request.
    static func haveCap(contextTokens: Int, maxTokens: Int, system: String) -> Int {
        let cap = min(maxHaveChars, max(minHaveChars, contextTokens * 3 / 4))
        return min(cap, max(300, MeetingAI.bodyBudget(contextTokens: contextTokens, maxTokens: maxTokens, fixed: system) / 3))
    }

    static func retryNote(_ why: String) -> String {
        "Your previous answer couldn't be used (\(why)). Answer again with only the JSON object in the shape described, and nothing else."
    }

    // MARK: Parsing the answer (untrusted)

    private static func isBool(_ n: NSNumber) -> Bool { CFGetTypeID(n) == CFBooleanGetTypeID() }

    /// The same text whatever the case of its ASCII letters (the Mac's `eq_ignore_ascii_case`)
    private static func same(_ a: String, _ b: String) -> Bool {
        func lower(_ c: UInt8) -> UInt8 { c >= 65 && c <= 90 ? c + 32 : c }
        return a.utf8.count == b.utf8.count && zip(a.utf8, b.utf8).allSatisfy { lower($0) == lower($1) }
    }

    /// How an answer says an entry comes from what was said ("from": "recording")
    private static let saidWords = ["recording", "transcript", "lecture", "class", "meeting", "said"]

    private static func list(_ o: [String: Any], _ key: String) -> ArraySlice<Any> {
        ((o[key] as? [Any]) ?? []).prefix(maxRead)
    }

    /// A time the answer gave, if it is inside the recording.
    static func moment(_ v: Any?, durationMs: Int) -> Int? {
        let ms: Int?
        if let s = v as? String {
            ms = StudyParse.parseClock(s)
        } else if let n = v as? NSNumber, !isBool(n) {
            ms = StudyParse.wholeMs(seconds: n.doubleValue)
        } else {
            return nil
        }
        // A few seconds past the end is the transcriber's last line
        guard let ms, ms >= 0, durationMs <= 0 || ms <= durationMs + 5_000 else { return nil }
        return ms
    }

    /// What a model answer holds (the Mac's `fill::parse`). Anything missing
    /// or mis-shaped is dropped; an answer that isn't a JSON object, or has
    /// neither list, is an error. Empty lists are a valid answer: the notes
    /// already cover the recording.
    static func parse(_ raw: String, kind: RecordingKind, durationMs: Int) throws -> Filled {
        guard let o = try StudyParse.extractJSON(raw) as? [String: Any] else { throw ParseFailure.notAnObject }
        guard o["missed"] != nil || o["more"] != nil else { throw ParseFailure.noLists }

        var missed: [Missed] = []
        for x in list(o, "missed") {
            let text: String?
            var at: Int?
            if let e = x as? [String: Any] {
                text = StudyParse.clean(e["point"] ?? e["text"], max: maxPoint)
                at = moment(e["at"] ?? e["time"], durationMs: durationMs)
            } else {
                text = StudyParse.clean(x, max: maxPoint)
            }
            guard let text, !missed.contains(where: { same($0.text, text) }) else { continue }
            missed.append(Missed(atMs: at, text: text))
        }
        // In the order said; the ones without a time keep their place at the end
        missed = missed.enumerated().sorted { a, b in
            let (x, y) = (a.element.atMs ?? .max, b.element.atMs ?? .max)
            return x == y ? a.offset < b.offset : x < y
        }.map(\.element)
        missed = Array(missed.prefix(maxMissed))

        var more: [More] = []
        for x in list(o, "more") {
            guard let e = x as? [String: Any],
                  let about = StudyParse.clean(e["about"] ?? e["term"] ?? e["concept"], max: maxAbout),
                  let detail = StudyParse.clean(e["detail"] ?? e["text"] ?? e["explanation"], max: maxMoreDetail) else { continue }
            let at = moment(e["at"] ?? e["time"], durationMs: durationMs)
            // Said in the recording only when the answer says so, or gives its
            // time and says nothing else. Anything else ("textbook", "outside
            // knowledge", a flag) is background: it must never pass as said.
            let from = ((e["from"] ?? e["source"]) as? String)?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            let flagged: Bool
            switch e["background"] {
            case let n as NSNumber where isBool(n): flagged = n.boolValue
            case let t as String: flagged = !["", "false", "no"].contains(t.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())
            default: flagged = false
            }
            let said = from.map { f in saidWords.contains { f.hasPrefix($0) } } ?? (at != nil)
            let background = flagged || !said
            // Outside a class nothing may come from anywhere but the recording
            if background && kind != .class { continue }
            if more.contains(where: { same($0.about, about) }) { continue }
            // Background was never said, so it never has a time
            more.append(More(about: about, detail: detail, atMs: background ? nil : at, background: background))
        }
        more = Array(more.prefix(maxMore))
        return Filled(missed: missed, more: more)
    }

    // MARK: The two sections, as Markdown

    private static func oneLine(_ s: String) -> String {
        s.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }

    /// The two sections as they are kept in the notes and sent to the Mac:
    /// character for character what the Mac's `fill::markdown` writes. nil
    /// when nothing was found.
    static func markdown(_ filled: Filled) -> String? {
        var parts: [String] = []
        if !filled.missed.isEmpty {
            let bullets = filled.missed.map { m in
                m.atMs.map { "- [\(StudyParse.clock($0))] \(oneLine(m.text))" } ?? "- \(oneLine(m.text))"
            }
            parts.append("## \(missedHeading)\n" + bullets.joined(separator: "\n"))
        }
        if !filled.more.isEmpty {
            let bullets = filled.more.map { m -> String in
                let about = oneLine(m.about).replacingOccurrences(of: "**", with: "")
                let tail = m.background ? " (\(backgroundNote))" : m.atMs.map { " [\(StudyParse.clock($0))]" } ?? ""
                return "- **\(about)**: \(oneLine(m.detail))\(tail)"
            }
            parts.append("## \(moreHeading)\n" + bullets.joined(separator: "\n"))
        }
        return parts.isEmpty ? nil : parts.joined(separator: "\n\n")
    }

    /// Lines as the Mac reads them (`str::lines`): split at "\n" or "\r\n"
    private static func lines(_ markdown: String) -> [Substring] {
        markdown.split(omittingEmptySubsequences: false, whereSeparator: { $0 == "\n" || $0 == "\r\n" })
    }

    /// A heading line's text: a `#` line, or a line that is only **bold**
    /// (how notes from the Mac write their headings). nil for any other line.
    private static func headingText(_ line: Substring) -> String? {
        let t = line.trimmingCharacters(in: .whitespacesAndNewlines)
        if t.hasPrefix("#") {
            return t.trimmingCharacters(in: CharacterSet(charactersIn: "#")).trimmingCharacters(in: .whitespacesAndNewlines)
                .trimmingCharacters(in: CharacterSet(charactersIn: "*")).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        if t.utf8.count > 4, t.hasPrefix("**"), t.hasSuffix("**") {
            let inner = t.dropFirst(2).dropLast(2)
            if !inner.contains("**") { return inner.trimmingCharacters(in: .whitespacesAndNewlines) }
        }
        return nil
    }

    /// `## What you missed` / `## More detail`, whatever the case
    private static func isOurs(_ heading: String) -> Bool {
        same(heading, missedHeading) || same(heading, moreHeading)
    }

    /// The notes have one of the two sections (written here, or on the Mac)
    static func has(_ markdown: String) -> Bool {
        lines(markdown).contains { headingText($0).map(isOurs) ?? false }
    }

    /// Notes Markdown without the two sections: each from its heading to the
    /// next heading or the end, wherever it sits. The rest is left as it
    /// is, but for blank lines at the end. The words inside a bullet or a
    /// paragraph are never a heading. (The Mac's `fill::strip_sections`.)
    static func strip(_ markdown: String) -> String {
        var out: [Substring] = []
        var skipping = false
        for line in lines(markdown) {
            if let heading = headingText(line) { skipping = isOurs(heading) }
            if !skipping { out.append(line) }
        }
        // No blank lines left hanging where a section was
        while let last = out.last, last.allSatisfy(\.isWhitespace) { out.removeLast() }
        return out.joined(separator: "\n")
    }

    /// What every removal calls: the notes without the two sections, and
    /// exactly as they were when they have neither.
    static func removed(from notes: String) -> String {
        has(notes) ? strip(notes) : notes
    }

    /// The notes with the two sections replaced by `sections` at the end (a
    /// refill never duplicates them). nil: just removed.
    static func replacing(in notes: String, with sections: String?) -> String {
        let base = strip(notes)
        guard let sections, !sections.isEmpty else { return base }
        return base.isEmpty ? sections : base + "\n\n" + sections
    }
}

// MARK: - The request

extension MeetingAI {
    /// Room for the answer (the Mac's `max_tokens`)
    static func fillMaxTokens(contextTokens: Int) -> Int {
        min(2500, max(256, contextTokens * 3 / 10))
    }

    /// The type's prompt, then how to read the input (the rules the notes prompt uses)
    static func fillSystem(for kind: RecordingKind, notebook: String?) -> String {
        FillInTheBlanks.basePrompt(for: kind, notebook: notebook) + "\n" + readingRules(for: kind)
    }

    /// The header, what the user already has, then the material (the
    /// transcript, or notes condensed from the whole of it). Ids never appear.
    static func fillUserMessage(_ input: NotesInput, have: String, condensed: Bool, body: String) -> String {
        let have = have.trimmingCharacters(in: .whitespacesAndNewlines)
        return "\(notesHeader(input))\nNOTES SO FAR:\n\(have.isEmpty ? "(none)" : have)\n\n"
            + (condensed ? "NOTES (condensed from the whole transcript, with times)" : "TRANSCRIPT")
            + ":\n\(body)\n\nJSON RESPONSE:"
    }

    /// Fill in the blanks: one request once the transcript fits (condensed
    /// in parts when it is long, never cut), and one more if the answer
    /// can't be used. AI access errors stop at once. `notes`: the notes as
    /// they are now. The caller's Pro, "AI set up" and consent checks are
    /// already done; nothing calls this but the user's tap (`NotesStore.fill`).
    static func fill(_ input: NotesInput, notes: String?, contextTokens: Int, complete: Complete,
                     progress: @Sendable (String) async -> Void = { _ in }) async throws -> FillInTheBlanks.Filled {
        guard input.study.hasTranscript else { throw NotesFailure.noTranscript }
        let kind = input.study.recordingKind
        let system = fillSystem(for: kind, notebook: input.study.courseName)
        let maxTokens = fillMaxTokens(contextTokens: contextTokens)
        let have = FillInTheBlanks.haveBlock(notes: notes, typed: input.typed,
                                             budget: FillInTheBlanks.haveCap(contextTokens: contextTokens, maxTokens: maxTokens, system: system))
        let fixed = fillUserMessage(input, have: have, condensed: true, body: "")
        let budget = bodyBudget(contextTokens: contextTokens, maxTokens: maxTokens, fixed: system + fixed)
        let source = StudyWords(kind).source
        let fit = try await condense(input.study, toFit: budget, contextTokens: contextTokens, complete: complete) { step in
            if case .reading(let part, let parts) = step {
                await progress("\(FillInTheBlanks.working) Reading the \(source) (\(part) of \(parts))…")
            }
        }
        let lines = spread(fit.lines, budget: budget)
        await progress(FillInTheBlanks.working)
        let user = fillUserMessage(input, have: have, condensed: fit.condensed, body: lines.joined(separator: "\n"))
        var why = ""
        for attempt in 0..<2 {
            let text = attempt == 0 ? user : user + "\n\n" + FillInTheBlanks.retryNote(why)
            let raw: String
            do {
                raw = try await complete([ChatMessage(role: "system", content: system), ChatMessage(role: "user", content: text)],
                                         maxTokens, attempt == 0 ? 0.3 : 0.4)
            } catch {
                if !isAnswerError(error) { throw error }
                why = error.localizedDescription
                continue
            }
            do {
                return try FillInTheBlanks.parse(raw, kind: kind, durationMs: input.study.durationMs)
            } catch {
                // Why, in fixed words: the answer itself is never repeated
                why = (error as? LocalizedError)?.errorDescription ?? "the answer couldn't be used"
            }
        }
        throw FillInTheBlanks.Failure.unusable
    }
}
