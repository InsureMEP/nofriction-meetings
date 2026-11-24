import AVFoundation
import ImageIO
import SwiftData
import UIKit
import UniformTypeIdentifiers
import XCTest
@testable import noFriction

// Settings → Storage: sizes, smaller photos, removing audio (ios/NoFriction/Space).
// The Apple Watch re-delivery case is with the importer's fixtures, in WatchImportTests.

final class SpaceFormatTests: XCTestCase {
    func testSizesReadAsPlainText() {
        XCTAssertEqual(SpaceFormat.bytes(0), "0 KB")
        XCTAssertEqual(SpaceFormat.bytes(-12), "0 KB")
        XCTAssertFalse(SpaceFormat.bytes(0).localizedCaseInsensitiveContains("zero"))
        XCTAssertEqual(SpaceFormat.bytes(34_000_000), "34 MB")
        XCTAssertEqual(SpaceFormat.bytes(512_000), "512 KB")
        XCTAssertEqual(SpaceFormat.bytes(2_000_000_000), "2 GB")
    }

    func testSizesAreSpokenAsWords() {
        XCTAssertEqual(SpaceFormat.spoken(34_000_000), "34 megabytes")
        XCTAssertEqual(SpaceFormat.spoken(1_000_000), "1 megabyte")
        XCTAssertEqual(SpaceFormat.spoken(2_000_000_000), "2 gigabytes")
        XCTAssertEqual(SpaceFormat.spoken(0), "0 kilobytes")
    }

    func testSettingsDefaultToKeepingEverything() throws {
        let suite = "nf-space-tests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        XCTAssertEqual(SpaceSettings.keepAudioDays(defaults), 0, "Forever unless the user chooses")
        XCTAssertTrue(SpaceSettings.newPhotosSmaller(defaults))
        defaults.set(SpaceSettings.PhotoSize.original.rawValue, forKey: SpaceSettings.photoSizeKey)
        defaults.set(90, forKey: SpaceSettings.keepAudioDaysKey)
        XCTAssertFalse(SpaceSettings.newPhotosSmaller(defaults))
        XCTAssertEqual(SpaceSettings.keepAudioDays(defaults), 90)
        XCTAssertEqual(SpaceSettings.keepAudioChoices.map(SpaceSettings.keepAudioLabel), ["Forever", "1 year", "90 days", "30 days"])
        XCTAssertEqual(SpaceSettings.keepAudioChoices.first, 0)
    }
}

