import Foundation
import HealthKit

// Apple Health on the phone, read-only, sent to the Mac's `health` capability.
//
// A Mac has no HealthKit store, so this is the only live path. What lives here is what only
// Apple's API can do: the permission prompt, daily statistics (HealthKit merges a watch and a phone
// by the user's source order, which an export cannot), and the raw sleep samples. The night rule
// is the Mac's (`crates/health/src/export.rs`, `fold_sleep`), so the export and the phone agree.
//
// Rust asks (`sjel_ios_health_collect`) when the app comes to the foreground and a Mac is paired.
// This answers once with `sjel_ios_health_upload`, and Rust confirms a stored upload with
// `sjel_ios_health_finished`, which moves the next start forward or allows a retry.
final class HealthSync {
  static let shared = HealthSync()

  private let store = HKHealthStore()
  private let defaults = UserDefaults.standard
  private let syncedKey = "health.synced-through"
  private let attemptKey = "health.last-attempt"
  private let lastSyncedKey = "health.last-synced"
  private let statusKey = "health.sync-status"
  /// Days re-sent before the last synced one: today's totals grow until midnight, and a watch can
  /// deliver late.
  private let overlapDays = 2
  /// The first sync reaches this far back. Older days come from the Health app's export.
  private let firstSyncDays = 30
  /// Foreground events come often; one read every quarter hour is enough.
  private let minimumGap: TimeInterval = 15 * 60
  private var running = false

  private struct Quantity {
    let key: String
    let id: HKQuantityTypeIdentifier
    let unit: HKUnit
    let option: HKStatisticsOptions
    /// HealthKit gives a percentage as a fraction.
    var scale: Double = 1
  }

  private let bpm = HKUnit.count().unitDivided(by: .minute())

  private lazy var quantities: [Quantity] = [
    Quantity(key: "steps", id: .stepCount, unit: .count(), option: .cumulativeSum),
    Quantity(
      key: "distance_walking_km", id: .distanceWalkingRunning, unit: .meterUnit(with: .kilo),
      option: .cumulativeSum),
    Quantity(key: "flights_climbed", id: .flightsClimbed, unit: .count(), option: .cumulativeSum),
    Quantity(
      key: "active_energy_kcal", id: .activeEnergyBurned, unit: .kilocalorie(),
      option: .cumulativeSum),
    Quantity(key: "exercise_min", id: .appleExerciseTime, unit: .minute(), option: .cumulativeSum),
    Quantity(key: "resting_hr", id: .restingHeartRate, unit: bpm, option: .discreteAverage),
    Quantity(key: "walking_hr", id: .walkingHeartRateAverage, unit: bpm, option: .discreteAverage),
    Quantity(
      key: "hrv_sdnn_ms", id: .heartRateVariabilitySDNN, unit: .secondUnit(with: .milli),
      option: .discreteAverage),
    Quantity(key: "heart_rate_avg", id: .heartRate, unit: bpm, option: .discreteAverage),
    Quantity(key: "heart_rate_min", id: .heartRate, unit: bpm, option: .discreteMin),
    Quantity(key: "heart_rate_max", id: .heartRate, unit: bpm, option: .discreteMax),
    Quantity(
      key: "body_mass_kg", id: .bodyMass, unit: .gramUnit(with: .kilo), option: .mostRecent),
    Quantity(
      key: "body_fat_pct", id: .bodyFatPercentage, unit: .percent(), option: .mostRecent, scale: 100),
    Quantity(
      key: "lean_mass_kg", id: .leanBodyMass, unit: .gramUnit(with: .kilo), option: .mostRecent),
    Quantity(key: "bmi", id: .bodyMassIndex, unit: .count(), option: .mostRecent),
  ]

  private lazy var readTypes: Set<HKObjectType> = {
    var types = Set<HKObjectType>(quantities.map { HKQuantityType($0.id) })
    types.insert(HKCategoryType(.sleepAnalysis))
    return types
  }()

  private let day: DateFormatter = {
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.calendar = Calendar(identifier: .gregorian)
    formatter.timeZone = .current
    formatter.dateFormat = "yyyy-MM-dd"
    return formatter
  }()

  /// The export's timestamp form, so the Mac parses both with one reader.
  private let stamp: DateFormatter = {
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.calendar = Calendar(identifier: .gregorian)
    formatter.timeZone = .current
    formatter.dateFormat = "yyyy-MM-dd HH:mm:ss Z"
    return formatter
  }()

  func report(_ status: String? = nil) {
    let last = (defaults.object(forKey: lastSyncedKey) as? Date)?
      .formatted(date: .abbreviated, time: .shortened)
    let message = status ?? (running ? "Syncing…" : defaults.string(forKey: statusKey))
      ?? (last == nil ? "No upload confirmed yet." : "Upload confirmed by your Mac.")
    message.withCString { message in
      if let last {
        last.withCString { sjel_ios_health_state(message, $0, running) }
      } else {
        sjel_ios_health_state(message, nil, running)
      }
    }
  }

  func collect(manual: Bool) {
    guard !running else { return }
    guard HKHealthStore.isHealthDataAvailable() else {
      failed("Apple Health is unavailable on this device.")
      return
    }
    let now = Date()
    if !manual, let last = defaults.object(forKey: attemptKey) as? Date,
      now.timeIntervalSince(last) < minimumGap {
      return
    }
    running = true
    defaults.set(now, forKey: attemptKey)
    report("Checking Apple Health…")
    store.requestAuthorization(toShare: [], read: readTypes) { completed, _ in
      DispatchQueue.main.async {
        // A completed request does not tell us which read permissions were granted.
        guard completed else {
          self.failed("Could not request Apple Health access.")
          return
        }
        self.read(until: Date())
      }
    }
  }

