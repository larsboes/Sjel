import Foundation
import UIKit
import UserNotifications

// The phone's lock screen: the system's notification centre, and the one list Rust hands it.
//
// The rule that decides what is worth announcing is platform-free (`sjel_gpui_core::notify`); what
// lives here is what only Apple's API can do — the permission prompt, the requests themselves, and
// the count the card shows. `ios/src/notify.rs` decides, this schedules, and the card's number comes
// from `getPendingNotificationRequests` rather than from Rust's memory of what it asked for: a line
// that says "4 reminders" is the system's answer.
//
// Local only. Nothing here talks to a push server: there is no `registerForRemoteNotifications`, no
// device token and no server that would need one. A reminder is the phone reading its own replica
// and setting an alarm in itself.
final class Reminders: NSObject, UNUserNotificationCenterDelegate {
  static let shared = Reminders()

  /// False until Rust's view exists (`sjel_ios_notifications_ready`). A tap can arrive before it:
  /// iOS launches the app because of the tap, so the response lands while the window is still being
  /// built, and a link sent then would be dropped by an event channel with no reader yet.
  private var rustReady = false
  private var deferred: String?

  /// Called once from the host's `viewDidLoad`: the delegate, so a reminder is visible while the app
  /// is open, and one state report, so the card can say what the last launch left behind.
  func configure() {
    UNUserNotificationCenter.current().delegate = self
    report()
  }

  /// Rust's view is up: flush any tap that arrived first, then report what the system holds.
  func ready() {
    rustReady = true
    if let link = deferred {
      deferred = nil
      announce(link: link)
    }
    report()
  }

