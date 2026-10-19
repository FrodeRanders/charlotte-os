-------------------------- MODULE CharlotteBootTrust --------------------------
\* Existing verifier/fixture/handoff, plus a separately configured conditional
\* protected-installation contract. No protected installer exists in Rust yet.
EXTENDS Naturals, FiniteSets
CONSTANTS Modes, ProtectedBoundary, MaxBoots
ASSUME Modes \subseteq {"Fixture", "Development", "Contract"} /\ Modes # {}
ASSUME ProtectedBoundary \in BOOLEAN
ASSUME MaxBoots >= 2

Records == {"A", "B", "C", "D", "Gap", "Foreign", "Attacker", "Malformed"}
Authentic == {"A", "B", "C", "D", "Gap"}
PolicyIds == Records \cup {"None", "Development"}
Sequence(p) == CASE p = "A" -> 1 [] p \in {"B", "C", "D"} -> 2
    [] p = "Gap" -> 3 [] OTHER -> 0
Predecessor(p) == IF p \in {"B", "C", "Gap"} THEN "A" ELSE "None"
Accepted(p, expected) ==
    p \in Authentic /\ IF expected = "None"
    THEN Sequence(p) >= 1 /\ Predecessor(p) = "None"
    ELSE p = expected \/ (Sequence(p) = Sequence(expected) + 1 /\ Predecessor(p) = expected)

VARIABLE s
vars == <<s>>
Init == \E mode \in Modes, initial \in {"None", "A"} :
    /\ (mode = "Fixture" => initial = "A")
    /\ (mode = "Development" => initial = "None")
    /\ s = [mode |-> mode, persistent |-> initial,
        committed |-> IF initial = "None" THEN {} ELSE {initial},
        everInstalled |-> (initial # "None"), stage |-> "Idle",
        expected |-> "None", verified |-> "None", prepared |-> "None",
        recipient |-> "None", boot |-> "None", bootReceipt |-> "None",
        bootRecipient |-> "None", publications |-> 0, epoch |-> 1,
        consumer |-> "None", selected |-> "None", platformAuthorities |-> {},
        bootstrapLoaded |-> FALSE, platformLoaded |-> FALSE,
        commitFailed |-> FALSE, rejected |-> FALSE, observedFailure |-> FALSE]

CanUseProtected == s.mode = "Fixture" \/ (s.mode = "Contract" /\ ProtectedBoundary)
ReadInstalled ==
    /\ s.stage = "Idle" /\ s.persistent # "None" /\ s.mode # "Development"
    /\ s' = [s EXCEPT !.stage = "Expected", !.expected = s.persistent,
        !.commitFailed = FALSE, !.rejected = FALSE]
ReadEnrollment ==
    /\ s.stage = "Idle" /\ s.persistent = "None" /\ ~s.everInstalled
    /\ s.mode = "Contract" /\ ProtectedBoundary
    /\ s' = [s EXCEPT !.stage = "Expected", !.expected = "None",
        !.commitFailed = FALSE, !.rejected = FALSE]
ReadFailure ==
    /\ s.stage = "Idle" /\ s.persistent # "None" /\ s.mode # "Development"
    /\ s' = [s EXCEPT !.stage = "Blocked", !.observedFailure = TRUE]
Verify(p) ==
    /\ s.stage = "Expected" /\ p \in Records /\ Accepted(p, s.expected)
    /\ s' = [s EXCEPT !.stage = "Verified", !.verified = p]
RejectRecord(p) ==
    /\ s.stage = "Expected" /\ p \in Records /\ ~Accepted(p, s.expected)
    /\ s' = [s EXCEPT !.stage = "Blocked", !.rejected = TRUE]

\* Conditional, atomic rollback-resistant CAS, not an implemented file write.
CommitPolicy ==
    /\ s.selected = "None"
    /\ s.mode = "Contract" /\ ProtectedBoundary /\ s.boot = "None"
    /\ s.stage = "Verified" /\ s.persistent = s.expected /\ s.verified # s.persistent
    /\ s' = [s EXCEPT !.persistent = s.verified, !.committed = @ \cup {s.verified},
        !.everInstalled = TRUE, !.stage = "Idle", !.expected = "None",
        !.verified = "None", !.commitFailed = FALSE]
CommitFailure ==
    /\ s.selected = "None"
    /\ s.mode = "Contract" /\ ProtectedBoundary /\ s.boot = "None"
    /\ s.stage = "Verified" /\ s.persistent = s.expected /\ s.verified # s.persistent
    /\ ~s.commitFailed
    /\ s' = [s EXCEPT !.commitFailed = TRUE, !.stage = "Blocked"]
