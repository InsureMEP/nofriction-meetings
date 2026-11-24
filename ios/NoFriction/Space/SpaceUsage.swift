import Foundation
import Observation
import SwiftData

// Storage (Settings → Storage): how much room noFriction takes on this
// device, by recording and in total. "Space" in code, because `Storage` is
// the paths enum. Sizes are read from the files; nothing here deletes.

/// Sizes as text, the same everywhere they are shown.
enum SpaceFormat {
    /// 34_000_000 → "34 MB"; nothing → "0 KB" (never "Zero KB").
    static func bytes(_ count: Int64) -> String {
        guard count > 0 else { return "0 KB" }
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        formatter.allowsNonnumericFormatting = false
        return formatter.string(fromByteCount: count)
    }

    /// For VoiceOver: "34 megabytes".
    static func spoken(_ count: Int64) -> String {
        let shown = bytes(count)
        guard let space = shown.lastIndex(where: { $0 == " " || $0 == "\u{a0}" }) else { return shown }
        let number = String(shown[..<space])
        let words = ["bytes": "bytes", "byte": "bytes", "KB": "kilobytes", "MB": "megabytes", "GB": "gigabytes", "TB": "terabytes"]
        guard let unit = words[String(shown[shown.index(after: space)...])] else { return shown }
        return "\(number) \(number == "1" ? String(unit.dropLast()) : unit)"
    }
}

/// What one recording keeps on this device.
struct RecordingSpace: Equatable, Sendable {
    var audio: Int64 = 0
    var pictures: Int64 = 0
    /// Photos and screens of the recording (whether or not each file is here)
    var pictureCount = 0

    var total: Int64 { audio + pictures }
}

/// What the whole app keeps on this device.
struct SpaceTotals: Equatable, Sendable {
    var audio: Int64 = 0
    var pictures: Int64 = 0
    /// The store with every transcript and note, Apple Watch recordings
    /// waiting to be imported, and what Sync remembers
    var other: Int64 = 0
    /// Free on the device; nil when the system doesn't say
    var free: Int64?

    var total: Int64 { audio + pictures + other }
}

/// The file names of one recording. Collected on the main actor (SwiftData
/// models live there); the sizes are read off it.
struct RecordingFiles: Equatable, Sendable {
    var id: UUID
    var audio: String?
    var pictures: [String]
}

enum SpaceUsage {
    /// Where the files are (the app's folders; a temporary folder in tests).
    struct Folders: Sendable {
        var audio: URL
        var snapshots: URL
        /// Counted whole as "Everything else"
        var other: [URL]
        /// Any file on the volume whose free space is shown
        var volume: URL

        static let live = Folders(audio: Storage.audio, snapshots: Storage.snapshots,
                                  other: [URL.applicationSupportDirectory, Storage.watchInbox],
                                  volume: Storage.documents)
    }

    struct Measured: Equatable, Sendable {
        var recordings: [UUID: RecordingSpace] = [:]
        var totals = SpaceTotals()
    }

    @MainActor
    static func files(of meeting: Meeting) -> RecordingFiles {
        RecordingFiles(id: meeting.id, audio: meeting.audioFileName, pictures: meeting.snapshots.map(\.fileName))
    }

    @MainActor
    static func files(in context: ModelContext) -> [RecordingFiles] {
        ((try? context.fetch(FetchDescriptor<Meeting>())) ?? []).map(files(of:))
    }

    /// Size of one file in bytes; 0 when it isn't there.
    static func fileSize(_ url: URL) -> Int64 {
        // A URL remembers what it read before; the file may have changed since
        var url = url
        url.removeAllCachedResourceValues()
        return size(url)
    }

    private static func size(_ url: URL) -> Int64 {
        guard let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
              values.isRegularFile == true else { return 0 }
        return Int64(values.fileSize ?? 0)
    }

    /// Everything in a folder, including its subfolders; 0 when it isn't there.
    static func folderSize(_ url: URL) -> Int64 {
        guard let files = FileManager.default.enumerator(at: url, includingPropertiesForKeys: [.isRegularFileKey, .fileSizeKey]) else { return 0 }
        var total: Int64 = 0
        for case let file as URL in files { total += size(file) }
        return total
    }

    static func measure(_ files: RecordingFiles, folders: Folders = .live) -> RecordingSpace {
        RecordingSpace(audio: files.audio.map { fileSize(folders.audio.appending(path: $0)) } ?? 0,
                       pictures: files.pictures.reduce(0) { $0 + fileSize(folders.snapshots.appending(path: $1)) },
                       pictureCount: files.pictures.count)
    }

    /// The totals are of the folders, not of the recordings added up: a
    /// file no recording points to still takes room, and is counted.
    static func totals(folders: Folders = .live) -> SpaceTotals {
        SpaceTotals(audio: folderSize(folders.audio), pictures: folderSize(folders.snapshots),
                    other: folders.other.reduce(0) { $0 + folderSize($1) }, free: freeSpace(at: folders.volume))
    }

    static func measure(_ files: [RecordingFiles], folders: Folders = .live) -> Measured {
        var out = Measured(totals: totals(folders: folders))
        for f in files { out.recordings[f.id] = measure(f, folders: folders) }
        return out
    }

    /// Room the system would make for something the user asked for.
    static func freeSpace(at url: URL) -> Int64? {
        var url = url
        url.removeAllCachedResourceValues()
        return (try? url.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey]))?.volumeAvailableCapacityForImportantUsage
    }
}

/// Sizes by recording and in total, for the Recordings list, Settings and
/// the Storage screen. Names are collected on the main actor and the files
/// are read off it, so a refresh never holds up scrolling; until the first
/// one finishes there are simply no sizes to show.
@MainActor
@Observable
final class SpaceIndex {
    private(set) var recordings: [UUID: RecordingSpace] = [:]
    private(set) var totals: SpaceTotals?
    @ObservationIgnored var folders = SpaceUsage.Folders.live
    /// Only the newest refresh is kept
    @ObservationIgnored private var generation = 0

    func space(for id: UUID) -> RecordingSpace? { recordings[id] }

    /// Bytes of a recording, once it has been measured.
    func bytes(for id: UUID) -> Int64? { recordings[id]?.total }

    /// Measure again: when the Recordings list appears, and after anything
    /// that frees or takes room.
    func refresh(context: ModelContext) async {
        generation += 1
        let mine = generation
        let files = SpaceUsage.files(in: context)
        let folders = folders
        let measured = await Task.detached(priority: .utility) { SpaceUsage.measure(files, folders: folders) }.value
        guard mine == generation else { return }
        if recordings != measured.recordings { recordings = measured.recordings }
        if totals != measured.totals { totals = measured.totals }
    }
}
