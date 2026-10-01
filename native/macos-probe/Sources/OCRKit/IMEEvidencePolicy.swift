import Foundation

public enum AuthoritativeCompositionState: String, Codable, Equatable {
    case safe = "SAFE"
    case composing = "COMPOSING"
    case unavailable = "UNAVAILABLE"
}

public enum IMEEvidenceClass: String, Codable, Equatable {
    case authoritative = "AUTHORITATIVE"
    case positiveBlocker = "POSITIVE_BLOCKER"
    case contextOnly = "CONTEXT_ONLY"
    case unavailable = "UNAVAILABLE"
}

public struct IMEEvidenceFacts: Equatable {
    public var authoritativeState: AuthoritativeCompositionState
    public var appFocusedUIAvailable: Bool
    public var systemFocusedUIAvailable: Bool
    public var selectedTextRangeAvailable: Bool
    public var inputSourceAvailable: Bool
    public var inputMethodProcessCount: Int
    public var inputMethodWindowCount: Int
    public var inputMethodOnScreenWindowCount: Int
    public var recentUserInput: Bool
    public var targetFrontmost: Bool

    public init(
        authoritativeState: AuthoritativeCompositionState,
        appFocusedUIAvailable: Bool,
        systemFocusedUIAvailable: Bool,
        selectedTextRangeAvailable: Bool,
        inputSourceAvailable: Bool,
        inputMethodProcessCount: Int,
        inputMethodWindowCount: Int,
        inputMethodOnScreenWindowCount: Int,
        recentUserInput: Bool,
        targetFrontmost: Bool
    ) {
        self.authoritativeState = authoritativeState
        self.appFocusedUIAvailable = appFocusedUIAvailable
        self.systemFocusedUIAvailable = systemFocusedUIAvailable
        self.selectedTextRangeAvailable = selectedTextRangeAvailable
        self.inputSourceAvailable = inputSourceAvailable
        self.inputMethodProcessCount = inputMethodProcessCount
        self.inputMethodWindowCount = inputMethodWindowCount
        self.inputMethodOnScreenWindowCount = inputMethodOnScreenWindowCount
        self.recentUserInput = recentUserInput
        self.targetFrontmost = targetFrontmost
    }
}

public struct IMEEvidenceDecision: Equatable {
    public let compositionVerified: Bool
    public let composing: Bool
    public let positiveBlockers: [String]
}

public enum IMEEvidencePolicy {
    public static func evaluate(_ facts: IMEEvidenceFacts) -> IMEEvidenceDecision {
        var blockers: [String] = []
        if facts.inputMethodOnScreenWindowCount > 0 {
            blockers.append("INPUT_METHOD_WINDOW_VISIBLE")
        }
        if facts.recentUserInput {
            blockers.append("RECENT_USER_INPUT")
        }
        switch facts.authoritativeState {
        case .safe:
            return IMEEvidenceDecision(
                compositionVerified: true,
                composing: false,
                positiveBlockers: blockers
            )
        case .composing:
            blockers.append("AUTHORITATIVE_COMPOSING")
            return IMEEvidenceDecision(
                compositionVerified: true,
                composing: true,
                positiveBlockers: blockers
            )
        case .unavailable:
            return IMEEvidenceDecision(
                compositionVerified: false,
                composing: false,
                positiveBlockers: blockers
            )
        }
    }
}