\* An interrupted conditional commit may have atomically taken effect. The
\* caller cannot publish or reuse its receipt until a fresh protected read.
CommitInterrupted(tookEffect) ==
    /\ s.selected = "None"
    /\ s.mode = "Contract" /\ ProtectedBoundary /\ s.boot = "None"
    /\ s.stage = "Verified" /\ s.persistent = s.expected /\ s.verified # s.persistent
    /\ tookEffect \in BOOLEAN
    /\ s' = [s EXCEPT !.persistent = IF tookEffect THEN s.verified ELSE s.persistent,
        !.committed = IF tookEffect THEN @ \cup {s.verified} ELSE @,
        !.everInstalled = (s.everInstalled \/ tookEffect),
        !.stage = "Blocked", !.commitFailed = TRUE]
\* Another authorized installer can win after our expectation was captured.
CompetingCommit(p) ==
    /\ s.selected = "None"
    /\ s.mode = "Contract" /\ ProtectedBoundary /\ s.boot = "None"
    /\ s.persistent = "A" /\ p \in {"B", "C"}
    /\ s' = [s EXCEPT !.persistent = p, !.committed = @ \cup {p}, !.everInstalled = TRUE]

AcquireRecipient(material) ==
    /\ s.stage = "Verified" /\ material \in {"Correct", "Wrong"}
    /\ s' = [s EXCEPT !.recipient = material, !.stage = "KeyOwned"]
PrepareSigned ==
    /\ s.stage = "KeyOwned" /\ s.recipient = "Correct" /\ CanUseProtected
    /\ s.verified = s.persistent /\ s.expected = s.persistent
    /\ s' = [s EXCEPT !.prepared = s.verified, !.stage = "Prepared"]
RejectRecipient ==
    /\ s.stage = "KeyOwned" /\ s.recipient = "Wrong"
    /\ s' = [s EXCEPT !.recipient = "None", !.stage = "Blocked", !.rejected = TRUE]
PrepareDevelopment ==
    /\ s.mode = "Development" /\ s.stage = "Idle"
    /\ s' = [s EXCEPT !.stage = "Prepared", !.prepared = "Development", !.recipient = "Correct"]

\* Public artifact authority is selected once, before the bootstrap name
\* service can load. Key custody stays in the existing preparation owner.
SelectAuthority ==
    /\ s.stage = "Prepared" /\ s.selected = "None"
    /\ s.prepared = s.persistent \/ (s.mode = "Development" /\ s.prepared = "Development")
    /\ s' = [s EXCEPT !.selected = s.prepared]
LoadBootstrap ==
    /\ s.selected # "None" /\ ~s.bootstrapLoaded
    /\ s' = [s EXCEPT !.bootstrapLoaded = TRUE,
        !.platformAuthorities = @ \cup {s.selected}]
LoadPlatform ==
    /\ s.boot # "None" /\ ~s.platformLoaded
    /\ s' = [s EXCEPT !.platformLoaded = TRUE,
        !.platformAuthorities = @ \cup {s.selected}]

PublishMutation(policy) ==
    s' = [s EXCEPT !.boot = policy, !.bootReceipt = s.persistent,
        !.bootRecipient = s.recipient, !.publications = @ + 1,
        !.prepared = "None", !.verified = "None", !.recipient = "None", !.stage = "Idle"]
Publish ==
    /\ s.stage = "Prepared" /\ s.boot = "None"
    /\ s.selected = s.prepared /\ s.bootstrapLoaded
    /\ IF s.mode = "Development" THEN s.prepared = "Development"
       ELSE CanUseProtected /\ s.prepared = s.persistent
    /\ PublishMutation(s.prepared)
RejectPublication ==
    /\ s.stage = "Prepared" /\ ~s.rejected
    /\ s.boot # "None" \/ (s.prepared # "Development" /\ s.prepared # s.persistent)
    /\ s' = [s EXCEPT !.rejected = TRUE]
BorrowView ==
    /\ s.boot # "None" /\ s.consumer = "None"
    /\ s' = [s EXCEPT !.consumer = s.boot]
DropPreparation ==
    /\ s.stage \in {"KeyOwned", "Prepared", "Blocked", "Verified", "Expected"}
    /\ s' = [s EXCEPT !.prepared = "None", !.verified = "None", !.recipient = "None",
        !.expected = "None", !.stage = "Idle", !.rejected = FALSE]
