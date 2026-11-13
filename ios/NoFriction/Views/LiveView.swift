import PhotosUI
import SwiftData
import SwiftUI

/// Record. The transcript is the screen; the controls sit under your thumb.
struct LiveView: View {
    @Environment(RecordingSession.self) private var session
    @Environment(ScreenCaptureCenter.self) private var screenCapture
    @State private var showCamera = false
    @State private var photoItem: PhotosPickerItem?
    @State private var snapFlash = false
    @AppStorage("recordingNoticeAccepted") private var recordingNoticeAccepted = false
    @State private var showRecordingNotice = false
    /// "What is it?", "How long?" and a notebook before recording (the Record sheet)
    @State private var showPlanSheet = false
    @State private var planAfterNotice = false
    @AppStorage(ClassNotice.shownKey) private var classNoticeShown = false
    @State private var showClassNotice = false
    // v2 (docs/design/FADELL_V2.md): Up next, the note line, starts from outside the app
    @Environment(AppRouter.self) private var router
    @Environment(\.modelContext) private var context
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage(Onboarding.completedKey) private var onboardingCompleted = false
    /// The one suggestion on the idle screen (calendar event now, else the routine)
    @State private var upNext: UpNextSuggestion?
    /// What the suggested notebook's notes still have open ("Still open from last time")
    @State private var upNextOpen = NotebookItems.Preview()
    @AppStorage(MeetingEndDetector.Config.enabledKey) private var stopsWhenOver = true
    /// Up next's Record was tapped before the recording notice was ever accepted: start this once it is
    @State private var startAfterNotice: UpNextSuggestion?
    /// The note line has the keyboard: the big controls step aside so the transcript stays readable
    @FocusState private var noteFocused: Bool
    /// What is in the note line. Kept here so a line still in the field when
    /// the recording stops is kept on that recording, not lost
    @State private var noteDraft = NoteDraft()

