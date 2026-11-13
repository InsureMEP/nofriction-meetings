#if DEBUG
import SwiftData
import UIKit

/// Launch with -NFSeedDemo to fill an empty store with sample meetings
/// (App Store screenshots / layout checks); add -NFDemoLive to show a
/// meeting being recorded. `-NFOpenRecording "<title>"` opens that recording
/// on its Notes and `-NFOpenNotebook "<name>"` opens that notebook's page
/// (layout checks without tapping). Debug builds only.
///
/// Everything here is invented: people, companies (on the reserved
/// `.example` domain), links and conversations. No real personal data.
enum DemoData {
    static let liveTitle = "Q4 roadmap review"

    /// Today at 9:41, matching the status bar in App Store screenshots.
    static var anchor: Date {
        Calendar.current.date(bySettingHour: 9, minute: 41, second: 0, of: .now) ?? .now
    }

    private struct DemoPerson {
        let email: String
        let name: String
        let linkedIn: String?
    }

    private static let people: [DemoPerson] = [
        DemoPerson(email: "priya.shah@lumen-labs.example", name: "Priya Shah", linkedIn: "https://www.linkedin.com/in/nofriction-demo-priya"),
        DemoPerson(email: "marcus.lee@lumen-labs.example", name: "Marcus Lee", linkedIn: nil),
        DemoPerson(email: "dana.whitfield@brightwater.example", name: "Dana Whitfield", linkedIn: "https://www.linkedin.com/in/nofriction-demo-dana"),
        DemoPerson(email: "jonah.kim@brightwater.example", name: "Jonah Kim", linkedIn: nil),
        DemoPerson(email: "sofia.moreno@alder-studio.example", name: "Sofia Moreno", linkedIn: "https://www.linkedin.com/in/nofriction-demo-sofia"),
        DemoPerson(email: "alex.rivera@lumen-labs.example", name: "Alex Rivera", linkedIn: nil),   // "you"
    ]
    static let selfIndex = 5

