//
//  LocalTTSClient.swift
//  leanring-buddy
//
//  On-device neural text-to-speech via FluidAudio. Two engines:
//  - Kokoro-82M (default): the most natural English voice. It has no Hindi,
//    so sentences containing Devanagari always go to Supertonic-3.
//  - Supertonic-3: one model for English and Hindi, tiny and fast. Always
//    loaded, because it is the only Hindi path and the fallback when Kokoro
//    is loading or fails.
//  The macOS system voice is the last resort. Engine and voices are saved in
//  UserDefaults and can be switched at runtime from the menu bar panel.
//

import AVFoundation
import Combine
import FluidAudio
import NaturalLanguage

enum TTSEngine: String, CaseIterable {
    case kokoro
    case supertonic

    var displayName: String {
        switch self {
        case .kokoro: return "Kokoro"
        case .supertonic: return "Supertonic"
        }
    }

    var modelDisplayName: String {
        switch self {
        case .kokoro: return "Kokoro-82M"
        case .supertonic: return "Supertonic-3"
        }
    }
}

enum NeuralVoiceLoadState {
    case notLoaded
    case loading
    case ready
    case failed
}

@MainActor
final class LocalTTSClient: ObservableObject {
    private static let selectedEngineUserDefaultsKey = "selectedTTSEngine"
    private static let kokoroVoiceUserDefaultsKey = "selectedKokoroVoice"
    private static let supertonicVoiceUserDefaultsKey = "selectedSupertonicVoice"

    /// American and British English Kokoro voices. The other packs (Hindi,
    /// Japanese, ...) only change the accent of English text in the English
    /// variant, and Devanagari input comes out garbled.
    static let availableKokoroVoices = KokoroAneConstants.englishVoices
        .filter { $0.hasPrefix("af_") || $0.hasPrefix("am_") || $0.hasPrefix("bf_") || $0.hasPrefix("bm_") }

    /// Kokoro's model chain is kept next to the Whisper model. Its shared G2P
    /// files always live in ~/.cache/fluidaudio (FluidAudio pins that path).
    private static let kokoroModelsDirectory = FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("hellobhai/Models", isDirectory: true)

    @Published private(set) var selectedEngine: TTSEngine
    @Published private(set) var selectedKokoroVoice: String
    @Published private(set) var selectedSupertonicVoice: Supertonic3Voice
    @Published private(set) var kokoroLoadState: NeuralVoiceLoadState = .notLoaded
    @Published private(set) var supertonicLoadState: NeuralVoiceLoadState = .notLoaded

    private var kokoroManager: KokoroAneManager?
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
        let userDefaults = UserDefaults.standard
        selectedEngine = userDefaults.string(forKey: Self.selectedEngineUserDefaultsKey)
            .flatMap(TTSEngine.init(rawValue:)) ?? .kokoro
        selectedKokoroVoice = userDefaults.string(forKey: Self.kokoroVoiceUserDefaultsKey)
            .flatMap { Self.availableKokoroVoices.contains($0) ? $0 : nil } ?? "af_heart"
        selectedSupertonicVoice = userDefaults.string(forKey: Self.supertonicVoiceUserDefaultsKey)
            .flatMap(Supertonic3Voice.init(rawValue:)) ?? .f1

