// Native Core Audio Process Tap helper for Interview Copilot.
// Lifecycle adapted from Meetily (MIT) and AudioCap (BSD-2-Clause).
// Requires macOS 14.4+ and NSAudioCaptureUsageDescription in the host app.

import Foundation
import CoreAudio
import AudioToolbox
import AVFoundation

enum TapError: Error, CustomStringConvertible {
    case status(String, OSStatus)
    case message(String)

    var description: String {
        switch self {
        case let .status(operation, code): return "\(operation) failed (OSStatus \(code))"
        case let .message(message): return message
        }
    }
}

private func checked(_ operation: String, _ status: OSStatus) throws {
    guard status == noErr else { throw TapError.status(operation, status) }
}

private func defaultOutputDevice() throws -> AudioObjectID {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioHardwarePropertyDefaultOutputDevice,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var device = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    try checked("read default output device", AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device
    ))
    return device
}

private func deviceUID(_ device: AudioObjectID) throws -> String {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioDevicePropertyDeviceUID,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var value: CFString = "" as CFString
    var size = UInt32(MemoryLayout<CFString>.size)
    try checked("read output device UID", AudioObjectGetPropertyData(
        device, &address, 0, nil, &size, &value
    ))
    return value as String
}

private func tapFormat(_ tap: AudioObjectID) throws -> AudioStreamBasicDescription {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioTapPropertyFormat,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
    var format = AudioStreamBasicDescription()
    var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    try checked("read tap format", AudioObjectGetPropertyData(
        tap, &address, 0, nil, &size, &format
    ))
    return format
}

final class SystemAudioTap {
    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var aggregateID = AudioObjectID(kAudioObjectUnknown)
    private var ioProcID: AudioDeviceIOProcID?
    private let queue = DispatchQueue(label: "com.interviewcopilot.audio-tap", qos: .userInitiated)
    private let writerQueue = DispatchQueue(label: "com.interviewcopilot.audio-writer", qos: .userInitiated)
    private var sampleRate: Double = 48_000

    func start() throws {
        guard #available(macOS 14.4, *) else {
            throw TapError.status("Core Audio Process Tap requires macOS 14.4+", -1)
        }

        let output = try defaultOutputDevice()
        let outputUID = try deviceUID(output)
        let tapDescription = CATapDescription(monoGlobalTapButExcludeProcesses: [])
        tapDescription.uuid = UUID()
        tapDescription.name = "Interview Copilot System Audio"
        tapDescription.isPrivate = true
        tapDescription.muteBehavior = .unmuted

        try checked("create process tap", AudioHardwareCreateProcessTap(tapDescription, &tapID))
        sampleRate = try tapFormat(tapID).mSampleRate

        let aggregateDescription: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Interview Copilot Audio Tap",
            kAudioAggregateDeviceUIDKey: UUID().uuidString,
            kAudioAggregateDeviceMainSubDeviceKey: outputUID,
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceIsStackedKey: false,
            kAudioAggregateDeviceTapAutoStartKey: true,
            kAudioAggregateDeviceTapListKey: [[
                kAudioSubTapDriftCompensationKey: true,
                kAudioSubTapUIDKey: tapDescription.uuid.uuidString
            ]]
        ]
        try checked("create aggregate device", AudioHardwareCreateAggregateDevice(
            aggregateDescription as CFDictionary, &aggregateID
        ))

        let outputHandle = FileHandle.standardOutput
        let ioBlock: AudioDeviceIOBlock = { [writerQueue] _, inputData, _, _, _ in
            let buffers = UnsafeMutableAudioBufferListPointer(
                UnsafeMutablePointer(mutating: inputData)
            )
            // monoGlobalTap produces one Float32 buffer. Copy it before leaving
            // the real-time callback, then perform pipe I/O on a separate queue.
            guard let buffer = buffers.first, let pointer = buffer.mData else { return }
            let bytes = Data(bytes: pointer, count: Int(buffer.mDataByteSize))
            writerQueue.async {
                outputHandle.write(bytes)
            }
        }

        try checked("create IOProc", AudioDeviceCreateIOProcIDWithBlock(
            &ioProcID, aggregateID, queue, ioBlock
        ))
        let header = "{\"sampleRate\":\(Int(sampleRate)),\"channels\":1,\"format\":\"f32le\"}\n"
        outputHandle.write(Data(header.utf8))
        try checked("start aggregate device", AudioDeviceStart(aggregateID, ioProcID))
    }

    func stop() {
        if aggregateID != kAudioObjectUnknown {
            _ = AudioDeviceStop(aggregateID, ioProcID)
            if let ioProcID { _ = AudioDeviceDestroyIOProcID(aggregateID, ioProcID) }
            _ = AudioHardwareDestroyAggregateDevice(aggregateID)
            aggregateID = AudioObjectID(kAudioObjectUnknown)
        }
        if tapID != kAudioObjectUnknown {
            _ = AudioHardwareDestroyProcessTap(tapID)
            tapID = AudioObjectID(kAudioObjectUnknown)
        }
    }

    deinit { stop() }
}

