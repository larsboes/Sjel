import Foundation
import FoundationModels
import Tauri
import UIKit
import WebKit

// The on-device rung of Sjel's model ladder (dashboard/src/lib/intelligence/). The app's floor is
// iOS 16, so every FoundationModels use sits behind `#available`. On an ineligible device the
// plugin still loads and says why it cannot run; it never pretends to answer.
//
// API shapes follow the installed SDK's FoundationModels.swiftinterface (iPhoneOS27.0.sdk), per
// capabilities/foundation-models/README.md "Sources": the documentation site does not fetch as text.

private struct RespondArguments: Decodable {
  let prompt: String
  let instructions: String?
  let maxResponseTokens: Int?
  let temperature: Double?
}

private struct ModelStatus: Encodable {
  /// "available" or "unavailable".
  let status: String
  /// Set only when unavailable: osTooOld, deviceNotEligible, appleIntelligenceNotEnabled,
  /// modelNotReady, systemNotReady, unsupportedLocale.
  let reason: String?
  /// Tokens shared between prompt and reply. Set only for the on-device model.
  let contextSize: Int?

  static func unavailable(_ reason: String) -> ModelStatus {
    ModelStatus(status: "unavailable", reason: reason, contextSize: nil)
  }
}

private struct AvailabilityPayload: Encodable {
  let onDevice: ModelStatus
  /// Reported so the app can show it; the plugin never sends a prompt to Private Cloud Compute.
  let privateCloud: ModelStatus
}

private struct RespondPayload: Encodable {
  let text: String
  let source = "on-device"
  /// Prompt plus instructions, when the OS can count them (iOS 26.4 and later).
  let promptTokens: Int?
}

/// The reply budget when the caller names none. Matches the 4,096-token window arithmetic in
/// capabilities/foundation-models/README.md: a short answer, with most of the window for the source.
private let defaultReplyTokens = 512

final class FoundationModelsPlugin: Plugin {
  @objc public func availability(_ invoke: Invoke) {
    invoke.resolve(
      AvailabilityPayload(onDevice: Self.onDeviceStatus(), privateCloud: Self.privateCloudStatus())
    )
  }

  @objc public func respond(_ invoke: Invoke) {
    let args: RespondArguments
    do {
      args = try invoke.parseArgs(RespondArguments.self)
    } catch {
      invoke.reject(error.localizedDescription, code: "invalid_arguments")
      return
    }
    guard #available(iOS 26.0, *) else {
      invoke.reject("The on-device model needs iOS 26 or later.", code: "unavailable")
      return
    }
    Task { await Self.run(args, invoke) }
  }

  private static func onDeviceStatus() -> ModelStatus {
    guard #available(iOS 26.0, *) else { return .unavailable("osTooOld") }
    let model = SystemLanguageModel.default
    switch model.availability {
    case .available:
      guard model.supportsLocale() else { return .unavailable("unsupportedLocale") }
      return ModelStatus(status: "available", reason: nil, contextSize: model.contextSize)
    case .unavailable(let reason):
      return .unavailable(Self.name(reason))
    }
  }

  private static func privateCloudStatus() -> ModelStatus {
    guard #available(iOS 27.0, *) else { return .unavailable("osTooOld") }
    switch PrivateCloudComputeLanguageModel().availability {
    case .available:
      return ModelStatus(status: "available", reason: nil, contextSize: nil)
    case .unavailable(.deviceNotEligible):
      return .unavailable("deviceNotEligible")
    case .unavailable(.systemNotReady):
      return .unavailable("systemNotReady")
    case .unavailable:
      return .unavailable("systemNotReady")
    }
  }

  @available(iOS 26.0, *)
  private static func name(_ reason: SystemLanguageModel.Availability.UnavailableReason) -> String {
    switch reason {
    case .deviceNotEligible: return "deviceNotEligible"
    case .appleIntelligenceNotEnabled: return "appleIntelligenceNotEnabled"
    case .modelNotReady: return "modelNotReady"
    @unknown default: return "modelNotReady"
    }
  }

  @available(iOS 26.0, *)
  private static func run(_ args: RespondArguments, _ invoke: Invoke) async {
    let status = onDeviceStatus()
    guard status.status == "available" else {
      invoke.reject(
        "The on-device model is unavailable: \(status.reason ?? "unknown").", code: "unavailable")
      return
    }
    let model = SystemLanguageModel.default
    let reply = args.maxResponseTokens ?? defaultReplyTokens

    // Refuse an over-window request before it runs, never truncate it: a shortened prompt
    // summarizes half a document and says nothing (capabilities/foundation-models/README.md,
    // "The 4,096-token window is the feature").
    var promptTokens: Int?
    if #available(iOS 26.4, *) {
      do {
        var used = try await model.tokenCount(for: args.prompt)
        if let instructions = args.instructions {
          used += try await model.tokenCount(for: Instructions(instructions))
        }
        promptTokens = used
        if used + reply > model.contextSize {
          invoke.reject(
            "The prompt needs \(used) tokens plus \(reply) for the reply; the window is \(model.contextSize).",
            code: "context_length_exceeded")
          return
        }
      } catch {
        // Counting failed. The session still refuses an over-window prompt with its own error.
      }
    }

    let session = LanguageModelSession(model: model, instructions: args.instructions)
    let options = GenerationOptions(temperature: args.temperature, maximumResponseTokens: reply)
    do {
      let response: LanguageModelSession.Response<String> = try await session.respond(
        to: args.prompt, options: options)
      invoke.resolve(RespondPayload(text: response.content, promptTokens: promptTokens))
    } catch {
      invoke.reject(error.localizedDescription, code: Self.code(for: error))
    }
  }

  /// A stable code for the web side. The router reads `context_length_exceeded` and `unavailable`
  /// as "try the next rung"; every other code is shown to the reader.
  @available(iOS 26.0, *)
  private static func code(for error: Error) -> String {
    if #available(iOS 27.0, *), let error = error as? LanguageModelError {
      switch error {
      case .contextSizeExceeded: return "context_length_exceeded"
      case .guardrailViolation, .refusal: return "refused"
      case .unsupportedLanguageOrLocale: return "unsupported_locale"
      case .rateLimited, .timeout: return "busy"
      default: return "generation_failed"
      }
    }
    if let error = error as? LanguageModelSession.GenerationError {
      switch error {
      case .exceededContextWindowSize: return "context_length_exceeded"
      case .guardrailViolation, .refusal: return "refused"
      case .unsupportedLanguageOrLocale: return "unsupported_locale"
      case .assetsUnavailable: return "unavailable"
      case .rateLimited, .concurrentRequests: return "busy"
      default: return "generation_failed"
      }
    }
    return "generation_failed"
  }
}

@_cdecl("init_plugin_foundation_models")
func initPlugin() -> Plugin {
  return FoundationModelsPlugin()
}