/// Pictures made in the test: a red block in the stored top-left quarter,
/// grey blocks elsewhere (so the file isn't trivially small).
enum PictureFixtures {
    static func image(width: Int, height: Int) throws -> CGImage {
        let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                              space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                              bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.setFillColor(red: 0.15, green: 0.2, blue: 0.3, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        for _ in 0..<300 {
            let grey = CGFloat.random(in: 0.1...0.9)
            context.setFillColor(red: grey, green: grey, blue: grey, alpha: 1)
            let w = CGFloat.random(in: 10...CGFloat(width) / 6), h = CGFloat.random(in: 10...CGFloat(height) / 6)
            context.fill(CGRect(x: .random(in: 0...CGFloat(width)), y: .random(in: 0...CGFloat(height)), width: w, height: h))
        }
        // Core Graphics counts from the bottom: this is the top-left quarter
        context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
        context.fill(CGRect(x: 0, y: height / 2, width: width / 2, height: height - height / 2))
        return try XCTUnwrap(context.makeImage())
    }

    /// `orientation`: the EXIF tag (1 = upright, 3 = upside down, 6 = turned). With a location and a camera name.
    static func jpeg(width: Int, height: Int, orientation: Int = 1) throws -> Data {
        let out = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(out, UTType.jpeg.identifier as CFString, 1, nil))
        let properties: [CFString: Any] = [
            kCGImageDestinationLossyCompressionQuality: 0.9,
            kCGImagePropertyOrientation: orientation,
            kCGImagePropertyGPSDictionary: [kCGImagePropertyGPSLatitude: 40.7, kCGImagePropertyGPSLatitudeRef: "N",
                                            kCGImagePropertyGPSLongitude: 74.0, kCGImagePropertyGPSLongitudeRef: "W"] as [CFString: Any],
            kCGImagePropertyTIFFDictionary: [kCGImagePropertyTIFFMake: "TestCam", kCGImagePropertyTIFFModel: "Fixture 1"] as [CFString: Any],
        ]
        CGImageDestinationAddImage(destination, try image(width: width, height: height), properties as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return out as Data
    }

    static func png(width: Int, height: Int) throws -> Data {
        let out = NSMutableData()
        let destination = try XCTUnwrap(CGImageDestinationCreateWithData(out, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, try image(width: width, height: height), nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return out as Data
    }

    static func properties(_ data: Data) throws -> [CFString: Any] {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        return try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
    }

    /// Stored pixels (width, height), before any orientation tag.
    static func size(_ data: Data) throws -> (width: Int, height: Int) {
        let p = try properties(data)
        return (try XCTUnwrap(p[kCGImagePropertyPixelWidth] as? Int), try XCTUnwrap(p[kCGImagePropertyPixelHeight] as? Int))
    }

    /// Is the stored pixel at this place red? `x`, `y`: 0…1 from the top-left, ignoring any orientation tag.
    static func isRed(_ data: Data, x: Double, y: Double) throws -> Bool {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        let context = try XCTUnwrap(CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0,
                                              space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                              bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        let bytes = try XCTUnwrap(context.data).assumingMemoryBound(to: UInt8.self)
        // The bitmap's first row is the top of the picture
        let px = min(image.width - 1, Int(x * Double(image.width))), py = min(image.height - 1, Int(y * Double(image.height)))
        let p = bytes + py * context.bytesPerRow + px * 4
        return p[0] > 180 && p[1] < 90 && p[2] < 90
    }
}

final class PictureShrinkerTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appending(path: "nf-shrink-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    private func write(_ data: Data, _ name: String) throws -> URL {
        let url = dir.appending(path: name)
        try data.write(to: url)
        return url
    }

    private func names() throws -> [String] {
        try FileManager.default.contentsOfDirectory(atPath: dir.path(percentEncoded: false)).sorted()
    }

    func testACameraSizedPhotoBecomesSmallerInPlace() throws {
        let original = try PictureFixtures.jpeg(width: 4000, height: 3000)
        let url = try write(original, "photo.jpg")
        let estimate = PictureShrinker.estimatedSavings(at: url)
        XCTAssertGreaterThan(estimate, 0)
        XCTAssertLessThan(estimate, Int64(original.count))

        let saved = PictureShrinker.shrinkFile(at: url)

        let after = try Data(contentsOf: url)
        let size = try PictureFixtures.size(after)
        XCTAssertEqual(max(size.width, size.height), 2048)
        XCTAssertEqual(min(size.width, size.height), 1536)
        XCTAssertLessThan(after.count, original.count)
        XCTAssertEqual(saved, Int64(original.count - after.count))
        XCTAssertTrue(try PictureFixtures.isRed(after, x: 0.25, y: 0.25), "the same picture")
        XCTAssertFalse(try PictureFixtures.isRed(after, x: 0.75, y: 0.75))
        XCTAssertEqual(try names(), ["photo.jpg"], "no temporary file left behind")
        // Small now: a second run changes nothing
        XCTAssertEqual(PictureShrinker.shrinkFile(at: url), 0)
        XCTAssertEqual(PictureShrinker.estimatedSavings(at: url), 0)
        XCTAssertEqual(try Data(contentsOf: url), after)
    }

    func testASmallPictureIsUntouched() throws {
        let original = try PictureFixtures.jpeg(width: 1600, height: 1200)
        let url = try write(original, "slide.jpg")
        XCTAssertEqual(PictureShrinker.estimatedSavings(at: url), 0)
        XCTAssertEqual(PictureShrinker.shrinkFile(at: url), 0)
        XCTAssertEqual(try Data(contentsOf: url), original)
        XCTAssertNil(PictureShrinker.shrunk(original), "nothing to do")
        XCTAssertEqual(try names(), ["slide.jpg"])
    }

    func testAPNGIsUntouched() throws {
        let original = try PictureFixtures.png(width: 2600, height: 2200)
        let url = try write(original, "sync-screen.png")
        XCTAssertEqual(PictureShrinker.estimatedSavings(at: url), 0)
        XCTAssertEqual(PictureShrinker.shrinkFile(at: url), 0)
        XCTAssertEqual(try Data(contentsOf: url), original)
        XCTAssertEqual(try names(), ["sync-screen.png"])
    }

    func testATurnedPhotoComesOutUprightWithNoOrientationTagOrMetadata() throws {
        // Stored 1200 × 900 on its side (EXIF 6): upright it is 900 × 1200,
        // and the stored top-left is at the top-right
        let turned = try PictureFixtures.jpeg(width: 1200, height: 900, orientation: 6)
        let before = try PictureFixtures.properties(turned)
        XCTAssertEqual(before[kCGImagePropertyOrientation] as? Int, 6, "fixture")
        XCTAssertNotNil(before[kCGImagePropertyGPSDictionary], "fixture")
        let url = try write(turned, "turned.jpg")

        XCTAssertGreaterThan(PictureShrinker.shrinkFile(at: url, maxSide: 600), 0)

        let after = try Data(contentsOf: url)
        let size = try PictureFixtures.size(after)
        XCTAssertEqual(size.width, 450)
        XCTAssertEqual(size.height, 600)
        XCTAssertTrue(try PictureFixtures.isRed(after, x: 0.75, y: 0.25), "upright: the red block is top-right")
        XCTAssertFalse(try PictureFixtures.isRed(after, x: 0.25, y: 0.25))
        XCTAssertFalse(try PictureFixtures.isRed(after, x: 0.75, y: 0.75))
        let properties = try PictureFixtures.properties(after)
        XCTAssertTrue([nil, 1].contains(properties[kCGImagePropertyOrientation] as? Int), "no orientation left to apply")
        XCTAssertNil(properties[kCGImagePropertyGPSDictionary], "no location")
        let tiff = properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any]
        XCTAssertNil(tiff?[kCGImagePropertyTIFFMake], "no camera details")
        XCTAssertNil(tiff?[kCGImagePropertyTIFFModel])
        XCTAssertNil(after.range(of: Data("TestCam".utf8)))
    }

    func testAnUpsideDownPhotoComesOutUpright() throws {
        let data = try PictureFixtures.jpeg(width: 1200, height: 900, orientation: 3)
        let after = try XCTUnwrap(PictureShrinker.shrunk(data, maxSide: 600))
        let size = try PictureFixtures.size(after)
        XCTAssertEqual(size.width, 600)
        XCTAssertEqual(size.height, 450)
        XCTAssertTrue(try PictureFixtures.isRed(after, x: 0.75, y: 0.75), "upright: the red block is bottom-right")
        XCTAssertFalse(try PictureFixtures.isRed(after, x: 0.25, y: 0.25))
        XCTAssertTrue([nil, 1].contains(try PictureFixtures.properties(after)[kCGImagePropertyOrientation] as? Int))
    }

    func testAPictureDeletedMeanwhileIsNotBroughtBack() throws {
        let url = try write(try PictureFixtures.jpeg(width: 1200, height: 900), "struck.jpg")
        // A Delete or Strike removes the picture while its smaller copy is being written
        let saved = PictureShrinker.shrinkFile(at: url, maxSide: 600, beforeReplace: {
            try? FileManager.default.removeItem(at: url)
        })
        XCTAssertEqual(saved, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path(percentEncoded: false)), "nothing written back")
        XCTAssertEqual(try names(), [], "and no temporary file")
    }

    func testNotAPictureIsLeftAlone() throws {
        let junk = Data([0xFF, 0xD8, 0xFF])
        let url = try write(junk, "broken.jpg")
        XCTAssertNil(PictureShrinker.shrunk(junk))
        XCTAssertEqual(PictureShrinker.shrinkFile(at: url), 0)
        XCTAssertEqual(PictureShrinker.estimatedSavings(at: url), 0)
        XCTAssertEqual(try Data(contentsOf: url), junk)
        XCTAssertEqual(PictureShrinker.shrinkFile(at: dir.appending(path: "missing.jpg")), 0)
        XCTAssertEqual(try names(), ["broken.jpg"])
    }

    func testManyFilesReportProgressAndTheTotalSaved() throws {
        let big = try PictureFixtures.jpeg(width: 1200, height: 900)
        let urls = [try write(big, "a.jpg"), try write(try PictureFixtures.png(width: 900, height: 700), "b.png"), try write(big, "c.jpeg")]
        final class Seen: @unchecked Sendable { var done: [Int] = [] }
        let seen = Seen()
        let saved = PictureShrinker.shrinkFiles(urls, maxSide: 600) { seen.done.append($0) }
        XCTAssertEqual(seen.done, [1, 2, 3])
        XCTAssertEqual(saved, Int64(big.count * 2) - SpaceUsage.fileSize(urls[0]) - SpaceUsage.fileSize(urls[2]))
        XCTAssertGreaterThan(saved, 0)
        XCTAssertEqual(try names(), ["a.jpg", "b.png", "c.jpeg"])
    }

    func testLeftoverTemporaryFilesAreRemovedAtLaunch() throws {
        _ = try write(Data([1]), "\(PictureShrinker.temporaryPrefix)\(UUID().uuidString).jpg")
        _ = try write(Data([1]), "keep.jpg")
        PictureShrinker.removeLeftoverTemporaryFiles(in: dir)
        XCTAssertEqual(try names(), ["keep.jpg"])
    }
}

@MainActor
final class SpaceTests: XCTestCase {
    private var container: ModelContainer!
    private var context: ModelContext!
    private var created: [URL] = []
    private let start = Date(timeIntervalSince1970: 1_790_000_000)