  /// Asks for the permission, once. iOS offers this prompt one time in the app's life; a person who
  /// refuses changes it in Settings, which is why the card says where.
  func authorize() {
    UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) {
      allowed, _ in
      // `allowed` is not what is reported: `getNotificationSettings` is the system's word for the
      // whole app, including a change made in Settings while the app was away.
      self.report()
    }
  }

  /// Holds exactly the reminders in the list, and nothing else.
  ///
  /// The identifiers are stable (`cal:<entry id>`, `task:<path>`), so adding one the centre already
  /// holds replaces it in place, and the ones the list no longer names are cancelled — an entry
  /// deleted on the phone, a task ticked off at the Mac and pulled back.
  func schedule(_ json: String) {
    struct Body: Decodable { let reminders: [Item] }
    struct Item: Decodable {
      let id: String
      let title: String
      let body: String
      let at: Double
      let link: String
    }
    guard let data = json.data(using: .utf8),
      let body = try? JSONDecoder().decode(Body.self, from: data)
    else {
      report()
      return
    }
    let center = UNUserNotificationCenter.current()
    let keep = Set(body.reminders.map(\.id))
    center.getPendingNotificationRequests { pending in
      let stale = pending.map(\.identifier).filter { !keep.contains($0) }
      if !stale.isEmpty {
        center.removePendingNotificationRequests(withIdentifiers: stale)
      }
      let group = DispatchGroup()
      for item in body.reminders {
        group.enter()
        let content = UNMutableNotificationContent()
        content.title = item.title
        content.body = item.body
        content.sound = .default
        content.userInfo = ["link": item.link, "id": item.id]
        // A wall-clock moment rather than a countdown: a request must still be right after the phone
        // has been asleep for a day, and an interval started at scheduling time is not.
        let moment = Date(timeIntervalSince1970: item.at)
        let parts = Calendar.current.dateComponents(
          [.year, .month, .day, .hour, .minute], from: moment)
        let trigger = UNCalendarNotificationTrigger(dateMatching: parts, repeats: false)
        let request = UNNotificationRequest(
          identifier: item.id, content: content, trigger: trigger)
        center.add(request) { _ in group.leave() }
      }
      group.notify(queue: .main) { self.report() }
    }
  }

  /// Takes every request away: what turning reminders off means. The permission stays as it is,
  /// because revoking it is not this app's to do.
  func clear() {
    UNUserNotificationCenter.current().removeAllPendingNotificationRequests()
    report()
  }

  /// Reports what the system holds, for the card's line. Called after every ask above.
  func report() {
    let center = UNUserNotificationCenter.current()
    center.getNotificationSettings { settings in
      let authorization: String
      switch settings.authorizationStatus {
      case .authorized, .provisional: authorization = "allowed"
      case .denied: authorization = "denied"
      case .notDetermined, .ephemeral: authorization = "notasked"
      @unknown default: authorization = "notasked"
      }
      center.getPendingNotificationRequests { requests in
        var payload: [String: Any] = [
          "authorization": authorization,
          "scheduled": requests.count,
        ]
        let next =
          requests
          .compactMap { request -> (Date, UNNotificationRequest)? in
            guard let trigger = request.trigger as? UNCalendarNotificationTrigger,
              let date = trigger.nextTriggerDate()
            else { return nil }
            return (date, request)
          }
          .min { $0.0 < $1.0 }
        if let (date, request) = next {
          payload["next"] = [
            "at": Int(date.timeIntervalSince1970),
            "title": request.content.title,
          ]
        }
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
          let text = String(data: data, encoding: .utf8)
        else { return }
        DispatchQueue.main.async {
          text.withCString { sjel_ios_notifications_state($0) }
        }
      }
    }
  }

  /// Opens this app's page in iOS Settings. The second ask for a permission does not exist, so a
  /// refusal's way back is here rather than in another prompt.
  func openSettings() {
    guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
    UIApplication.shared.open(url)
  }

  /// Tells Rust where a tap goes, or keeps it until Rust can hear it.
  private func announce(link: String) {
    guard rustReady else {
      deferred = link
      return
    }
    let payload = ["link": link]
    guard let data = try? JSONSerialization.data(withJSONObject: payload),
      let text = String(data: data, encoding: .utf8)
    else { return }
    DispatchQueue.main.async {
      text.withCString { sjel_ios_notification_opened($0) }
    }
  }

  // MARK: - UNUserNotificationCenterDelegate

  /// A reminder that arrives while the app is open is shown as a banner, the way it would be on the
  /// lock screen. The default is silence, which reads as a reminder that never came.
  func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    willPresent notification: UNNotification,
    withCompletionHandler completionHandler:
      @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    completionHandler([.banner, .list, .sound])
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    if let link = response.notification.request.content.userInfo["link"] as? String {
      announce(link: link)
    }
    completionHandler()
  }
}

/// `ios/src/notify.rs` asks the system for the permission once the person turns reminders on.
@_cdecl("sjel_ios_notifications_authorize")
public func sjelIOSNotificationsAuthorize() {
  Reminders.shared.authorize()
}

/// `ios/src/notify.rs` hands over the whole list to hold, as JSON.
@_cdecl("sjel_ios_notifications_schedule")
public func sjelIOSNotificationsSchedule(_ json: UnsafePointer<CChar>?) {
  guard let json else { return }
  Reminders.shared.schedule(String(cString: json))
}

/// `ios/src/notify.rs` takes every request away: reminders were turned off.
@_cdecl("sjel_ios_notifications_clear")
public func sjelIOSNotificationsClear() {
  Reminders.shared.clear()
}

/// `ios/src/notify.rs` asks what the system holds, for the line under the switch.
@_cdecl("sjel_ios_notifications_report")
public func sjelIOSNotificationsReport() {
  Reminders.shared.report()
}

/// `ios/src/notify.rs`'s view exists: a tap that arrived first may now be delivered.
@_cdecl("sjel_ios_notifications_ready")
public func sjelIOSNotificationsReady() {
  Reminders.shared.ready()
}

/// `ios/src/notify.rs` sends the person to this app's page in iOS Settings after a refusal.
@_cdecl("sjel_ios_open_settings")
public func sjelIOSOpenSettings() {
  Reminders.shared.openSettings()
}