    @MainActor
    static func seedIfRequested(_ context: ModelContext) {
        // -NFResetDemo: start from a clean store (UI tests that edit the sample meetings)
        if ProcessInfo.processInfo.arguments.contains("-NFResetDemo") {
            for m in (try? context.fetch(FetchDescriptor<Meeting>())) ?? [] { context.delete(m) }
            for p in (try? context.fetch(FetchDescriptor<Person>())) ?? [] { context.delete(p) }
            for t in (try? context.fetch(FetchDescriptor<ChatThread>())) ?? [] { context.delete(t) }
            try? context.save()
            // Up next as on a first day: nothing hidden by an earlier Not now
            UserDefaults.standard.removeObject(forKey: UpNextDismissals.key)
            UserDefaults.standard.removeObject(forKey: RoutineReminders.promptAnsweredKey)
        }
        guard ProcessInfo.processInfo.arguments.contains("-NFSeedDemo"),
              ((try? context.fetchCount(FetchDescriptor<Meeting>())) ?? 0) == 0 else { return }

        let day: TimeInterval = 86_400
        let a = anchor

        // Yesterday: a customer call with AI notes, a photo and a stricken line
        let brightwater = meeting(
            "Brightwater pilot kickoff", start: a.addingTimeInterval(-day + 4 * 3600 + 19 * 60), minutes: 34,
            lines: [
                "Thanks for making the time, Dana. Can you walk us through how inspections work today?",
                "Sure. Our inspectors fill in paper checklists on site, and someone retypes them at the end of the week.",
                "So a report can be a week old before a client sees it.",
                "Exactly. And when a client asks what was said on site, we're digging through notebooks.",
                "STRIKE",
                "Okay. For the pilot, let's start with five inspectors and see how the summaries land with clients.",
                "Jonah, could you send over the three report templates you use most?",
                "Will do. I'll have them to you by Wednesday.",
                "Great. We'll set up a shared folder for feedback and check in two weeks after launch.",
                "One more thing: can inspectors add photos of the site to the summary?",
                "Yes. Snap a photo during the visit and it lands in the timeline next to what was said.",
                "Perfect. That covers most of what our clients ask for.",
                "Then let's lock October 14 for the start. I'll send the agreement by Friday.",
                "Sounds good. Thanks, everyone.",
            ],
            spacing: 75, attendees: [2, 3, selfIndex], organizer: selfIndex, in: context)
        brightwater.aiNotes = """
        **Summary**
        Brightwater's inspectors use paper checklists that are retyped weekly, so client reports lag by up to a week. They want searchable site notes and a one-page summary per visit.

        **Decisions**
        • Pilot with five inspectors, starting October 14
        • Share visit summaries as PDF

        **Action items**
        • Alex: send the pilot agreement by Friday
        • Jonah: share the three most-used report templates by Wednesday
        • Dana: choose the five pilot inspectors
        """
        brightwater.aiNotesAt = brightwater.endedAt
        brightwater.kind = .meeting
        brightwater.courseName = "Brightwater pilot"
        if let line = brightwater.orderedSegments.first(where: { $0.text == "STRIKE" }) {
            let r = Redaction(kind: .line, action: .strike, mediaStart: 56, mediaEnd: 68,
                              coveredFrom: line.start, coveredTo: line.start.addingTimeInterval(12),
                              reason: "pricing under NDA", createdAt: a.addingTimeInterval(-day + 6 * 3600))
            context.insert(r)
            r.meeting = brightwater
            line.text = RedactionText.markerToken(r.id)
        }
        addSlide(to: brightwater, at: brightwater.startedAt.addingTimeInterval(9 * 60), in: context)

        // Earlier this week
        meeting("Design critique: onboarding", start: a.addingTimeInterval(-2 * day + 5 * 3600 + 19 * 60), minutes: 45,
                lines: [
                    "Let's start with the permission screens. Sofia, what did people say in testing?",
                    "They liked seeing why each permission is needed, right next to the button.",
                    "Can we cut the welcome down to one screen?",
                    "I'd keep the consent step. It's the one place we explain recording laws.",
                ],
                attendees: [4, 1, selfIndex], organizer: 4, in: context)
        meeting("Weekly sync with Marcus", start: a.addingTimeInterval(-3 * day + 49 * 60), minutes: 25,
                lines: [
                    "Quick one today. The Android build is green again.",
                    "Nice. Anything blocking the beta?",
                    "Just the store listing. Screenshots and the privacy answers.",
                ],
                attendees: [1, selfIndex], organizer: selfIndex, in: context)

        // Today: a class in its notebook, with marked moments and lecture notes
        let lecture = meeting("Lecture 7: Cellular respiration", start: a.addingTimeInterval(-100 * 60), minutes: 50,
                              lines: [
                                  "Today we follow a glucose molecule all the way to ATP.",
                                  "Glycolysis happens in the cytoplasm and doesn't need oxygen.",
                                  "It splits glucose into two pyruvate molecules and nets two ATP.",
                                  "The Krebs cycle runs in the mitochondrial matrix.",
                                  "This will be on the exam: the electron transport chain makes most of the ATP.",
                                  "Oxygen is the final electron acceptor; without it the chain stops.",
                                  "Problem set four is due next Thursday.",
                              ],
                              spacing: 6 * 60, attendees: [], organizer: -1, calendar: false, in: context)
        lecture.kind = .class
        lecture.courseName = "BIO 101"
        lecture.plannedMinutes = 60
        lecture.aiNotes = """
        ## Summary
        How cells turn glucose into ATP: glycolysis, the Krebs cycle and the electron transport chain.

        ## Key concepts
        - Glycolysis splits glucose into two pyruvate in the cytoplasm (no oxygen needed)
        - The Krebs cycle runs in the mitochondrial matrix
        - The electron transport chain makes most of the ATP

        ## Definitions
        - **Final electron acceptor**: oxygen; without it the chain stops

        ## Examples
        None.

        ## Announcements and deadlines
        - Problem set four is due next Thursday
        """
        lecture.aiNotesAt = lecture.endedAt
        for (minute, kind, note) in [(13.0, MarkerKind.important, nil as String?), (25.0, .test, nil), (31.0, .question, "why oxygen?")] {
            let marker = MomentMarker(at: lecture.startedAt.addingTimeInterval(minute * 60), kind: kind, note: note)
            context.insert(marker)
            marker.meeting = lecture
        }

        // Personal: an appointment, in its own notebook
        let physio = meeting("Physio check-in", start: a.addingTimeInterval(-3 * day + 8 * 3600), minutes: 20,
                             lines: [
                                 "Your shoulder is moving much better than two weeks ago.",
                                 "Keep the band exercises, three sets of ten, morning and evening.",
                                 "Ice it after the long runs, not before.",
                                 "Book the next visit for the end of the month.",
                             ],
                             spacing: 4 * 60, attendees: [], organizer: -1, calendar: false, in: context)
        physio.kind = .personal
        physio.courseName = "Health"
        physio.aiNotes = """
        ## Summary
        Shoulder check-up: good progress since the last visit.

        ## Key points
        - Range of motion is much better than two weeks ago
        - Ice after long runs, not before

        ## To-dos and reminders
        - Band exercises: three sets of ten, morning and evening
        - Book the next visit for the end of the month
        """
        physio.aiNotesAt = physio.endedAt
        let remember = MomentMarker(at: physio.startedAt.addingTimeInterval(4 * 60 + 30), kind: .test)
        context.insert(remember)
        remember.meeting = physio
        try? context.save()
        // -NFFilm: more lectures and a stored study guide (FilmDemo)
        FilmDemo.seed(context)
        // v2: typed notes, "What you wrote", things due, study guides and a routine
        seedV2(context)
    }

