import SwiftData
import SwiftUI

/// A notebook's page (docs/design/FADELL_V2.md §3 C and §7): what's due,
/// the ✎ marks and questions, every study guide together, and its
/// recordings. Reached from Recordings (a notebook chip, then "Open …") and
/// from the notebook on a recording. Reading it is free; nothing here calls
/// the AI.
struct NotebookView: View {
    let name: String
    @Query(sort: \Meeting.startedAt, order: .reverse) private var meetings: [Meeting]
    @Environment(RecordingSession.self) private var session
    @Environment(AppRouter.self) private var router
    @State private var reviewing: ReviewPart?
    /// A practice question's "Jump to this moment"
    @State private var moment: Moment?
    /// The ticked items are folded away until asked for
    @State private var showDone = false

    enum ReviewPart: String, Identifiable {
        case flashcards, quiz
        var id: String { rawValue }
    }

    struct Moment: Hashable {
        let meeting: Meeting
        let at: Date
    }

    /// Recordings whose notebook matches, ignoring case, newest first (not the one being recorded)
    private var recordings: [Meeting] {
        meetings.filter { $0.id != session.meeting?.id && Notebook.matches($0.courseName, filter: name) }
    }

    var body: some View {
        let recordings = recordings
        let kind = NotebookPage.usualKind(recordings)
        let sources = recordings.map(NotebookReview.source)
        let all = NotebookReview.everything(sources)
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                header(recordings, kind: kind)
                items(recordings, kind: kind)
                // Review everything leads once there is something to review;
                // until then it follows what was marked
                if !all.isEmpty { review(all, kind: kind) }
                marked(recordings, kind: kind)
                if all.isEmpty { review(all, kind: kind) }
                list(recordings)
            }
            .padding(20)
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(Theme.background)
        // The name is the page's own header, like a recording's title: not repeated in the bar
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(item: $moment) { m in
            MeetingDetailView(meeting: m.meeting, jumpTo: m.at, inNotebook: true)
        }
        .sheet(item: $reviewing) { part in
            NotebookReviewSheet(part: part, sources: sources, kind: kind) { question, ms in
                reviewing = nil
                if let m = recordings.first(where: { $0.id == question.meetingID }) {
                    moment = Moment(meeting: m, at: m.startedAt.addingTimeInterval(Double(ms) / 1000))
                }
            }
        }
    }

    // MARK: Header

    private func header(_ recordings: [Meeting], kind: RecordingKind) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(name)
                    .font(.title.weight(.semibold))
                    .accessibilityAddTraits(.isHeader)
                Label(NotebookPage.summary(kind: kind, count: recordings.count, seconds: recordings.compactMap(\.duration).reduce(0, +)),
                      systemImage: kind.systemImage)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("notebook-summary")
            }
            // Side by side; stacked when large text wouldn't fit
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) { headerButtons(kind) }
                VStack(alignment: .leading, spacing: 8) { headerButtons(kind) }
            }
        }
    }

    @ViewBuilder private func headerButtons(_ kind: RecordingKind) -> some View {
        Button {
            router.record(notebook: name, kind: kind)
        } label: {
            Label("Record", systemImage: "record.circle").frame(minHeight: 28)
        }
        .buttonStyle(.borderedProminent)
        .tint(Theme.recordingStrong)
        .disabled(session.isActive)
        .accessibilityHint(session.isActive ? "A recording is already running" : "Opens the Record sheet with this notebook and \(kind.label) filled in")
        .accessibilityIdentifier("notebook-record")
        Button {
            router.ask(about: .notebook(name))
        } label: {
            Label("Ask", systemImage: "bubble.left.and.text.bubble.right").frame(minHeight: 28)
        }
        .buttonStyle(.bordered)
        .tint(Theme.ai)
        .accessibilityHint("Opens Chat on this notebook's recordings")
        .accessibilityIdentifier("notebook-ask")
    }

    // MARK: Due / Action items / To-dos

    private func items(_ recordings: [Meeting], kind: RecordingKind) -> some View {
        let found = recordings.flatMap { m in NotebookItems.items(of: m).map { (meeting: m, item: $0) } }
        let open = found.filter { !NotebookItems.isDone($0.item, in: $0.meeting) }
        let done = found.filter { NotebookItems.isDone($0.item, in: $0.meeting) }
        return PageSection(title: NotebookItems.title(for: kind), count: open.count) {
            Card {
                if found.isEmpty {
                    EmptyLine(text: NotebookItems.emptyText(for: kind, anyNotes: recordings.contains { $0.aiNotes != nil }))
                } else {
                    if open.isEmpty { EmptyLine(text: "All done.") }
                    ForEach(open, id: \.item.id) { entry in
                        NotebookItemRow(item: entry.item, meeting: entry.meeting)
                        if entry.item.id != open.last?.item.id { Divider().overlay(Theme.hairline) }
                    }
                    if !done.isEmpty {
                        if !open.isEmpty { Divider().overlay(Theme.hairline) }
                        Button {
                            withAnimation(.easeOut(duration: 0.2)) { showDone.toggle() }
                        } label: {
                            Text("\(showDone ? "Hide" : "Show") \(done.count) done")
                                .font(.footnote.weight(.medium))
                                .frame(minHeight: 32)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("notebook-items-done-toggle")
                        if showDone {
                            ForEach(done, id: \.item.id) { entry in
                                NotebookItemRow(item: entry.item, meeting: entry.meeting)
                            }
                        }
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("notebook-items")
        }
    }

    // MARK: On the test and questions

    private func marked(_ recordings: [Meeting], kind: RecordingKind) -> some View {
        let found = recordings.flatMap { m in
            m.marks.filter { $0.markerKind == .test || $0.markerKind == .question }.map { (meeting: m, marker: $0) }
        }
        return PageSection(title: NotebookItems.markedTitle(for: kind), count: found.count) {
            Card {
                if found.isEmpty {
                    EmptyLine(text: NotebookItems.markedEmpty(for: kind))
                } else {
                    ForEach(found, id: \.marker.id) { entry in
                        NavigationLink {
                            MeetingDetailView(meeting: entry.meeting, jumpTo: entry.marker.at, inNotebook: true)
                        } label: {
                            MarkedRow(marker: entry.marker, meeting: entry.meeting)
                        }
                        .buttonStyle(.plain)
                        .accessibilityHint("Opens the recording at this moment")
                        if entry.marker.id != found.last?.marker.id { Divider().overlay(Theme.hairline) }
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("notebook-marked")
        }
    }

    // MARK: Review everything

    private func review(_ all: NotebookReview.Everything, kind: RecordingKind) -> some View {
        PageSection(title: "Review everything", count: 0) {
            Card {
                if all.isEmpty {
                    EmptyLine(text: NotebookItems.reviewEmpty(for: kind))
                } else {
                    Text("\(count(all.cards.count, "flashcard")) and \(count(all.questions.count, "practice question")) from this notebook's \(kind == .class ? "study" : "review") guides, as one.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 10) { reviewButtons(all) }
                        VStack(alignment: .leading, spacing: 8) { reviewButtons(all) }
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("notebook-review")
        }
    }

    @ViewBuilder private func reviewButtons(_ all: NotebookReview.Everything) -> some View {
        if !all.cards.isEmpty {
            Button { reviewing = .flashcards } label: {
                Label("Flashcards", systemImage: "rectangle.on.rectangle").frame(minHeight: 28)
            }
            .buttonStyle(.borderedProminent)
            .tint(Theme.ai)
            .accessibilityIdentifier("notebook-flashcards")
        }
        if !all.questions.isEmpty {
            Button { reviewing = .quiz } label: {
                Label("Practice quiz", systemImage: "checklist").frame(minHeight: 28)
            }
            .buttonStyle(.bordered)
            .tint(Theme.ai)
            .accessibilityIdentifier("notebook-quiz")
        }
    }

    private func count(_ n: Int, _ noun: String) -> String { n == 1 ? "1 \(noun)" : "\(n) \(noun)s" }

    // MARK: Recordings

    private func list(_ recordings: [Meeting]) -> some View {
        PageSection(title: "Recordings", count: 0) {
            Card {
                if recordings.isEmpty {
                    EmptyLine(text: "No recordings in this notebook. Tap Record to make the first one.")
                } else {
                    ForEach(recordings) { m in
                        NavigationLink {
                            MeetingDetailView(meeting: m, inNotebook: true)
                        } label: {
                            HStack(spacing: 10) {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(m.title).font(.body.weight(.medium)).lineLimit(2).multilineTextAlignment(.leading)
                                    Text([m.startedAt.formatted(date: .abbreviated, time: .shortened), m.duration?.minutesLabel]
                                        .compactMap { $0 }.joined(separator: " · "))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer(minLength: 0)
                                Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                                    .accessibilityHidden(true)
                            }
                            .frame(minHeight: 44)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        if m.id != recordings.last?.id { Divider().overlay(Theme.hairline) }
                    }
                }
            }
        }
    }
}

/// A section of the page: its title (the look of a recording's sections)
/// and, beside it, how many things it holds. The count is neutral: yellow
/// is for actions and the ★ mark.
private struct PageSection<Content: View>: View {
    let title: String
    let count: Int
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .accessibilityAddTraits(.isHeader)
                if count > 0 { CountBadge(count: count) }
            }
            content
        }
    }
}

/// "3" beside a section title: secondary fill, primary text
struct CountBadge: View {
    let count: Int

    var body: some View {
        Text("\(count)")
            .font(.caption.weight(.semibold))
            .monospacedDigit()
            .padding(.horizontal, 7)
            .padding(.vertical, 1)
            .background(Color.white.opacity(0.1), in: Capsule())
            .accessibilityLabel("\(count)")
    }
}

/// "Friday" beside an item: when it is due. Neutral, like the counts.
struct DueChip: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption.weight(.medium))
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(Color.white.opacity(0.1), in: Capsule())
            .accessibilityLabel("Due \(text)")
    }
}

/// The card every section of the page sits in (the look of a recording's sections)
private struct Card<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) { content }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

/// An empty section says what fills it
private struct EmptyLine: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A tick for something the notes say is to be done. The same box in a
/// recording's Notes and on its notebook's page: ticking one ticks the other
/// (the tick is kept as a hash of the item's text, per recording). Its
/// accessibility label is the item itself; whether it is ticked is its value.
struct ItemTick: View {
    let item: NotebookItems.Item
    let meeting: Meeting
    @Environment(\.modelContext) private var context

    var body: some View {
        let done = NotebookItems.isDone(item, in: meeting)
        Button {
            NotebookItems.setDone(!done, item: item, meeting: meeting, context: context)
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        } label: {
            Image(systemName: done ? "checkmark.circle.fill" : "circle")
                .font(.title3)
                .foregroundStyle(done ? Theme.accent : .secondary)
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(item.text)
        .accessibilityValue(done ? "Done" : "Not done")
        .accessibilityAddTraits(done ? [.isToggle, .isSelected] : .isToggle)
        .accessibilityHint(done ? "Marks it not done" : "Marks it done")
        .accessibilityIdentifier("item-tick")
    }
}

/// One item with its tick, when it is due, and the recording it came from.
private struct NotebookItemRow: View {
    let item: NotebookItems.Item
    let meeting: Meeting

    var body: some View {
        let done = NotebookItems.isDone(item, in: meeting)
        HStack(alignment: .top, spacing: 4) {
            ItemTick(item: item, meeting: meeting)
            VStack(alignment: .leading, spacing: 4) {
                // The tick already says the item to VoiceOver
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(item.task)
                        .font(.callout)
                        .strikethrough(done)
                        .foregroundStyle(done ? .secondary : .primary)
                        .textSelection(.enabled)
                    if let due = item.due { DueChip(text: due) }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityHidden(true)
                HStack(spacing: 4) {
                    if let assignee = item.assignee {
                        Text("\(assignee) ·").font(.caption).foregroundStyle(.secondary)
                    }
                    NavigationLink {
                        MeetingDetailView(meeting: meeting, scrollToNotes: true, inNotebook: true)
                    } label: {
                        Text("\(meeting.title) · \(NotebookPage.shortDate(meeting.startedAt))")
                            .font(.caption)
                            .lineLimit(1)
                            .frame(minHeight: 24, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(Theme.accent)
                    .accessibilityLabel("From \(meeting.title), \(NotebookPage.shortDate(meeting.startedAt))")
                    .accessibilityHint("Opens the recording's notes")
                }
            }
            .padding(.top, 10)
        }
        .padding(.leading, -10)
    }
}

/// A ✎ or ? mark with what was written, its recording and its time.
private struct MarkedRow: View {
    let marker: MomentMarker
    let meeting: Meeting

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: marker.markerKind.systemImage)
                .foregroundStyle(marker.markerKind.color)
                .frame(width: 24)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(marker.label)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(marker.markerKind.color)
                Text(marker.note ?? "No note")
                    .font(.callout)
                    .foregroundStyle(marker.note == nil ? .secondary : .primary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .multilineTextAlignment(.leading)
                Text("\(meeting.title) · \(NotebookPage.shortDate(meeting.startedAt)) · \(marker.offset(in: meeting).clock)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
            }
            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
                .padding(.top, 4)
                .accessibilityHidden(true)
        }
        .frame(minHeight: 44)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// The notebook's one deck or one quiz, in the guide's own views. "Start
/// from" keeps one recording's guide and every later one ("since the midterm").
private struct NotebookReviewSheet: View {
    let part: NotebookView.ReviewPart
    let sources: [NotebookReview.Source]
    let kind: RecordingKind
    /// A question's "Jump to this moment": the question and the time in its recording (ms)
    let onJump: (NotebookReview.Question, Int) -> Void
    @Environment(\.dismiss) private var dismiss
    /// nil: everything
    @State private var from: UUID?

    var body: some View {
        let guides = NotebookReview.guides(sources)
        let everything = NotebookReview.everything(sources, from: from)
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if guides.count > 1 {
                        HStack(spacing: 8) {
                            Text("Start from").font(.subheadline).foregroundStyle(.secondary)
                            Picker("Start from", selection: Binding(get: { from ?? guides[0].meetingID }, set: { from = $0 == guides[0].meetingID ? nil : $0 })) {
                                ForEach(Array(guides.enumerated()), id: \.element.meetingID) { i, g in
                                    Text("\(g.title) · \(NotebookPage.shortDate(g.startedAt))\(i == 0 ? " (everything)" : "")").tag(g.meetingID)
                                }
                            }
                            .pickerStyle(.menu)
                            .tint(.primary)
                            .accessibilityIdentifier("notebook-review-from")
                            Spacer(minLength: 0)
                        }
                    }
                    switch part {
                    case .flashcards:
                        if everything.cards.isEmpty {
                            Text("No flashcards from this recording on. Start from an earlier one.").font(.footnote).foregroundStyle(.secondary)
                        } else {
                            FlashcardsView(cards: everything.cards).id(from)
                        }
                    case .quiz:
                        if everything.questions.isEmpty {
                            Text("No practice questions from this recording on. Start from an earlier one.").font(.footnote).foregroundStyle(.secondary)
                        } else {
                            QuizView(questions: everything.questions.map(\.item), onJump: { _ in }, onJumpQuestion: { index, ms in
                                guard everything.questions.indices.contains(index) else { return }
                                onJump(everything.questions[index], ms)
                            })
                            .id(from)
                        }
                    }
                }
                .textSelection(.enabled)
                .frame(maxWidth: 720, alignment: .leading)
                .padding(20)
                .frame(maxWidth: .infinity)
            }
            .background(Theme.background)
            .navigationTitle(part == .flashcards ? "Flashcards" : "Practice quiz")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
            }
        }
    }
}