\* Abstract reset of volatile state; actual firmware, RAM erasure and custody
\* reacquisition are outside this model. The committed record survives reset.
Reboot ==
    /\ s.epoch < MaxBoots
    /\ s' = [s EXCEPT !.stage = "Idle", !.expected = "None", !.verified = "None",
        !.prepared = "None", !.recipient = "None", !.boot = "None",
        !.bootReceipt = "None", !.bootRecipient = "None", !.publications = 0,
        !.consumer = "None", !.selected = "None", !.platformAuthorities = {},
        !.bootstrapLoaded = FALSE, !.platformLoaded = FALSE, !.epoch = @ + 1,
        !.commitFailed = FALSE, !.rejected = FALSE]

Next ==
    \/ ReadInstalled \/ ReadEnrollment \/ ReadFailure
    \/ (\E p \in Records : Verify(p) \/ RejectRecord(p))
    \/ CommitPolicy \/ CommitFailure
    \/ (\E b \in BOOLEAN : CommitInterrupted(b))
    \/ (\E p \in {"B", "C"} : CompetingCommit(p))
    \/ (\E k \in {"Correct", "Wrong"} : AcquireRecipient(k))
    \/ PrepareSigned \/ RejectRecipient \/ PrepareDevelopment
    \/ SelectAuthority \/ LoadBootstrap \/ LoadPlatform \/ Publish
    \/ RejectPublication \/ BorrowView \/ DropPreparation \/ Reboot
Spec == Init /\ [][Next]_vars

UnsafeUninstalledPublish ==
    /\ s.stage = "KeyOwned" /\ s.recipient = "Correct" /\ s.boot = "None"
    /\ s.verified # s.persistent /\ PublishMutation(s.verified)
UnsafeStaleCommit ==
    /\ s.mode = "Contract" /\ s.stage = "Verified" /\ s.boot = "None"
    /\ s.expected = "A" /\ s.persistent \in {"B", "C"}
    /\ s.verified \in {"B", "C"} \ {s.persistent}
    /\ s' = [s EXCEPT !.persistent = s.verified, !.committed = @ \cup {s.verified}]
UnsafeEnrollmentFallback ==
    /\ s.stage = "Blocked" /\ s.observedFailure /\ s.everInstalled
    /\ s' = [s EXCEPT !.stage = "Expected", !.expected = "None"]
UnsafeOverwrite ==
    /\ s.stage = "Prepared" /\ s.boot # "None" /\ PublishMutation(s.prepared)
UnsafeIndependentManifest ==
    /\ s.boot # "None"
    /\ s' = [s EXCEPT !.consumer = IF s.boot = "A" THEN "B" ELSE "A"]
UnsafeWrongRecipient ==
    /\ s.stage = "KeyOwned" /\ s.recipient = "Wrong" /\ s.verified = s.persistent
    /\ s' = [s EXCEPT !.prepared = s.verified, !.stage = "Prepared"]
UnsafeAuthenticate ==
    /\ s.stage = "Expected"
    /\ s' = [s EXCEPT !.verified = "Attacker", !.stage = "Verified"]
UnsafeDevelopmentFallback ==
    /\ s.mode # "Development" /\ s.stage = "Blocked" /\ s.boot = "None"
    /\ s' = [s EXCEPT !.recipient = "Correct", !.prepared = "Development", !.stage = "Prepared"]
UnsafeFailedCommitPublishes ==
    /\ s.mode = "Contract" /\ s.commitFailed /\ s.stage = "Blocked" /\ s.boot = "None"
    /\ s.verified # s.persistent
    /\ s' = [s EXCEPT !.boot = s.verified, !.bootReceipt = s.persistent,
        !.bootRecipient = "Correct", !.publications = @ + 1]
UnsafeUnsupportedProduction ==
    /\ s.mode = "Contract" /\ ~ProtectedBoundary /\ s.stage = "KeyOwned"
    /\ s.recipient = "Correct" /\ s.boot = "None" /\ s.verified = s.persistent
    /\ PublishMutation(s.verified)

UnsafePlatformFallback ==
    /\ s.selected \notin {"None", "Development"}
    /\ s' = [s EXCEPT !.platformAuthorities = @ \cup {"Development"}]
UnsafeSelectionOverwrite ==
    /\ s.selected = "A"
    /\ s' = [s EXCEPT !.selected = "B"]

