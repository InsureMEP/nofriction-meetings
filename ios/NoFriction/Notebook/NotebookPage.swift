import CryptoKit
import Foundation
import SwiftData

// The notebook page (docs/design/FADELL_V2.md §3 C and §6 "The notebook
// page"): everything for a course or a project in one place, computed on the
// device from what is already stored. Nothing here calls the AI, and reading
// the page is free.

/// An item on a notebook's page that the user checked off. Only a hash is
/// kept (`NotebookItems.hash`): the item's text is in the recording's notes
/// and nowhere else. Deleted with its recording (cascade), and when its item
/// leaves the notes (`NotebookItems.pruneDone`).
@Model
final class NotebookItemDone {
    /// SHA-256 of the recording's id and the item's normalized text
    var itemHash: String
    var doneAt: Date
    var meeting: Meeting?

    init(itemHash: String, doneAt: Date = .now) {
        self.itemHash = itemHash
        self.doneAt = doneAt
    }
}

/// What the notes found to do: the bullets under the notes' "Action items",
/// "Announcements and deadlines" or "To-dos and reminders" heading. Both
/// heading forms (`## Heading` and a line that is only `**Heading**`) and
/// both bullet forms (`- ` and `• `) are read, so notes written on the
/// iPhone and notes synced from the Mac give the same items. Each item has
/// a tick, the same one in the recording's Notes and on its notebook's page.
enum NotebookItems {
    static let headings = ["action items", "announcements and deadlines", "to-dos and reminders"]

    /// The section's title, by the notebook's usual type
    static func title(for kind: RecordingKind) -> String {
        switch kind {
        case .class: "Due"
        case .meeting: "Action items"
        case .personal: "To-dos"
        }
    }

    /// The empty section says what fills it (the Mac's words)
    static func emptyText(for kind: RecordingKind, anyNotes: Bool) -> String {
        guard anyNotes else { return "Nothing yet. Make notes on a recording, and what it says is to be done shows up here." }
        switch kind {
        case .class: return "No assignments, readings or deadlines were mentioned in the notes so far."
        case .meeting: return "No action items were found in the notes so far."
        case .personal: return "No to-dos or reminders were mentioned in the notes so far."
        }
    }

    /// The marks section holds the ✎ marks and the questions (not every ★),
    /// so it is named for them: "On the test and questions" for a class.
    static func markedTitle(for kind: RecordingKind) -> String {
        switch kind {
        case .class: "On the test and questions"
        case .meeting: "Follow ups and questions"
        case .personal: "To remember and questions"
        }
    }

    static func markedEmpty(for kind: RecordingKind) -> String {
        "Nothing marked yet. While you record, mark a moment and choose \(MarkerKind.test.label(for: kind)) or Question: it shows up here."
    }

    /// One short sentence when no guide has been made
    static func reviewEmpty(for kind: RecordingKind) -> String {
        "No \(kind == .class ? "study" : "review") guides yet. Make one on a recording, and its flashcards and practice questions join the others here."
    }

    private static func isItemsHeading(_ text: String) -> Bool {
        headings.contains(text.trimmingCharacters(in: CharacterSet(charactersIn: ":*_ ").union(.whitespaces)).lowercased())
    }

    /// "None recorded." and the like aren't items
    private static func isNothing(_ text: String) -> Bool {
        let t = text.lowercased().trimmingCharacters(in: CharacterSet.letters.inverted)
        return t.isEmpty || ["none", "none recorded", "none mentioned", "none stated", "nothing", "nothing mentioned", "n/a"].contains(t)
    }

    /// One bullet as an item
    struct Parsed: Equatable, Sendable {
        /// The bullet as shown and as the tick is kept for it: no checkbox, no bold marks, no "[owner not stated]"
        var text: String
        /// What to do, without the owner and the due date
        var task: String
        /// "[Alex] …" in meeting notes
        var assignee: String?
        /// "… (Friday)" in meeting notes
        var due: String?
    }