    override func setUp() async throws {
        container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        context = container.mainContext
        Storage.prepare()
    }

    override func tearDown() async throws {
        for url in created { try? FileManager.default.removeItem(at: url) }
        created = []
        container = nil
        context = nil
    }

    private func exists(_ url: URL) -> Bool { FileManager.default.fileExists(atPath: url.path(percentEncoded: false)) }

    /// A saved recording that ended `daysAgo` days before `now`.
    private func makeMeeting(_ lines: [String] = ["first line here", "second line here"], endedDaysAgo: Double = 0,
                             now: Date? = nil) -> Meeting {
        let ended = (now ?? start).addingTimeInterval(-endedDaysAgo * 86_400)
        let m = Meeting(title: "Test", startedAt: ended.addingTimeInterval(-120))
        m.endedAt = ended
        context.insert(m)
        for (i, line) in lines.enumerated() {
            let toks = RedactionText.tokens(line)
            let base = Double(i) * 10
            let timings = toks.enumerated().map { j, t in
                WordTiming(location: t.range.location, length: t.range.length, start: base + Double(j) * 0.5, end: base + Double(j) * 0.5 + 0.4)
            }
            let s = Segment(text: line, start: m.startedAt.addingTimeInterval(base), duration: 8, audioOffset: base, wordTimings: timings)
            context.insert(s)
            s.meeting = m
        }
        try? context.save()
        return m
    }

