import Foundation
import SwiftData
import UserNotifications

/// "Remind me when something starts" (Settings → Recording, on): local
/// notifications scheduled ahead from the routine (`Routine.reminders`), so
/// they fire with the app closed. At most the next 7 days; planned again
/// when the app becomes active, after a recording stops, after Not now and
/// when the setting changes; cancelled when the setting is off or the
/// routine fades. Tapping one opens the Record tab with the card showing
/// (`MeetingEndNotifier` is the notification delegate).
///
/// Permission is asked in context only: the first time a routine exists
/// while the app is in front, a card on the Record tab explains and its
/// button says "Continue" (never at launch). If permission was denied,
/// nothing is scheduled and nothing is asked.
@MainActor
enum RoutineReminders {
    nonisolated static let settingKey = "routineReminders"
    /// The user answered the card on the Record tab (Continue or Not now): it isn't shown again
    nonisolated static let promptAnsweredKey = "routineRemindersPromptAnswered"
    nonisolated static let category = "ROUTINE"
    nonisolated static let notebookKey = "notebook"
    nonisolated static let idPrefix = "routine-"
    /// iOS keeps 64 pending notifications per app; the routine takes a part
    nonisolated static let maxScheduled = 24

    static var isOn: Bool { UserDefaults.standard.object(forKey: settingKey) as? Bool ?? true }

    nonisolated static func body(_ notebook: String) -> String { "\(notebook) usually starts about now." }

    /// What `plan` found, for the Record tab
    struct Plan: Equatable {
        /// The next reminder's notebook, when a routine exists
        var next: String?
        /// Ask for permission (in context): a routine exists, the setting is on, and iOS was never asked
        var shouldAsk = false
    }

    private static func allowed(_ status: UNAuthorizationStatus) -> Bool {
        status == .authorized || status == .provisional || status == .ephemeral
    }

    private static func cancelAll(_ center: UNUserNotificationCenter) async {
        let ids = await center.pendingNotificationRequests().map(\.identifier).filter { $0.hasPrefix(idPrefix) }
        if !ids.isEmpty { center.removePendingNotificationRequests(withIdentifiers: ids) }
    }

    /// The request for one reminder: a calendar trigger, so it follows the wall clock.
    nonisolated static func request(_ reminder: Routine.Reminder, index: Int, calendar: Calendar) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = "Up next"
        content.body = body(reminder.notebook)
        content.categoryIdentifier = category
        content.userInfo = [notebookKey: reminder.notebook]
        content.sound = .default
        let parts = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: reminder.fireAt)
        return UNNotificationRequest(identifier: "\(idPrefix)\(index)", content: content,
                                     trigger: UNCalendarNotificationTrigger(dateMatching: parts, repeats: false))
    }

    /// The plan being made: the next one starts when it is done
    private static var planning: Task<Plan, Never>?
    private static var generation = 0

    /// Plan again from the saved recordings. Never asks for permission.
    /// One plan at a time, in the order asked: two plans that overlapped
    /// could leave an older one's reminders (or an older answer) in place.
    @discardableResult
    static func replan(context: ModelContext, now: Date = .now, calendar: Calendar = .current, excluding live: UUID? = nil) async -> Plan {
        let earlier = planning
        let task = Task { @MainActor in
            _ = await earlier?.value
            return await plan(context: context, now: now, calendar: calendar, excluding: live)
        }
        planning = task
        return await task.value
    }

    /// Plan, and tell the Record tab. Only the latest plan is told: an
    /// answer for an earlier state is dropped.
    static func replanAndPublish(context: ModelContext, excluding live: UUID? = nil) async {
        generation += 1
        let mine = generation
        let plan = await replan(context: context, excluding: live)
        if mine == generation { AppRouter.shared.reminderPlan = plan }
    }

    private static func plan(context: ModelContext, now: Date, calendar: Calendar, excluding live: UUID?) async -> Plan {
        let center = UNUserNotificationCenter.current()
        await cancelAll(center)
        guard isOn else { return Plan() }
        let reminders = Routine.reminders(Routine.recordings(in: context, now: now, excluding: live), now: now, calendar: calendar)
            // Not now hides a suggestion, and its reminder, until tomorrow
            .filter { !(calendar.isDate($0.fireAt, inSameDayAs: now) && UpNextDismissals.isHidden(UpNext.routineKey($0.notebook), now: now, calendar: calendar)) }
        guard let first = reminders.first else { return Plan() }
        let status = await center.notificationSettings().authorizationStatus
        guard allowed(status) else {
            let answered = UserDefaults.standard.bool(forKey: promptAnsweredKey)
            // Not right after Not now on a card, whichever notebook comes next: wait for another day
            let dismissedToday = UpNextDismissals.anyToday(now: now, calendar: calendar)
            return Plan(next: first.notebook, shouldAsk: status == .notDetermined && !answered && !dismissedToday)
        }
        for (i, r) in reminders.prefix(maxScheduled).enumerated() {
            try? await center.add(request(r, index: i, calendar: calendar))
        }
        return Plan(next: first.notebook)
    }

    /// The card's "Continue": the system prompt, then the plan.
    static func askThenPlan(context: ModelContext) async {
        UserDefaults.standard.set(true, forKey: promptAnsweredKey)
        _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])
        await replanAndPublish(context: context)
    }

    /// The card's "Not now": don't ask again (the Settings switch still works).
    static func declinePrompt() {
        UserDefaults.standard.set(true, forKey: promptAnsweredKey)
    }

    /// Settings: the switch changed. Turning it on lets the card ask again.
    /// Two quick taps end in the state of the last one: plans run one at a
    /// time and each reads the switch when it runs.
    static func settingChanged(on: Bool, context: ModelContext) async {
        if on { UserDefaults.standard.removeObject(forKey: promptAnsweredKey) }
        await replanAndPublish(context: context)
    }
}