    /// What the Up next card depends on: a change looks again
    private struct UpNextKey: Equatable {
        var idle: Bool
        var active: Bool
        var onboarded: Bool
        /// Back on the Record tab: something may have been recorded, ticked or deleted meanwhile
        var onTab: Bool
        var reminded: AppRouter.Reminded?
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                header
                if let countdown = session.endCountdown {
                    MeetingEndBanner(deadline: countdown.deadline,
                                     keep: { session.keepRecording() },
                                     stop: { session.stopNow(byUser: true) })
                } else if session.timeWarningVisible, let deadline = session.timeDeadline {
                    TimeLimitBanner(deadline: deadline,
                                    extend: { session.extendTimeLimit() },
                                    removeLimit: { session.removeTimeLimit() },
                                    dismiss: { session.dismissTimeWarning() })
                } else if let notice = session.notice {
                    NoticeBanner(text: notice)
                } else if screenCapture.hiddenNoticeDue {
                    HiddenVideoBanner { screenCapture.dismissHiddenNotice() }
                }
                if showClassNotice {
                    ClassNoticeBanner { showClassNotice = false }
                }
                // Idle: one card when something is starting, with the answer on its button
                if session.phase == .idle {
                    if let upNext {
                        UpNextCard(suggestion: upNext, plan: UpNext.planSummary(upNext, stopsWhenOver: stopsWhenOver), open: upNextOpen,
                                   record: { record(upNext) }, notNow: { notNow(upNext) })
                    } else if router.reminderPlan.shouldAsk, let notebook = router.reminderPlan.next {
                        ReminderPromptCard(notebook: notebook, onContinue: {
                            Task { await RoutineReminders.askThenPlan(context: context) }
                        }, notNow: {
                            RoutineReminders.declinePrompt()
                            router.reminderPlan.shouldAsk = false
                        })
                    }
                }
                TranscriptStream(meeting: session.meeting, partial: session.partial, phase: session.phase)
                // The recording notice is the sheet on the first Record (once); no standing caption
                if session.isActive {
                    // One line to type in; Return keeps it, stamped with this moment
                    NoteField(draft: $noteDraft, focused: $noteFocused)
                    // ★ / ? / ✎ moment markers (docs/STUDY_TOOLS.md); the ✎ label follows the type
                    if !noteFocused { MarkControl() }
                }
                // While typing a note the keyboard takes the bottom of the screen:
                // the transcript keeps the room (scroll it, or Done, to get the controls back)
                if !(session.isActive && noteFocused) { controls }
            }
            .background(Theme.background.ignoresSafeArea())
            // Apple's broadcast sheet for screen capture, opened from the
            // Capture screen row or right after Record (docs/SCREEN_CAPTURE_IOS.md)
            .background(alignment: .bottomLeading) {
                BroadcastPickerHost(center: screenCapture)
                    .frame(width: 1, height: 1)
                    .opacity(0.02)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
            .toolbar(.hidden, for: .navigationBar)
            .overlay {
                if snapFlash {
                    Color.white.opacity(0.35).ignoresSafeArea().transition(.opacity).allowsHitTesting(false)
                }
            }
            .sheet(isPresented: $showRecordingNotice, onDismiss: {
                let suggestion = startAfterNotice
                startAfterNotice = nil
                guard planAfterNotice else { return }
                planAfterNotice = false
                if let suggestion {
                    // Accepted from Up next: start what its button said
                    start(limit: suggestion.limit, kind: suggestion.kind, notebook: suggestion.notebook)
                } else {
                    // Accepted: on to the Record sheet ("What is it?", "How long?")
                    showPlanSheet = true
                }
            }) {
                RecordingNoticeSheet {
                    recordingNoticeAccepted = true
                    planAfterNotice = true
                }
            }
            .sheet(isPresented: $showPlanSheet, onDismiss: { router.recordPrefill = nil }) {
                // From a notebook's Record: that notebook and its usual type, filled in
                RecordPlanSheet(prefill: router.recordPrefill) { limit, kind, notebook in
                    // First Class recording ever: a one-time reminder about school policy
                    if kind == .class && !classNoticeShown {
                        classNoticeShown = true
                        showClassNotice = true
                    }
                    // "Capture screen" on: Apple's sheet opens once the recording runs
                    let captureScreen = ScreenCapturePrefs.captureOnRecord()
                    Task {
                        await session.start(limit: limit, kind: kind, notebook: notebook, captureScreen: captureScreen)
                        guard captureScreen, session.isActive, !screenCapture.isCapturing else { return }
                        try? await Task.sleep(for: .milliseconds(400))   // the Record sheet finishes closing
                        screenCapture.presentPicker()
                    }
                }
            }
            .fullScreenCover(isPresented: $showCamera) {
                CameraPicker { image in
                    if let image { saveSnapshot(image) }
                }
                .ignoresSafeArea()
            }
            .onChange(of: photoItem) { _, item in
                guard let item else { return }
                Task {
                    if let data = try? await item.loadTransferable(type: Data.self), let image = UIImage(data: data) {
                        saveSnapshot(image)
                    }
                    photoItem = nil
                }
            }
            // Up next: looked at again every half minute while the idle screen is in front
            .task(id: UpNextKey(idle: session.phase == .idle, active: scenePhase == .active, onboarded: onboardingCompleted,
                                onTab: router.tab == .now, reminded: router.reminded)) {
                guard session.phase == .idle, scenePhase == .active, onboardingCompleted, router.tab == .now else {
                    if session.phase != .idle { upNext = nil }
                    return
                }
                while !Task.isCancelled {
                    refreshUpNext()
                    try? await Task.sleep(for: .seconds(30))
                }
            }
            .onChange(of: session.phase) { _, phase in
                if phase == .idle { noteFocused = false }
            }
            // A line typed but not yet kept when the recording stops (by the
            // button, its time limit or the end of the meeting) belongs to it
            .onAppear {
                session.beforeStop = {
                    guard let line = noteDraft.take() else { return }
                    if session.addMarker(kind: .note, note: line) == nil { noteDraft.putBack(line) }
                }
            }
            // A notebook's Record, and the Start recording shortcut
            .onAppear { Task { @MainActor in takeRequests() } }
            .onChange(of: router.recordPrefill) { _, _ in takeRequests() }
            .onChange(of: router.quickStartRequested) { _, _ in takeRequests() }
        }
    }

    // MARK: Up next and starts without the sheet

    /// The calendar event that is on now or starts within 15 minutes (only
    /// when calendar access was already granted; the same lookup a recording
    /// uses to name itself), else the routine.
    private func refreshUpNext() {
        let now = Date()
        let calendar = Calendar.current
        let recordings = Routine.recordings(in: context, now: now)
        var routine = Routine.suggestion(recordings, now: now, calendar: calendar)
        // A reminder that was just tapped shows its notebook's card even a little outside the window
        if routine == nil, let reminded = router.reminded, now.timeIntervalSince(reminded.at) < 30 * 60 {
            routine = Routine.usual(reminded.notebook, recordings, now: now, calendar: calendar)
        }
        let saved = ((try? context.fetch(FetchDescriptor<Meeting>())) ?? []).filter { $0.id != session.meeting?.id }
        var event: UpNext.EventNow?
        if CalendarService.shared.isAuthorized {
            // A meeting (other people, a call link, or recorded before under
            // that title) that has no recording yet; never an all-day event
            let recorded = saved.map {
                UpNext.Recorded(title: $0.title, notebook: $0.courseName, kind: $0.kind, startedAt: $0.startedAt, calendarEventID: $0.calendarEventID)
            }
            let events = CalendarService.shared.events(from: now.addingTimeInterval(-6 * 3600), to: now.addingTimeInterval(3600))
            event = UpNext.event(events, recordings: recorded, now: now)
        }
        let found = UpNext.choose(event: event, routine: routine, now: now, calendar: calendar) {
            UpNextDismissals.isHidden($0, now: now, calendar: calendar)
        }
        // What you said you'd do, before you walk in: read from the notes that exist, no AI
        let open = found?.notebook.map { notebook in
            NotebookItems.preview(open: NotebookItems.open(in: saved.filter { Notebook.matches($0.courseName, filter: notebook) }))
        } ?? NotebookItems.Preview()
        if found != upNext || open != upNextOpen {
            withAnimation(.easeOut(duration: 0.2)) {
                upNext = found
                upNextOpen = open
            }
        }
    }

    /// **Record <name>**: no sheet. The one-time recording notice comes
    /// first if it was never accepted.
    private func record(_ suggestion: UpNextSuggestion) {
        guard recordingNoticeAccepted else {
            startAfterNotice = suggestion
            showRecordingNotice = true
            return
        }
        start(limit: suggestion.limit, kind: suggestion.kind, notebook: suggestion.notebook)
    }

    /// **Not now**: hidden until tomorrow, and so is today's reminder for it.
    private func notNow(_ suggestion: UpNextSuggestion) {
        UpNextDismissals.hide([suggestion.key], now: .now)
        if router.reminded?.notebook.caseInsensitiveCompare(suggestion.notebook ?? "") == .orderedSame { router.reminded = nil }
        withAnimation(.easeOut(duration: 0.2)) { upNext = nil }
        Task { await RoutineReminders.replanAndPublish(context: context) }
    }

    /// A start that skips the Record sheet. nil = the remembered type and
    /// length (`RecordingSession.start`), as for every such start. It only
    /// listens: the screen is never asked for here.
    private func start(limit: RecordingLimit?, kind: RecordingKind?, notebook: String?) {
        guard session.phase == .idle else { return }
        // First Class recording ever: a one-time reminder about school policy
        if (kind ?? RecordingKindStore.remembered()) == .class && !classNoticeShown {
            classNoticeShown = true
            showClassNotice = true
        }
        upNext = nil
        Task { await session.start(limit: limit, kind: kind, notebook: notebook) }
    }

    /// Requests from elsewhere in the app, each taken once.
    private func takeRequests() {
        guard onboardingCompleted else {
            // A start asked for from outside while the first-run pages are
            // up is dropped, so nothing starts later by surprise
            router.quickStartRequested = false
            return
        }
        if router.quickStartRequested {
            // The Start recording shortcut: the remembered type and length,
            // or the one-time recording notice instead if it was never accepted
            router.quickStartRequested = false
            if session.phase == .idle {
                router.recordPrefill = nil
                showPlanSheet = false
                if recordingNoticeAccepted { start(limit: nil, kind: nil, notebook: nil) } else { showRecordingNotice = true }
            }
        } else if router.recordPrefill != nil, !showPlanSheet, !showRecordingNotice {
            // A notebook's Record: the Record sheet, filled in
            if session.phase != .idle {
                router.recordPrefill = nil
            } else if recordingNoticeAccepted {
                showPlanSheet = true
            } else {
                showRecordingNotice = true
            }
        }
    }

    // MARK: Header

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(session.meeting?.title ?? "Ready")
                    .font(.title2.weight(.semibold))
                    .lineLimit(2)
                Spacer()
                if session.isActive {
                    TimelineView(.periodic(from: .now, by: 1)) { ctx in
                        // With a time limit: what's left; otherwise elapsed
                        if let left = session.timeRemaining(at: ctx.date) {
                            Text("\(left.clock) left")
                                .font(.system(.body, design: .monospaced).weight(.medium))
                                .foregroundStyle(left <= 300 ? Theme.accent : (session.phase == .paused ? .secondary : .primary))
                                .contentTransition(.numericText())
                                .accessibilityLabel("\(Duration.seconds(left).formatted(.units(allowed: [.hours, .minutes, .seconds], width: .wide))) left")
                        } else {
                            Text(session.elapsed(at: ctx.date).clock)
                                .font(.system(.body, design: .monospaced).weight(.medium))
                                .foregroundStyle(session.phase == .paused ? .secondary : .primary)
                                .contentTransition(.numericText())
                                .accessibilityLabel("Elapsed \(Duration.seconds(session.elapsed(at: ctx.date)).formatted(.units(allowed: [.hours, .minutes, .seconds], width: .wide)))")
                        }
                    }
                }
            }
            // Idle: the promise below says it once; no second line here
            if let subtitle {
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            if session.isActive, let deadline = session.timeDeadline {
                TimeLimitRow(deadline: deadline,
                             extend: { session.extendTimeLimit() },
                             removeLimit: { session.removeTimeLimit() })
                    .padding(.top, 4)
            }
            if session.isActive || screenCapture.isCapturing {
                ScreenCaptureRow().padding(.top, 4)
            }
            if let people = session.meeting?.people, !people.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(people, id: \.person.email) { item in
                            Text(item.person.displayName)
                                .font(.caption)
                                .padding(.horizontal, 10).padding(.vertical, 5)
                                .background(Theme.card, in: Capsule())
                        }
                    }
                }
                .padding(.top, 4)
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 12)
        .padding(.bottom, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var subtitle: String? {
        switch session.phase {
        case .idle: return nil
        case .starting: return "Starting…"
        case .paused: return "Paused — nothing is being recorded"
        case .stopping: return "Saving…"
        case .recording:
            if let m = session.meeting, let s = m.scheduledStart, let e = m.scheduledEnd {
                return "\(s.formatted(date: .omitted, time: .shortened)) – \(e.formatted(date: .omitted, time: .shortened))"
            }
            // "Class · BIO 101 · Recording"
            guard let m = session.meeting else { return "Recording" }
            return ([m.kind.label] + [m.courseName].compactMap { $0 } + ["Recording"]).joined(separator: " · ")
        }
    }

    // MARK: Controls

    private var controls: some View {
        HStack(alignment: .center) {
            // Pause
            CircleButton(systemImage: session.phase == .paused ? "play.fill" : "pause.fill",
                         label: session.phase == .paused ? "Resume" : "Pause",
                         enabled: session.phase == .recording || session.phase == .paused) {
                session.togglePause()
            }
            .accessibilityHint(session.phase == .paused ? "Continues recording" : "Pauses recording. Nothing is recorded while paused.")
            .opacity(session.isActive ? 1 : 0)
            .accessibilityHidden(!session.isActive)
            Spacer()
            RecordButton(phase: session.phase, level: session.level) {
                if session.isActive {
                    // The user's own Stop: the recording opens on its Notes (RootView)
                    Task { await session.stop(byUser: true) }
                } else if !recordingNoticeAccepted {
                    showRecordingNotice = true
                } else {
                    // The Record sheet first; it starts the recording
                    showPlanSheet = true
                }
            }
            Spacer()
            // Photo: camera where there is one; photo library as the alternative
            Menu {
                if UIImagePickerController.isSourceTypeAvailable(.camera) {
                    Button("Take Photo", systemImage: "camera") { showCamera = true }
                }
                PhotosPicker(selection: $photoItem, matching: .images) {
                    Label("Choose from Photos", systemImage: "photo.on.rectangle")
                }
            } label: {
                CircleButtonLabel(systemImage: "camera.fill", label: "Photo", enabled: session.isActive)
            } primaryAction: {
                if UIImagePickerController.isSourceTypeAvailable(.camera) { showCamera = true }
            }
            .disabled(!session.isActive)
            .opacity(session.isActive ? 1 : 0)
            .accessibilityHidden(!session.isActive)
            .accessibilityLabel("Photo")
            .accessibilityHint("Adds a photo of a slide, whiteboard or screen to this recording")
        }
        .padding(.horizontal, 36)
        .padding(.top, 14)
        .padding(.bottom, 10)
        .background(alignment: .top) {
            Rectangle().fill(Theme.hairline).frame(height: 1)
        }
    }

    private func saveSnapshot(_ image: UIImage) {
        session.addSnapshot(image)
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        withAnimation(.easeOut(duration: 0.08)) { snapFlash = true }
        withAnimation(.easeIn(duration: 0.25).delay(0.08)) { snapFlash = false }
    }
}