    /// A real audio file in the app's Audio folder.
    @discardableResult
    private func addAudio(_ m: Meeting, seconds: Double = 2) throws -> URL {
        let name = "test-\(UUID().uuidString).m4a"
        let url = Storage.audio.appending(path: name)
        created.append(url)
        try AudioFixtures.writeTone(url, seconds: seconds)
        m.audioFileName = name
        try context.save()
        return url
    }

    /// A picture row and its file in the app's Snapshots folder.
    private func addPicture(_ m: Meeting, _ data: Data, ext: String = "jpg", at offset: Double = 5) throws -> Snapshot {
        let name = "test-\(UUID().uuidString).\(ext)"
        let url = Storage.snapshots.appending(path: name)
        try data.write(to: url)
        created.append(url)
        let s = Snapshot(fileName: name, takenAt: m.startedAt.addingTimeInterval(offset))
        context.insert(s)
        s.meeting = m
        try context.save()
        return s
    }

    private func onDisk(_ url: URL) throws -> Int64 {
        Int64(try XCTUnwrap(FileManager.default.attributesOfItem(atPath: url.path(percentEncoded: false))[.size] as? Int))
    }

    // MARK: Sizes

    func testARecordingAddsUpItsAudioAndPictures() throws {
        let m = makeMeeting()
        let audio = try addAudio(m)
        _ = try addPicture(m, Data(repeating: 7, count: 1_000))
        _ = try addPicture(m, Data(repeating: 9, count: 2_500), ext: "png", at: 9)

        let files = SpaceUsage.files(of: m)
        XCTAssertEqual(files.id, m.id)
        XCTAssertEqual(files.audio, m.audioFileName)
        XCTAssertEqual(Set(files.pictures), Set(m.snapshots.map(\.fileName)))

        let space = SpaceUsage.measure(files)
        XCTAssertEqual(space.audio, try onDisk(audio))
        XCTAssertGreaterThan(space.audio, 0)
        XCTAssertEqual(space.pictures, 3_500)
        XCTAssertEqual(space.pictureCount, 2)
        XCTAssertEqual(space.total, space.audio + 3_500)
    }

