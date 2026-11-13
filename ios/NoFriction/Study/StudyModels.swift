import Foundation
import SwiftData

// `MarkerKind` (★ / ? / ✎ and their labels by recording type, and `note`) is
// shared with Apple Watch: ios/Shared/RecordingVocabulary.swift.

/// A moment the user marked while recording (★ Important, ? Question, and
/// ✎ On the test / Follow up / Remember by type), with an optional short
/// note; or, with kind `note`, a line the user typed while recording
/// ("Type a note", docs/design/FADELL_V2.md §6). The text is user content: it
/// goes with its recording (cascade). Same rules as the Mac
/// (`meeting_markers`). Markers made on Apple Watch keep the watch's marker
/// id, so a re-import never duplicates them.
@Model
final class MomentMarker {
    @Attribute(.unique) var id: UUID
    /// Wall clock, like `Segment.start`
    var at: Date
    /// `MarkerKind.rawValue`
    var kind: String
    var note: String?
    var createdAt: Date
    var meeting: Meeting?

    /// A mark's note
    static let maxNoteLength = MarkerKind.important.maxNoteLength
    /// A typed note (kind `note`): one line
    static let maxTypedNoteLength = MarkerKind.note.maxNoteLength

    init(id: UUID = UUID(), at: Date, kind: MarkerKind = .default, note: String? = nil) {
        self.id = id
        self.at = at
        self.kind = kind.rawValue
        self.note = MomentMarker.clean(note, kind: kind)
        self.createdAt = .now
    }

    var markerKind: MarkerKind { MarkerKind(rawValue: kind) ?? .default }

    /// A line the user typed, not one of the three marks
    var isTypedNote: Bool { kind == MarkerKind.note.rawValue }

    /// "On the test" / "Follow up" / "Remember" for ✎, by the recording's type
    var label: String { markerKind.label(for: meeting?.kind ?? .default) }

    func setKind(_ k: MarkerKind) { kind = k.rawValue }
    func setNote(_ n: String?) { note = MomentMarker.clean(n, kind: markerKind) }

    /// Seconds from the meeting start
    func offset(in meeting: Meeting) -> TimeInterval { max(0, at.timeIntervalSince(meeting.startedAt)) }

    /// One line: trimmed, whitespace closed up, at most the kind's length
    /// (280 on a mark, 1,000 for a typed note); empty → nil.
    static func clean(_ note: String?, kind: MarkerKind = .default) -> String? {
        guard let note else { return nil }
        let n = note.split(whereSeparator: { $0.isWhitespace || $0.isNewline }).joined(separator: " ")
        if n.isEmpty { return nil }
        return String(n.prefix(kind.maxNoteLength))
    }
}

/// One part of a meeting's study guide: validated JSON (`StudyParse`), never
/// raw model output. Deleted whenever the meeting's transcript is edited
/// (Delete or Strike, docs/REDACTION.md) and with its meeting.
@Model
final class StudyMaterial {
    /// `StudyKind.rawValue`
    var kind: String
    var json: String
    /// SHA-256 of the transcript the part was made from (`StudyInput.fingerprint`)
    var transcriptFingerprint: String
    var createdAt: Date
    var meeting: Meeting?

    init(kind: StudyKind, json: String, transcriptFingerprint: String, createdAt: Date = .now) {
        self.kind = kind.rawValue
        self.json = json
        self.transcriptFingerprint = transcriptFingerprint
        self.createdAt = createdAt
    }

    var studyKind: StudyKind? { StudyKind(rawValue: kind) }
}

extension Meeting {
    /// Marks and typed notes together, in time order
    var orderedMarkers: [MomentMarker] { markers.sorted { ($0.at, $0.createdAt) < ($1.at, $1.createdAt) } }

    /// The three kinds of mark (★ / ? / ✎), in time order. Never typed notes:
    /// the Marks list, its filter and its count read this.
    var marks: [MomentMarker] { orderedMarkers.filter { !$0.isTypedNote } }

    /// What the user typed while recording, in time order
    var typedNotes: [MomentMarker] { orderedMarkers.filter { $0.isTypedNote && $0.note != nil } }

    func studyMaterial(_ kind: StudyKind) -> StudyMaterial? {
        studyMaterials.filter { $0.kind == kind.rawValue }.max { $0.createdAt < $1.createdAt }
    }
}

/// Saving and purging study material.
@MainActor
enum StudyStore {
    enum Failure: LocalizedError, Equatable {
        case transcriptChanged
        var errorDescription: String? {
            "The transcript changed while the guide was being made (it was edited), so it wasn't saved. Make it again."
        }
    }

