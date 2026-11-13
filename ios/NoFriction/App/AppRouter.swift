import Foundation
import Observation

/// Where the app should go next, for the few moves that cross tabs (v2,
/// docs/design/FADELL_V2.md): Stop opens the recording, a notebook's Record
/// and Ask, a reminder's tap, and a start from outside the app. One shared
/// instance, because an App Intent and a notification have no view to ask.
/// Each request is taken once by the view that acts on it.
@MainActor
@Observable
final class AppRouter {
    static let shared = AppRouter()

    /// Record · Recordings · Chat · Settings. People live under Recordings.
    enum Tab: Hashable { case now, meetings, chat, settings }

    var tab: Tab = .now

    /// Open a recording in Recordings (scrolled to its Notes)
    struct OpenRecording: Equatable {
        var id: UUID
        var notes = true
    }
    var openRecording: OpenRecording?

    /// Open a notebook's page in Recordings
    var openNotebook: String?

    /// Open Chat on a new chat in this scope (a notebook's Ask)
    var chatScope: ChatScope?

    /// Open the Record sheet with these choices filled in (a notebook's Record)
    struct RecordPrefill: Equatable {
        var notebook: String?
        var kind: RecordingKind
    }
    var recordPrefill: RecordPrefill?

    /// Start recording with the remembered type and length, without the sheet
    /// (the Start recording shortcut); the Record tab shows the one-time
    /// recording notice instead if it was never accepted.
    var quickStartRequested = false

    /// A routine reminder was tapped: show that notebook's card on the Record tab
    struct Reminded: Equatable {
        var notebook: String
        var at: Date
    }
    var reminded: Reminded?

    /// The last plan of routine reminders: whether a routine exists, and
    /// whether to ask (in context, on the Record tab) for notifications
    var reminderPlan = RoutineReminders.Plan()

    /// The recording the user just stopped: Recordings, its Notes.
    func open(recording id: UUID) {
        openRecording = OpenRecording(id: id)
        tab = .meetings
    }

    func open(notebook name: String) {
        openNotebook = name
        tab = .meetings
    }

    func ask(about scope: ChatScope) {
        chatScope = scope
        tab = .chat
    }

    func record(notebook: String?, kind: RecordingKind) {
        recordPrefill = RecordPrefill(notebook: notebook, kind: kind)
        tab = .now
    }

    func startFromShortcut() {
        quickStartRequested = true
        tab = .now
    }

    func reminderTapped(notebook: String?) {
        if let notebook { reminded = Reminded(notebook: notebook, at: .now) }
        tab = .now
    }
}
