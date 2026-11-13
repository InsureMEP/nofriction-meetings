import Foundation
import Observation
import SwiftData

// Notes that read the whole recording and build on what the user wrote
// (docs/design/FADELL_V2.md §3 B and §6 "Notes generation").
//
// - Input: the whole stored transcript with times (`[m:ss] text`, stricken
//   spans as `[stricken from the record]`), the marks, and the typed notes
//   numbered 1..n with their times. Ids never go in a prompt.
// - A transcript that doesn't fit beside the prompt is condensed in parts
//   first (the review guide's map step, `MeetingAI.condense`). Nothing is
//   cut off the end or out of the middle, and the client never has to trim.
// - With typed notes, the Markdown starts with `## What you wrote`: one
//   bullet per line, `- **the user's line**: what was being said then`.
//   The per-type sections follow, unchanged.
//
// Requests go through `AIClient` (Apple on-device or the user's endpoint),
// with the caller's Pro, "AI set up" and consent checks already done.

/// What notes are made from. Built on the main actor (SwiftData models
/// aren't Sendable); the value itself is Sendable.
struct NotesInput: Sendable, Equatable {
    /// Title, type, notebook, the transcript with times, the marks and the typed notes
    var study: StudyInput
    /// "Wednesday, November 12, 2026 at 9:00 AM"
    var when: String
    /// "Dana Whitfield (Brightwater)"
    var attendees: [String]
    var inviteNotes: String?

    /// Invite notes can be a page of dial-in boilerplate
    static let maxInviteNotes = 800

    init(study: StudyInput, when: String = "", attendees: [String] = [], inviteNotes: String? = nil) {
        self.study = study
        self.when = when
        self.attendees = attendees
        self.inviteNotes = inviteNotes
    }

    @MainActor init(meeting m: Meeting) {
        study = StudyInput(meeting: m, screenPrefix: true)
        when = m.startedAt.formatted(date: .complete, time: .shortened)
        attendees = m.people.map { p in p.person.displayName + (p.person.company.map { " (\($0))" } ?? "") }
        inviteNotes = m.inviteNotes
    }

    /// The user's lines, in time order
    var typed: [StudyInput.Mark] { study.typed }
}

extension MeetingAI {
    /// Header of the numbered typed notes in the notes prompt
    static let typedHeader = "TYPED NOTES"

    enum NotesFailure: LocalizedError, Equatable {
        case noTranscript
        case transcriptChanged
        var errorDescription: String? {
            switch self {
            case .noTranscript: "This recording has no transcript to make notes from."
            case .transcriptChanged: "The transcript changed while the notes were being made (it was edited), so they weren't saved. Make them again."
            }
        }
    }

    /// The notes prompt: the type's prompt exactly as it was (Meeting, Class
    /// or Personal sections), then how to read the input, and, when the user
    /// typed notes, the `## What you wrote` section that comes first.
    static func notesSystem(for kind: RecordingKind, typedNotes: Bool) -> String {
        let w = StudyWords(kind)
        var s = notesSystem(for: kind) + "\n\n"
        s += """
            Each transcript line starts with its time as [m:ss]. A long recording arrives as time-stamped notes condensed from \
            the whole transcript instead; write from them the same way. Text shown as [stricken from the record] was removed by \
            the \(w.person): never guess at or mention what it said. The \(w.person) marked moments while listening (\(w.marksHeader))
            """
        s += typedNotes ? " and typed their own lines (\(typedHeader)). " : ". "
        s += "Cover what was marked\(typedNotes ? " and typed" : "") first: make sure the notes include what was being said at those moments."
        if typedNotes {
            s += """
                \n
                Start your answer with this section, before the sections listed above:
                ## \(WhatYouWrote.title)
                (one bullet for each numbered line under \(typedHeader), in the same order, as \
                "- **the line exactly as it was typed**: one or two sentences of what was being said at that time". \
                Copy each line word for word: never correct, translate or shorten it. If nothing was being said about it \
                then, write "Nothing was said about this.")
                Then write the sections listed above, exactly as described.
                """
        }
        return s
    }