    /// Save generated parts, replacing earlier ones of the same kinds — only
    /// if the transcript still reads exactly as it did when generation
    /// started, so a guide made before an edit is never kept after it.
    static func save(_ parts: [(StudyKind, String)], fingerprint: String, meeting: Meeting, context: ModelContext) throws {
        guard !parts.isEmpty else { return }
        guard StudyInput(meeting: meeting).fingerprint == fingerprint else { throw Failure.transcriptChanged }
        for (kind, json) in parts {
            for old in meeting.studyMaterials where old.kind == kind.rawValue { context.delete(old) }
            let m = StudyMaterial(kind: kind, json: json, transcriptFingerprint: fingerprint)
            context.insert(m)
            m.meeting = meeting
        }
        try context.save()
    }

    /// Purge: every study material of the meeting. A guide paraphrases the
    /// lecture, so matching the removed words can't clean it.
    static func purge(_ meeting: Meeting, context: ModelContext) {
        for m in meeting.studyMaterials { context.delete(m) }
    }
}

extension RecordingSession {
    /// Mark the current moment of the live meeting (one tap: ★), or keep a
    /// typed note (`kind: .note`) stamped with it. A note with no text isn't kept.
    @discardableResult
    func addMarker(kind: MarkerKind = .default, note: String? = nil, at time: Date = .now) -> MomentMarker? {
        guard isActive, let meeting, let context = meeting.modelContext else { return nil }
        if kind == .note, MomentMarker.clean(note, kind: .note) == nil { return nil }
        var time = time
        #if DEBUG
        // A demo (`showDemo`) shows a recording whose lines carry the demo's
        // clock (today from 9:16), not the real one. The moment goes to the
        // same point of the demo, so it sits after the last line at any hour.
        if engineName == "Demo", let startedAt {
            time = time.addingTimeInterval(meeting.startedAt.timeIntervalSince(startedAt))
        }
        #endif
        let marker = MomentMarker(at: time, kind: kind, note: note)
        context.insert(marker)
        marker.meeting = meeting
        if kind == .note {
            // The user's words: kept only when they are saved, so the field
            // can put the line back instead of losing it
            do { try context.save() } catch {
                context.delete(marker)
                return nil
            }
        } else {
            try? context.save()
        }
        return marker
    }
}

/// The text in the note line. A line leaves the field at once when Return is
/// pressed and comes back, ahead of anything typed since, only if it
/// couldn't be kept: the field never loses what was typed.
struct NoteDraft: Equatable {
    var text = ""

    /// The line to keep; the field is empty for the next one. nil for an empty line.
    mutating func take() -> String? {
        let line = MomentMarker.clean(text, kind: .note)
        text = ""
        return line
    }

    /// It couldn't be kept: back in the field, before what was typed since.
    mutating func putBack(_ line: String) {
        let since = text.trimmingCharacters(in: .whitespaces)
        text = String((since.isEmpty ? line : line + " " + since).prefix(MomentMarker.maxTypedNoteLength))
    }
}

/// Editing and deleting what the user typed (the same path as a mark's note),
/// keeping the notes' "What you wrote" section in step, in the same step: a
/// typed note's detail never outlives the note, and an edited line's bullet
/// follows it (docs/design/FADELL_V2.md §6). The notes change, so they are
/// sent to the other device again.
@MainActor
enum TypedNoteStore {
    /// Change a typed note's text. Empty text changes nothing (delete it instead).
    static func edit(_ marker: MomentMarker, text: String, context: ModelContext) {
        guard marker.isTypedNote, let new = MomentMarker.clean(text, kind: .note), new != marker.note else { return }
        followInNotes(marker, newText: new)
        marker.note = new
        try? context.save()
    }

    static func delete(_ marker: MomentMarker, context: ModelContext) {
        if marker.isTypedNote, let meeting = marker.meeting {
            removeFromNotes(marker, in: meeting)
        }
        context.delete(marker)
        try? context.save()
    }

    /// Take what the notes wrote under a typed note out of them (before the
    /// note itself is deleted, here or by the other device).
    static func removeFromNotes(_ marker: MomentMarker, in meeting: Meeting) {
        guard let notes = meeting.aiNotes, let index = meeting.typedNotes.firstIndex(where: { $0.id == marker.id }) else { return }
        meeting.aiNotes = WhatYouWrote.removing(typedIndex: index, typed: meeting.typedNotes.compactMap(\.note), from: notes)
    }

    /// A typed note's text is about to become `newText` (edited here, or on
    /// the other device): its bullet's line follows.
    static func followInNotes(_ marker: MomentMarker, newText: String) {
        guard let meeting = marker.meeting, let notes = meeting.aiNotes,
              let index = meeting.typedNotes.firstIndex(where: { $0.id == marker.id }) else { return }
        meeting.aiNotes = WhatYouWrote.replacing(typedIndex: index, with: newText, typed: meeting.typedNotes.compactMap(\.note), in: notes)
    }
}
