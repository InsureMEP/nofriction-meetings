#if DEBUG
import AVFoundation
import SwiftData
import UIKit

/// Launch with -NFSeedDemo -NFSpaceDemo: one sample recording gets audio and
/// a camera-sized photo, so the Storage screen has something to show and to
/// free (UI tests, layout checks). A tone and coloured blocks; nothing real.
/// Debug builds only.
enum SpaceDemo {
    static let title = "Brightwater pilot kickoff"
    static let audioName = "demo-space-audio.m4a"
    static let photoName = "demo-space-photo.jpg"

    @MainActor
    static func seedIfRequested(_ context: ModelContext) {
        guard ProcessInfo.processInfo.arguments.contains("-NFSpaceDemo") else { return }
        let title = title
        guard let meeting = (try? context.fetch(FetchDescriptor<Meeting>(predicate: #Predicate { $0.title == title })))?.first,
              meeting.audioFileName == nil, meeting.audioRemovedAt == nil else { return }
        if (try? writeTone(Storage.audio.appending(path: audioName), seconds: 180)) != nil {
            meeting.audioFileName = audioName
        }
        if let photo = photo().jpegData(compressionQuality: 0.85),
           (try? photo.write(to: Storage.snapshots.appending(path: photoName), options: .atomic)) != nil {
            let snapshot = Snapshot(fileName: photoName, takenAt: meeting.startedAt.addingTimeInterval(240))
            context.insert(snapshot)
            snapshot.meeting = meeting
        }
        try? context.save()
    }

    /// Like a recording made by the app: AAC, mono, 64 kbit/s (about 1.4 MB for 3 minutes).
    private static func writeTone(_ url: URL, seconds: Double, rate: Double = 44_100) throws {
        try? FileManager.default.removeItem(at: url)
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: rate,
            AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 64_000,
        ]
        let file = try AVAudioFile(forWriting: url, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        let chunk = AVAudioFrameCount(rate)
        guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: chunk),
              let samples = buffer.floatChannelData?[0] else { return }
        buffer.frameLength = chunk
        for second in 0..<Int(seconds) {
            let hz = Float(220 + (second % 5) * 55)
            for i in 0..<Int(chunk) { samples[i] = 0.3 * sinf(2 * .pi * hz * Float(i) / Float(rate)) }
            try file.write(from: buffer)
        }
    }

    /// 4000 × 3000, like a camera photo.
    private static func photo() -> UIImage {
        let size = CGSize(width: 4000, height: 3000)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        return UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            UIColor(white: 0.93, alpha: 1).setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            for _ in 0..<6000 {
                UIColor(hue: .random(in: 0...1), saturation: .random(in: 0.2...0.8), brightness: .random(in: 0.3...0.95), alpha: 1).setFill()
                ctx.fill(CGRect(x: .random(in: 0...size.width), y: .random(in: 0...size.height),
                                width: .random(in: 8...160), height: .random(in: 8...160)))
            }
        }
    }
}
#endif
