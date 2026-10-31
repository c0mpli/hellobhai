//
//  CompanionPanelView.swift
//  leanring-buddy
//
//  The SwiftUI content hosted inside the menu bar panel. Shows the companion
//  status, the push-to-talk hint, and every setting the app has: Claude
//  model, voice engine and voices, speech-to-text status, shortcut,
//  permissions, launch at login, onboarding replay, and quit.
//

import AVFoundation
import FluidAudio
import ServiceManagement
import SwiftUI

struct CompanionPanelView: View {
    @ObservedObject var companionManager: CompanionManager

    /// Same UserDefaults key BuddyPushToTalkShortcut reads on every key event,
    /// so picking a shortcut here takes effect immediately.
    @AppStorage(BuddyPushToTalkShortcut.currentShortcutOptionUserDefaultsKey)
    private var selectedShortcutOptionRawValue = BuddyPushToTalkShortcut.ShortcutOption.controlOption.rawValue

    @State private var isPermissionListExpanded = false
    @State private var isLaunchAtLoginEnabled = SMAppService.mainApp.status == .enabled

    private static let panelWidth: CGFloat = 340

    /// Model aliases the `claude` CLI accepts for `--model`. Storing the alias
    /// (not a dated id) means new model versions are picked up automatically.
    private static let claudeModelOptions: [(label: String, modelAlias: String)] = [
        ("Fable", "fable"),
        ("Opus", "opus"),
        ("Sonnet", "sonnet"),
        ("Haiku", "haiku"),
    ]

