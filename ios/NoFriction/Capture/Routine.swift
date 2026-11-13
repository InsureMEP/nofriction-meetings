import CryptoKit
import Foundation
import SwiftData

// "It was already listening" (docs/design/FADELL_V2.md §3 A and §6 "The
// routine"): what to offer on the idle Record tab, learned from the
// recordings themselves. There is no schedule to set up, and nothing here
// leaves the device. Pure functions (unit-tested with fixed dates and time
// zones); the same rules as the Mac (`src-tauri/src/up_next.rs`).

/// The routine: a notebook the user records at this time on this weekday.
///
/// All times are local, in minutes of the day, with no wrap past midnight.
/// - Candidates for a notebook: its recordings that started in the last 42
///   days (1 to 42 days ago), on today's weekday, and not today.
/// - A candidate that started at `s` matches now when now is in
///   `[s - 10 min, s + 25 min]`.
/// - The notebook is suggested when at least 2 candidates match now and none
///   of its recordings started today.
/// - Between notebooks: more matching recordings, then the most recent recording.
/// - Usual type: the type of its most recent recording. Usual length: the
///   median length of the matching recordings in minutes, rounded up to
///   15 / 30 / 60 / 90; above 90, or unknown, means no limit.
/// - Names compare ignoring case; the most recent spelling is shown.
enum Routine {
    static let lookbackDays = 42
    static let minutesBefore = 10
    static let minutesAfter = 25
    static let minimumMatches = 2
    /// Reminders: starts within this many minutes of each other are one cluster
    static let clusterMinutes = 35
    static let reminderDays = 7

    /// A saved recording that has a notebook
    struct Recording: Equatable, Sendable {
        var notebook: String
        var kind: RecordingKind
        var startedAt: Date
        /// Whole minutes it ran; nil when it has no end
        var minutes: Int?

        init(notebook: String, kind: RecordingKind = .meeting, startedAt: Date, minutes: Int? = nil) {
            self.notebook = notebook
            self.kind = kind
            self.startedAt = startedAt
            self.minutes = minutes
        }
    }

    struct Suggestion: Equatable, Sendable {
        /// As most recently spelled
        var notebook: String
        var kind: RecordingKind
        var limit: RecordingLimit
        /// Recordings that match now
        var matches: Int
        /// The median start of the matching recordings, in minutes of the day ("around 9:05")
        var usualMinute: Int
    }

    /// A scheduled "usually starts about now"
    struct Reminder: Equatable, Sendable {
        var notebook: String
        var fireAt: Date
    }

    /// Minutes since local midnight
    static func minuteOfDay(_ date: Date, _ calendar: Calendar) -> Int {
        let c = calendar.dateComponents([.hour, .minute], from: date)
        return (c.hour ?? 0) * 60 + (c.minute ?? 0)
    }

    /// The middle value (the lower of the two middle ones for an even count), as on the Mac
    static func median(_ values: [Int]) -> Int? {
        let sorted = values.sorted()
        return sorted.isEmpty ? nil : sorted[(sorted.count - 1) / 2]
    }

    /// The recordings of each notebook (names ignoring case), oldest first
    private static func notebooks(_ recordings: [Recording]) -> [[Recording]] {
        Dictionary(grouping: recordings) { $0.notebook.trimmingCharacters(in: .whitespaces).lowercased() }
            .filter { !$0.key.isEmpty }
            .sorted { $0.key < $1.key }
            .map { $0.value.sorted { $0.startedAt < $1.startedAt } }
    }