    /// "Class: Lecture 7" / When / Notebook / Attendees / Invite notes / Length
    static func notesHeader(_ input: NotesInput) -> String {
        let i = input.study
        let title = i.title.trimmingCharacters(in: .whitespacesAndNewlines)
        var s = "\(i.recordingKind.label): \(title.isEmpty ? "Untitled" : title)\n"
        if !input.when.isEmpty { s += "When: \(input.when)\n" }
        if let c = i.courseName?.trimmingCharacters(in: .whitespacesAndNewlines), !c.isEmpty { s += "\(Notebook.label): \(c)\n" }
        if !input.attendees.isEmpty { s += "Attendees: \(input.attendees.joined(separator: ", "))\n" }
        if let notes = input.inviteNotes?.trimmingCharacters(in: .whitespacesAndNewlines), !notes.isEmpty {
            s += "Invite notes: \(StudyInput.clip(notes, to: NotesInput.maxInviteNotes))\n"
        }
        if i.durationMs > 0 { s += "Length: \(StudyParse.clock(i.durationMs))\n" }
        return s
    }

    /// Whole lines that fit `budget` characters: each line is clipped first,
    /// and what still doesn't fit is left off with a count. Marks and typed
    /// notes are short, so this only acts on a small model window.
    static func fitLines(_ lines: [String], budget: Int) -> [String] {
        guard lines.reduce(0, { $0 + $1.count + 1 }) > budget else { return lines }
        let per = max(60, budget / max(1, lines.count) - 1)
        var out: [String] = []
        var used = 0
        for (i, l) in lines.enumerated() {
            let line = StudyInput.clip(l, to: per)
            if used + line.count + 1 > budget, !out.isEmpty {
                out.append("(and \(lines.count - i) more)")
                break
            }
            out.append(line)
            used += line.count + 1
        }
        return out
    }

    /// The marks in the review guide's format (never the typed notes), and
    /// the typed notes numbered 1..n with their times: `1. [3:05] the line`.
    static func notesMarks(_ input: NotesInput, budget: Int) -> (marks: String, typed: String?) {
        let marks = input.study.marksBlock(typedNotes: false)
        let marksFit = marks == "(none)" ? marks : fitLines(marks.components(separatedBy: "\n"), budget: budget / 2).joined(separator: "\n")
        let typed = input.typed
        guard !typed.isEmpty else { return (marksFit, nil) }
        let lines = typed.enumerated().map { n, t in
            "\(n + 1). [\(StudyParse.clock(t.ms))] \((t.note ?? "").trimmingCharacters(in: .whitespacesAndNewlines))"
        }
        // The typed notes get their half, and whatever the marks left of theirs
        return (marksFit, fitLines(lines, budget: max(budget / 2, budget - marksFit.count - 1)).joined(separator: "\n"))
    }

    static func notesUserMessage(_ input: NotesInput, condensed: Bool, body: String, marksBudget: Int = .max) -> String {
        let w = StudyWords(input.study.recordingKind)
        let blocks = notesMarks(input, budget: marksBudget)
        var s = "\(notesHeader(input))\n\(w.marksHeader):\n\(blocks.marks)\n\n"
        if let typed = blocks.typed { s += "\(typedHeader):\n\(typed)\n\n" }
        s += (condensed ? "NOTES (condensed from the transcript, with times)" : "TRANSCRIPT") + ":\n\(body)"
        return s
    }

    /// Room for the answer: the type's sections, plus a bullet per typed note.
    static func notesMaxTokens(typedNotes: Int, contextTokens: Int) -> Int {
        min(1500 + 80 * min(max(0, typedNotes), 40), max(256, contextTokens * 3 / 10))
    }

    /// Characters the marks and typed notes may take in the prompt: a
    /// quarter of what fits beside the system prompt and the answer.
    static func notesMarksBudget(contextTokens: Int, maxTokens: Int, system: String) -> Int {
        max(300, bodyBudget(contextTokens: contextTokens, maxTokens: maxTokens, fixed: system) / 4)
    }

