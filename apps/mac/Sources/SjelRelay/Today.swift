import Foundation

/// One calendar entry as `GET /calendar/api/entries` serves it (`CalendarEntry` in
/// `dashboard/src/lib/api.ts`). Only the fields the menu bar shows.
public struct CalendarEntry: Sendable, Equatable, Decodable, Identifiable {
    public let id: String
    public let title: String
    /// `yyyy-MM-dd` when `allDay`, else local `yyyy-MM-ddTHH:mm:ss`. An all-day end is
    /// exclusive: a one-day entry on the 8th ends on the 9th.
    public let startsAt: String
    public let endsAt: String
    public let allDay: Bool
    /// "possible", "planned" or "committed". Home ranks a possible entry as a decision.
    public let commitment: String

    enum CodingKeys: String, CodingKey {
        case id, title, commitment
        case startsAt = "starts_at"
        case endsAt = "ends_at"
        case allDay = "all_day"
    }

    public var isPossible: Bool { commitment == "possible" }
}

/// The calendar entries that touch today or tomorrow, in the order a person reads a day.
public enum Today {
    private static func formatter(_ format: String) -> DateFormatter {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = .current
        f.dateFormat = format
        return f
    }

    static let day = formatter("yyyy-MM-dd")
    static let time = formatter("yyyy-MM-dd'T'HH:mm:ss")

    public static func start(_ entry: CalendarEntry) -> Date? {
        (entry.allDay ? day : time).date(from: entry.startsAt)
    }

    public static func end(_ entry: CalendarEntry) -> Date? {
        (entry.allDay ? day : time).date(from: entry.endsAt)
    }

    /// The query range: today and tomorrow. `to` is inclusive: asking for the 8th to the
    /// 9th returned an entry starting on the 9th (2026-10-08).
    public static func range(now: Date, calendar: Calendar = .current) -> (from: String, to: String) {
        let today = calendar.startOfDay(for: now)
        let after = calendar.date(byAdding: .day, value: 1, to: today) ?? today
        return (day.string(from: today), day.string(from: after))
    }

    /// Entries not yet over, all-day first, then by start. A finished timed entry is gone:
    /// the strip is about what is still ahead.
    public static func ahead(_ entries: [CalendarEntry], now: Date) -> [CalendarEntry] {
        entries
            .filter { (end($0) ?? .distantFuture) > now }
            .sorted { a, b in
                if a.allDay != b.allDay { return a.allDay }
                return (start(a) ?? .distantPast) < (start(b) ?? .distantPast)
            }
    }

    /// What goes on the right of the row: the time, "until Oct 14" for a multi-day entry,
    /// or "tomorrow".
    public static func when(_ entry: CalendarEntry, now: Date, calendar: Calendar = .current) -> String {
        guard let start = start(entry) else { return "" }
        // The range is today and tomorrow, so a later day than `now` is tomorrow.
        let isTomorrow = start > now && !calendar.isDate(start, inSameDayAs: now)
        if entry.allDay {
            if let end = end(entry), let last = calendar.date(byAdding: .day, value: -1, to: end),
               !calendar.isDate(last, inSameDayAs: start), start <= now {
                return "until \(last.formatted(.dateTime.month(.abbreviated).day()))"
            }
            return isTomorrow ? "tomorrow" : "all day"
        }
        let clock = start.formatted(date: .omitted, time: .shortened)
        return isTomorrow ? "tomorrow \(clock)" : clock
    }
}