    private var selectedShortcutOption: BuddyPushToTalkShortcut.ShortcutOption {
        BuddyPushToTalkShortcut.ShortcutOption(rawValue: selectedShortcutOptionRawValue) ?? .controlOption
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            panelHeader
            panelDivider

            // The panel is sized to fit its content. When the screen is too
            // short for that, MenuBarPanelManager caps the height and the
            // scrolling version below is used instead.
            ViewThatFits(in: .vertical) {
                panelSections
                ScrollView(.vertical, showsIndicators: false) {
                    panelSections
                }
            }

            panelDivider
            footerSection
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
        }
        .frame(width: Self.panelWidth)
        .background(panelBackground)
    }

    private var panelSections: some View {
        VStack(alignment: .leading, spacing: 18) {
            // Inset to line up with the row contents below.
            introSection
                .padding(.horizontal, 6)

            if !companionManager.hasCompletedOnboarding && companionManager.allPermissionsGranted {
                startButton
            }

            modelSection
            PanelVoiceSection(localTTSClient: companionManager.localTTSClient)
            listeningSection
            permissionsSection
            generalSection
        }
        .padding(16)
    }

    private var panelDivider: some View {
        Divider()
            .background(DS.Colors.borderSubtle)
            .padding(.horizontal, 16)
    }

    // MARK: - Header

    private var panelHeader: some View {
        HStack {
            HStack(spacing: 8) {
                Circle()
                    .fill(statusDotColor)
                    .frame(width: 8, height: 8)
                    .shadow(color: statusDotColor.opacity(0.6), radius: 4)

                Text("hellobhai")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundColor(DS.Colors.textPrimary)
            }

            Spacer()

            Text(statusText)
                .font(.system(size: 12, weight: .medium))
                .foregroundColor(DS.Colors.textTertiary)

            Button(action: {
                NotificationCenter.default.post(name: .hellobhaiDismissPanel, object: nil)
            }) {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(DS.Colors.textTertiary)
                    .frame(width: 20, height: 20)
                    .background(
                        Circle()
                            .fill(Color.white.opacity(0.08))
                    )
            }
            .buttonStyle(.plain)
            .pointerCursor()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
    }

    // MARK: - Intro

    @ViewBuilder
    private var introSection: some View {
        if companionManager.hasCompletedOnboarding && companionManager.allPermissionsGranted {
            HStack(spacing: 5) {
                Text("Hold")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundColor(DS.Colors.textSecondary)
                PanelKeyCapsules(keyLabels: selectedShortcutOption.keyCapsuleLabels)
                Text("to talk.")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundColor(DS.Colors.textSecondary)
            }
        } else if companionManager.allPermissionsGranted {
            Text("You're all set. Hit Start to meet hellobhai.")
                .font(.system(size: 12, weight: .medium))
                .foregroundColor(DS.Colors.textSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        } else if companionManager.hasCompletedOnboarding {
            // Permissions were revoked after onboarding — tell user to re-grant
            VStack(alignment: .leading, spacing: 6) {
                Text("Permissions needed")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundColor(DS.Colors.textSecondary)

                Text("Some permissions were revoked. Grant all four below to keep using hellobhai.")
                    .font(.system(size: 11))
                    .foregroundColor(DS.Colors.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            VStack(alignment: .leading, spacing: 6) {
                Text("Hey, this is hellobhai.")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundColor(DS.Colors.textSecondary)

                Text("A local-first buddy that sees your screen, talks back, and points at stuff. Runs on your Mac with claude -p.")
                    .font(.system(size: 11))
                    .foregroundColor(DS.Colors.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)

                Text("Nothing runs in the background. hellobhai only takes a screenshot when you hold the hotkey, and speech is transcribed on-device.")
                    .font(.system(size: 11))
                    .foregroundColor(Color(red: 0.9, green: 0.4, blue: 0.4))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var startButton: some View {
        Button(action: {
            companionManager.triggerOnboarding()
        }) {
            Text("Start")
                .font(.system(size: 14, weight: .semibold))
                .foregroundColor(DS.Colors.textOnAccent)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 10)
                .background(
                    RoundedRectangle(cornerRadius: DS.CornerRadius.large, style: .continuous)
                        .fill(DS.Colors.accent)
                )
        }
        .buttonStyle(.plain)
        .pointerCursor()
    }

    // MARK: - Model

    private var modelSection: some View {
        VStack(alignment: .leading, spacing: 2) {
            PanelSectionLabel(title: "Model")

            PanelSettingsRow(iconName: "sparkles", label: "Claude") {
                PanelSegmentedControl(
                    options: Self.claudeModelOptions.map { (label: $0.label, optionID: $0.modelAlias) },
                    selectedOptionID: companionManager.selectedModel,
                    onSelectOption: { selectedModelAlias in
                        companionManager.setSelectedModel(selectedModelAlias)
                    }
                )
            }
        }
    }

    // MARK: - Listening

    private var listeningSection: some View {
        VStack(alignment: .leading, spacing: 2) {
            PanelSectionLabel(title: "Listening")

            PanelSettingsRow(iconName: "mic", label: "Speech to Text") {
                // The Whisper model loads in the background with no published
                // state, so poll it while the panel is open.
                TimelineView(.periodic(from: .now, by: 2)) { _ in
                    if WhisperTranscriptionProvider.isWhisperModelLoaded {
                        PanelStatusLabel(statusText: "Whisper large-v3", statusColor: DS.Colors.accentText)
                    } else {
                        PanelStatusLabel(statusText: "Loading Whisper", statusColor: DS.Colors.warning)
                    }
                }
            }

            PanelSettingsRow(iconName: "keyboard", label: "Shortcut") {
                Menu {
                    ForEach(BuddyPushToTalkShortcut.ShortcutOption.allCases, id: \.rawValue) { shortcutOption in
                        Button(shortcutOption.displayText) {
                            selectedShortcutOptionRawValue = shortcutOption.rawValue
                        }
                    }
                } label: {
                    HStack(spacing: 4) {
                        PanelKeyCapsules(keyLabels: selectedShortcutOption.keyCapsuleLabels)
                        Image(systemName: "chevron.up.chevron.down")
                            .font(.system(size: 8, weight: .semibold))
                            .foregroundColor(DS.Colors.textTertiary)
                    }
                }
                .menuStyle(.button)
                .buttonStyle(.plain)
                .fixedSize()
                .pointerCursor()
            }
        }
    }

    // MARK: - Permissions

    private var numberOfPermissionsGranted: Int {
        [
            companionManager.hasMicrophonePermission,
            companionManager.hasAccessibilityPermission,
            companionManager.hasScreenRecordingPermission,
            companionManager.hasScreenContentPermission,
        ].filter { $0 }.count
    }

    private var permissionsSection: some View {
        VStack(alignment: .leading, spacing: 2) {
            PanelSectionLabel(title: "Permissions")

            if companionManager.allPermissionsGranted {
                Button(action: {
                    withAnimation(.easeOut(duration: DS.Animation.fast)) {
                        isPermissionListExpanded.toggle()
                    }
                }) {
                    PanelSettingsRow(iconName: "checkmark.shield", label: "All 4 granted") {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundColor(DS.Colors.textTertiary)
                            .rotationEffect(.degrees(isPermissionListExpanded ? 90 : 0))
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .panelRowHoverHighlight()
                .pointerCursor()
            }

            if !companionManager.allPermissionsGranted || isPermissionListExpanded {
                microphonePermissionRow
                accessibilityPermissionRow
                screenRecordingPermissionRow
                // The screen content picker only works once screen recording is granted.
                if companionManager.hasScreenRecordingPermission {
                    screenContentPermissionRow
                }
            }
        }
    }

    private var microphonePermissionRow: some View {
        PanelPermissionRow(
            iconName: "mic",
            label: "Microphone",
            isGranted: companionManager.hasMicrophonePermission,
            grantAction: {
                // Triggers the native macOS microphone permission dialog on
                // first attempt. If already denied, opens System Settings.
                let microphoneAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .audio)
                if microphoneAuthorizationStatus == .notDetermined {
                    AVCaptureDevice.requestAccess(for: .audio) { _ in }
                } else if let microphoneSettingsURL = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                    NSWorkspace.shared.open(microphoneSettingsURL)
                }
            }
        )
    }

    private var accessibilityPermissionRow: some View {
        PanelPermissionRow(
            iconName: "hand.raised",
            label: "Accessibility",
            isGranted: companionManager.hasAccessibilityPermission,
            grantAction: {
                // Triggers the system accessibility prompt (AXIsProcessTrustedWithOptions)
                // on first attempt, then opens System Settings on subsequent attempts.
                WindowPositionManager.requestAccessibilityPermission()
            },
            findAppAction: {
                // Reveals the app in Finder so the user can drag it into
                // the Accessibility list if it doesn't appear automatically
                // (common with unsigned dev builds).
                WindowPositionManager.revealAppInFinder()
                WindowPositionManager.openAccessibilitySettings()
            }
        )
    }

    private var screenRecordingPermissionRow: some View {
        let isGranted = companionManager.hasScreenRecordingPermission
        return PanelPermissionRow(
            iconName: "rectangle.dashed.badge.record",
            label: "Screen Recording",
            subtitle: isGranted
                ? "Only takes a screenshot when you use the hotkey"
                : "Quit and reopen after granting",
            isGranted: isGranted,
            grantAction: {
                // Triggers the native macOS screen recording prompt on first
                // attempt (auto-adds app to the list), then opens System Settings
                // on subsequent attempts.
                WindowPositionManager.requestScreenRecordingPermission()
            }
        )
    }

    private var screenContentPermissionRow: some View {
        PanelPermissionRow(
            iconName: "eye",
            label: "Screen Content",
            isGranted: companionManager.hasScreenContentPermission,
            grantAction: {
                companionManager.requestScreenContentPermission()
            }
        )
    }

    // MARK: - General

    private var generalSection: some View {
        VStack(alignment: .leading, spacing: 2) {
            PanelSectionLabel(title: "General")

            PanelSettingsRow(iconName: "power.circle", label: "Launch at login") {
                Toggle("", isOn: Binding(
                    get: { isLaunchAtLoginEnabled },
                    set: { shouldLaunchAtLogin in
                        setLaunchAtLogin(shouldLaunchAtLogin)
                    }
                ))
                .toggleStyle(.switch)
                .tint(DS.Colors.accent)
                .scaleEffect(0.8)
                .labelsHidden()
                .pointerCursor()
            }
            .onAppear {
                // The user may have changed it in System Settings since last time.
                isLaunchAtLoginEnabled = SMAppService.mainApp.status == .enabled
            }

            if companionManager.hasCompletedOnboarding {
                Button(action: {
                    companionManager.replayOnboarding()
                }) {
                    PanelSettingsRow(iconName: "play.circle", label: "Replay onboarding") {
                        Image(systemName: "arrow.counterclockwise")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundColor(DS.Colors.textTertiary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .panelRowHoverHighlight()
                .pointerCursor()
            }
        }
    }

    private func setLaunchAtLogin(_ shouldLaunchAtLogin: Bool) {
        do {
            if shouldLaunchAtLogin {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            print("⚠️ hellobhai: Failed to change launch at login: \(error)")
        }
        isLaunchAtLoginEnabled = SMAppService.mainApp.status == .enabled
    }

    // MARK: - Footer

    private var footerSection: some View {
        HStack {
            Button(action: {
                NSApp.terminate(nil)
            }) {
                HStack(spacing: 6) {
                    Image(systemName: "power")
                        .font(.system(size: 11, weight: .medium))
                    Text("Quit hellobhai")
                        .font(.system(size: 12, weight: .medium))
                }
                .foregroundColor(DS.Colors.textTertiary)
            }
            .buttonStyle(.plain)
            .pointerCursor()

            Spacer()

            if let appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String {
                Text("v\(appVersion)")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(DS.Colors.textTertiary)
            }
        }
    }

    // MARK: - Visual Helpers

    private var panelBackground: some View {
        RoundedRectangle(cornerRadius: 12, style: .continuous)
            .fill(DS.Colors.background)
            .shadow(color: Color.black.opacity(0.5), radius: 20, x: 0, y: 10)
            .shadow(color: Color.black.opacity(0.3), radius: 4, x: 0, y: 2)
    }

    private var statusDotColor: Color {
        if !companionManager.isOverlayVisible {
            return DS.Colors.textTertiary
        }
        switch companionManager.voiceState {
        case .idle:
            return DS.Colors.accentText
        case .listening:
            return DS.Colors.blue400
        case .processing, .responding:
            return DS.Colors.blue400
        }
    }

    private var statusText: String {
        if !companionManager.hasCompletedOnboarding || !companionManager.allPermissionsGranted {
            return "Setup"
        }
        if !companionManager.isOverlayVisible {
            return "Ready"
        }
        switch companionManager.voiceState {
        case .idle:
            return "Active"
        case .listening:
            return "Listening"
        case .processing:
            return "Processing"
        case .responding:
            return "Responding"
        }
    }
}

// MARK: - Voice Section

/// Its own view so it can observe LocalTTSClient directly: engine load
/// states change in the background without CompanionManager publishing.
private struct PanelVoiceSection: View {
    @ObservedObject var localTTSClient: LocalTTSClient

    private static let englishPreviewSentence = "Hi, I'm hellobhai. Hold the shortcut and ask me anything."
    private static let hindiPreviewSentence = "नमस्ते, मैं hellobhai हूँ। शॉर्टकट दबाकर कुछ भी पूछो।"

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            PanelSectionLabel(title: "Voice")

            PanelSettingsRow(iconName: "waveform", label: "Engine") {
                PanelSegmentedControl(
                    options: TTSEngine.allCases.map { (label: $0.displayName, optionID: $0.rawValue) },
                    selectedOptionID: localTTSClient.selectedEngine.rawValue,
                    onSelectOption: { selectedEngineRawValue in
                        if let selectedEngine = TTSEngine(rawValue: selectedEngineRawValue) {
                            localTTSClient.setSelectedEngine(selectedEngine)
                        }
                    }
                )
            }

            if localTTSClient.selectedEngine == .kokoro {
                PanelSettingsRow(iconName: "person.wave.2", label: "English voice") {
                    HStack(spacing: 6) {
                        PanelMenuPicker(
                            selectedTitle: Self.kokoroVoiceDisplayName(localTTSClient.selectedKokoroVoice),
                            options: LocalTTSClient.availableKokoroVoices.map { kokoroVoice in
                                (title: Self.kokoroVoiceDisplayName(kokoroVoice), optionID: kokoroVoice)
                            },
                            onSelectOption: { selectedKokoroVoice in
                                localTTSClient.setSelectedKokoroVoice(selectedKokoroVoice)
                            }
                        )
                        PanelPreviewButton {
                            localTTSClient.speakText(Self.englishPreviewSentence)
                        }
                    }
                }
            }

            // Supertonic-3 is the only engine that speaks Hindi, so with
            // Kokoro selected this voice is still used for Hindi sentences.
            PanelSettingsRow(
                iconName: "person.wave.2",
                label: localTTSClient.selectedEngine == .kokoro ? "Hindi voice" : "Voice"
            ) {
                HStack(spacing: 6) {
                    PanelMenuPicker(
                        selectedTitle: localTTSClient.selectedSupertonicVoice.rawValue,
                        options: Supertonic3Voice.allCases.map { supertonicVoice in
                            (title: supertonicVoice.rawValue, optionID: supertonicVoice.rawValue)
                        },
                        onSelectOption: { selectedSupertonicVoiceRawValue in
                            if let selectedSupertonicVoice = Supertonic3Voice(rawValue: selectedSupertonicVoiceRawValue) {
                                localTTSClient.setSelectedSupertonicVoice(selectedSupertonicVoice)
                            }
                        }
                    )
                    PanelPreviewButton {
                        localTTSClient.speakText(
                            localTTSClient.selectedEngine == .kokoro ? Self.hindiPreviewSentence : Self.englishPreviewSentence
                        )
                    }
                }
            }

            PanelSettingsRow(iconName: "cpu", label: "Status") {
                PanelStatusLabel(statusText: engineStatus.text, statusColor: engineStatus.color)
            }
        }
    }

    /// What will actually speak the next English sentence.
    private var engineStatus: (text: String, color: Color) {
        let supertonicStatus: (text: String, color: Color)
        switch localTTSClient.supertonicLoadState {
        case .ready:
            supertonicStatus = ("Supertonic-3", DS.Colors.accentText)
        case .failed:
            supertonicStatus = ("System voice", DS.Colors.warning)
        case .loading, .notLoaded:
            supertonicStatus = ("Loading Supertonic-3", DS.Colors.warning)
        }

        guard localTTSClient.selectedEngine == .kokoro else { return supertonicStatus }

        switch localTTSClient.kokoroLoadState {
        case .ready:
            return ("Kokoro-82M", DS.Colors.accentText)
        case .loading, .notLoaded:
            return ("Loading Kokoro, using \(supertonicStatus.text)", DS.Colors.warning)
        case .failed:
            return ("Kokoro failed, using \(supertonicStatus.text)", DS.Colors.warning)
        }
    }

    /// "af_heart" -> "Heart (US, F)". The prefix encodes accent (a = US,
    /// b = UK) and gender (f / m).
    private static func kokoroVoiceDisplayName(_ kokoroVoice: String) -> String {
        let voiceNameParts = kokoroVoice.split(separator: "_", maxSplits: 1)
        guard voiceNameParts.count == 2, voiceNameParts[0].count == 2 else { return kokoroVoice }
        let accentLabel = voiceNameParts[0].first == "b" ? "UK" : "US"
        let genderLabel = voiceNameParts[0].last == "m" ? "M" : "F"
        return "\(voiceNameParts[1].capitalized) (\(accentLabel), \(genderLabel))"
    }
}

// MARK: - Shared Panel Components

private struct PanelSectionLabel: View {
    let title: String

    var body: some View {
        Text(title.uppercased())
            .font(.system(size: 10, weight: .semibold, design: .rounded))
            .foregroundColor(DS.Colors.textTertiary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, 6)
            .padding(.bottom, 4)
    }
}

/// Icon + label on the left, any control on the right.
private struct PanelSettingsRow<TrailingContent: View>: View {
    let iconName: String
    let label: String
    var subtitle: String? = nil
    var iconColor: Color = DS.Colors.textTertiary
    @ViewBuilder let trailingContent: () -> TrailingContent

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: iconName)
                .font(.system(size: 12, weight: .medium))
                .foregroundColor(iconColor)
                .frame(width: 16)

            VStack(alignment: .leading, spacing: 1) {
                Text(label)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundColor(DS.Colors.textSecondary)

                if let subtitle {
                    Text(subtitle)
                        .font(.system(size: 10))
                        .foregroundColor(DS.Colors.textTertiary)
                }
            }

            Spacer(minLength: 8)

            trailingContent()
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 6)
        .frame(minHeight: 30)
    }
}

private struct PanelPermissionRow: View {
    let iconName: String
    let label: String
    var subtitle: String? = nil
    let isGranted: Bool
    let grantAction: () -> Void
    var findAppAction: (() -> Void)? = nil

    var body: some View {
        PanelSettingsRow(
            iconName: iconName,
            label: label,
            subtitle: subtitle,
            iconColor: isGranted ? DS.Colors.textTertiary : DS.Colors.warning
        ) {
            if isGranted {
                PanelStatusLabel(statusText: "Granted", statusColor: DS.Colors.accentText, textColor: DS.Colors.accentText)
            } else {
                HStack(spacing: 6) {
                    Button(action: grantAction) {
                        Text("Grant")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundColor(DS.Colors.textOnAccent)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                            .background(Capsule().fill(DS.Colors.accent))
                    }
                    .buttonStyle(.plain)
                    .pointerCursor()

                    if let findAppAction {
                        Button(action: findAppAction) {
                            Text("Find App")
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundColor(DS.Colors.textSecondary)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 4)
                                .background(Capsule().stroke(DS.Colors.borderSubtle, lineWidth: 0.8))
                        }
                        .buttonStyle(.plain)
                        .pointerCursor()
                    }
                }
            }
        }
    }
}

/// A 6pt dot next to short status text, e.g. "Granted" or "Kokoro-82M".
private struct PanelStatusLabel: View {
    let statusText: String
    let statusColor: Color
    var textColor: Color = DS.Colors.textTertiary

    var body: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(statusColor)
                .frame(width: 6, height: 6)
            Text(statusText)
                .font(.system(size: 11, weight: .medium))
                .foregroundColor(textColor)
                .lineLimit(1)
        }
    }
}

private struct PanelKeyCapsules: View {
    let keyLabels: [String]

    var body: some View {
        HStack(spacing: 3) {
            ForEach(keyLabels, id: \.self) { keyLabel in
                Text(keyLabel)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundColor(DS.Colors.textPrimary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(
                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                            .fill(Color.white.opacity(0.08))
                    )
            }
        }
    }
}

private struct PanelSegmentedControl: View {
    let options: [(label: String, optionID: String)]
    let selectedOptionID: String
    let onSelectOption: (String) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(options, id: \.optionID) { option in
                PanelSegmentButton(
                    label: option.label,
                    isSelected: option.optionID == selectedOptionID,
                    action: { onSelectOption(option.optionID) }
                )
            }
        }
        .padding(1)
        .background(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(Color.white.opacity(0.06))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 6, style: .continuous)
                .stroke(DS.Colors.borderSubtle, lineWidth: 0.5)
        )
    }
}