    /// A bullet of the items section. Meeting notes write an item as
    /// "[owner] task (due date)": the owner and the date are read out of
    /// that form only (a parenthesis in a class's deadline is part of it).
    static func parse(_ bullet: String) -> Parsed? {
        var t = bullet.replacingOccurrences(of: "**", with: "").split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        for box in ["[ ]", "[x]", "[X]"] where t.hasPrefix(box) { t = String(t.dropFirst(box.count)).trimmingCharacters(in: .whitespaces) }
        var assignee: String?
        var due: String?
        var task = t
        if t.hasPrefix("["), let close = t.firstIndex(of: "]") {
            let owner = t[t.index(after: t.startIndex)..<close].trimmingCharacters(in: .whitespaces)
            task = t[t.index(after: close)...].trimmingCharacters(in: .whitespaces)
            if !owner.isEmpty, owner.caseInsensitiveCompare("owner not stated") != .orderedSame { assignee = owner }
            if task.hasSuffix(")"), let open = task.range(of: " (", options: .backwards) {
                let inside = task[open.upperBound...].dropLast().trimmingCharacters(in: .whitespaces)
                let before = task[..<open.lowerBound].trimmingCharacters(in: .whitespaces)
                if !inside.isEmpty, inside.count <= 40, !before.isEmpty {
                    due = inside
                    task = before
                }
            }
            t = (assignee.map { "[\($0)] " } ?? "") + String(t[t.index(after: close)...]).trimmingCharacters(in: .whitespaces)
        }
        guard !task.isEmpty, !isNothing(task) else { return nil }
        return Parsed(text: t, task: task, assignee: assignee, due: due)
    }

    /// The bullets of the notes' items section (the last such heading), by
    /// their place in `blocks`: what the Notes view puts a tick beside.
    static func itemBlocks(_ blocks: [NotesMarkdown.Block]) -> [Int: Parsed] {
        guard let start = blocks.lastIndex(where: { if case .heading(let t, _) = $0 { return isItemsHeading(t) } else { return false } }) else { return [:] }
        var out: [Int: Parsed] = [:]
        for i in blocks.indices where i > start {
            if case .heading = blocks[i] { break }
            if case .bullet(let text, _) = blocks[i], let item = parse(text) { out[i] = item }
        }
        return out
    }

    /// The items in a recording's notes, in order, each once.
    static func parsed(inNotes markdown: String) -> [Parsed] {
        let found = itemBlocks(NotesMarkdown.blocks(markdown))
        var seen = Set<String>()
        return found.keys.sorted().compactMap { i in
            guard let item = found[i], seen.insert(normalize(item.text)).inserted else { return nil }
            return item
        }
    }

    static func texts(inNotes markdown: String) -> [String] { parsed(inNotes: markdown).map(\.text) }

    /// Lowercased, whitespace closed up, trailing punctuation dropped: the
    /// same item keeps its tick when the notes are made again
    static func normalize(_ text: String) -> String {
        let line = text.lowercased().split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        return line.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "[.!;,:]+$", with: "", options: .regularExpression)
    }

    /// Done state is kept by this, per recording: SHA-256 of the recording's
    /// id and the item's normalized text. The text itself is never stored
    /// with the tick.
    static func hash(_ text: String, meetingID: UUID) -> String {
        let data = Data((meetingID.uuidString.lowercased() + "\n" + normalize(text)).utf8)
        return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    struct Item: Identifiable, Equatable, Sendable {
        var meetingID: UUID
        /// The bullet as shown (the tick's accessibility label)
        var text: String
        var task: String
        var assignee: String?
        var due: String?
        var hash: String
        var id: String { hash }
    }

    static func item(_ p: Parsed, meetingID: UUID) -> Item {
        Item(meetingID: meetingID, text: p.text, task: p.task, assignee: p.assignee, due: p.due, hash: hash(p.text, meetingID: meetingID))
    }

    @MainActor
    static func items(of meeting: Meeting) -> [Item] {
        guard let notes = meeting.aiNotes else { return [] }
        return parsed(inNotes: notes).map { item($0, meetingID: meeting.id) }
    }

    @MainActor
    static func isDone(_ item: Item, in meeting: Meeting) -> Bool {
        meeting.doneItems.contains { $0.itemHash == item.hash }
    }

    @MainActor
    static func setDone(_ done: Bool, item: Item, meeting: Meeting, context: ModelContext) {
        let existing = meeting.doneItems.filter { $0.itemHash == item.hash }
        if done, existing.isEmpty {
            let row = NotebookItemDone(itemHash: item.hash)
            context.insert(row)
            row.meeting = meeting
        } else if !done {
            for row in existing { context.delete(row) }
        }
        try? context.save()
    }

    /// The notes changed (made again, redacted, deleted): done marks whose
    /// item is no longer in them go. Doesn't save: the caller's change does.
    @MainActor
    static func pruneDone(_ meeting: Meeting, context: ModelContext) {
        guard !meeting.doneItems.isEmpty else { return }
        let current = Set(items(of: meeting).map(\.hash))
        for row in meeting.doneItems where !current.contains(row.itemHash) { context.delete(row) }
    }

    /// The open items to show under a suggestion: the first few, and how many more
    struct Preview: Equatable, Sendable {
        var shown: [Item] = []
        var more = 0
    }

    static func preview(open: [Item], max: Int = 3) -> Preview {
        Preview(shown: Array(open.prefix(max)), more: Swift.max(0, open.count - max))
    }

    /// What a notebook's notes still have open, newest recording first.
    @MainActor
    static func open(in recordings: [Meeting]) -> [Item] {
        recordings.sorted { $0.startedAt > $1.startedAt }.flatMap { m in items(of: m).filter { !isDone($0, in: m) } }
    }
}

