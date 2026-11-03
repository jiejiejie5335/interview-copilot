import AVFoundation
import SwiftRs
import Tauri

@_silgen_name("interview_audio_push_ffi")
private func pushInterviewAudio(
  _ samples: UnsafePointer<Float>,
  _ sampleCount: Int32,
  _ sampleRate: UInt32
)

private final class CaptureArgs: Decodable {
  let preferredSampleRate: Double?
}

final class IosAmbientAudioPlugin: Plugin {
  private let engine = AVAudioEngine()
  private var capturing = false

  @objc public func startCapture(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CaptureArgs.self)
    requestRecordPermission { [weak self] granted in
      guard granted else {
        invoke.reject("麦克风权限未授予，请在 iPhone 设置中允许访问麦克风")
        return
      }
      DispatchQueue.main.async {
        do {
          let sampleRate = try self?.startEngine(preferredSampleRate: args.preferredSampleRate)
          invoke.resolve(["sampleRate": sampleRate ?? 0])
        } catch {
          invoke.reject("无法启动 iPhone 麦克风：\(error.localizedDescription)")
        }
      }
    }
  }

  @objc public func stopCapture(_ invoke: Invoke) {
    DispatchQueue.main.async { [weak self] in
      self?.stopEngine()
      invoke.resolve()
    }
  }

  private func requestRecordPermission(completion: @escaping (Bool) -> Void) {
    if #available(iOS 17.0, *) {
      AVAudioApplication.requestRecordPermission(completionHandler: completion)
    } else {
      AVAudioSession.sharedInstance().requestRecordPermission(completion)
    }
  }

  private func startEngine(preferredSampleRate: Double?) throws -> Double {
    if capturing {
      return engine.inputNode.outputFormat(forBus: 0).sampleRate
    }

    let session = AVAudioSession.sharedInstance()
    try session.setCategory(.record, mode: .measurement, options: [.allowBluetooth])
    if let preferredSampleRate, preferredSampleRate > 0 {
      try session.setPreferredSampleRate(preferredSampleRate)
    }
    try session.setActive(true, options: .notifyOthersOnDeactivation)

    let input = engine.inputNode
    let format = input.outputFormat(forBus: 0)
    guard format.sampleRate > 0, format.channelCount > 0 else {
      throw NSError(
        domain: "InterviewCopilotAudio",
        code: 1,
        userInfo: [NSLocalizedDescriptionKey: "麦克风没有返回可用的音频格式"]
      )
    }

    input.installTap(onBus: 0, bufferSize: 1_024, format: format) { buffer, _ in
      guard let channels = buffer.floatChannelData else { return }
      let frameCount = Int(buffer.frameLength)
      guard frameCount > 0 else { return }
      pushInterviewAudio(channels[0], Int32(frameCount), UInt32(format.sampleRate.rounded()))
    }

    engine.prepare()
    try engine.start()
    capturing = true
    UIApplication.shared.isIdleTimerDisabled = true
    return format.sampleRate
  }

  private func stopEngine() {
    guard capturing else { return }
    engine.inputNode.removeTap(onBus: 0)
    engine.stop()
    capturing = false
    UIApplication.shared.isIdleTimerDisabled = false
    try? AVAudioSession.sharedInstance().setActive(
      false,
      options: .notifyOthersOnDeactivation
    )
  }

  deinit {
    stopEngine()
  }
}

@_cdecl("init_plugin_ios_ambient_audio")
func initPlugin() -> Plugin {
  IosAmbientAudioPlugin()
}