final class MicrophoneCapture {
    private let engine = AVAudioEngine()
    private let writerQueue = DispatchQueue(
        label: "com.interviewcopilot.microphone-writer",
        qos: .userInitiated
    )
    private var tapInstalled = false

    private func requestPermission() throws {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return
        case .denied, .restricted:
            throw TapError.message("麦克风权限未开启，请到 系统设置 → 隐私与安全性 → 麦克风 中允许 Interview Copilot")
        case .notDetermined:
            let semaphore = DispatchSemaphore(value: 0)
            var granted = false
            AVCaptureDevice.requestAccess(for: .audio) { allowed in
                granted = allowed
                semaphore.signal()
            }
            semaphore.wait()
            if !granted {
                throw TapError.message("未获得麦克风权限")
            }
        @unknown default:
            throw TapError.message("无法确认麦克风权限状态")
        }
    }

    func start() throws {
        try requestPermission()

        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw TapError.message("默认麦克风没有可用的输入格式")
        }

        let outputHandle = FileHandle.standardOutput
        let channelCount = Int(format.channelCount)
        input.installTap(onBus: 0, bufferSize: 2_048, format: format) {
            [writerQueue] buffer, _ in
            guard let channels = buffer.floatChannelData else { return }
            let frameCount = Int(buffer.frameLength)
            guard frameCount > 0 else { return }

            var mono = [Float](repeating: 0, count: frameCount)
            for channel in 0..<channelCount {
                let samples = channels[channel]
                for frame in 0..<frameCount {
                    mono[frame] += samples[frame]
                }
            }
            if channelCount > 1 {
                let scale = 1.0 / Float(channelCount)
                for frame in 0..<frameCount {
                    mono[frame] *= scale
                }
            }

            writerQueue.async {
                mono.withUnsafeBytes { bytes in
                    outputHandle.write(Data(bytes))
                }
            }
        }
        tapInstalled = true

        let header = "{\"sampleRate\":\(Int(format.sampleRate)),\"channels\":1,\"format\":\"f32le\"}\n"
        outputHandle.write(Data(header.utf8))
        engine.prepare()
        try engine.start()
    }

    func stop() {
        if tapInstalled {
            engine.inputNode.removeTap(onBus: 0)
            tapInstalled = false
        }
        engine.stop()
    }

    deinit { stop() }
}

@main
enum AudioTapMain {
    static func main() {
        if CommandLine.arguments.dropFirst().first == "microphone" {
            let microphone = MicrophoneCapture()
            do {
                try microphone.start()
                dispatchMain()
            } catch {
                FileHandle.standardError.write(Data("\(error)\n".utf8))
                exit(1)
            }
        } else {
            let tap = SystemAudioTap()
            do {
                try tap.start()
                dispatchMain()
            } catch {
                FileHandle.standardError.write(Data("\(error)\n".utf8))
                exit(1)
            }
        }
    }
}