// MARK: - Transcript

private struct TranscriptStream: View {
    let meeting: Meeting?
    let partial: String
    let phase: RecordingSession.Phase

    var body: some View {
        let segments = meeting?.orderedSegments ?? []
        // What the user typed, where it was written (the placement rule marks use)
        let notes = meeting?.typedNotes ?? []
        let hosted = TranscriptPlacement.hosted(notes.map(\.at), rows: segments.map(\.start))
        ScrollViewReader { proxy in
            ScrollView {
                if segments.isEmpty && partial.isEmpty && notes.isEmpty {
                    EmptyTranscript(phase: phase)
                        .frame(maxWidth: .infinity)
                        .padding(.top, 80)
                } else {
                    LazyVStack(alignment: .leading, spacing: 14) {
                        // Typed before anything was said
                        if segments.isEmpty {
                            ForEach(notes) { note in TypedNoteRow(text: note.note ?? "", time: note.at) }
                        }
                        ForEach(Array(segments.enumerated()), id: \.element.persistentModelID) { index, segment in
                            TranscriptLine(time: segment.start, text: segment.text)
                            ForEach(hosted[index] ?? [], id: \.self) { n in
                                TypedNoteRow(text: notes[n].note ?? "", time: notes[n].at)
                            }
                        }
                        if !partial.isEmpty {
                            TranscriptLine(time: nil, text: partial, provisional: true)
                        }
                        Color.clear.frame(height: 1).id("end")
                    }
                    .padding(.horizontal, 20)
                    .padding(.vertical, 12)
                    .frame(maxWidth: 720, alignment: .leading)
                    .frame(maxWidth: .infinity)
                }
            }
            .scrollDismissesKeyboard(.immediately)
            // A live transcript opens at its newest line
            .defaultScrollAnchor(.bottom)
            .onChange(of: segments.count) { _, _ in withAnimation(.easeOut(duration: 0.25)) { proxy.scrollTo("end", anchor: .bottom) } }
            .onChange(of: notes.count) { _, _ in withAnimation(.easeOut(duration: 0.25)) { proxy.scrollTo("end", anchor: .bottom) } }
            .onChange(of: partial) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
        }
    }
}

