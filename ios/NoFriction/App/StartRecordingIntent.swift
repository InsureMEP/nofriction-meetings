import AppIntents

/// "Start a recording in noFriction" on the iPhone and iPad: Siri, the
/// Shortcuts app and, through a shortcut, the Action button. It opens the
/// app first (recording starts in the foreground), then starts with the
/// remembered choices from the Record sheet (type and "How long?", no
/// notebook), so a timed recording still stops by itself. If the one-time
/// recording notice was never accepted, the app shows it instead of
/// recording.
///
/// The watch app has its own intent of the same name
/// (ios/NoFrictionWatch/App/StartRecordingIntent.swift): each is compiled
/// into its own app and registered by that app's bundle.
///
/// App Store processing rejects intent text containing "apple"
/// (ITMS-90626): keep it out of the title, description and phrases.
struct StartRecordingIntent: AppIntent {
    static let title: LocalizedStringResource = "Start Recording"
    static let description = IntentDescription("Opens noFriction and starts recording with your last choices.")
    static let openAppWhenRun = true

    @MainActor
    func perform() async throws -> some IntentResult {
        AppRouter.shared.startFromShortcut()
        return .result()
    }
}

struct NoFrictionShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: StartRecordingIntent(),
            phrases: [
                "Start a recording in \(.applicationName)",
                "Record with \(.applicationName)",
            ],
            shortTitle: "Start Recording",
            systemImageName: "mic.fill"
        )
    }
}
