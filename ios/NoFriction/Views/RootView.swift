import SwiftData
import SwiftUI

/// iPhone: a tab bar. iPad: the same tabs become a sidebar
/// (.sidebarAdaptable), and Recordings becomes a list + detail split.
struct RootView: View {
    @Environment(RecordingSession.self) private var session
    @Environment(\.modelContext) private var context
    @Environment(\.scenePhase) private var scenePhase
    @Environment(RedactionCenter.self) private var redactions
    @Environment(WatchImporter.self) private var watchImporter
    /// Moves across tabs: Stop opens the recording, a notebook's Record and Ask, a reminder's tap
    @Environment(AppRouter.self) private var router
    @Environment(Store.self) private var store
    @Environment(AISettings.self) private var aiSettings
    @Environment(AutoNotes.self) private var autoNotes
    @AppStorage(Onboarding.completedKey) private var onboardingCompleted = false

    /// Record · Recordings · Chat · Settings. People live under Recordings.
    typealias AppTab = AppRouter.Tab

    var body: some View {
        @Bindable var router = router
        TabView(selection: $router.tab) {
            Tab("Record", systemImage: "waveform", value: AppTab.now) {
                LiveView()
            }
            Tab("Recordings", systemImage: "rectangle.stack", value: AppTab.meetings) {
                MeetingsView()
            }
            // Chat with your recordings (docs/TOPICS_AND_CHAT.md)
            Tab("Chat", systemImage: "bubble.left.and.text.bubble.right", value: AppTab.chat) {
                ChatView()
            }
            Tab("Settings", systemImage: "gearshape", value: AppTab.settings) {
                SettingsView()
            }
        }
        .tabViewStyle(.sidebarAdaptable)
        .overlay(alignment: .bottom) {
            RedactionToast().padding(.bottom, 64)
        }
        .fullScreenCover(isPresented: Binding(get: { !onboardingCompleted }, set: { onboardingCompleted = !$0 })) {
            OnboardingView { onboardingCompleted = true }
        }
        .onAppear {
            session.attach(context)
            // Finish any Delete cut short by the app being killed in its undo window
            redactions.recover(context: context)
            #if DEBUG
            DemoData.seedIfRequested(context)
            DemoData.seedWatchDemoIfRequested(context)
            if ProcessInfo.processInfo.arguments.contains("-NFDemoLive") {
                DemoData.showLiveMeeting(session: session, context: context)
            }
            if ProcessInfo.processInfo.arguments.contains("-NFAutoRecord") {
                Task { await session.start() }
            }
            DemoData.openIfRequested(router: router, context: context)
            #endif
        }
        .onChange(of: session.phase) { _, phase in
            // Watch imports wait while this iPhone records; carry on after
            if phase == .idle { watchImporter.resume() }
        }
        .onChange(of: session.lastStopped) { _, stopped in
            guard let stopped else { return }
            recordingStopped(stopped)
        }
        .onChange(of: scenePhase) { _, phase in
            // Pick up meetings recorded before calendar access was granted
            if phase == .active { _ = MeetingLinker.backfill(in: context) }
            // Apple Watch recordings waiting (or cut short in the background)
            if phase == .active { watchImporter.resume() }
            // Leaving the foreground commits a pending Delete (never silently dropped)
            if phase != .active { redactions.commitPending() }
            // A meeting-end countdown follows the app off screen as a notification
            session.sceneDidChange(active: phase == .active)
            // The routine's reminders for the next days (never asks for permission here)
            if phase == .active { planReminders() }
        }
    }

    /// A recording just stopped (docs/design/FADELL_V2.md).
    private func recordingStopped(_ stopped: RecordingSession.Stopped) {
        let id = stopped.meetingID
        let meeting = (try? context.fetch(FetchDescriptor<Meeting>(predicate: #Predicate { $0.id == id })))?.first
        if let meeting {
            // Notes by themselves, when nothing would have to be asked
            // (Pro, AI set up, consent already given); otherwise Make notes is there
            // and never while the next recording runs
            let session = session
            autoNotes.recordingStopped(meeting, isPro: store.isPro, endpoint: aiSettings.endpoint(), context: context,
                                       isRecording: { session.phase != .idle })
            // The user's own Stop opens the recording on its Notes. A stop
            // the app made by itself leaves the screen alone.
            if stopped.byUser, scenePhase == .active { router.open(recording: id) }
        }
        // It may have made, or kept, a routine
        planReminders()
    }

    private func planReminders() {
        Task { await RoutineReminders.replanAndPublish(context: context, excluding: session.meeting?.id) }
    }
}
