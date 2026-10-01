import Foundation

public enum NativeSendGateBlocker: String, Codable, CaseIterable {
    case captureUnavailable = "CAPTURE_UNAVAILABLE"
    case appNotFrontmost = "APP_NOT_FRONTMOST"
    case unstableSurface = "UNSTABLE_SURFACE"
    case applicationSessionMismatch = "APPLICATION_SESSION_MISMATCH"
    case conversationUnresolved = "CONVERSATION_UNRESOLVED"
    case conversationMismatch = "CONVERSATION_MISMATCH"
    case draftMismatch = "DRAFT_MISMATCH"
    case recentUserInput = "RECENT_USER_INPUT"
    case composingUnverified = "COMPOSING_UNVERIFIED"
}

public struct NativeSendGateFacts: Equatable {
    public var captureReady: Bool
    public var frontmost: Bool
    public var stableTwoReads: Bool
    public var applicationSessionMatches: Bool
    public var conversationResolved: Bool
    public var conversationMatches: Bool
    public var draftMatches: Bool
    public var recentUserInput: Bool
    public var composingVerifiedSafe: Bool

    public init(
        captureReady: Bool,
        frontmost: Bool,
        stableTwoReads: Bool,
        applicationSessionMatches: Bool,
        conversationResolved: Bool,
        conversationMatches: Bool,
        draftMatches: Bool,
        recentUserInput: Bool,
        composingVerifiedSafe: Bool
    ) {
        self.captureReady = captureReady
        self.frontmost = frontmost
        self.stableTwoReads = stableTwoReads
        self.applicationSessionMatches = applicationSessionMatches
        self.conversationResolved = conversationResolved
        self.conversationMatches = conversationMatches
        self.draftMatches = draftMatches
        self.recentUserInput = recentUserInput
        self.composingVerifiedSafe = composingVerifiedSafe
    }
}

public struct NativeSendGateDecision: Equatable {
    public let ready: Bool
    public let blockers: [NativeSendGateBlocker]
}

public enum NativeSendGatePolicy {
    public static func evaluate(_ facts: NativeSendGateFacts) -> NativeSendGateDecision {
        var blockers: [NativeSendGateBlocker] = []
        if !facts.captureReady { blockers.append(.captureUnavailable) }
        if !facts.frontmost { blockers.append(.appNotFrontmost) }
        if !facts.stableTwoReads { blockers.append(.unstableSurface) }
        if !facts.applicationSessionMatches { blockers.append(.applicationSessionMismatch) }
        if !facts.conversationResolved { blockers.append(.conversationUnresolved) }
        if !facts.conversationMatches { blockers.append(.conversationMismatch) }
        if !facts.draftMatches { blockers.append(.draftMismatch) }
        if facts.recentUserInput { blockers.append(.recentUserInput) }
        if !facts.composingVerifiedSafe { blockers.append(.composingUnverified) }
        return NativeSendGateDecision(ready: blockers.isEmpty, blockers: blockers)
    }
}
