//
//  LocalTTSClient.swift
//  leanring-buddy
//
//  On-device neural text-to-speech via FluidAudio's Supertonic-3. One model
//  and one voice for both English and Hindi — each sentence is tagged "en"
//  or "hi" by script. The ~400MB model downloads on first launch; until it
//  loads, or if synthesis fails, the macOS system voice is used instead.
//

import AVFoundation
import FluidAudio
import NaturalLanguage

@MainActor
final class LocalTTSClient {
    private static let supertonicVoice: Supertonic3Voice = .f1

    private var supertonicManager: Supertonic3Manager?
    private var supertonicVoiceStyle: Supertonic3VoiceStyle?

    private let systemSpeechSynthesizer = AVSpeechSynthesizer()
    private var audioPlayer: AVAudioPlayer?
    private var playbackTask: Task<Void, Never>?
    /// Bumped on every speakText so a cancelled playback finishing late
    /// can't clear the speaking flag for the playback that replaced it.
    private var playbackGeneration = 0
    private let onSpeakingStateChanged: (Bool) -> Void

    init(onSpeakingStateChanged: @escaping (Bool) -> Void) {
        self.onSpeakingStateChanged = onSpeakingStateChanged
        Task { await loadNeuralVoices() }
    }

    private func loadNeuralVoices() async {
        do {
            print("🔊 Supertonic-3: downloading/loading model")
            let loadedSupertonicManager = try await Supertonic3Manager.downloadAndCreate()
            supertonicVoiceStyle = try await Supertonic3ResourceDownloader.loadVoiceStyle(Self.supertonicVoice)
            supertonicManager = loadedSupertonicManager
            print("🔊 Supertonic-3: ready")
        } catch {
            print("⚠️ Supertonic-3: failed to load, using system voice: \(error)")
        }
    }

    /// Starts speaking `text` and returns immediately. Synthesizes one
    /// sentence at a time and prepares the next sentence while the current
    /// one plays, so speech starts fast and has no gaps.
    func speakText(_ text: String) {
        stopPlayback()
        playbackGeneration += 1
        let thisPlaybackGeneration = playbackGeneration

        onSpeakingStateChanged(true)
        playbackTask = Task {
            await speakSentences(Self.splitIntoSentences(text), fullText: text)
            if playbackGeneration == thisPlaybackGeneration {
                onSpeakingStateChanged(false)
            }
        }
    }

    /// Speaks `text` and returns once it has finished (or was interrupted),
    /// so callers can line other things up with the speech.
    func speakTextAndWaitUntilFinished(_ text: String) async {
        speakText(text)
        await playbackTask?.value
    }

    private func speakSentences(_ sentences: [String], fullText: String) async {
        guard let supertonicManager, let supertonicVoiceStyle, !sentences.isEmpty else {
            await speakWithSystemVoiceUntilFinished(fullText)
            return
        }

        var nextClipSynthesisTask: Task<Data, Error>? = makeClipSynthesisTask(for: sentences[0], supertonicManager: supertonicManager, supertonicVoiceStyle: supertonicVoiceStyle)

        for sentenceIndex in sentences.indices {
            guard let clipSynthesisTask = nextClipSynthesisTask else { break }

            let clipWavData: Data
            do {
                clipWavData = try await clipSynthesisTask.value
            } catch {
                guard !Task.isCancelled else { return }
                print("⚠️ Neural TTS failed, finishing with system voice: \(error)")
                await speakWithSystemVoiceUntilFinished(sentences[sentenceIndex...].joined(separator: " "))
                return
            }
            guard !Task.isCancelled else { return }

            let nextSentenceIndex = sentenceIndex + 1
            nextClipSynthesisTask = nextSentenceIndex < sentences.count
                ? makeClipSynthesisTask(for: sentences[nextSentenceIndex], supertonicManager: supertonicManager, supertonicVoiceStyle: supertonicVoiceStyle)
                : nil

            guard let clipPlayer = try? AVAudioPlayer(data: clipWavData) else { continue }
            audioPlayer = clipPlayer
            clipPlayer.play()
            while clipPlayer.isPlaying {
                try? await Task.sleep(for: .milliseconds(30))
                guard !Task.isCancelled else { return }
            }
        }
    }

    func stopPlayback() {
        playbackTask?.cancel()
        playbackTask = nil
        audioPlayer?.stop()
        audioPlayer = nil
        systemSpeechSynthesizer.stopSpeaking(at: .immediate)
        onSpeakingStateChanged(false)
    }

    private func makeClipSynthesisTask(
        for sentence: String,
        supertonicManager: Supertonic3Manager,
        supertonicVoiceStyle: Supertonic3VoiceStyle
    ) -> Task<Data, Error> {
        let sentenceLanguageCode = Self.containsDevanagari(sentence) ? "hi" : "en"
        return Task {
            let (samples, _) = try await supertonicManager.synthesize(
                text: sentence,
                language: sentenceLanguageCode,
                style: supertonicVoiceStyle
            )
            return try AudioWAV.data(from: samples, sampleRate: Double(Supertonic3Constants.sampleRate))
        }
    }

    private func speakWithSystemVoiceUntilFinished(_ text: String) async {
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = Self.containsDevanagari(text)
            ? AVSpeechSynthesisVoice(language: "hi-IN")
            : Self.bestInstalledSystemVoice
        systemSpeechSynthesizer.speak(utterance)
        // isSpeaking flips true asynchronously, so give it a moment first.
        try? await Task.sleep(for: .milliseconds(150))
        while systemSpeechSynthesizer.isSpeaking {
            try? await Task.sleep(for: .milliseconds(50))
            guard !Task.isCancelled else { return }
        }
    }

    private static let bestInstalledSystemVoice: AVSpeechSynthesisVoice? = {
        let preferredLanguageCode = Locale.current.language.languageCode?.identifier ?? "en"
        // AVSpeechSynthesisVoiceQuality raw values go default(1) < enhanced(2) < premium(3).
        return AVSpeechSynthesisVoice.speechVoices()
            .filter { $0.language.hasPrefix(preferredLanguageCode) }
            .max { $0.quality.rawValue < $1.quality.rawValue }
    }()

    private static func containsDevanagari(_ text: String) -> Bool {
        text.unicodeScalars.contains { (0x0900...0x097F).contains($0.value) }
    }

    private static func splitIntoSentences(_ text: String) -> [String] {
        let sentenceTokenizer = NLTokenizer(unit: .sentence)
        sentenceTokenizer.string = text
        return sentenceTokenizer.tokens(for: text.startIndex..<text.endIndex)
            .map { text[$0].trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }
}