  private func failed(_ message: String) {
    running = false
    defaults.removeObject(forKey: attemptKey)
    defaults.set(message, forKey: statusKey)
    report()
  }

  /// Only a Mac acknowledgement advances the date and records a successful sync.
  func finished(through: String?, error: String?) {
    if let error {
      failed(error)
      return
    }
    if let through, let date = day.date(from: through) {
      defaults.set(date, forKey: syncedKey)
    }
    running = false
    defaults.set(Date(), forKey: lastSyncedKey)
    defaults.removeObject(forKey: statusKey)
    report()
  }

  private func startDay(_ now: Date) -> Date {
    let calendar = Calendar.current
    let today = calendar.startOfDay(for: now)
    let earliest = calendar.date(byAdding: .day, value: -firstSyncDays, to: today)!
    guard let synced = defaults.object(forKey: syncedKey) as? Date else { return earliest }
    let back = calendar.date(byAdding: .day, value: -overlapDays, to: calendar.startOfDay(for: synced))!
    return max(earliest, back)
  }

  private func read(until now: Date) {
    let calendar = Calendar.current
    let from = startDay(now)
    let group = DispatchGroup()
    let lock = NSLock()
    var values: [[String: Any]] = []
    var sleep: [[String: String]] = []
    var readFailed = false

    func recordFailure(_ error: Error?) {
      guard let error, (error as? HKError)?.code != .errorNoData else { return }
      lock.lock()
      readFailed = true
      lock.unlock()
    }

    for quantity in quantities {
      group.enter()
      let query = HKStatisticsCollectionQuery(
        quantityType: HKQuantityType(quantity.id),
        quantitySamplePredicate: HKQuery.predicateForSamples(withStart: from, end: now),
        options: quantity.option,
        anchorDate: from,
        intervalComponents: DateComponents(day: 1))
      query.initialResultsHandler = { _, collection, error in
        defer { group.leave() }
        recordFailure(error)
        guard let collection else { return }
        var found: [[String: Any]] = []
        collection.enumerateStatistics(from: from, to: now) { statistics, _ in
          let quantityValue: HKQuantity?
          switch quantity.option {
          case .cumulativeSum: quantityValue = statistics.sumQuantity()
          case .discreteAverage: quantityValue = statistics.averageQuantity()
          case .discreteMin: quantityValue = statistics.minimumQuantity()
          case .discreteMax: quantityValue = statistics.maximumQuantity()
          default: quantityValue = statistics.mostRecentQuantity()
          }
          guard let quantityValue else { return }
          found.append([
            "metric": quantity.key,
            "day": self.day.string(from: statistics.startDate),
            "value": quantityValue.doubleValue(for: quantity.unit) * quantity.scale,
          ])
        }
        lock.lock()
        values.append(contentsOf: found)
        lock.unlock()
      }
      store.execute(query)
    }

    // Sleep from noon of the day before, so the first night arrives whole.
    group.enter()
    let sleepFrom = calendar.date(byAdding: .hour, value: -12, to: from)!
    let sleepQuery = HKSampleQuery(
      sampleType: HKCategoryType(.sleepAnalysis),
      predicate: HKQuery.predicateForSamples(withStart: sleepFrom, end: now),
      limit: HKObjectQueryNoLimit,
      sortDescriptors: nil
    ) { _, samples, error in
      defer { group.leave() }
      recordFailure(error)
      var found: [[String: String]] = []
      for case let sample as HKCategorySample in samples ?? [] {
        guard let stage = Self.stage(sample.value) else { continue }
        found.append([
          "source": sample.sourceRevision.source.name,
          "start": self.stamp.string(from: sample.startDate),
          "end": self.stamp.string(from: sample.endDate),
          "stage": stage,
        ])
      }
      lock.lock()
      sleep = found
      lock.unlock()
    }
    store.execute(sleepQuery)

    group.notify(queue: .main) {
      guard !values.isEmpty || !sleep.isEmpty else {
        self.failed(readFailed ? "Could not read Apple Health." : "No readable data from Apple Health.")
        return
      }
      let body: [String: Any] = [
        "source": "HealthKit",
        "from": self.day.string(from: from),
        "values": values,
        "sleep": sleep,
      ]
      guard let data = try? JSONSerialization.data(withJSONObject: body),
        let text = String(data: data, encoding: .utf8)
      else {
        self.failed("Could not prepare the Health upload.")
        return
      }
      self.report("Sending to your Mac…")
      text.withCString { sjel_ios_health_upload($0) }
    }
  }

  private static func stage(_ value: Int) -> String? {
    switch HKCategoryValueSleepAnalysis(rawValue: value) {
    case .inBed: return "InBed"
    case .awake: return "Awake"
    case .asleepCore: return "AsleepCore"
    case .asleepDeep: return "AsleepDeep"
    case .asleepREM: return "AsleepREM"
    case .asleepUnspecified: return "AsleepUnspecified"
    default: return nil
    }
  }
}

@_cdecl("sjel_ios_health_collect")
func sjelIosHealthCollect(_ manual: Bool) {
  DispatchQueue.main.async { HealthSync.shared.collect(manual: manual) }
}

@_cdecl("sjel_ios_health_finished")
func sjelIosHealthFinished(_ through: UnsafePointer<CChar>?, _ error: UnsafePointer<CChar>?) {
  let through = through.map { String(cString: $0) }
  let error = error.map { String(cString: $0) }
  DispatchQueue.main.async { HealthSync.shared.finished(through: through, error: error) }
}
