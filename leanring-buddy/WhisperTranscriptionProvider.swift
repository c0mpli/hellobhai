//
//  WhisperTranscriptionProvider.swift
//  leanring-buddy
//
//  On-device push-to-talk transcription with WhisperKit (Whisper large-v3
//  turbo on Core ML). Auto-detects the spoken language, so Hindi, English,
//  and Hinglish all work. The ~626MB model downloads on first launch; until
//  it's loaded, sessions fall back to Apple Speech.
//

import AVFoundation
import Foundation
import WhisperKit

final class WhisperTranscriptionProvider: BuddyTranscriptionProvider {
    let displayName = "Whisper"
    let isConfigured = true
    let unavailableExplanation: String? = nil

    /// Only the Apple Speech fallback needs speech recognition permission.
    var requiresSpeechRecognitionPermission: Bool {
        Self.loadedWhisperKit == nil
    }

    private static var loadedWhisperKit: WhisperKit?
    /// Shown in the menu bar panel: Whisper vs the Apple Speech fallback.
    static var isWhisperModelLoaded: Bool { loadedWhisperKit != nil }
    private static var hasStartedLoadingModel = false
    private let appleSpeechFallbackProvider = AppleSpeechTranscriptionProvider()

    init() {
        Self.startLoadingModelIfNeeded()
    }

    private static let whisperModelsDirectory = FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("hellobhai/Models", isDirectory: true)

    private static func startLoadingModelIfNeeded() {
        guard !hasStartedLoadingModel else { return }
        hasStartedLoadingModel = true
        Task {
            do {
                print("🎙️ Whisper: downloading/loading model (first launch downloads ~626MB)")
                let whisperKit = try await WhisperKit(WhisperKitConfig(
                    model: "large-v3-v20240930_626MB",
                    // Explicit so the model lands in Application Support rather
                    // than ~/Documents (which would trigger a folder-access prompt).
                    downloadBase: whisperModelsDirectory,
                    verbose: false,
                    // The first run of a Core ML model triggers a one-time Neural
                    // Engine compile that takes minutes for large-v3. Prewarming
                    // pays that cost here, while Apple Speech still handles push-
                    // to-talk, instead of stalling the user's first transcription.
                    prewarm: true
                ))
                loadedWhisperKit = whisperKit
                print("🎙️ Whisper: ready")
            } catch {
                print("⚠️ Whisper: failed to load, staying on Apple Speech: \(error)")
            }
        }
    }

    func startStreamingSession(
        keyterms: [String],
        onTranscriptUpdate: @escaping (String) -> Void,
        onFinalTranscriptReady: @escaping (String) -> Void,
        onError: @escaping (Error) -> Void
    ) async throws -> any BuddyStreamingTranscriptionSession {
        guard let whisperKit = Self.loadedWhisperKit else {
            print("🎙️ Whisper: model not ready yet, using Apple Speech for this session")
            return try await appleSpeechFallbackProvider.startStreamingSession(
                keyterms: keyterms,
                onTranscriptUpdate: onTranscriptUpdate,
                onFinalTranscriptReady: onFinalTranscriptReady,
                onError: onError
            )
        }

        return WhisperTranscriptionSession(
            whisperKit: whisperKit,
            onFinalTranscriptReady: onFinalTranscriptReady,
            onError: onError
        )
    }
}

/// Buffers mic audio while the hotkey is held, then transcribes it all at
/// once on release. Whisper isn't a streaming model, and push-to-talk clips
/// are short, so batch transcription is both simpler and more accurate.
private final class WhisperTranscriptionSession: BuddyStreamingTranscriptionSession {
    /// The first transcription after launch can take a few seconds while
    /// Core ML warms up, so give it more room than streaming providers.
    let finalTranscriptFallbackDelaySeconds: TimeInterval = 10

    private static let whisperSampleRate: Double = 16_000
    /// Whisper hallucinates on near-empty clips; skip anything under 0.3s.
    private static let minimumSampleCount = Int(whisperSampleRate * 0.3)

    private let whisperKit: WhisperKit
    private let onFinalTranscriptReady: (String) -> Void
    private let onError: (Error) -> Void

    /// appendAudioBuffer runs on the audio tap thread while
    /// requestFinalTranscript runs on main, so guard the sample buffer.
    private let recordedSamplesLock = NSLock()
    private var recordedSamples: [Float] = []
    private var transcriptionTask: Task<Void, Never>?

    init(
        whisperKit: WhisperKit,
        onFinalTranscriptReady: @escaping (String) -> Void,
        onError: @escaping (Error) -> Void
    ) {
        self.whisperKit = whisperKit
        self.onFinalTranscriptReady = onFinalTranscriptReady
        self.onError = onError
    }

    func appendAudioBuffer(_ audioBuffer: AVAudioPCMBuffer) {
        guard let resampledBuffer = AudioProcessor.resampleAudio(
            fromBuffer: audioBuffer,
            toSampleRate: Self.whisperSampleRate,
            channelCount: 1
        ) else { return }
        let resampledSamples = AudioProcessor.convertBufferToArray(buffer: resampledBuffer)

        recordedSamplesLock.lock()
        recordedSamples.append(contentsOf: resampledSamples)
        recordedSamplesLock.unlock()
    }

    func requestFinalTranscript() {
        recordedSamplesLock.lock()
        let samplesToTranscribe = recordedSamples
        recordedSamplesLock.unlock()

        guard samplesToTranscribe.count >= Self.minimumSampleCount else {
            onFinalTranscriptReady("")
            return
        }

        transcriptionTask = Task {
            do {
                let transcriptionStartDate = Date()
                // language nil + detectLanguage lets Whisper pick hi or en per
                // clip instead of forcing English.
                let decodingOptions = DecodingOptions(
                    language: nil,
                    detectLanguage: true,
                    skipSpecialTokens: true,
                    withoutTimestamps: true
                )
                var transcriptionResults = try await whisperKit.transcribe(
                    audioArray: samplesToTranscribe,
                    decodeOptions: decodingOptions
                )
                guard !Task.isCancelled else { return }

                // Spoken Hindi and Urdu sound almost identical, so detection
                // sometimes picks Urdu and writes Urdu script. Redo it as Hindi.
                if transcriptionResults.first?.language == "ur" {
                    let hindiDecodingOptions = DecodingOptions(
                        language: "hi",
                        detectLanguage: false,
                        skipSpecialTokens: true,
                        withoutTimestamps: true
                    )
                    transcriptionResults = try await whisperKit.transcribe(
                        audioArray: samplesToTranscribe,
                        decodeOptions: hindiDecodingOptions
                    )
                    guard !Task.isCancelled else { return }
                }

                let transcriptText = transcriptionResults
                    .map(\.text)
                    .joined(separator: " ")
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                let detectedLanguage = transcriptionResults.first?.language ?? "?"
                print("🎙️ Whisper: [\(detectedLanguage)] in \(String(format: "%.2f", Date().timeIntervalSince(transcriptionStartDate)))s")
                onFinalTranscriptReady(transcriptText)
            } catch {
                guard !Task.isCancelled else { return }
                onError(error)
            }
        }
    }

    func cancel() {
        transcriptionTask?.cancel()
        transcriptionTask = nil
    }
}