/// Where a mark or a typed note goes in a transcript: after the line being
/// spoken at that moment (the last line that started at or before it), or
/// after the first line when it came before any. Pure; unit-tested.
enum TranscriptPlacement {
    /// The index of the row a moment follows; nil without rows.
    static func host(of moment: Date, rows: [Date]) -> Int? {
        guard !rows.isEmpty else { return nil }
        return rows.lastIndex { $0 <= moment } ?? 0
    }

    /// For each row index, the indices of the moments it hosts, in order.
    static func hosted(_ moments: [Date], rows: [Date]) -> [Int: [Int]] {
        var out: [Int: [Int]] = [:]
        for (i, m) in moments.enumerated() {
            if let row = host(of: m, rows: rows) { out[row, default: []].append(i) }
        }
        return out
    }
}

/// The note line (docs/design/FADELL_V2.md §3 B): one field, "Type a note".
/// Return keeps the line, stamped with that moment, clears the field and
/// keeps the keyboard up for the next one. Typing and keeping notes is free.
private struct NoteField: View {
    @Environment(RecordingSession.self) private var session
    @Binding var draft: NoteDraft
    var focused: FocusState<Bool>.Binding
    /// The last line couldn't be kept: it is back in the field
    @State private var failed = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Image(systemName: MarkerKind.note.systemImage)
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                TextField("Type a note", text: $draft.text)
                    .focused(focused)
                    .submitLabel(.done)
                    .onSubmit(save)
                    .onChange(of: draft.text) { _, text in
                        // One line of at most 1,000 characters
                        if text.count > MomentMarker.maxTypedNoteLength { draft.text = String(text.prefix(MomentMarker.maxTypedNoteLength)) }
                    }
                    .frame(minHeight: 28)
                    .accessibilityLabel("Type a note")
                    .accessibilityHint("Return keeps it in the transcript at this moment")
                    .accessibilityIdentifier("note-field")
                if focused.wrappedValue {
                    Button("Done") { focused.wrappedValue = false }
                        .font(.subheadline.weight(.semibold))
                        .frame(minWidth: 44, minHeight: 28)
                        .accessibilityLabel("Hide keyboard")
                        .accessibilityHint("Shows the recording controls again")
                        .accessibilityIdentifier("note-done")
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            if failed {
                Label("That note couldn't be kept. It is back in the field: press Return to try again.", systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .accessibilityIdentifier("note-failed")
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 6)
        .padding(.bottom, 8)
        .frame(maxWidth: 720)
    }

    private func save() {
        // The line leaves the field at once. It belongs to the recording and
        // the moment of Return: both are taken here, before anything else…
        let stamp = Date.now
        if let line = draft.take() {
            if session.addMarker(kind: .note, note: line, at: stamp) != nil {
                failed = false
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                UIAccessibility.post(notification: .announcement, argument: "Note kept")
            } else {
                // …and comes back, ahead of anything typed since, only if it couldn't be kept
                draft.putBack(line)
                failed = true
                UIAccessibility.post(notification: .announcement, argument: "That note couldn't be kept. It is back in the field.")
            }
        }
        // Stay in the field for the next line
        focused.wrappedValue = true
    }
}

/// **Up next**: the one suggestion on the idle Record screen, with the
/// answer on its button. Under the plan, what the suggested notebook's notes
/// still have open. Not now hides it until tomorrow.
private struct UpNextCard: View {
    let suggestion: UpNextSuggestion
    /// "Class · 60 min · BIO 101": the Record sheet's three answers, already filled in
    let plan: String
    let open: NotebookItems.Preview
    let record: () -> Void
    let notNow: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Label("Up next", systemImage: suggestion.source == .calendar ? "calendar" : "clock")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.secondary)
                Text(suggestion.name)
                    .font(.title3.weight(.semibold))
                    .lineLimit(2)
                Text(suggestion.detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                Text(plan)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            if !open.shown.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Still open from last time")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ForEach(open.shown) { item in
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Text("•").foregroundStyle(.secondary)
                            Text(item.task + (item.due.map { " · \($0)" } ?? ""))
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .font(.footnote)
                    }
                    if open.more > 0 {
                        Text("and \(open.more) more")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.top, 2)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("up-next-open")
            }
            // Side by side; stacked when large text wouldn't fit
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 14) { buttons }
                VStack(alignment: .leading, spacing: 8) { buttons }
            }
        }
        .padding(14)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay { RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(Theme.hairline, lineWidth: 1) }
        .frame(maxWidth: 520)
        .padding(.horizontal, 16)
        .padding(.bottom, 6)
        .transition(.opacity)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("up-next")
    }

    @ViewBuilder private var buttons: some View {
        Button(action: {
            UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
            record()
        }) {
            Label("Record \(suggestion.name)", systemImage: "record.circle")
                .font(.subheadline.weight(.semibold))
                .lineLimit(1)
                .frame(minHeight: 30)
        }
        .buttonStyle(.borderedProminent)
        .tint(Theme.recordingStrong)
        .accessibilityHint("Starts recording now as \(plan)")
        .accessibilityIdentifier("up-next-record")
        Button("Not now", action: notNow)
            .font(.subheadline)
            .frame(minHeight: 44)
            .accessibilityHint("Hides this until tomorrow")
            .accessibilityIdentifier("up-next-not-now")
    }
}

