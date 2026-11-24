import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Makes photos smaller (Settings → Storage): at most 2048 pixels on the
/// long side, JPEG quality 0.75. A slide or a whiteboard stays readable and
/// the file is a fraction of the camera's. Only the picture is kept: no
/// location, camera details or other metadata.
///
/// Only the file changes, never the `Snapshot` row, so nothing is re-sent
/// by Sync and the Mac keeps the copy it has.
enum PictureShrinker {
    static let maxSide = 2048
    static let quality = 0.75
    /// A file being written, next to the picture it will replace
    static let temporaryPrefix = ".shrinking-"

    /// The picture, smaller and upright. nil when it isn't a picture or its
    /// long side is `maxSide` or less already (nothing to do).
    static func shrunk(_ data: Data, maxSide: Int = maxSide, quality: Double = quality) -> Data? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        return shrunk(source, maxSide: maxSide, quality: quality)
    }

    private static func shrunk(_ source: CGImageSource, maxSide: Int, quality: Double) -> Data? {
        guard let longSide = longSide(source), longSide > maxSide else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            // Turned as the camera held it, so no orientation tag is needed
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxSide,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        let out = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(out, UTType.jpeg.identifier as CFString, 1, nil) else { return nil }
        // Only the pixels and the quality: the source's properties are not copied
        CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return out as Data
    }

    /// Longer side in pixels, read from the file's header.
    private static func longSide(_ source: CGImageSource) -> Int? {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue,
              let height = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue,
              width > 0, height > 0 else { return nil }
        return max(width, height)
    }

    /// JPEG files only; a PNG (a screenshot from the Mac) is left as it is.
    static func isJPEG(_ url: URL) -> Bool {
        ["jpg", "jpeg"].contains(url.pathExtension.lowercased())
    }

    /// Make the picture file at `url` smaller, in place. Returns the bytes
    /// saved; 0 when nothing changed (not a JPEG, small already, no gain,
    /// or it couldn't be read or written).
    ///
    /// The smaller picture is written next to the original and the two are
    /// swapped in one step, which fails if the original is gone. A Delete
    /// or Strike that removed the picture meanwhile is therefore never
    /// undone: nothing is put back at its path.
    @discardableResult
    static func shrinkFile(at url: URL, maxSide: Int = maxSide, quality: Double = quality,
                           beforeReplace: (() -> Void)? = nil) -> Int64 {
        guard isJPEG(url) else { return 0 }
        let fm = FileManager.default
        let before = SpaceUsage.fileSize(url)
        guard before > 0, let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let data = shrunk(source, maxSide: maxSide, quality: quality), Int64(data.count) < before else { return 0 }

        let temp = url.deletingLastPathComponent().appending(path: "\(temporaryPrefix)\(UUID().uuidString).jpg")
        // After the swap this is the original; either way it goes
        defer { try? fm.removeItem(at: temp) }
        do { try data.write(to: temp) } catch { return 0 }
        // Protected like the picture it replaces
        if let protection = (try? fm.attributesOfItem(atPath: url.path(percentEncoded: false)))?[.protectionKey] {
            try? fm.setAttributes([.protectionKey: protection], ofItemAtPath: temp.path(percentEncoded: false))
        }
        beforeReplace?()
        guard swap(temp, withExisting: url) else { return 0 }
        return before - Int64(data.count)
    }

    /// Exchange two files atomically. False when either is missing, so a
    /// picture deleted a moment ago is not brought back.
    private static func swap(_ temp: URL, withExisting original: URL) -> Bool {
        temp.withUnsafeFileSystemRepresentation { from in
            original.withUnsafeFileSystemRepresentation { to in
                guard let from, let to else { return false }
                if renamex_np(from, to, UInt32(RENAME_SWAP)) == 0 { return true }
                // A volume that can't swap: check, then move over the original
                guard errno == ENOTSUP, access(to, F_OK) == 0 else { return false }
                return rename(from, to) == 0
            }
        }
    }

    /// Make each picture smaller, one at a time. Returns the bytes saved.
    /// `progress` is called after each file with how many are done.
    static func shrinkFiles(_ urls: [URL], maxSide: Int = maxSide, quality: Double = quality,
                            progress: (@Sendable (Int) -> Void)? = nil) -> Int64 {
        var saved: Int64 = 0
        for (i, url) in urls.enumerated() {
            if Task.isCancelled { break }
            saved += shrinkFile(at: url, maxSide: maxSide, quality: quality)
            progress?(i + 1)
        }
        return saved
    }

    /// About how much `shrinkFile` would save, without decoding the picture:
    /// the file's size scaled by the change in area. 0 for anything it would skip.
    static func estimatedSavings(at url: URL, maxSide: Int = maxSide) -> Int64 {
        guard isJPEG(url) else { return 0 }
        let bytes = SpaceUsage.fileSize(url)
        guard bytes > 0, let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let longSide = longSide(source), longSide > maxSide else { return 0 }
        let scale = Double(maxSide) / Double(longSide)
        let after = max(Double(bytes) * scale * scale, Double(bytes) * 0.05)
        return max(0, bytes - Int64(after))
    }

    static func estimatedSavings(_ urls: [URL], maxSide: Int = maxSide) -> Int64 {
        urls.reduce(0) { $0 + estimatedSavings(at: $1, maxSide: maxSide) }
    }

    /// At launch: a file left by a run that was killed while it was writing.
    static func removeLeftoverTemporaryFiles(in folder: URL = Storage.snapshots) {
        let fm = FileManager.default
        for url in (try? fm.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)) ?? []
        where url.lastPathComponent.hasPrefix(temporaryPrefix) {
            try? fm.removeItem(at: url)
        }
    }
}