    /// How many of the lines the notes were made from are no longer in the
    /// transcript as they were (deleted, stricken or edited). Each line
    /// counts as often as it occurs; added lines don't count: the
    /// recognizer's last lines can land after Stop, and that is no reason to
    /// throw the notes away. (Same rule as the Mac's `lost_lines`.)
    static func lostLines(read: [String], now: [String]) -> Int {
        var have: [String: Int] = [:]
        for line in now { have[line, default: 0] += 1 }
        var lost = 0
        for line in read {
            if let n = have[line], n > 0 { have[line] = n - 1 } else { lost += 1 }
        }
        return lost
    }

    /// Last resort, when three rounds of condensing still don't fit: lines
    /// spread evenly over the whole recording, so the start, the middle and
    /// the end are all still there and the client has nothing to trim.
    static func spread(_ lines: [String], budget: Int) -> [String] {
        let total = lines.reduce(0) { $0 + $1.count + 1 }
        guard total > budget, lines.count > 1 else {
            return total > budget ? lines.map { StudyInput.clip($0, to: max(1, budget - 1)) } : lines
        }
        let average = max(1, total / lines.count)
        var keep = max(1, min(lines.count, budget / average))
        while keep >= 1 {
            let picked = keep == 1 ? [lines[lines.count - 1]]
                : (0..<keep).map { lines[$0 * (lines.count - 1) / (keep - 1)] }
            if picked.reduce(0, { $0 + $1.count + 1 }) <= budget || keep == 1 {
                return keep == 1 ? picked.map { StudyInput.clip($0, to: max(1, budget - 1)) } : picked
            }
            keep -= 1
        }
        return []
    }

    /// Make notes: the model's Markdown, as it answered. `NotesStore` turns
    /// it into what is stored (`WhatYouWrote.finalize`). AI access errors,
    /// and an unusable answer, throw.
    static func notes(_ input: NotesInput, contextTokens: Int, complete: Complete,
                      progress: @Sendable (String) async -> Void = { _ in }) async throws -> String {
        guard input.study.hasTranscript else { throw NotesFailure.noTranscript }
        let kind = input.study.recordingKind
        let system = notesSystem(for: kind, typedNotes: !input.typed.isEmpty)
        let maxTokens = notesMaxTokens(typedNotes: input.typed.count, contextTokens: contextTokens)
        let marksBudget = notesMarksBudget(contextTokens: contextTokens, maxTokens: maxTokens, system: system)
        let fixed = notesUserMessage(input, condensed: true, body: "", marksBudget: marksBudget)
        let budget = bodyBudget(contextTokens: contextTokens, maxTokens: maxTokens, fixed: system + fixed)
        let source = StudyWords(kind).source
        let fit = try await condense(input.study, toFit: budget, contextTokens: contextTokens, complete: complete) { step in
            if case .reading(let part, let parts) = step { await progress("Reading the \(source) (\(part) of \(parts))…") }
        }
        let lines = spread(fit.lines, budget: budget)
        await progress("Writing the notes…")
        let user = notesUserMessage(input, condensed: fit.condensed, body: lines.joined(separator: "\n"), marksBudget: marksBudget)
        return try await complete([ChatMessage(role: "system", content: system), ChatMessage(role: "user", content: user)], maxTokens, 0.2)
    }
}

// MARK: - "What you wrote"

/// The notes' first section when the user typed notes (iPhone Markdown;
/// docs/design/FADELL_V2.md §6): `## What you wrote`, then one bullet per
/// line, `- **the user's line**: what was being said then`. It is AI
/// output: redaction treats it like the rest of the notes.
enum WhatYouWrote {
    static let title = "What you wrote"

    struct Entry: Equatable, Sendable {
        /// The user's line
        var line: String
        /// What was being said then ("" when the model gave none)
        var detail: String
    }