    func testAMissingFileCountsAsNothing() throws {
        let m = makeMeeting()
        m.audioFileName = "gone-\(UUID().uuidString).m4a"
        let s = Snapshot(fileName: "gone-\(UUID().uuidString).jpg", takenAt: m.startedAt)
        context.insert(s)
        s.meeting = m
        try context.save()
        let space = SpaceUsage.measure(SpaceUsage.files(of: m))
        XCTAssertEqual(space.audio, 0)
        XCTAssertEqual(space.pictures, 0)
        XCTAssertEqual(space.pictureCount, 1, "the photo is still listed")
        XCTAssertEqual(space.total, 0)
        XCTAssertEqual(SpaceUsage.folderSize(Storage.documents.appending(path: "no-such-folder-\(UUID().uuidString)")), 0)
    }

    func testTotalsAreOfTheFoldersWithFreeSpace() throws {
        let root = FileManager.default.temporaryDirectory.appending(path: "nf-space-\(UUID().uuidString)", directoryHint: .isDirectory)
        created.append(root)
        let folders = SpaceUsage.Folders(audio: root.appending(path: "Audio"), snapshots: root.appending(path: "Snapshots"),
                                         other: [root.appending(path: "Support"), root.appending(path: "Inbox")], volume: root)
        for dir in [folders.audio, folders.snapshots, folders.other[0].appending(path: "Sync"), folders.other[1]] {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        }
        try Data(count: 4_000).write(to: folders.audio.appending(path: "a.m4a"))
        try Data(count: 1_000).write(to: folders.audio.appending(path: "nobody-points-here.m4a"))
        try Data(count: 700).write(to: folders.snapshots.appending(path: "p.jpg"))
        try Data(count: 300).write(to: folders.other[0].appending(path: "default.store"))
        try Data(count: 20).write(to: folders.other[0].appending(path: "Sync").appending(path: "state.json"))
        try Data(count: 5).write(to: folders.other[1].appending(path: "part.m4a"))

        let totals = SpaceUsage.totals(folders: folders)
        XCTAssertEqual(totals.audio, 5_000, "a file no recording points to still takes room")
        XCTAssertEqual(totals.pictures, 700)
        XCTAssertEqual(totals.other, 325, "the store, what Sync remembers and the watch inbox")
        XCTAssertEqual(totals.total, 6_025)
        XCTAssertGreaterThan(try XCTUnwrap(totals.free), 0)
    }

    func testTheIndexMeasuresEveryRecordingAndFollowsAPrune() async throws {
        let m = makeMeeting()
        let audio = try addAudio(m)
        _ = try addPicture(m, Data(repeating: 1, count: 2_000))
        let other = makeMeeting(["only words"])
        let index = SpaceIndex()
        XCTAssertNil(index.bytes(for: m.id), "nothing until the first refresh")
        XCTAssertNil(index.totals)

        await index.refresh(context: context)
        let audioBytes = try onDisk(audio)
        XCTAssertEqual(index.bytes(for: m.id), audioBytes + 2_000)
        XCTAssertEqual(index.space(for: m.id)?.pictureCount, 1)
        XCTAssertEqual(index.bytes(for: other.id), 0)
        let before = try XCTUnwrap(index.totals)
        XCTAssertGreaterThanOrEqual(before.audio, audioBytes)
        XCTAssertGreaterThanOrEqual(before.pictures, 2_000)

        XCTAssertEqual(try SpacePrune.removeAudio(from: m, context: context), audioBytes)
        await index.refresh(context: context)
        XCTAssertEqual(index.bytes(for: m.id), 2_000)
        XCTAssertEqual(index.space(for: m.id)?.audio, 0)
    }

    // MARK: Remove audio