    /// v2 samples (docs/design/FADELL_V2.md), not in film runs (the film has
    /// its own BIO 101 lectures): two earlier BIO 101 lectures with typed
    /// notes, notes that open with "What you wrote", things due, ✎ and ?
    /// marks and stored study guides (the notebook page has something in
    /// every section), and a notebook recorded at this time on this weekday
    /// for three weeks, so the Record tab shows its Up next card.
    @MainActor
    private static func seedV2(_ context: ModelContext) {
        guard !FilmDemo.isOn else { return }
        let day: TimeInterval = 86_400
        let a = anchor
        func lecture(_ title: String, daysAgo: Double, lines: [String]) -> Meeting {
            let m = meeting(title, start: a.addingTimeInterval(-daysAgo * day - 100 * 60), minutes: 50, lines: lines,
                            spacing: 6 * 60, attendees: [], organizer: -1, calendar: false, in: context)
            m.kind = .class
            m.courseName = "BIO 101"
            m.plannedMinutes = 60
            return m
        }
        func mark(_ m: Meeting, minute: Double, _ kind: MarkerKind, _ note: String?) {
            let marker = MomentMarker(at: m.startedAt.addingTimeInterval(minute * 60), kind: kind, note: note)
            context.insert(marker)
            marker.meeting = m
        }
        func guide(_ m: Meeting, cards: [StudyCards.Card], quiz: [StudyQuiz.Item]) {
            let encoder = JSONEncoder()
            func json<T: Encodable>(_ value: T) -> String { (try? encoder.encode(value)).map { String(decoding: $0, as: UTF8.self) } ?? "{}" }
            let fingerprint = StudyInput(meeting: m).fingerprint
            for (kind, body) in [(StudyKind.flashcards, json(StudyCards(cards: cards))), (.quiz, json(StudyQuiz(questions: quiz)))] {
                let material = StudyMaterial(kind: kind, json: body, transcriptFingerprint: fingerprint,
                                             createdAt: (m.endedAt ?? m.startedAt).addingTimeInterval(120))
                context.insert(material)
                material.meeting = m
            }
        }

        let enzymes = lecture("Lecture 6: Enzymes", daysAgo: 5, lines: [
            "Enzymes speed up reactions without being used up.",
            "Each one fits its substrate at the active site, like a key in a lock.",
            "A competitive inhibitor sits in the active site and blocks the substrate.",
            "A non-competitive inhibitor binds somewhere else and changes the enzyme's shape.",
            "Heat does the same thing: the shape changes, and the enzyme stops working.",
            "pH matters too. Each enzyme has a range where it works best.",
            "Lab report two is due Monday. Use the template on the course page, and read chapter 8 before Thursday.",
        ])
        mark(enzymes, minute: 7.5, .note, "lock and key = active site?")
        mark(enzymes, minute: 15, .test, "competitive vs non-competitive inhibitors")
        mark(enzymes, minute: 25.5, .note, "ask why heat stops it")
        mark(enzymes, minute: 32, .question, "does pH work the same way as heat?")
        mark(enzymes, minute: 37.5, .note, "lab report: use the template")
        enzymes.aiNotes = """
        ## What you wrote
        - **lock and key = active site?**: Yes. Each enzyme fits its substrate at the active site, the way a key fits one lock.
        - **ask why heat stops it**: Heat changes the enzyme's shape, so the substrate no longer fits the active site and the reaction stops.
        - **lab report: use the template**: Lab report two has to follow the template on the course page.

        ## Summary
        How enzymes speed up reactions, what the active site does, and what stops an enzyme from working.

        ## Key concepts
        - Enzymes speed up reactions without being used up
        - Each enzyme fits its substrate at the active site
        - Heat and pH change the enzyme's shape

        ## Definitions
        - **Active site**: where the substrate binds to the enzyme
        - **Inhibitor**: a molecule that slows an enzyme down

        ## Examples
        - A competitive inhibitor blocks the active site; a non-competitive one changes the enzyme's shape

        ## Announcements and deadlines
        - Lab report two is due Monday
        - Read chapter 8 before Thursday's lecture
        """
        enzymes.aiNotesAt = enzymes.endedAt
        guide(enzymes, cards: [
            .init(front: "What does an enzyme do?", back: "It speeds up a reaction without being used up."),
            .init(front: "Where does the substrate bind?", back: "At the active site."),
            .init(front: "What does heat do to an enzyme?", back: "It changes the enzyme's shape, so it stops working."),
        ], quiz: [
            .init(question: "Where does a competitive inhibitor bind?",
                  choices: ["The active site", "The cell membrane", "The nucleus", "The substrate"], answer: 0,
                  explanation: "It sits in the active site and blocks the substrate.", atMs: (2 * 6 * 60 + 20) * 1000),
            .init(question: "Why does an enzyme stop working when it gets too hot?",
                  choices: ["It is used up", "Its shape changes", "It splits in two", "It turns into its substrate"], answer: 1,
                  explanation: "Heat changes the shape, so the substrate no longer fits.", atMs: (4 * 6 * 60 + 20) * 1000),
        ])

        let membranes = lecture("Lecture 5: Cell membranes", daysAgo: 12, lines: [
            "The membrane is a double layer of phospholipids.",
            "Small, uncharged molecules slip through on their own.",
            "Everything else needs a channel or a pump.",
            "Quiz one is next Tuesday, on chapters 5 and 6.",
        ])
        mark(membranes, minute: 13, .test, "what crosses on its own, what needs a pump")
        membranes.aiNotes = """
        ## Summary
        What the cell membrane is made of, and how molecules get across it.

        ## Key concepts
        - The membrane is a double layer of phospholipids
        - Small, uncharged molecules cross on their own; everything else needs a channel or a pump

        ## Definitions
        None.

        ## Examples
        None.

        ## Announcements and deadlines
        - Quiz one is next Tuesday, on chapters 5 and 6
        """
        membranes.aiNotesAt = membranes.endedAt
        guide(membranes, cards: [
            .init(front: "What is the cell membrane made of?", back: "A double layer of phospholipids."),
            .init(front: "Which molecules cross the membrane on their own?", back: "Small, uncharged ones."),
        ], quiz: [
            .init(question: "What does a charged molecule need to cross the membrane?",
                  choices: ["Nothing", "A channel or a pump", "Heat", "An enzyme"], answer: 1,
                  explanation: "Only small, uncharged molecules slip through on their own.", atMs: (2 * 6 * 60 + 20) * 1000),
        ])

        // A routine: CHEM 201 at about this time on this weekday, three weeks running
        let now = Date.now
        for (weeks, minutes) in [(1, -4.0), (2, 3.0), (3, -1.0)] {
            guard let sameTime = Calendar.current.date(byAdding: .day, value: -7 * weeks, to: now) else { continue }
            let start = sameTime.addingTimeInterval(minutes * 60)
            let m = meeting(RecordingSession.notebookTitle("CHEM 201", at: start), start: start, minutes: 50, lines: [
                "Today we balance redox reactions in acidic solution.",
                "Split the reaction into two half-reactions first.",
                "Balance the atoms, then the charge, then add them back together.",
            ], spacing: 9 * 60, attendees: [], organizer: -1, calendar: false, in: context)
            m.kind = .class
            m.courseName = "CHEM 201"
            m.plannedMinutes = 60
            // What its notes still have open shows under the Up next card
            if weeks == 1 {
                m.aiNotes = """
                ## Summary
                Balancing redox reactions in acidic solution with half-reactions.

                ## Key concepts
                - Split the reaction into two half-reactions
                - Balance the atoms, then the charge

                ## Definitions
                None.

                ## Examples
                None.

                ## Announcements and deadlines
                - Problem set 6 is due Friday
                - Midterm two covers chapters 9 to 12
                """
                m.aiNotesAt = m.endedAt
            }
        }
        try? context.save()
    }

