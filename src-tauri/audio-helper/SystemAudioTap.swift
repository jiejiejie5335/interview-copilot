// Native Core Audio Process Tap helper for Interview Copilot.
// Lifecycle adapted from Meetily (MIT) and AudioCap (BSD-2-Clause).
// Requires macOS 14.4+ and NSAudioCaptureUsageDescription in the host app.

import Foundation
import CoreAudio
import AudioToolbox

enum TapError: Error, CustomStringConvertible {
    case status(String, OSStatus)

    var description: String {
        switch self {
        case let .status(operation, code): return "\(operation) failed (OSStatus \(code))"
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

@main
enum AudioTapMain {
    static func main() {
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