        // Supertonic first: it is small, loads fast, and is the only Hindi path.
        Task { await loadSupertonic() }
        if selectedEngine == .kokoro {
            Task { await loadKokoroIfNeeded() }
        }
    }

    // MARK: - Settings

    func setSelectedEngine(_ engine: TTSEngine) {
        guard engine != selectedEngine else { return }
        selectedEngine = engine
        UserDefaults.standard.set(engine.rawValue, forKey: Self.selectedEngineUserDefaultsKey)
        if engine == .kokoro {
            Task { await loadKokoroIfNeeded() }
        } else {
            unloadKokoro()
        }
    }

    func setSelectedKokoroVoice(_ voice: String) {
        selectedKokoroVoice = voice
        UserDefaults.standard.set(voice, forKey: Self.kokoroVoiceUserDefaultsKey)
    }

    func setSelectedSupertonicVoice(_ voice: Supertonic3Voice) {
        selectedSupertonicVoice = voice
        UserDefaults.standard.set(voice.rawValue, forKey: Self.supertonicVoiceUserDefaultsKey)
        Task {
            do {
                let loadedVoiceStyle = try await Supertonic3ResourceDownloader.loadVoiceStyle(voice)
                // Ignore a slow load that finishes after the user picked yet another voice.
                if selectedSupertonicVoice == voice {
                    supertonicVoiceStyle = loadedVoiceStyle
                }
            } catch {
                print("⚠️ Supertonic-3: failed to load voice \(voice.rawValue): \(error)")
            }
        }
    }

    // MARK: - Loading

    private func loadSupertonic() async {
        supertonicLoadState = .loading
        do {
            print("🔊 Supertonic-3: downloading/loading model")
            let loadedSupertonicManager = try await Supertonic3Manager.downloadAndCreate()
            supertonicVoiceStyle = try await Supertonic3ResourceDownloader.loadVoiceStyle(selectedSupertonicVoice)
            supertonicManager = loadedSupertonicManager
            supertonicLoadState = .ready
            print("🔊 Supertonic-3: ready")
        } catch {
            supertonicLoadState = .failed
            print("⚠️ Supertonic-3: failed to load: \(error)")
        }
    }

    private func loadKokoroIfNeeded() async {
        guard kokoroLoadState == .notLoaded || kokoroLoadState == .failed else { return }
        kokoroLoadState = .loading
        // Legacy runtime on purpose: the v3 runtime crashed on macOS 27 (MPSGraph assertion).
        let newKokoroManager = KokoroAneManager(
            variant: .english,
            defaultVoice: selectedKokoroVoice,
            directory: Self.kokoroModelsDirectory
        )
        do {
            print("🔊 Kokoro: downloading/loading model (first launch downloads ~140MB and compiles for ~1 min)")
            try await newKokoroManager.initialize(preloadVoices: [selectedKokoroVoice])
            // The first synthesis after load is ~10x slower than the rest, so
            // pay it here instead of on the user's first spoken sentence.
            _ = try await newKokoroManager.synthesize(text: "Hello.", voice: selectedKokoroVoice)
        } catch {
            kokoroLoadState = .failed
            print("⚠️ Kokoro: failed to load, using Supertonic-3: \(error)")
            return
        }
        // The user may have switched away from Kokoro while it was loading.
        guard selectedEngine == .kokoro else {
            await newKokoroManager.cleanup()
            kokoroLoadState = .notLoaded
            return
        }
        kokoroManager = newKokoroManager
        kokoroLoadState = .ready
        print("🔊 Kokoro: ready")
    }

    /// Frees Kokoro's memory (~450-800MB) when the user picks Supertonic.
    private func unloadKokoro() {
        guard let kokoroManagerToUnload = kokoroManager else { return }
        kokoroManager = nil
        kokoroLoadState = .notLoaded
        Task { await kokoroManagerToUnload.cleanup() }
    }

    // MARK: - Speaking

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

    private var isAnyNeuralEngineReady: Bool {
        kokoroManager != nil || (supertonicManager != nil && supertonicVoiceStyle != nil)
    }

    private func speakSentences(_ sentences: [String], fullText: String) async {
        guard isAnyNeuralEngineReady, !sentences.isEmpty else {
            await speakWithSystemVoiceUntilFinished(fullText)
            return
        }

        var nextClipSynthesisTask: Task<Data, Error>? = makeClipSynthesisTask(for: sentences[0])

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
                ? makeClipSynthesisTask(for: sentences[nextSentenceIndex])
                : nil

            // Each clip is its own WAV (24 kHz from Kokoro, 44.1 kHz from
            // Supertonic), so AVAudioPlayer handles the rate per clip.
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

    private enum NeuralSynthesisError: Error {
        case supertonicNotLoaded
    }

    /// Routes one sentence: Devanagari always goes to Supertonic-3 ("hi"),
    /// since Kokoro has no Hindi. Other sentences go to Kokoro when it is the
    /// selected engine and loaded; if Kokoro throws, that sentence is retried
    /// on Supertonic-3 ("en").
    private func makeClipSynthesisTask(for sentence: String) -> Task<Data, Error> {
        let sentenceContainsDevanagari = Self.containsDevanagari(sentence)
        let kokoroManagerForSentence = (selectedEngine == .kokoro && !sentenceContainsDevanagari) ? kokoroManager : nil
        let kokoroVoiceForSentence = selectedKokoroVoice
        let supertonicManagerForSentence = supertonicManager
        let supertonicVoiceStyleForSentence = supertonicVoiceStyle
        let supertonicLanguageCode = sentenceContainsDevanagari ? "hi" : "en"

        return Task {
            if let kokoroManagerForSentence {
                do {
                    return try await kokoroManagerForSentence.synthesize(text: sentence, voice: kokoroVoiceForSentence)
                } catch {
                    try Task.checkCancellation()
                    print("⚠️ Kokoro failed on a sentence, retrying with Supertonic-3: \(error)")
                }
            }
            guard let supertonicManagerForSentence, let supertonicVoiceStyleForSentence else {
                throw NeuralSynthesisError.supertonicNotLoaded
            }
            let (samples, _) = try await supertonicManagerForSentence.synthesize(
                text: sentence,
                language: supertonicLanguageCode,
                style: supertonicVoiceStyleForSentence
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