    func testRemovingAudioKeepsTheTranscriptAndEverythingElse() throws {
        let m = makeMeeting()
        m.aiNotes = "## Summary\nNotes stay."
        let audio = try addAudio(m)
        let photo = try addPicture(m, Data(repeating: 3, count: 900))
        let lines = m.orderedSegments.map(\.text)
        let timings = m.orderedSegments.map(\.wordTimingsJSON)
        let when = start.addingTimeInterval(86_400)

        let freed = try SpacePrune.removeAudio(from: m, context: context, now: when)

        XCTAssertGreaterThan(freed, 0)
        XCTAssertFalse(exists(audio))
        XCTAssertNil(m.audioFileName)
        XCTAssertEqual(m.audioRemovedAt, when)
        XCTAssertEqual(m.orderedSegments.map(\.text), lines, "the transcript is untouched")
        XCTAssertEqual(m.orderedSegments.map(\.wordTimingsJSON), timings)
        XCTAssertEqual(m.aiNotes, "## Summary\nNotes stay.")
        XCTAssertEqual(m.snapshots.count, 1)
        XCTAssertTrue(exists(photo.fileURL), "photos are kept")
        XCTAssertFalse(context.hasChanges, "saved")
        XCTAssertTrue(SpacePrune.removedCaption(when).hasPrefix("Audio removed to save space on "))
        XCTAssertTrue(SpacePrune.removedCaption(when).hasSuffix(". The transcript is kept."))
        // Again: nothing left to remove, and the date stays
        XCTAssertEqual(try SpacePrune.removeAudio(from: m, context: context, now: when.addingTimeInterval(60)), 0)
        XCTAssertEqual(m.audioRemovedAt, when)
    }

    /// A Strike from the Mac that was rewriting the audio while it was
    /// removed can leave a file no property points to. Delete recording
    /// takes it with the rest (and every other file of the recording).
    @MainActor
    func testDeleteRecordingRemovesAudioLeftUnderItsOwnName() throws {
        let m = makeMeeting()
        let audio = try addAudio(m)
        let photo = try addPicture(m, Data(repeating: 3, count: 900))
        let other = makeMeeting()
        let otherAudio = try addAudio(other)
        try SpacePrune.removeAudio(from: m, context: context)
        // The silencer finished afterwards and wrote the file back
        let orphan = Storage.audio.appending(path: "\(m.id.uuidString).m4a")
        try Data(repeating: 1, count: 2_000).write(to: orphan)
        created.append(orphan)
        XCTAssertNil(m.audioFileName)

        RecordingDeletion.delete(m, context: context, redactions: nil, screenCapture: nil)

        XCTAssertFalse(exists(orphan), "nothing of a deleted recording stays")
        XCTAssertFalse(exists(audio))
        XCTAssertFalse(exists(photo.fileURL))
        XCTAssertTrue(exists(otherAudio), "another recording's audio is untouched")
        XCTAssertEqual(try context.fetch(FetchDescriptor<Meeting>()).count, 1)
    }

    func testRemovingAudioIsRefusedWhileTheRecordingIsBeingTranscribed() throws {
        let m = makeMeeting()
        let audio = try addAudio(m)
        let name = m.audioFileName
        for state in [Meeting.ImportState.pending, .transcribing, .failed] {
            m.importState = state.rawValue
            XCTAssertThrowsError(try SpacePrune.removeAudio(from: m, context: context)) { error in
                XCTAssertEqual(error as? SpacePrune.Refusal, .transcribing)
                XCTAssertFalse((error as? LocalizedError)?.errorDescription?.isEmpty ?? true)
            }
        }
        XCTAssertTrue(exists(audio))
        XCTAssertEqual(m.audioFileName, name)
        XCTAssertNil(m.audioRemovedAt)
    }

    func testRemovingAudioIsRefusedForTheRecordingInProgress() throws {
        let m = makeMeeting()
        let audio = try addAudio(m)
        XCTAssertThrowsError(try SpacePrune.removeAudio(from: m, context: context, beingRecorded: m.id)) { error in
            XCTAssertEqual(error as? SpacePrune.Refusal, .recording)
        }
        XCTAssertTrue(exists(audio))
        XCTAssertNotNil(m.audioFileName)
        XCTAssertNil(m.audioRemovedAt)
    }

    func testStrikeStillWorksOnARecordingWithoutItsAudio() async throws {
        let m = makeMeeting(["alpha beta gamma delta"])
        try addAudio(m)
        try SpacePrune.removeAudio(from: m, context: context)
        try await RedactionEngine.strike(.words(m.orderedSegments[0], 2...2), reason: nil, meeting: m, context: context)
        XCTAssertEqual(RedactionText.plain(m.orderedSegments[0].text), "alpha beta [stricken from the record] delta")
        XCTAssertNil(m.audioFileName)
    }