    struct Parsed: Equatable, Sendable {
        /// The section's bullets as written (without their list mark)
        var bullets: [String]
        /// The notes without the section
        var rest: String

        var entries: [Entry] { bullets.map(WhatYouWrote.entry) }
    }

    private static func isTitle(_ line: String) -> Bool {
        guard let (text, _) = NotesMarkdown.heading(line.trimmingCharacters(in: .whitespaces)) else { return false }
        let t = text.trimmingCharacters(in: CharacterSet(charactersIn: ":*_ ")).lowercased()
        return t == title.lowercased()
    }

    /// One line, whitespace closed up, bold marks gone: a user's line as the
    /// section carries it, and how a bullet is compared with a line (the
    /// Mac's `quoted`).
    static func quoted(_ text: String) -> String {
        text.replacingOccurrences(of: "**", with: "").split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }

    /// `**line**: detail` (the model's shape), or `line: detail` without the bold.
    static func entry(_ bullet: String) -> Entry {
        let t = bullet.trimmingCharacters(in: .whitespaces)
        func detail(_ rest: Substring) -> String {
            var d = rest.trimmingCharacters(in: .whitespaces)
            for lead in [":", "—", "–", "-"] where d.hasPrefix(lead) { d = String(d.dropFirst(lead.count)).trimmingCharacters(in: .whitespaces); break }
            return d
        }
        if t.hasPrefix("**") {
            let body = t.dropFirst(2)
            // The user's own line may contain "**": the close is the one before the colon
            for close in ["**: ", "**:", "**"] {
                guard let r = body.range(of: close) else { continue }
                let line = body[..<r.lowerBound].trimmingCharacters(in: .whitespaces)
                if line.isEmpty { continue }
                let after = close == "**" ? body[r.upperBound...] : body[body.index(r.lowerBound, offsetBy: 2)...]
                return Entry(line: line, detail: detail(after))
            }
        }
        if let r = t.range(of: ": ") {
            return Entry(line: t[..<r.lowerBound].trimmingCharacters(in: .whitespaces), detail: detail(t[r.upperBound...]))
        }
        return Entry(line: t, detail: "")
    }

    /// The section's bullets and the rest of the notes; nil when the notes have no such section.
    static func parse(_ markdown: String) -> Parsed? {
        let lines = markdown.components(separatedBy: "\n")
        guard let start = lines.firstIndex(where: isTitle) else { return nil }
        var end = lines.count
        for i in (start + 1)..<lines.count where NotesMarkdown.heading(lines[i].trimmingCharacters(in: .whitespaces)) != nil {
            end = i
            break
        }
        var bullets: [String] = []
        for raw in lines[(start + 1)..<end] {
            let t = raw.trimmingCharacters(in: .whitespaces)
            if t.isEmpty { continue }
            if let bullet = NotesMarkdown.bullet(t) {
                bullets.append(bullet)
            } else if !bullets.isEmpty {
                // A wrapped detail carries on under its bullet
                bullets[bullets.count - 1] += " " + t
            }
        }
        let rest = (Array(lines[..<start]) + Array(lines[end...])).joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return Parsed(bullets: bullets, rest: rest)
    }

    /// Letters and digits only, lowercased: a model's copy of a line, compared loosely
    static func key(_ s: String) -> String { String(s.lowercased().filter { $0.isLetter || $0.isNumber }) }

    /// A bullet whose line was struck in part reads "ask about [stricken from
    /// the record]": it still belongs to the line it was written for.
    private static func matchesStruck(_ bulletLine: String, _ typed: String) -> Bool {
        guard bulletLine.contains(RedactionText.placeholder) else { return false }
        let parts = quoted(bulletLine).components(separatedBy: RedactionText.placeholder)
            .map { NSRegularExpression.escapedPattern(for: $0.trimmingCharacters(in: .whitespaces)) }
        guard let re = try? NSRegularExpression(pattern: "^" + parts.joined(separator: ".*") + "$", options: [.caseInsensitive, .dotMatchesLineSeparators]) else { return false }
        let line = quoted(typed)
        return re.firstMatch(in: line, range: NSRange(line.startIndex..., in: line)) != nil
    }

