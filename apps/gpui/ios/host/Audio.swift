import AVFoundation
import Foundation

// The phone's output device, and the one Rust function that fills it.
//
// The engine is the Mac's own (`sjel_gpui_core::soundscape`), and it is platform-free; what differs is
// who drives the clock. On the Mac a CPAL stream asks the engine for frames. Here `AVAudioSourceNode`
// does, on the audio thread, and Rust renders exactly the frames Core Audio asked for
// (`ios/src/audio.rs`). No timer, no lookahead, and nothing queued between the two.
//
// The session is `.playback` with `.mixWithOthers`: a focus soundscape is something played *with*
// whatever else the phone is doing, and taking the whole audio route away from a podcast would be a
// rude way to help somebody concentrate.
final class SoundscapeAudio {
  static let shared = SoundscapeAudio()

  private let engine = AVAudioEngine()
  private var node: AVAudioSourceNode?
  private var running = false
  private let rate: Double = 48_000

  /// Opens the session and the node, and answers what to say about it: `ok:<device>:<rate>` or
  /// `err:<why>`, the one string `ios/src/audio.rs` parses.
  func start() -> UnsafePointer<CChar>? {
    if running, let name = outputName {
      return reply("ok:\(name):\(Int(rate)) Hz")
    }
    do {
      let session = AVAudioSession.sharedInstance()
      try session.setCategory(.playback, mode: .default, options: [.mixWithOthers])
      try session.setActive(true)
    } catch {
      return reply("err:The audio session would not start: \(error.localizedDescription)")
    }
    guard
      let format = AVAudioFormat(standardFormatWithSampleRate: rate, channels: 2)
    else {
      return reply("err:This device has no stereo float format.")
    }
    let node = AVAudioSourceNode(format: format) { _, _, frameCount, buffers -> OSStatus in
      let list = UnsafeMutableAudioBufferListPointer(buffers)
      guard list.count >= 2,
        let left = list[0].mData,
        let right = list[1].mData
      else {
        return noErr
      }
      sjel_ios_soundscape_render(
        format.sampleRate,
        left.assumingMemoryBound(to: Float.self),
        right.assumingMemoryBound(to: Float.self),
        frameCount)
      return noErr
    }
    engine.attach(node)
    engine.connect(node, to: engine.mainMixerNode, format: format)
    do {
      engine.prepare()
      try engine.start()
    } catch {
      engine.detach(node)
      return reply("err:The device would not start: \(error.localizedDescription)")
    }
    self.node = node
    running = true
    return reply("ok:\(outputName ?? "this iPhone"):\(Int(rate)) Hz")
  }

  /// The route's own name, once there is a route: `Speaker`, `AirPods Pro`, …
  private var outputName: String? {
    let outputs = AVAudioSession.sharedInstance().currentRoute.outputs
    return outputs.isEmpty ? nil : outputs.map(\.portName).joined(separator: " + ")
  }

  /// A C string the Rust side reads and forgets. Held here so it outlives the call, and released on
  /// the next one: the drawer asks for the device once per launch, not once per frame.
  private func reply(_ text: String) -> UnsafePointer<CChar>? {
    lastReply?.deallocate()
    let bytes = Array(text.utf8) + [0]
    let buffer = UnsafeMutablePointer<Int8>.allocate(capacity: bytes.count)
    buffer.initialize(from: bytes.map { Int8(bitPattern: $0) }, count: bytes.count)
    lastReply = buffer
    return UnsafePointer(buffer)
  }

  private var lastReply: UnsafeMutablePointer<Int8>?
}

/// `ios/src/audio.rs` calls this to open the device. It answers `ok:<device>:<rate>` or `err:<why>`.
@_cdecl("sjel_ios_audio_start")
public func sjelIOSAudioStart() -> UnsafePointer<CChar>? {
  SoundscapeAudio.shared.start()
}