    // MARK: Keep audio for

    func testAudioDuePicksOnlyOldSavedRecordingsThatStillHaveAudio() throws {
        let now = start.addingTimeInterval(400 * 86_400)
        func meeting(_ title: String, endedDaysAgo: Double, audio: Bool = true) -> Meeting {
            let m = makeMeeting(["words"], endedDaysAgo: endedDaysAgo, now: now)
            m.title = title
            if audio { m.audioFileName = "\(title).m4a" }
            return m
        }
        let old = meeting("old", endedDaysAgo: 45)
        let older = meeting("older", endedDaysAgo: 120)
        let recent = meeting("recent", endedDaysAgo: 10)
        let noAudio = meeting("no-audio", endedDaysAgo: 45, audio: false)
        let importing = meeting("importing", endedDaysAgo: 45)
        importing.importState = Meeting.ImportState.pending.rawValue
        let failed = meeting("failed", endedDaysAgo: 45)
        failed.importState = Meeting.ImportState.failed.rawValue
        let unfinished = meeting("unfinished", endedDaysAgo: 45)
        unfinished.endedAt = nil
        let live = meeting("live", endedDaysAgo: 45)
        let all = [old, older, recent, noAudio, importing, failed, unfinished, live]

        func due(_ days: Int) -> Set<String> {
            Set(SpacePrune.audioDue(meetings: all, now: now, days: days, beingRecorded: live.id).map(\.title))
        }
        XCTAssertEqual(due(30), ["old", "older"])
        XCTAssertEqual(due(90), ["older"])
        XCTAssertEqual(due(365), [])
        XCTAssertEqual(due(0), [], "Forever removes nothing")
        XCTAssertEqual(due(-1), [])
        XCTAssertEqual(Set(SpacePrune.audioDue(meetings: all, now: now, days: 30).map(\.title)), ["old", "older", "live"],
                       "without a recording in progress, that one is an ordinary old recording")
    }

    func testKeepAudioForRemovesOldAudioAndForeverRemovesNone() async throws {
        let now = start.addingTimeInterval(400 * 86_400)
        let old = makeMeeting(endedDaysAgo: 45, now: now)
        let oldAudio = try addAudio(old)
        let recent = makeMeeting(endedDaysAgo: 3, now: now)
        let recentAudio = try addAudio(recent)

        let none = await SpacePrune.applyRetention(context: context, session: nil, redactions: nil, days: 0, now: now)
        XCTAssertEqual(none, 0)
        XCTAssertTrue(exists(oldAudio))
        XCTAssertNil(old.audioRemovedAt)

        let freed = await SpacePrune.applyRetention(context: context, session: nil, redactions: nil, days: 30, now: now)
        XCTAssertGreaterThan(freed, 0)
        XCTAssertFalse(exists(oldAudio))
        XCTAssertNil(old.audioFileName)
        XCTAssertEqual(old.audioRemovedAt, now)
        XCTAssertEqual(old.segments.count, 2, "the transcript stays")
        XCTAssertTrue(exists(recentAudio))
        XCTAssertNotNil(recent.audioFileName)
        XCTAssertNil(recent.audioRemovedAt)
        // Once more: nothing left that is due
        let again = await SpacePrune.applyRetention(context: context, session: nil, redactions: nil, days: 30, now: now)
        XCTAssertEqual(again, 0)
    }

    func testAnAudioFilePutBackAfterItsRemovalIsSweptAndNoOtherFileIs() throws {
        let m = makeMeeting()
        try addAudio(m)
        let kept = makeMeeting()
        let keptAudio = try addAudio(kept)
        try SpacePrune.removeAudio(from: m, context: context)
        // A purge that was rewriting the file finished after the removal
        let stray = Storage.audio.appending(path: "\(m.id.uuidString).m4a")
        created.append(stray)
        try Data([1, 2, 3]).write(to: stray)
        // The other recording has an audio file under its usual name too
        let usual = Storage.audio.appending(path: "\(kept.id.uuidString).m4a")
        created.append(usual)
        try Data([4, 5, 6]).write(to: usual)

        SpacePrune.sweepRemovedAudio(context: context)

        XCTAssertFalse(exists(stray))
        XCTAssertTrue(exists(usual), "only recordings whose audio was removed are swept")
        XCTAssertTrue(exists(keptAudio))
    }