    /// What was merged: for each typed note, what the notes wrote under it;
    /// and the bullets no typed note matches, which stay visible.
    struct Merged: Equatable, Sendable {
        /// For each typed note (in the order typed): the index of its bullet
        var bullet: [Int?]
        /// For each typed note: the detail ("" when none was written)
        var details: [String]
        /// Bullets that quote no typed note here (its note hasn't crossed
        /// yet, or the other device is older): never hidden
        var unmatched: [Entry]
    }

    /// The section merged with the typed notes this device holds, by line
    /// (the Mac's `mergeSyncedWritten`): a bullet matches the longest typed
    /// line it equals or starts with (followed by ":"), each note at most
    /// once. A bullet that only differs in case or punctuation, or whose
    /// line was struck in part, still finds its note. Every typed note has
    /// its row whether or not a bullet matched; a bullet that matched none
    /// stays.
    static func merge(_ bullets: [String], typed: [String]) -> Merged {
        var bullet = [Int?](repeating: nil, count: typed.count)
        var details = [String](repeating: "", count: typed.count)
        var taken = Set<Int>()
        let lines = typed.map(quoted)
        // As written: equal to the line, or the line followed by ":"
        for (b, raw) in bullets.enumerated() {
            let text = quoted(raw)
            var best: Int?
            for (i, q) in lines.enumerated() where !q.isEmpty && bullet[i] == nil && (text == q || text.hasPrefix(q + ":")) {
                if best.map({ q.count > lines[$0].count }) ?? true { best = i }
            }
            guard let i = best else { continue }
            bullet[i] = b
            taken.insert(b)
            var rest = text.dropFirst(lines[i].count).trimmingCharacters(in: .whitespaces)
            if rest.hasPrefix(":") { rest = String(rest.dropFirst()).trimmingCharacters(in: .whitespaces) }
            details[i] = rest
        }
        // Loosely: the same words in another case or punctuation, or a line struck in part
        for (b, raw) in bullets.enumerated() where !taken.contains(b) {
            let e = entry(raw)
            guard let i = lines.indices.first(where: { bullet[$0] == nil && !lines[$0].isEmpty && (key(e.line) == key(lines[$0]) || matchesStruck(e.line, lines[$0])) }) else { continue }
            bullet[i] = b
            taken.insert(b)
            details[i] = e.detail
        }
        return Merged(bullet: bullet, details: details, unmatched: bullets.indices.filter { !taken.contains($0) }.map { entry(bullets[$0]) })
    }

    private static func bulletLine(_ line: String, _ detail: String) -> String {
        let d = detail.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        return d.isEmpty ? "- **\(quoted(line))**" : "- **\(quoted(line))**: \(d)"
    }

    /// `## What you wrote` listing every typed note in the order typed:
    /// `- **line**: detail` when something was written under it,
    /// `- **line**` when not. One line each, whitespace closed up, `**`
    /// removed from the user's line. Then any bullet no note matches.
    /// "" when there is nothing to list.
    static func section(typed: [String], details: [String], unmatched: [Entry] = []) -> String {
        let listed = zip(typed, details).filter { !quoted($0.0).isEmpty }.map { bulletLine($0.0, $0.1) }
            + unmatched.filter { !quoted($0.line).isEmpty }.map { bulletLine($0.line, $0.detail) }
        return listed.isEmpty ? "" : "## \(title)\n" + listed.joined(separator: "\n")
    }

    private static func assemble(_ section: String, rest: String) -> String {
        [section, rest].filter { !$0.isEmpty }.joined(separator: "\n\n")
    }

