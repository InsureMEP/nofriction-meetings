import Observation
import SwiftData
import SwiftUI

/// Settings → Storage: a row with the total, and the screen behind it.
struct SpaceSection: View {
    @Environment(SpaceIndex.self) private var space
    @Environment(\.modelContext) private var context

    var body: some View {
        Section {
            NavigationLink {
                SpaceView()
            } label: {
                LabeledContent("Storage", value: space.totals.map { SpaceFormat.bytes($0.total) } ?? "")
                    .accessibilityLabel("Storage")
                    .accessibilityValue(space.totals.map { SpaceFormat.spoken($0.total) } ?? "")
            }
            .accessibilityIdentifier("space-settings-row")
            .task { await space.refresh(context: context) }
        } footer: {
            Text("See what noFriction keeps on this \(DeviceName.current) and free space: remove audio you no longer play, and make photos smaller. Transcripts and notes are always kept.")
        }
    }
}

/// Making photos smaller: the estimate, the progress and the result.
@MainActor
@Observable
final class SpaceWork {
    struct Progress: Equatable {
        var done: Int
        var total: Int
    }

    /// About what Make photos smaller would save now
    private(set) var estimate: Int64 = 0
    private(set) var progress: Progress?
    /// What the last action did, in words
    var message: String?

    var isShrinking: Bool { progress != nil }

    func updateEstimate(_ urls: [URL]) async {
        let value = await Task.detached(priority: .utility) { PictureShrinker.estimatedSavings(urls) }.value
        if !isShrinking { estimate = value }
    }

    /// Make these picture files smaller, off the main actor. Returns the bytes freed.
    @discardableResult
    func shrink(_ urls: [URL]) async -> Int64 {
        guard !isShrinking else { return 0 }
        progress = Progress(done: 0, total: urls.count)
        message = nil
        let freed = await Task.detached(priority: .utility) { [weak self] in
            PictureShrinker.shrinkFiles(urls) { done in self?.report(done) }
        }.value
        progress = nil
        message = freed > 0 ? "Freed \(SpaceFormat.bytes(freed))." : "These photos are small already. Nothing changed."
        return freed
    }

    /// Called off the main actor after each file.
    nonisolated private func report(_ done: Int) {
        Task { @MainActor in self.progress?.done = done }
    }
}

struct SpaceView: View {
    @Environment(SpaceIndex.self) private var space
    @Environment(RecordingSession.self) private var session
    @Environment(RedactionCenter.self) private var redactions
    @Environment(ScreenCaptureCenter.self) private var screenCapture
    @Environment(\.modelContext) private var context
    @Query(sort: \Meeting.startedAt, order: .reverse) private var meetings: [Meeting]
    @AppStorage(SpaceSettings.keepAudioDaysKey) private var keepAudioDays = 0
    @AppStorage(SpaceSettings.photoSizeKey) private var photoSize = SpaceSettings.PhotoSize.smaller.rawValue
    @State private var work = SpaceWork()
    /// The recording whose Remove audio / Delete recording is being confirmed
    @State private var removingAudio: UUID?
    @State private var deleting: UUID?
    @State private var retentionChange: RetentionChange?
    @State private var error: String?

    static let largestCount = 20

    /// A shorter "Keep audio for" that would remove audio right away
    struct RetentionChange: Equatable {
        var days: Int
        var recordings: Int
        var bytes: Int64
    }

    private struct Entry: Identifiable {
        let meeting: Meeting
        let space: RecordingSpace
        var id: UUID { meeting.id }
    }

    /// The recordings that take the most room, largest first. The one being
    /// recorded is left out: nothing here may touch it.
    private var largest: [Entry] {
        meetings.filter { $0.id != session.meeting?.id }
            .compactMap { m in space.space(for: m.id).map { Entry(meeting: m, space: $0) } }
            .filter { $0.space.total > 0 }
            .sorted { $0.space.total > $1.space.total }
            .prefix(Self.largestCount)
            .map { $0 }
    }

    var body: some View {
        Form {
            summarySection
            photosSection
            audioSection
            largestSection
        }
        .scrollContentBackground(.hidden)
        .background(Theme.background)
        .navigationTitle("Storage")
        .task { await opened() }
        .alert("Couldn't do that", isPresented: .init(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error ?? "")
        }
    }

    // MARK: Sections