    // MARK: Smaller photos

    func testOnlyJPEGsOfOtherRecordingsAreOffered() throws {
        let m = makeMeeting()
        let photo = try addPicture(m, Data([1]))
        _ = try addPicture(m, Data([1]), ext: "png")
        let live = makeMeeting()
        _ = try addPicture(live, Data([1]))
        XCTAssertEqual(SpacePrune.pictureURLs(of: [m, live], excluding: live.id).map(\.lastPathComponent), [photo.fileName])
        XCTAssertEqual(SpacePrune.pictureURLs(of: [m, live]).count, 2)
    }

    func testAShrunkPhotoKeepsItsRowAndIsStillPurgedByDeleteAndStrike() async throws {
        let m = makeMeeting()
        let big = try PictureFixtures.jpeg(width: 1200, height: 900)
        let deleted = try addPicture(m, big, at: 5)
        let struck = try addPicture(m, big, at: 9)
        let names = Set([deleted.fileName, struck.fileName])
        let urls = SpacePrune.pictureURLs(of: [m])
        XCTAssertEqual(Set(urls.map(\.lastPathComponent)), names)

        XCTAssertGreaterThan(PictureShrinker.shrinkFiles(urls, maxSide: 600), 0)

        XCTAssertEqual(Set(m.snapshots.map(\.fileName)), names, "the rows are untouched: nothing for Sync to delete")
        XCTAssertEqual(m.snapshots.count, 2)
        XCTAssertLessThan(SpaceUsage.fileSize(deleted.fileURL), Int64(big.count))
        let leftovers = try FileManager.default.contentsOfDirectory(atPath: Storage.snapshots.path(percentEncoded: false))
            .filter { $0.hasPrefix(PictureShrinker.temporaryPrefix) }
        XCTAssertEqual(leftovers, [])

        // Delete: the row goes at once, the file when the undo window closes
        let deletedURL = deleted.fileURL, struckURL = struck.fileURL
        let pending = try RedactionEngine.delete(.screens([deleted]), meeting: m, context: context)
        try await RedactionEngine.commit(pending, context: context)
        XCTAssertFalse(exists(deletedURL))
        // Strike: the file goes and a marker stays
        try await RedactionEngine.strike(.screens([struck]), reason: nil, meeting: m, context: context)
        XCTAssertFalse(exists(struckURL))
        XCTAssertTrue(m.snapshots.isEmpty)
        XCTAssertEqual(m.screenStrikes.count, 1)
    }

    func testNewPhotosAreSavedSmallerUnlessOriginalIsChosen() throws {
        let defaults = UserDefaults.standard
        let saved = defaults.string(forKey: SpaceSettings.photoSizeKey)
        defer { defaults.set(saved, forKey: SpaceSettings.photoSizeKey) }
        let m = makeMeeting()
        let session = RecordingSession()
        session.attach(context)
        session.showDemo(meeting: m, startedAt: .now, partial: "")
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let photo = UIGraphicsImageRenderer(size: CGSize(width: 4000, height: 3000), format: format).image { ctx in
            UIColor.darkGray.setFill()
            ctx.fill(CGRect(x: 0, y: 0, width: 4000, height: 3000))
            UIColor.red.setFill()
            ctx.fill(CGRect(x: 0, y: 0, width: 2000, height: 1500))
        }

        func longSide(after change: () -> Void) throws -> Int {
            let before = Set(m.snapshots.map(\.fileName))
            change()
            session.addSnapshot(photo)
            let added = try XCTUnwrap(m.snapshots.first { !before.contains($0.fileName) })
            created.append(added.fileURL)
            let size = try PictureFixtures.size(try Data(contentsOf: added.fileURL))
            return max(size.width, size.height)
        }
        XCTAssertEqual(try longSide { defaults.removeObject(forKey: SpaceSettings.photoSizeKey) }, 2048, "smaller by default")
        XCTAssertEqual(try longSide { defaults.set(SpaceSettings.PhotoSize.original.rawValue, forKey: SpaceSettings.photoSizeKey) }, 4000)
        XCTAssertEqual(try longSide { defaults.set(SpaceSettings.PhotoSize.smaller.rawValue, forKey: SpaceSettings.photoSizeKey) }, 2048)
        XCTAssertEqual(m.snapshots.count, 3)
    }
}