/// Before the system asks about notifications (never at launch: only once a
/// routine exists). The button says Continue; iOS asks next.
private struct ReminderPromptCard: View {
    let notebook: String
    let onContinue: () -> Void
    let notNow: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text("Get a reminder when \(notebook) starts?")
                    .font(.subheadline.weight(.semibold))
                Text("noFriction can remind you when something you usually record is about to start. You can turn this off in Settings.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            HStack(spacing: 14) {
                Button("Continue", action: onContinue)
                    .buttonStyle(.bordered)
                    .tint(Theme.accent)
                    .accessibilityHint("iOS asks whether noFriction may send notifications")
                    .accessibilityIdentifier("reminder-prompt-continue")
                Button("Not now", action: notNow)
                    .frame(minHeight: 44)
                    .accessibilityIdentifier("reminder-prompt-not-now")
            }
            .font(.subheadline)
        }
        .padding(14)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .frame(maxWidth: 520)
        .padding(.horizontal, 16)
        .padding(.bottom, 6)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("reminder-prompt")
    }
}

struct TranscriptLine: View {
    let time: Date?
    let text: String
    var provisional = false
    /// Time column grows with Dynamic Type so "10:42" never wraps
    @ScaledMetric(relativeTo: .caption2) var timeWidth: CGFloat = 52
    @Environment(\.horizontalSizeClass) private var sizeClass

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 14) {
            Text(time.map { $0.formatted(date: .omitted, time: .shortened) } ?? "")
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(.tertiary)
                .frame(width: timeWidth, alignment: .trailing)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
            (Text(text) + (provisional ? Text(" ▍").foregroundStyle(Theme.accent) : Text("")))
                .font(sizeClass == .regular ? .title3 : .body)   // iPad: readable across the room
                .lineSpacing(3)
                .foregroundStyle(provisional ? .secondary : .primary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(provisional ? "\(text), still listening" : text)
        .accessibilityValue(time.map { $0.formatted(date: .omitted, time: .shortened) } ?? "")
    }
}