    private var summarySection: some View {
        Section {
            VStack(alignment: .leading, spacing: 4) {
                Text("noFriction uses \(space.totals.map { SpaceFormat.bytes($0.total) } ?? "…")")
                    .font(.title3.weight(.semibold))
                if let free = space.totals?.free {
                    Text("\(SpaceFormat.bytes(free)) free on this \(DeviceName.current)")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.vertical, 4)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(summarySpoken)
            .accessibilityIdentifier("space-total")
            sizeRow("Audio", space.totals?.audio)
            sizeRow("Photos and screens", space.totals?.pictures)
            sizeRow("Everything else", space.totals?.other)
        } footer: {
            Text("Everything else is your transcripts and notes, Apple Watch recordings still arriving, and what Sync remembers.")
        }
    }

    private func sizeRow(_ title: String, _ bytes: Int64?) -> some View {
        LabeledContent(title, value: bytes.map(SpaceFormat.bytes) ?? "")
            .accessibilityLabel(title)
            .accessibilityValue(bytes.map(SpaceFormat.spoken) ?? "")
    }

    private var summarySpoken: String {
        guard let totals = space.totals else { return "Measuring what noFriction uses" }
        var text = "noFriction uses \(SpaceFormat.spoken(totals.total))."
        if let free = totals.free { text += " \(SpaceFormat.spoken(free)) free on this \(DeviceName.current)." }
        return text
    }

    private var photosSection: some View {
        Section {
            if let progress = work.progress {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Making photos smaller… \(progress.done) of \(progress.total)")
                    ProgressView(value: Double(progress.done), total: Double(max(progress.total, 1)))
                        .tint(Theme.accent)
                        .accessibilityLabel("Photos made smaller")
                }
                .accessibilityIdentifier("space-shrink-progress")
            } else if work.estimate > 0 {
                Button {
                    shrink(meetings)
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Make photos smaller")
                        Text("Saves about \(SpaceFormat.bytes(work.estimate))")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .accessibilityLabel("Make photos smaller, saves about \(SpaceFormat.spoken(work.estimate))")
                .accessibilityIdentifier("space-shrink")
            }
            if let message = work.message, !work.isShrinking {
                Text(message)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("space-shrink-result")
            }
            Picker("New photos", selection: $photoSize) {
                ForEach(SpaceSettings.PhotoSize.allCases) { size in
                    Text(size.label).tag(size.rawValue)
                }
            }
            .accessibilityIdentifier("space-photo-size")
        } header: {
            Text("Photos")
        } footer: {
            Text("A smaller photo is at most \(String(PictureShrinker.maxSide)) pixels on its long side: a slide or a whiteboard stays readable, in a fraction of the space. Screens are small already. Photos and screens already synced to your Mac are not changed there.")
        }
    }

    private var audioSection: some View {
        Section {
            Picker("Keep audio for", selection: Binding(get: { keepAudioDays }, set: { setKeepAudio($0) })) {
                ForEach(SpaceSettings.keepAudioChoices, id: \.self) { days in
                    Text(SpaceSettings.keepAudioLabel(days)).tag(days)
                }
            }
            .accessibilityIdentifier("space-keep-audio")
            // A shorter time that would remove audio right now asks first
            .confirmationDialog(retentionChange.map(retentionTitle) ?? "", isPresented: present($retentionChange), titleVisibility: .visible,
                                presenting: retentionChange) { change in
                Button("Remove audio", role: .destructive) {
                    keepAudioDays = change.days
                    Task { await applyRetention() }
                }
            } message: { change in
                Text(retentionMessage(change))
            }
        } header: {
            Text("Audio")
        } footer: {
            Text("Audio is only for playing a recording back. Removing it keeps the transcript, notes, photos and screens, but the audio can't be played again: it is on this \(DeviceName.current) only and is never synced to your Mac. With a time chosen here, older audio is removed when you open noFriction.")
        }
    }

    @ViewBuilder private var largestSection: some View {
        let entries = largest
        if !entries.isEmpty {
            Section {
                ForEach(entries) { entry in
                    row(entry)
                }
            } header: {
                Text("Largest recordings")
            } footer: {
                Text("Remove audio and Make photos smaller keep the recording. Delete recording removes all of it.")
            }
        }
    }

    private func row(_ entry: Entry) -> some View {
        let meeting = entry.meeting
        return HStack(spacing: 12) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(meeting.title).lineLimit(1)
                    Group {
                        Text(meeting.startedAt.formatted(date: .abbreviated, time: .omitted))
                        Text(detail(entry))
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                }
                Spacer(minLength: 0)
                Text(SpaceFormat.bytes(entry.space.total))
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(spoken(entry))
            .accessibilityIdentifier("space-row")
            Menu {
                if meeting.audioFileName != nil {
                    Button("Remove audio", systemImage: "waveform.slash") { removingAudio = meeting.id }
                        .disabled(meeting.importState != nil)
                }
                if entry.space.pictureCount > 0 {
                    Button("Make photos smaller", systemImage: "photo") { shrink([meeting]) }
                        .disabled(work.isShrinking)
                }
                Button("Delete recording", systemImage: "trash", role: .destructive) { deleting = meeting.id }
            } label: {
                Image(systemName: "ellipsis.circle")
                    .font(.body)
                    .frame(width: 36, height: 36)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("Free space in \(meeting.title)")
            .accessibilityIdentifier("space-row-menu")
        }
        // On the row, so each dialog opens beside the recording it is about
        .confirmationDialog("Remove this recording's audio?", isPresented: present($removingAudio, meeting.id), titleVisibility: .visible) {
            Button("Remove audio", role: .destructive) { removeAudio(meeting) }
        } message: {
            Text("The transcript, notes and photos are kept. The audio can't be played again.")
        }
        // The same words as Delete Recording on the recording itself
        .confirmationDialog("Delete this recording?", isPresented: present($deleting, meeting.id), titleVisibility: .visible) {
            Button("Delete", role: .destructive) { delete(meeting) }
        } message: {
            Text(meeting.screens.isEmpty ? "Its transcript, audio and photos are removed from this device."
                 : "Its transcript, audio, photos and screens are removed from this device.")
        }
    }

    /// "Audio 29 MB · Photos 5 MB"
    private func detail(_ entry: Entry) -> String {
        var parts: [String] = []
        if entry.space.audio > 0 { parts.append("Audio \(SpaceFormat.bytes(entry.space.audio))") }
        if entry.space.pictures > 0 { parts.append("Photos \(SpaceFormat.bytes(entry.space.pictures))") }
        return parts.joined(separator: " · ")
    }

    private func spoken(_ entry: Entry) -> String {
        var parts = [entry.meeting.title, entry.meeting.startedAt.formatted(date: .abbreviated, time: .omitted),
                     SpaceFormat.spoken(entry.space.total)]
        if entry.space.audio > 0 { parts.append("audio \(SpaceFormat.spoken(entry.space.audio))") }
        if entry.space.pictures > 0 { parts.append("photos \(SpaceFormat.spoken(entry.space.pictures))") }
        return parts.joined(separator: ", ")
    }

    // MARK: Actions

    private func present<T>(_ item: Binding<T?>) -> Binding<Bool> {
        Binding(get: { item.wrappedValue != nil }, set: { if !$0 { item.wrappedValue = nil } })
    }

    /// True for the one row whose recording is being asked about.
    private func present(_ item: Binding<UUID?>, _ id: UUID) -> Binding<Bool> {
        Binding(get: { item.wrappedValue == id }, set: { if !$0, item.wrappedValue == id { item.wrappedValue = nil } })
    }

    private func opened() async {
        await space.refresh(context: context)
        if await SpacePrune.applyRetention(context: context, session: session, redactions: redactions) > 0 {
            await space.refresh(context: context)
        }
        await work.updateEstimate(shrinkable(meetings))
    }

    /// Never the recording in progress.
    private func shrinkable(_ meetings: [Meeting]) -> [URL] {
        SpacePrune.pictureURLs(of: meetings, excluding: session.meeting?.id)
    }

    private func refreshAll() async {
        await space.refresh(context: context)
        await work.updateEstimate(shrinkable(meetings))
    }

    private func shrink(_ which: [Meeting]) {
        let urls = shrinkable(which)
        Task {
            await work.shrink(urls)
            await refreshAll()
        }
    }

    private func removeAudio(_ meeting: Meeting) {
        Task {
            // A Delete or Strike still rewriting this audio finishes first
            await redactions.flush()
            guard meeting.modelContext != nil, !meeting.isDeleted else { return }
            do {
                try SpacePrune.removeAudio(from: meeting, context: context, beingRecorded: session.meeting?.id)
            } catch {
                self.error = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
            }
            await refreshAll()
        }
    }

    private func delete(_ meeting: Meeting) {
        guard meeting.id != session.meeting?.id else { return }
        RecordingDeletion.delete(meeting, context: context, redactions: redactions, screenCapture: screenCapture)
        Task { await refreshAll() }
    }

    // MARK: Keep audio for

    /// A choice that would remove audio right now asks first; any other is just saved.
    private func setKeepAudio(_ days: Int) {
        let due = SpacePrune.audioDue(meetings: meetings, now: .now, days: days, beingRecorded: session.meeting?.id)
        guard !due.isEmpty else {
            keepAudioDays = days
            return
        }
        let bytes = due.reduce(Int64(0)) { $0 + (space.space(for: $1.id)?.audio ?? 0) }
        retentionChange = RetentionChange(days: days, recordings: due.count, bytes: bytes)
    }

    private func retentionTitle(_ change: RetentionChange) -> String {
        "Remove audio older than \(SpaceSettings.keepAudioLabel(change.days))?"
    }

    private func retentionMessage(_ change: RetentionChange) -> String {
        let which = change.recordings == 1 ? "1 recording loses its audio now" : "\(change.recordings) recordings lose their audio now"
        let freeing = change.bytes > 0 ? ", freeing about \(SpaceFormat.bytes(change.bytes))" : ""
        return "\(which)\(freeing). The transcript, notes and photos are kept. The audio can't be played again."
    }

    private func applyRetention() async {
        await SpacePrune.applyRetention(context: context, session: session, redactions: redactions)
        await refreshAll()
    }
}