private struct PanelSegmentButton: View {
    let label: String
    let isSelected: Bool
    let action: () -> Void

    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.system(size: 11, weight: .medium))
                .foregroundColor(isSelected ? DS.Colors.textPrimary : (isHovered ? DS.Colors.textSecondary : DS.Colors.textTertiary))
                .padding(.horizontal, 9)
                .padding(.vertical, 5)
                .background(
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .fill(isSelected ? DS.Colors.accent : Color.clear)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .pointerCursor()
    }
}

/// A compact dropdown: current value + chevron, native menu on click.
private struct PanelMenuPicker: View {
    let selectedTitle: String
    let options: [(title: String, optionID: String)]
    let onSelectOption: (String) -> Void

    @State private var isHovered = false

    var body: some View {
        Menu {
            ForEach(options, id: \.optionID) { option in
                Button(option.title) { onSelectOption(option.optionID) }
            }
        } label: {
            HStack(spacing: 4) {
                Text(selectedTitle)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(DS.Colors.textPrimary)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.system(size: 8, weight: .semibold))
                    .foregroundColor(DS.Colors.textTertiary)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(Color.white.opacity(isHovered ? 0.10 : 0.06))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .stroke(DS.Colors.borderSubtle, lineWidth: 0.5)
            )
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .fixedSize()
        .onHover { isHovered = $0 }
        .pointerCursor()
    }
}

private struct PanelPreviewButton: View {
    let action: () -> Void

    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            Image(systemName: "play.fill")
                .font(.system(size: 8, weight: .semibold))
                .foregroundColor(isHovered ? DS.Colors.textPrimary : DS.Colors.textSecondary)
                .frame(width: 22, height: 22)
                .background(
                    Circle()
                        .fill(Color.white.opacity(isHovered ? 0.12 : 0.08))
                )
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .help("Preview voice")
        .pointerCursor()
    }
}

/// White 4% fill on hover, for rows that are buttons, so they read as clickable.
private struct PanelRowHoverHighlight: ViewModifier {
    @State private var isHovered = false

    func body(content: Content) -> some View {
        content
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(Color.white.opacity(isHovered ? 0.04 : 0))
            )
            .onHover { isHovered = $0 }
    }
}

private extension View {
    func panelRowHoverHighlight() -> some View {
        modifier(PanelRowHoverHighlight())
    }
}