    /// Model output → the notes that are stored: the section comes first and
    /// lists every typed note exactly as typed (a model corrects spelling
    /// and shortens), each with the model's sentence about it when it wrote
    /// one. The model was asked for one bullet per line in order, so with
    /// one bullet per line they pair by position; otherwise by their words.
    /// No typed notes: no section.
    static func finalize(_ raw: String, typed: [String]) -> String {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let parsed = parse(text)
        guard !typed.isEmpty else { return parsed?.rest ?? text }
        guard let parsed else { return assemble(section(typed: typed, details: typed.map { _ in "" }), rest: text) }
        let entries = parsed.entries
        let details = entries.count == typed.count ? entries.map(\.detail) : merge(parsed.bullets, typed: typed).details
        return assemble(section(typed: typed, details: details), rest: parsed.rest)
    }

    /// The same, when the typed notes may have changed while the notes were
    /// being written (one deleted or edited here, one arriving from the
    /// other device). `asked` are the notes the request numbered, in that
    /// order; `now` the notes the recording holds. Each sentence stays with
    /// the note it was written for, by identity (the Mac keys them by
    /// marker id): an edited note keeps its sentence, a deleted note's
    /// sentence is dropped and never lands under another line, and a note
    /// the request didn't have gets none.
    static func finalize(_ raw: String, asked: [UUID], now: [(id: UUID, line: String)]) -> String {
        let lines = now.map(\.line)
        guard asked != now.map(\.id) else { return finalize(raw, typed: lines) }
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let parsed = parse(text)
        guard !now.isEmpty else { return parsed?.rest ?? text }
        guard let parsed else { return assemble(section(typed: lines, details: lines.map { _ in "" }), rest: text) }
        var detail: [UUID: String] = [:]
        let entries = parsed.entries
        if entries.count == asked.count {
            // One bullet per numbered line, in order
            for (id, e) in zip(asked, entries) { detail[id] = e.detail }
        } else {
            // By their words, among the notes that were asked about
            let known = Set(asked)
            let kept = now.filter { known.contains($0.id) }
            for (n, d) in zip(kept, merge(parsed.bullets, typed: kept.map(\.line)).details) { detail[n.id] = d }
        }
        return assemble(section(typed: lines, details: now.map { detail[$0.id] ?? "" }), rest: parsed.rest)
    }

    /// The user deleted their line number `typedIndex`: what the notes wrote
    /// under it leaves with it, in the same step. The section still lists
    /// every other typed note.
    static func removing(typedIndex: Int, typed: [String], from markdown: String) -> String {
        guard let parsed = parse(markdown), typed.indices.contains(typedIndex) else { return markdown }
        let merged = merge(parsed.bullets, typed: typed)
        guard merged.bullet[typedIndex] != nil else { return markdown }
        var lines = typed, details = merged.details
        lines.remove(at: typedIndex)
        details.remove(at: typedIndex)
        return assemble(section(typed: lines, details: details, unmatched: merged.unmatched), rest: parsed.rest)
    }

    /// The user changed their line number `typedIndex`: its bullet shows the new words.
    static func replacing(typedIndex: Int, with line: String, typed: [String], in markdown: String) -> String {
        guard let parsed = parse(markdown), typed.indices.contains(typedIndex) else { return markdown }
        let merged = merge(parsed.bullets, typed: typed)
        guard merged.bullet[typedIndex] != nil else { return markdown }
        var lines = typed
        lines[typedIndex] = line
        return assemble(section(typed: lines, details: merged.details, unmatched: merged.unmatched), rest: parsed.rest)
    }
}

// MARK: - Making and saving

/// Make notes for a recording and save them: the one path for Make notes on
/// a recording, in the Export sheet, and automatic notes.
@MainActor
enum NotesStore {
    /// Runs the requests, then saves in one step (`save`). Topics are named
    /// with the notes, as before; a failure there is the caller's to show
    /// or ignore (`topicsError`).
    @discardableResult
    static func make(_ meeting: Meeting, contextTokens: Int, complete: MeetingAI.Complete, context: ModelContext,
                     progress: @Sendable (String) async -> Void = { _ in }) async throws -> String {
        let input = NotesInput(meeting: meeting)
        let read = input.study.transcriptLines
        // The typed notes as the request numbers them, by identity
        let asked = typedNotes(meeting).map(\.id)
        let raw = try await MeetingAI.notes(input, contextTokens: contextTokens, complete: complete, progress: progress)
        return try save(raw, madeFrom: read, asked: asked, to: meeting, context: context)
    }

