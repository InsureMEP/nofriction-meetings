import Foundation
import SwiftData

/// The two choices on the Storage screen (UserDefaults; read where photos
/// are saved and where old audio is removed).
enum SpaceSettings {
    /// "New photos": `PhotoSize`; unset = smaller
    static let photoSizeKey = "space.photoSize"
    /// "Keep audio for", in days; 0 or unset = forever
    static let keepAudioDaysKey = "space.keepAudioDays"

    enum PhotoSize: String, CaseIterable, Identifiable {
        case smaller, original

        var id: String { rawValue }
        var label: String { self == .smaller ? "Smaller (recommended)" : "Original" }
    }

    /// Forever first: the default never removes anything.
    static let keepAudioChoices = [0, 365, 90, 30]

    static func keepAudioLabel(_ days: Int) -> String {
        switch days {
        case ...0: "Forever"
        case 365: "1 year"
        default: "\(days) days"
        }
    }

    static func newPhotosSmaller(_ defaults: UserDefaults = .standard) -> Bool {
        defaults.string(forKey: photoSizeKey) != PhotoSize.original.rawValue
    }

    static func keepAudioDays(_ defaults: UserDefaults = .standard) -> Int {
        max(0, defaults.integer(forKey: keepAudioDaysKey))
    }
}

/// Freeing room without losing a recording (Settings → Storage). Audio can
/// be removed while the transcript, notes, photos and screens stay. Nothing
/// here deletes a `Meeting` or a `Snapshot` row: a deleted row would be
/// deleted on the Mac too at the next sync, and only Delete recording
/// (`RecordingDeletion`) is meant to do that.
@MainActor
enum SpacePrune {
    enum Refusal: LocalizedError, Equatable {
        case recording
        case transcribing

        var errorDescription: String? {
            switch self {
            case .recording: "This recording is still being recorded. Stop it first, then remove its audio."
            case .transcribing: "This recording is still being transcribed, and that needs its audio. Remove the audio when the transcript is finished."
            }
        }
    }

    /// Remove a recording's audio file and remember when. Everything else of
    /// the recording is untouched. Returns the bytes freed (0 when it had no
    /// audio). `beingRecorded`: the recording in progress, if any.
    @discardableResult
    static func removeAudio(from meeting: Meeting, context: ModelContext, beingRecorded: UUID? = nil,
                            audioDirectory: URL = Storage.audio, now: Date = .now) throws -> Int64 {
        guard meeting.id != beingRecorded else { throw Refusal.recording }
        guard meeting.importState == nil else { throw Refusal.transcribing }
        guard let name = meeting.audioFileName else { return 0 }
        let url = audioDirectory.appending(path: name)
        let bytes = SpaceUsage.fileSize(url)
        // The file first: if it can't be removed, nothing has changed
        if FileManager.default.fileExists(atPath: url.path(percentEncoded: false)) {
            try FileManager.default.removeItem(at: url)
        }
        meeting.audioFileName = nil
        meeting.audioRemovedAt = now
        try context.save()
        return bytes
    }

    /// "Keep audio for": saved recordings that ended more than `days` ago
    /// and still have audio. Never the one being recorded, never one still
    /// being transcribed (that needs the audio). `days` 0 = forever = none.
    static func audioDue(meetings: [Meeting], now: Date, days: Int, beingRecorded: UUID? = nil) -> [Meeting] {
        guard days > 0 else { return [] }
        let cutoff = now.addingTimeInterval(-Double(days) * 86_400)
        return meetings.filter { m in
            guard m.audioFileName != nil, m.importState == nil, m.id != beingRecorded, let ended = m.endedAt else { return false }
            return ended < cutoff
        }
    }

    /// Apply "Keep audio for": at launch and when the Storage screen opens.
    /// Does nothing with the default (Forever). Returns the bytes freed.
    @discardableResult
    static func applyRetention(context: ModelContext, session: RecordingSession?, redactions: RedactionCenter?,
                               days: Int = SpaceSettings.keepAudioDays(), audioDirectory: URL = Storage.audio,
                               now: Date = .now) async -> Int64 {
        sweepRemovedAudio(context: context, audioDirectory: audioDirectory)
        guard days > 0 else { return 0 }
        func due() -> [Meeting] {
            let withAudio = (try? context.fetch(FetchDescriptor<Meeting>(predicate: #Predicate { $0.audioFileName != nil }))) ?? []
            return audioDue(meetings: withAudio, now: now, days: days, beingRecorded: session?.meeting?.id)
        }
        guard !due().isEmpty else { return 0 }
        // A Delete or Strike still rewriting an audio file finishes first
        await redactions?.flush()
        var freed: Int64 = 0
        for meeting in due() {
            freed += (try? removeAudio(from: meeting, context: context, beingRecorded: session?.meeting?.id,
                                       audioDirectory: audioDirectory, now: now)) ?? 0
        }
        return freed
    }

    /// A Strike that arrived from the Mac while the audio was being removed
    /// can finish rewriting the file afterwards and leave it behind, with
    /// no recording pointing to it. A recording whose audio was removed has
    /// no audio file: any under its own two names goes.
    static func sweepRemovedAudio(context: ModelContext, audioDirectory: URL = Storage.audio) {
        let removed = (try? context.fetch(FetchDescriptor<Meeting>(
            predicate: #Predicate { $0.audioRemovedAt != nil && $0.audioFileName == nil }))) ?? []
        for meeting in removed {
            for name in audioFileNames(of: meeting) {
                try? FileManager.default.removeItem(at: audioDirectory.appending(path: name))
            }
        }
    }

    /// The names a recording's audio file can have: recorded on this
    /// device, or imported from Apple Watch.
    static func audioFileNames(of meeting: Meeting) -> [String] {
        ["\(meeting.id.uuidString).m4a"] + (meeting.sourceRecordingID.map { ["watch-\($0).\(WatchTransfer.fileExtension)"] } ?? [])
    }

    /// The picture files of these recordings that could be made smaller
    /// (JPEG only), leaving out the recording in progress.
    static func pictureURLs(of meetings: [Meeting], excluding beingRecorded: UUID? = nil,
                            snapshotsDirectory: URL = Storage.snapshots) -> [URL] {
        meetings.filter { $0.id != beingRecorded }
            .flatMap { $0.snapshots.map { snapshotsDirectory.appending(path: $0.fileName) } }
            .filter(PictureShrinker.isJPEG)
    }

    /// Shown on a recording where its playback would be.
    static func removedCaption(_ date: Date) -> String {
        "Audio removed to save space on \(date.formatted(date: .abbreviated, time: .omitted)). The transcript is kept."
    }
}