UnsafeUninstalledSpec == Init /\ [][Next \/ UnsafeUninstalledPublish]_vars
UnsafeStaleSpec == Init /\ [][Next \/ UnsafeStaleCommit]_vars
UnsafeEnrollmentSpec == Init /\ [][Next \/ UnsafeEnrollmentFallback]_vars
UnsafeOverwriteSpec == Init /\ [][Next \/ UnsafeOverwrite]_vars
UnsafeManifestSpec == Init /\ [][Next \/ UnsafeIndependentManifest]_vars
UnsafeRecipientSpec == Init /\ [][Next \/ UnsafeWrongRecipient]_vars
UnsafeAuthenticateSpec == Init /\ [][Next \/ UnsafeAuthenticate]_vars
UnsafeDevelopmentSpec == Init /\ [][Next \/ UnsafeDevelopmentFallback]_vars
UnsafeFailureSpec == Init /\ [][Next \/ UnsafeFailedCommitPublishes]_vars
UnsafeBoundarySpec == Init /\ [][Next \/ UnsafeUnsupportedProduction]_vars
UnsafePlatformSpec == Init /\ [][Next \/ UnsafePlatformFallback]_vars
UnsafeSelectionSpec == Init /\ [][Next \/ UnsafeSelectionOverwrite]_vars

TypeOK ==
    /\ s.mode \in Modes /\ s.persistent \in Authentic \cup {"None"}
    /\ s.committed \subseteq Authentic /\ s.epoch \in 1..MaxBoots
    /\ s.stage \in {"Idle", "Expected", "Blocked", "Verified", "KeyOwned", "Prepared"}
    /\ s.expected \in Authentic \cup {"None"} /\ s.verified \in Records \cup {"None"}
    /\ s.prepared \in PolicyIds /\ s.boot \in PolicyIds /\ s.consumer \in PolicyIds
    /\ s.selected \in PolicyIds /\ s.platformAuthorities \subseteq PolicyIds
    /\ s.bootstrapLoaded \in BOOLEAN /\ s.platformLoaded \in BOOLEAN
    /\ s.bootReceipt \in Authentic \cup {"None"} /\ s.publications \in 0..1
    /\ s.recipient \in {"None", "Correct", "Wrong"}
    /\ s.bootRecipient \in {"None", "Correct", "Wrong"}
    /\ s.commitFailed \in BOOLEAN
    /\ s.rejected \in BOOLEAN /\ s.everInstalled \in BOOLEAN /\ s.observedFailure \in BOOLEAN
VerifiedAuthenticated == s.verified # "None" => s.verified \in Authentic
InstalledBeforePublication == s.boot \notin {"None", "Development"} =>
    s.boot = s.bootReceipt /\ s.boot \in s.committed
NoForkedInstallation == \A a,b \in s.committed : Sequence(a) = Sequence(b) => a = b
NoRollback == \A p \in s.committed : Sequence(s.persistent) >= Sequence(p)
NoEnrollmentFallback == s.stage = "Expected" /\ s.everInstalled => s.expected # "None"
OnePublicationPerBoot == s.publications <= 1
SharedPolicy == s.consumer # "None" => s.consumer = s.boot
RecipientBound == (s.prepared # "None" => s.recipient = "Correct") /\
    (s.boot # "None" => s.bootRecipient = "Correct")
ExplicitDevelopment == s.prepared = "Development" \/ s.boot = "Development" => s.mode = "Development"
ConditionalBoundary == s.mode = "Contract" /\ s.boot # "None" => ProtectedBoundary
SelectedAuthorityMatchesState == s.selected # "None" =>
    IF s.mode = "Development" THEN s.selected = "Development" ELSE s.selected = s.persistent
PlatformImagesUseSelectedAuthority == s.platformAuthorities \subseteq {s.selected}
PublicationUsesSelectedAuthority == s.boot # "None" => s.boot = s.selected /\ s.bootstrapLoaded
Invariants == TypeOK /\ VerifiedAuthenticated /\ InstalledBeforePublication
    /\ NoForkedInstallation /\ NoRollback /\ NoEnrollmentFallback /\ OnePublicationPerBoot
    /\ SharedPolicy /\ RecipientBound /\ ExplicitDevelopment /\ ConditionalBoundary
    /\ SelectedAuthorityMatchesState /\ PlatformImagesUseSelectedAuthority
    /\ PublicationUsesSelectedAuthority
=============================================================================