    /// The typed notes a request numbers (`StudyInput.typed`): in time order, never a blank one
    private static func typedNotes(_ meeting: Meeting) -> [(id: UUID, line: String)] {
        meeting.typedNotes.compactMap { m in
            guard let line = m.note, !line.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            return (m.id, line)
        }
    }

    /// The save and its check are one step: synchronous on the model
    /// context, with no `await` between the check and the write, so a Delete
    /// or Strike can't land in between. The notes are written only if every
    /// transcript line they were made from is still there as it was (lines
    /// added meanwhile, the recognizer's last ones after Stop, are no reason
    /// to refuse) and no Delete is waiting in its undo window. `asked`: the
    /// typed notes the request was made with, so each sentence is kept with
    /// its own note if they changed meanwhile (`WhatYouWrote.finalize`).
    @discardableResult
    static func save(_ raw: String, madeFrom read: [String], asked: [UUID]? = nil, to meeting: Meeting, context: ModelContext) throws -> String {
        guard meeting.modelContext != nil, !meeting.isDeleted,
              !meeting.redactions.contains(where: { !$0.isStrike }),
              MeetingAI.lostLines(read: read, now: StudyInput(meeting: meeting, screenPrefix: true).transcriptLines) == 0 else {
            throw MeetingAI.NotesFailure.transcriptChanged
        }
        // The user's lines as they are now (one may have been edited or deleted meanwhile)
        let now = typedNotes(meeting)
        let text = WhatYouWrote.finalize(raw, asked: asked ?? now.map(\.id), now: now)
        meeting.aiNotes = text
        meeting.aiNotesAt = .now
        meeting.aiNotesStale = false
        NotebookItems.pruneDone(meeting, context: context)
        try? context.save()
        return text
    }
}

/// When automatic notes run (pure; unit-tested).
enum AutoNotesRule {
    /// Six minutes; one minute for a recording with a typed note or a mark
    static let minimumSeconds: TimeInterval = 6 * 60
    static let engagedMinimumSeconds: TimeInterval = 60

    struct Facts: Equatable, Sendable {
        var settingOn = true
        var isPro = false
        /// An AI connection is set up (Apple on-device, or the user's endpoint)
        var aiReady = false
        /// A public endpoint already has the user's consent (always true for on-device and local ones)
        var consentGranted = false
        var seconds: TimeInterval = 0
        var typedNotes = 0
        var marks = 0
        var hasTranscript = false
        var hasNotes = false
        /// App audio from screen capture, or an imported recording, is still
        /// being transcribed: the notes wait for it
        var stillTranscribing = false
    }

    /// All of: the setting, Pro, AI set up, consent already given, a
    /// transcript (or one still arriving), no notes yet, and long enough
    /// (six minutes, or one with a typed note or a mark).
    static func shouldRun(_ f: Facts) -> Bool {
        guard f.settingOn, f.isPro, f.aiReady, f.consentGranted, f.hasTranscript || f.stillTranscribing, !f.hasNotes else { return false }
        if f.seconds >= minimumSeconds { return true }
        return f.seconds >= engagedMinimumSeconds && (f.typedNotes > 0 || f.marks > 0)
    }
}

/// Automatic notes (Settings → Recording → Make notes automatically, on):
/// after a recording stops, its notes are made in the background when
/// nothing would have to be asked. It never raises the paywall, the AI setup
/// sheet or the consent dialog, and a failure is silent: the recording keeps
/// its Make notes button.
///
/// The notes start once the transcript has settled. `RecordingSession.stop()`
/// has already waited for the recognizer's final flush by the time a
/// recording counts as stopped; what can still arrive is the app audio from
/// screen capture (and an imported recording's text), so the run waits for
/// that. "Making notes…" shows from the moment of Stop, through the wait.
///
/// Nothing automatic calls the AI while a recording runs: if the next
/// recording has started, every request of the run waits for it to stop.
/// (The user's own Make notes is theirs to press at any time.)
@MainActor
@Observable
final class AutoNotes {
    nonisolated static let settingKey = "autoNotes"
    /// How long the notes wait for a transcript that is still arriving
    nonisolated static let longestWait: Duration = .seconds(15 * 60)