private struct EmptyTranscript: View {
    let phase: RecordingSession.Phase

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: "waveform")
                .font(.system(size: 34, weight: .light))
                .foregroundStyle(phase == .recording ? Theme.accent : .secondary)
                .symbolEffect(.variableColor.iterative, isActive: phase == .recording)
                .accessibilityHidden(true)
            Text(phase == .recording ? "Listening" : "Tap to record")
                .font(.headline)
            if phase == .recording {
                Text("Words appear as they're spoken.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else {
                PrivacyPromise()
                    .padding(.top, 6)
            }
        }
        .padding(.horizontal, 32)
    }
}

/// "iPhone" / "iPad", for copy that names this device.
enum DeviceName {
    static var current: String {
        UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
    }
}

/// The home screen's promise: recording and on-device transcription work
/// offline, and nothing leaves the device unless the user sends it (AI
/// through a server they set up, exporting, sharing).
struct PrivacyPromise: View {
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "lock.fill")
                .font(.subheadline)
                .foregroundStyle(Theme.accent)
                .frame(width: 30, height: 30)
                .background(Theme.accent.opacity(0.12), in: Circle())
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text("Recording and transcription work offline.")
                    .font(.subheadline.weight(.semibold))
                Text("Nothing leaves this \(DeviceName.current) unless you want it to.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .frame(maxWidth: 420, alignment: .leading)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .combine)
        .accessibilityHint("Something leaves only when you choose: AI through a server you set up, or exporting and sharing.")
        .accessibilityIdentifier("privacy-promise")
    }
}