    /// A notebook's recordings on `day`'s weekday, 1 to 42 days before it
    private static func candidates(_ group: [Recording], on day: Date, _ calendar: Calendar) -> [Recording] {
        let weekday = calendar.component(.weekday, from: day)
        let today = calendar.startOfDay(for: day)
        return group.filter {
            guard calendar.component(.weekday, from: $0.startedAt) == weekday,
                  let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: $0.startedAt), to: today).day else { return false }
            return (1...lookbackDays).contains(days)
        }
    }

    /// The median, rounded up to 15 / 30 / 60 / 90 minutes; above 90, or with no lengths, no limit
    /// (a recording is never cut short by a guess).
    static func usualLength(_ minutes: [Int]) -> RecordingLimit {
        guard let m = median(minutes.filter { $0 > 0 }) else { return .noLimit }
        for choice in [15, 30, 60, 90] where m <= choice { return .minutes(choice) }
        return .noLimit
    }

    /// The notebook to offer now; nil when nothing is usually recorded now.
    static func suggestion(_ recordings: [Recording], now: Date, calendar: Calendar = .current) -> Suggestion? {
        let minute = minuteOfDay(now, calendar)
        var best: (suggestion: Suggestion, latest: Date)?
        for group in notebooks(recordings) {
            // Already recorded today
            guard let latest = group.last, !group.contains(where: { calendar.isDate($0.startedAt, inSameDayAs: now) }) else { continue }
            let matching = candidates(group, on: now, calendar).filter {
                let s = minuteOfDay($0.startedAt, calendar)
                return minute >= s - minutesBefore && minute <= s + minutesAfter
            }
            guard matching.count >= minimumMatches else { continue }
            let found = Suggestion(notebook: latest.notebook.trimmingCharacters(in: .whitespaces), kind: latest.kind,
                                   limit: usualLength(matching.compactMap(\.minutes)), matches: matching.count,
                                   usualMinute: median(matching.map { minuteOfDay($0.startedAt, calendar) }) ?? minute)
            if let b = best, (b.suggestion.matches, b.latest) >= (found.matches, latest.startedAt) { continue }
            best = (found, latest.startedAt)
        }
        return best?.suggestion
    }

    /// Starts within 35 minutes of each other, at least two: (median start, the recordings)
    private static func clusters(_ candidates: [Recording], _ calendar: Calendar) -> [(minute: Int, recordings: [Recording])] {
        let sorted = candidates.map { (minuteOfDay($0.startedAt, calendar), $0) }.sorted { $0.0 < $1.0 }
        var out: [(Int, [Recording])] = []
        var current: [(Int, Recording)] = []
        func close() {
            guard current.count >= minimumMatches, let m = median(current.map(\.0)) else { return }
            out.append((m, current.map(\.1)))
        }
        for item in sorted {
            if let first = current.first, item.0 - first.0 > clusterMinutes {
                close()
                current = []
            }
            current.append(item)
        }
        close()
        return out
    }

    /// The reminders to schedule for the next `days` days: for each notebook
    /// and weekday, a cluster of at least two recordings whose starts are
    /// within 35 minutes of each other fires at its median start time.
    /// The 42 days are counted back from the day it would fire on, so a
    /// routine that will have faded by then isn't scheduled. Nothing for
    /// today once the notebook was recorded today.
    static func reminders(_ recordings: [Recording], now: Date, days: Int = reminderDays, calendar: Calendar = .current) -> [Reminder] {
        var out: [Reminder] = []
        let today = calendar.startOfDay(for: now)
        let past = recordings.filter { $0.startedAt <= now }
        for offset in 0..<max(0, days) {
            guard let day = calendar.date(byAdding: .day, value: offset, to: today) else { continue }
            for group in notebooks(past) {
                guard let latest = group.last else { continue }
                if offset == 0, group.contains(where: { calendar.isDate($0.startedAt, inSameDayAs: now) }) { continue }
                for cluster in clusters(candidates(group, on: day, calendar), calendar) {
                    guard let fireAt = calendar.date(bySettingHour: cluster.minute / 60, minute: cluster.minute % 60, second: 0, of: day),
                          fireAt > now else { continue }
                    out.append(Reminder(notebook: latest.notebook.trimmingCharacters(in: .whitespaces), fireAt: fireAt))
                }
            }
        }
        return out.sorted { ($0.fireAt, $0.notebook) < ($1.fireAt, $1.notebook) }
    }

    /// A reminder for `notebook` was tapped: its usual type and length on
    /// this weekday (the cluster nearest to now), so the card can show even
    /// when now is outside the strict window. nil when it was recorded today
    /// or has no cluster today.
    static func usual(_ notebook: String, _ recordings: [Recording], now: Date, calendar: Calendar = .current) -> Suggestion? {
        let key = notebook.trimmingCharacters(in: .whitespaces).lowercased()
        guard let group = notebooks(recordings).first(where: { $0.first?.notebook.trimmingCharacters(in: .whitespaces).lowercased() == key }),
              let latest = group.last, !group.contains(where: { calendar.isDate($0.startedAt, inSameDayAs: now) }) else { return nil }
        let minute = minuteOfDay(now, calendar)
        guard let cluster = clusters(candidates(group, on: now, calendar), calendar).min(by: { abs($0.minute - minute) < abs($1.minute - minute) }) else { return nil }
        return Suggestion(notebook: latest.notebook.trimmingCharacters(in: .whitespaces), kind: latest.kind,
                          limit: usualLength(cluster.recordings.compactMap(\.minutes)), matches: cluster.recordings.count, usualMinute: cluster.minute)
    }

    /// Whole minutes a recording ran. A recording that stopped at its
    /// 60-minute limit ends a second or two after the hour: it ran 60
    /// minutes, not 61 (rounded up, that second would make its usual length
    /// 90, then no limit).
    static func minutes(_ duration: TimeInterval?) -> Int? {
        guard let duration, duration > 0 else { return nil }
        return max(1, Int(duration / 60))
    }

    /// Saved recordings with a notebook from the last 43 days (not the one being recorded).
    @MainActor
    static func recordings(in context: ModelContext, now: Date = .now, excluding live: UUID? = nil) -> [Recording] {
        let from = now.addingTimeInterval(-Double(lookbackDays + 1) * 86_400)
        let found = (try? context.fetch(FetchDescriptor<Meeting>(predicate: #Predicate { $0.courseName != nil && $0.startedAt >= from }))) ?? []
        return found.compactMap { m in
            guard m.id != live, let name = Notebook.normalize(m.courseName) else { return nil }
            return Recording(notebook: name, kind: m.kind, startedAt: m.startedAt, minutes: minutes(m.duration))
        }
    }
}

/// The one "Up next" card on the idle Record tab: the calendar event that is
/// on now or starts within 15 minutes, else the routine. One button with the
/// answer on it; Not now hides that suggestion until the next local day.
struct UpNextSuggestion: Equatable, Sendable {
    enum Source: Equatable, Sendable { case calendar, routine }

    var source: Source
    /// On the button: "Record <name>"
    var name: String
    /// Why it is offered, in one line
    var detail: String
    var notebook: String?
    var kind: RecordingKind
    var limit: RecordingLimit
    /// What Not now hides
    var key: String
}

enum UpNext {
    /// How many of the events around now are looked at
    static let maxEvents = 6

    static func routineKey(_ notebook: String) -> String { "routine:" + notebook.lowercased() }
    static func calendarKey(_ eventID: String, start: Date) -> String { "calendar:\(eventID):\(Int(start.timeIntervalSince1970))" }

    /// A saved recording, as the calendar rules read it
    struct Recorded: Equatable, Sendable {
        var title: String
        var notebook: String?
        var kind: RecordingKind
        var startedAt: Date
        var calendarEventID: String?
    }

    /// A calendar event that is on now or about to start
    struct EventNow: Equatable, Sendable {
        var id: String
        var title: String
        var start: Date
        var end: Date
        /// It has other people on it or a call link, or it was recorded before under this title
        var meetingLike: Bool
        /// The notebook and type the last recording with this title had
        var notebook: String?
        var kind: RecordingKind?
        /// A recording of this event already exists
        var recorded: Bool
    }

    /// One event with what the recordings say about it.
    static func eventNow(_ event: CalendarEventInfo, recordings: [Recorded]) -> EventNow {
        let title = event.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let last = recordings.filter { $0.title.trimmingCharacters(in: .whitespacesAndNewlines).caseInsensitiveCompare(title) == .orderedSame }
            .max { $0.startedAt < $1.startedAt }
        let recorded = recordings.contains {
            $0.calendarEventID == event.id && $0.startedAt >= event.start.addingTimeInterval(-30 * 60) && $0.startedAt <= event.end
        }
        let others = event.participants.contains { !$0.isSelf }
        return EventNow(id: event.id, title: title, start: event.start, end: event.end,
                        meetingLike: others || event.meetingURL != nil || last != nil,
                        notebook: last.flatMap { Notebook.normalize($0.notebook) }, kind: last?.kind, recorded: recorded)
    }

    /// The calendar event to offer: of the events on now or about to start
    /// (`CalendarMatching.currentCandidates`, never an all-day one), the
    /// first that is a meeting (other people, a call link, or recorded
    /// before under that title) and has no recording yet.
    static func event(_ events: [CalendarEventInfo], recordings: [Recorded], now: Date) -> EventNow? {
        CalendarMatching.currentCandidates(events, now: now)
            .filter { !$0.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .prefix(maxEvents)
            .map { eventNow($0, recordings: recordings) }
            .first { $0.meetingLike && !$0.recorded }
    }

    private static let weekdays = ["Sundays", "Mondays", "Tuesdays", "Wednesdays", "Thursdays", "Fridays", "Saturdays"]

    /// "9:05 AM" in the calendar's time zone
    static func clock(_ date: Date, calendar: Calendar, locale: Locale) -> String {
        var style = Date.FormatStyle(date: .omitted, time: .shortened, locale: locale, calendar: calendar)
        style.timeZone = calendar.timeZone
        return date.formatted(style)
    }

    /// The one suggestion: the calendar event, then the routine. A hidden
    /// one (Not now, today) is passed over.
    static func choose(event: EventNow?, routine: Routine.Suggestion?, now: Date, calendar: Calendar = .current,
                       locale: Locale = .current, hidden: (String) -> Bool) -> UpNextSuggestion? {
        if let e = event, e.meetingLike, !e.recorded, e.end > now, !hidden(calendarKey(e.id, start: e.start)) {
            let detail: String
            if e.start > now {
                let minutes = max(1, Int((e.start.timeIntervalSince(now) + 59) / 60))
                detail = "On your calendar, starts in \(minutes) min"
            } else {
                detail = "On your calendar until \(clock(e.end, calendar: calendar, locale: locale))"
            }
            // It stops when the event is over (MeetingEndDetector), so no time limit
            return UpNextSuggestion(source: .calendar, name: e.title, detail: detail, notebook: e.notebook, kind: e.kind ?? .meeting,
                                    limit: .noLimit, key: calendarKey(e.id, start: e.start))
        }
        guard let r = routine, !hidden(routineKey(r.notebook)) else { return nil }
        let weekday = weekdays[(calendar.component(.weekday, from: now) - 1 + 7) % 7]
        let around = calendar.date(bySettingHour: r.usualMinute / 60, minute: r.usualMinute % 60, second: 0, of: now) ?? now
        return UpNextSuggestion(source: .routine, name: r.notebook,
                                detail: "You usually record this on \(weekday) around \(clock(around, calendar: calendar, locale: locale))",
                                notebook: r.notebook, kind: r.kind, limit: r.limit, key: routineKey(r.notebook))
    }

    /// How it will be recorded, as the card says it: "Class · 60 min · BIO 101".
    /// The answers to the Record sheet's three questions, already filled in.
    static func planSummary(_ s: UpNextSuggestion, stopsWhenOver: Bool) -> String {
        var parts = [s.kind.label, s.limit.minutes.map { "\($0) min" } ?? (stopsWhenOver ? "Stops when it's over" : "No limit")]
        if let notebook = s.notebook, notebook.caseInsensitiveCompare(s.name) != .orderedSame { parts.append(notebook) }
        return parts.joined(separator: " · ")
    }
}

/// **Not now**: a suggestion stays hidden until the next local day. Only a
/// hash of what was hidden and the day are kept (no notebook or event name
/// is stored a second time), and yesterday's entries are dropped.
enum UpNextDismissals {
    static let key = "upNextHidden"

    /// "2026-11-12", in the calendar's time zone
    static func day(_ date: Date, _ calendar: Calendar) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    static func hash(_ suggestionKey: String) -> String {
        SHA256.hash(data: Data(suggestionKey.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    static func isHidden(_ suggestionKey: String, now: Date, calendar: Calendar = .current, defaults: UserDefaults = .standard) -> Bool {
        (defaults.dictionary(forKey: key) as? [String: String])?[hash(suggestionKey)] == day(now, calendar)
    }

    /// Not now was tapped on a card today (any card)
    static func anyToday(now: Date, calendar: Calendar = .current, defaults: UserDefaults = .standard) -> Bool {
        ((defaults.dictionary(forKey: key) as? [String: String]) ?? [:]).values.contains(day(now, calendar))
    }

    static func hide(_ suggestionKeys: [String], now: Date, calendar: Calendar = .current, defaults: UserDefaults = .standard) {
        let today = day(now, calendar)
        var kept = ((defaults.dictionary(forKey: key) as? [String: String]) ?? [:]).filter { $0.value == today }
        for k in suggestionKeys { kept[hash(k)] = today }
        defaults.set(kept, forKey: key)
    }
}