/// **Review everything**: the flashcards and practice questions of every
/// stored study guide in the notebook, as one deck and one quiz. No AI call:
/// it is the guides already made, together. It can start from any recording
/// ("since the midterm"): that recording and every later one.
enum NotebookReview {
    /// One recording's stored guide
    struct Source: Equatable, Sendable {
        var meetingID: UUID
        var title: String
        var startedAt: Date
        var cards: [StudyCards.Card]
        var questions: [StudyQuiz.Item]

        init(meetingID: UUID, title: String = "", startedAt: Date, cards: [StudyCards.Card], questions: [StudyQuiz.Item]) {
            self.meetingID = meetingID
            self.title = title
            self.startedAt = startedAt
            self.cards = cards
            self.questions = questions
        }

        var hasGuide: Bool { !cards.isEmpty || !questions.isEmpty }
    }

    /// A practice question and the recording it jumps to
    struct Question: Equatable, Sendable {
        var item: StudyQuiz.Item
        var meetingID: UUID
    }

    struct Everything: Equatable, Sendable {
        var cards: [StudyCards.Card] = []
        var questions: [Question] = []
        /// How many recordings' guides it comes from
        var guides = 0

        var isEmpty: Bool { cards.isEmpty && questions.isEmpty }
    }

    private static func key(_ s: String) -> String { String(s.lowercased().filter { $0.isLetter || $0.isNumber }) }

    /// The recordings that have a guide, oldest first: what "Start from" lists.
    static func guides(_ sources: [Source]) -> [Source] {
        sources.filter(\.hasGuide).sorted { ($0.startedAt, $0.meetingID.uuidString) < ($1.startedAt, $1.meetingID.uuidString) }
    }

    /// The guides from one recording on: that recording and every later
    /// one. No id, the first one or an unknown one is every guide.
    static func guides(_ sources: [Source], from start: UUID?) -> [Source] {
        let all = guides(sources)
        guard let start, let index = all.firstIndex(where: { $0.meetingID == start }), index > 0 else { return all }
        return Array(all[index...])
    }

    /// The union, in recording order (oldest first); a card or a question
    /// that two guides share is kept once. `from`: start at that recording.
    static func everything(_ sources: [Source], from start: UUID? = nil) -> Everything {
        var out = Everything()
        var seenCards = Set<String>()
        var seenQuestions = Set<String>()
        for s in guides(sources, from: start) {
            out.guides += 1
            for c in s.cards where seenCards.insert(key(c.front) + "|" + key(c.back)).inserted { out.cards.append(c) }
            for q in s.questions where seenQuestions.insert(key(q.question)).inserted { out.questions.append(Question(item: q, meetingID: s.meetingID)) }
        }
        return out
    }

    @MainActor
    static func source(_ meeting: Meeting) -> Source {
        let cards = meeting.studyMaterial(.flashcards).flatMap { StudyParse.decode(StudyCards.self, $0.json) }?.cards ?? []
        let quiz = meeting.studyMaterial(.quiz).flatMap { StudyParse.decode(StudyQuiz.self, $0.json) }?.questions ?? []
        return Source(meetingID: meeting.id, title: meeting.title, startedAt: meeting.startedAt, cards: cards, questions: quiz)
    }
}

enum NotebookPage {
    /// The notebook's usual type: the type of its latest recording
    @MainActor
    static func usualKind(_ recordings: [Meeting]) -> RecordingKind {
        recordings.max { $0.startedAt < $1.startedAt }?.kind ?? .default
    }

    /// "Class · 3 recordings · 2 h 30 min" under the notebook's name
    static func summary(kind: RecordingKind, count: Int, seconds: TimeInterval) -> String {
        var parts = [kind.label, count == 1 ? "1 recording" : "\(count) recordings"]
        if seconds >= 60 { parts.append(seconds.minutesLabel) }
        return parts.joined(separator: " · ")
    }

    /// "Nov 7" for the line under an item or a mark
    static func shortDate(_ date: Date) -> String {
        date.formatted(.dateTime.month(.abbreviated).day())
    }
}