// MARK: - Buttons

private struct RecordButton: View {
    let phase: RecordingSession.Phase
    let level: Float
    let action: () -> Void

    var body: some View {
        let active = phase == .recording || phase == .paused || phase == .starting
        Button(action: {
            UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
            action()
        }) {
            ZStack {
                Circle()
                    .stroke(Color.white.opacity(0.9), lineWidth: 3)
                    .frame(width: 78, height: 78)
                // Level ring
                Circle()
                    .stroke(Theme.recording.opacity(0.35), lineWidth: 6)
                    .frame(width: 78 + CGFloat(level) * 18, height: 78 + CGFloat(level) * 18)
                    .opacity(phase == .recording ? 1 : 0)
                    .animation(.easeOut(duration: 0.1), value: level)
                RoundedRectangle(cornerRadius: active ? 8 : 32, style: .continuous)
                    .fill(Theme.recording)
                    .frame(width: active ? 30 : 64, height: active ? 30 : 64)
                    .animation(.spring(response: 0.35, dampingFraction: 0.7), value: active)
                if phase == .starting || phase == .stopping {
                    ProgressView().tint(.white)
                }
            }
            .frame(width: 100, height: 100)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .disabled(phase == .starting || phase == .stopping)
        .accessibilityLabel(active ? "Stop recording" : "Start recording")
        .accessibilityHint(active ? "Stops and saves the recording" : "Records and transcribes on this device")
        .accessibilityIdentifier("record-button")
    }
}

private struct CircleButton: View {
    let systemImage: String
    let label: String
    let enabled: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) { CircleButtonLabel(systemImage: systemImage, label: label, enabled: enabled) }
            .buttonStyle(.plain)
            .disabled(!enabled)
    }
}