    /// -NFOpenRecording "<title>" / -NFOpenNotebook "<name>": straight to a
    /// recording's Notes or a notebook's page.
    @MainActor
    static func openIfRequested(router: AppRouter, context: ModelContext) {
        let args = ProcessInfo.processInfo.arguments
        func value(_ flag: String) -> String? {
            guard let i = args.firstIndex(of: flag), args.indices.contains(i + 1) else { return nil }
            return args[i + 1]
        }
        if let name = value("-NFOpenNotebook") { router.open(notebook: name) }
        if let title = value("-NFOpenRecording"),
           let m = try? context.fetch(FetchDescriptor<Meeting>(predicate: #Predicate { $0.title == title })).first {
            router.open(recording: m.id)
        }
    }

    /// -NFSeedWatchDemo: two Apple Watch recordings, one transcribed and one
    /// still being transcribed (UI checks of the watch badge and import state).
    @MainActor
    static func seedWatchDemoIfRequested(_ context: ModelContext) {
        guard ProcessInfo.processInfo.arguments.contains("-NFSeedWatchDemo") else { return }
        let watch = Meeting.Source.watch
        let existing = (try? context.fetchCount(FetchDescriptor<Meeting>(predicate: #Predicate { $0.source == watch }))) ?? 0
        guard existing == 0 else { return }
        let a = anchor
        let done = meeting("Site walk with Dana", start: a.addingTimeInterval(-5 * 3600), minutes: 22,
                           lines: [
                               "This is the north stairwell. The fire door closes, but slowly.",
                               "Let's log that with a photo for the report.",
                               "And the exit sign on level two is out.",
                           ],
                           spacing: 40, attendees: [2, selfIndex], organizer: selfIndex, in: context)
        done.source = watch
        done.sourceRecordingID = UUID().uuidString
        let pending = Meeting(title: "Hallway chat", startedAt: a.addingTimeInterval(-40 * 60))
        pending.endedAt = pending.startedAt.addingTimeInterval(18 * 60)
        pending.source = watch
        pending.sourceRecordingID = UUID().uuidString
        pending.importState = Meeting.ImportState.transcribing.rawValue
        pending.audioDuration = 18 * 60
        pending.importProgress = 7 * 60
        context.insert(pending)
        try? context.save()
    }

    /// -NFDemoLive: a meeting in progress with a live transcript.
    @MainActor
    static func showLiveMeeting(session: RecordingSession, context: ModelContext) {
        // Film footage: a live class, or this meeting with its lines arriving one by one
        if FilmDemo.liveClass { return FilmDemo.showLiveClass(session: session, context: context) }
        if FilmDemo.liveFeed { return FilmDemo.showLiveMeetingFeed(session: session, context: context) }
        let title = liveTitle
        let existing = try? context.fetch(FetchDescriptor<Meeting>(predicate: #Predicate { $0.title == title && $0.endedAt == nil })).first
        let start = anchor.addingTimeInterval(-25 * 60)   // 9:16
        let m = existing ?? meeting(
            liveTitle, start: start, minutes: nil,
            lines: [
                "Okay, let's get started. The goal today is to lock the Q4 roadmap and agree on owners.",
                "Priya, do you want to start with where the beta landed?",
                "Sure. Retention is up eleven percent since offline mode shipped, and support tickets are down by a third.",
                "That's great. What's behind the drop in tickets?",
                "Mostly the new onboarding. People find calendar sync on their own now.",
                "Marcus, where are we on the Android timeline?",
                "Design is done for the recording screen. The open question is whether calendar matching ships in the first beta.",
                "I'd ship it. It's the thing people mention first in interviews.",
                "Agreed. Sofia, can the onboarding cover the calendar permission too?",
                "Yes, it's already in the new flow. One screen, with the reason right next to the button.",
            ],
            spacing: 2 * 60,
            attendees: [0, 1, 4, selfIndex], organizer: 0, in: context)
        m.scheduledStart = anchor.addingTimeInterval(-26 * 60)  // 9:15
        m.scheduledEnd = anchor.addingTimeInterval(19 * 60)     // 10:00
        // A line typed while it was said ("Type a note"), where it was written
        if m.typedNotes.isEmpty {
            let note = MomentMarker(at: start.addingTimeInterval(19 * 60), kind: .note, note: "get the new onboarding flow from Sofia")
            context.insert(note)
            note.meeting = m
        }
        try? context.save()
        session.showDemo(meeting: m, startedAt: Date.now.addingTimeInterval(-25 * 60 - 12),
                         partial: "We can have a beta in six weeks if we keep the scope to recording and")
    }

    @MainActor @discardableResult
    static func meeting(_ title: String, start: Date, minutes: Double?, lines: [String], spacing: TimeInterval = 14,
                                attendees: [Int], organizer: Int, calendar: Bool = true, in context: ModelContext) -> Meeting {
        let m = Meeting(title: title, startedAt: start)
        m.endedAt = minutes.map { start.addingTimeInterval($0 * 60) }
        if calendar {
            m.scheduledStart = start
            m.scheduledEnd = m.endedAt
            m.meetingURL = "https://meet.example.com/q4-review"
            m.calendarEventID = UUID().uuidString
        }
        context.insert(m)
        for (i, line) in lines.enumerated() {
            let s = Segment(text: line, start: start.addingTimeInterval(Double(i) * spacing + 20), duration: 12)
            context.insert(s)
            s.meeting = m
        }
        for i in attendees {
            let d = people[i]
            let p = MeetingLinker.findOrCreatePerson(email: d.email, in: context)
            p.name = d.name
            p.company = PersonNames.company(fromEmail: d.email)
            p.linkedinURL = p.linkedinURL ?? d.linkedIn
            p.isSelf = i == selfIndex
            let a = Attendance(role: i == organizer ? "organizer" : "attendee")
            context.insert(a)
            a.meeting = m
            a.person = p
        }
        return m
    }

    /// A generated "slide" photo, so the Photos section isn't empty.
    @MainActor
    private static func addSlide(to m: Meeting, at time: Date, in context: ModelContext) {
        let size = CGSize(width: 1200, height: 900)
        let image = UIGraphicsImageRenderer(size: size).image { ctx in
            UIColor(red: 0.96, green: 0.95, blue: 0.92, alpha: 1).setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            UIColor(red: 0.98, green: 0.80, blue: 0.08, alpha: 1).setFill()
            ctx.fill(CGRect(x: 90, y: 120, width: 120, height: 14))
            let title: [NSAttributedString.Key: Any] = [.font: UIFont.systemFont(ofSize: 76, weight: .bold), .foregroundColor: UIColor(white: 0.1, alpha: 1)]
            ("Pilot plan" as NSString).draw(at: CGPoint(x: 90, y: 170), withAttributes: title)
            let body: [NSAttributedString.Key: Any] = [.font: UIFont.systemFont(ofSize: 44, weight: .regular), .foregroundColor: UIColor(white: 0.25, alpha: 1)]
            for (i, line) in ["•  5 inspectors, 6 weeks", "•  Summary per site visit", "•  Check-in after 2 weeks"].enumerated() {
                (line as NSString).draw(at: CGPoint(x: 96, y: 330 + CGFloat(i) * 92), withAttributes: body)
            }
        }
        guard let data = image.jpegData(compressionQuality: 0.85) else { return }
        let name = "demo-slide-\(UUID().uuidString).jpg"
        guard (try? data.write(to: Storage.snapshots.appending(path: name), options: .atomic)) != nil else { return }
        let s = Snapshot(fileName: name, takenAt: time)
        context.insert(s)
        s.meeting = m
    }
}
#endif