    static var isOn: Bool { UserDefaults.standard.object(forKey: settingKey) as? Bool ?? true }

    /// Recordings whose notes are being made right now ("Making notes…")
    private(set) var working: Set<UUID> = []

    func isWorking(_ meeting: Meeting) -> Bool { working.contains(meeting.id) }

    private static func stillTranscribing(_ meeting: Meeting) -> Bool {
        meeting.importPhase != nil || meeting.screenAudioJSON != nil
    }

    static func facts(_ meeting: Meeting, isPro: Bool, endpoint: AIEndpoint?) -> AutoNotesRule.Facts {
        AutoNotesRule.Facts(settingOn: isOn, isPro: isPro, aiReady: endpoint != nil,
                            consentGranted: endpoint.map { !$0.needsConsent || $0.consentGranted } ?? false,
                            seconds: meeting.duration ?? 0, typedNotes: meeting.typedNotes.count, marks: meeting.marks.count,
                            hasTranscript: StudyInput(meeting: meeting).hasTranscript, hasNotes: meeting.aiNotes != nil,
                            stillTranscribing: stillTranscribing(meeting))
    }

    /// A recording just stopped. `complete` is the AI (tests pass a mock);
    /// `poll` is how often a transcript that is still arriving, or a
    /// recording that is running, is looked at; `isRecording` says whether
    /// one is running now.
    @discardableResult
    func recordingStopped(_ meeting: Meeting, isPro: Bool, endpoint: AIEndpoint?, context: ModelContext,
                          complete: MeetingAI.Complete? = nil, poll: Duration = .seconds(1),
                          isRecording: @escaping @MainActor @Sendable () -> Bool = { false }) -> Task<Void, Never>? {
        guard let endpoint, AutoNotesRule.shouldRun(Self.facts(meeting, isPro: isPro, endpoint: endpoint)),
              !working.contains(meeting.id) else { return nil }
        let id = meeting.id
        let ask = complete ?? MeetingAI.liveComplete(endpoint)
        // Every request waits while a recording runs
        let run: MeetingAI.Complete = { messages, maxTokens, temperature in
            while await isRecording() { try await Task.sleep(for: poll) }
            return try await ask(messages, maxTokens, temperature)
        }
        // From the moment of Stop
        working.insert(id)
        return Task { @MainActor in
            defer { working.remove(id) }
            // The transcript settles first
            let giveUp = ContinuousClock.now + Self.longestWait
            while Self.stillTranscribing(meeting) {
                guard ContinuousClock.now < giveUp, meeting.modelContext != nil, !meeting.isDeleted else { return }
                try? await Task.sleep(for: poll)
            }
            // And the next recording, if one has started, stops first
            while isRecording() {
                guard meeting.modelContext != nil, !meeting.isDeleted else { return }
                try? await Task.sleep(for: poll)
            }
            // The user may have pressed Make notes meanwhile, or deleted it
            guard meeting.modelContext != nil, !meeting.isDeleted, meeting.aiNotes == nil, StudyInput(meeting: meeting).hasTranscript else { return }
            guard (try? await NotesStore.make(meeting, contextTokens: endpoint.contextTokens, complete: run, context: context)) != nil else { return }
            // Same hook as Make notes: topics are named with the notes (quietly)
            if let found = try? await MeetingAI.findTopics(StudyInput(meeting: meeting), contextTokens: endpoint.contextTokens, complete: run),
               meeting.modelContext != nil, !meeting.isDeleted {
                _ = try? TopicStore.applyAI(found, to: meeting, context: context)
            }
        }
    }
}