private struct CircleButtonLabel: View {
    let systemImage: String
    let label: String
    let enabled: Bool

    var body: some View {
        VStack(spacing: 6) {
            Image(systemName: systemImage)
                .font(.system(size: 18, weight: .semibold))
                .frame(width: 52, height: 52)
                .background(Theme.card, in: Circle())
            Text(label).font(.caption2).foregroundStyle(.secondary)
        }
        .opacity(enabled ? 1 : 0.35)
        .accessibilityElement(children: .combine)
    }
}

struct NoticeBanner: View {
    let text: String
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.accent)
                .accessibilityHidden(true)
            Text(text).font(.footnote)
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
        .padding(12)
        .background(Theme.accent.opacity(0.1), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .padding(.horizontal, 16)
        .padding(.bottom, 6)
    }
}

/// "This seems to have ended — stopping in 30 s" with the two choices.
struct MeetingEndBanner: View {
    let deadline: Date
    let keep: () -> Void
    let stop: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Image(systemName: "moon.zzz.fill").foregroundStyle(Theme.accent)
                    .accessibilityHidden(true)
                TimelineView(.periodic(from: .now, by: 1)) { ctx in
                    let left = max(0, Int(deadline.timeIntervalSince(ctx.date).rounded(.up)))
                    Text("This seems to have ended — stopping in \(left) s")
                        .font(.footnote.weight(.semibold))
                        .contentTransition(.numericText())
                }
                Spacer(minLength: 0)
            }
            // Side by side; stacked when large text wouldn't fit
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) { bannerButtons }
                VStack(alignment: .leading, spacing: 8) { bannerButtons }
            }
            .font(.footnote.weight(.medium))
        }
        .padding(12)
        .background(Theme.accent.opacity(0.1), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .padding(.horizontal, 16)
        .padding(.bottom, 6)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("meeting-end-banner")
    }

    @ViewBuilder private var bannerButtons: some View {
        Button("Keep recording", action: keep)
            .buttonStyle(.bordered)
            .fixedSize()
            .accessibilityHint("Cancels the automatic stop")
            .accessibilityIdentifier("meeting-end-keep")
        Button("Stop now", role: .destructive, action: stop)
            .buttonStyle(.borderedProminent)
            .tint(Theme.recordingStrong)
            .fixedSize()
            .accessibilityHint("Stops and saves the recording now")
            .accessibilityIdentifier("meeting-end-stop")
    }
}

/// UIKit camera, wrapped.
struct CameraPicker: UIViewControllerRepresentable {
    let onFinish: (UIImage?) -> Void
    @Environment(\.dismiss) private var dismiss

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_: UIImagePickerController, context _: Context) {}
    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let parent: CameraPicker
        init(_ parent: CameraPicker) { self.parent = parent }

        func imagePickerController(_: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            parent.onFinish(info[.originalImage] as? UIImage)
            parent.dismiss()
        }

        func imagePickerControllerDidCancel(_: UIImagePickerController) {
            parent.onFinish(nil)
            parent.dismiss()
        }
    }
}
