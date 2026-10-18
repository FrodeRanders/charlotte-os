---------------------- MODULE CharlotteNamePublication ----------------------
\* Serialized local NS publication. A live entry owns its connection and watch.
\* CQ hints may alias; only the current watch's own result establishes death.
EXTENDS Naturals, FiniteSets

CONSTANTS Names, Principals, NoName, NoPrincipal, LiveLimit,
          MaxCandidates, MaxGeneration
ASSUME NoName \notin Names /\ NoPrincipal \notin Principals
ASSUME Cardinality(Names) > LiveLimit /\ LiveLimit > 0
ASSUME MaxCandidates >= MaxGeneration /\ MaxGeneration >= 2

Ids == 1..MaxCandidates
Classes == {"Public", "Policy"}
Owners == {"Free", "Source", "Watched", "Live", "Detached", "WatchDisposed", "Released"}
NoRequest == [name |-> NoName, class |-> "Public", registrar |-> NoPrincipal,
    manager |-> FALSE, previous |-> 0, previousGeneration |-> 0]
NoResult == [state |-> "None", mode |-> "Public", class |-> "Public",
    generation |-> 0, policyGeneration |-> 0, authorized |-> FALSE,
    deferred |-> FALSE]

VARIABLE s
vars == <<s>>

Init == s = [phase |-> "Idle", next |-> 1, candidate |-> 0, disposal |-> 0,
    request |-> NoRequest,
    entry |-> [n \in Names |-> 0], generation |-> [n \in Names |-> 0],
    policyGeneration |-> [n \in Names |-> 0], policyLive |-> [n \in Names |-> FALSE],
    owner |-> [i \in Ids |-> "Free"], watch |-> [i \in Ids |-> FALSE],
    source |-> [i \in Ids |-> FALSE], closed |-> [i \in Ids |-> FALSE],
    class |-> [i \in Ids |-> "Public"], registrar |-> [i \in Ids |-> NoPrincipal],
    hint |-> 0, waiter |-> [state |-> "None", name |-> NoName, mode |-> "Public"],
    result |-> NoResult, retiredByDeath |-> {}, badReplacement |-> FALSE,
    badRetirement |-> FALSE, badGeneration |-> FALSE,
    aborted |-> FALSE, frozenOwners |-> [i \in Ids |-> "Free"],
    frozenWatches |-> [i \in Ids |-> FALSE], frozenSources |-> [i \in Ids |-> FALSE],
    frozenEntries |-> [n \in Names |-> 0]]

Live == {n \in Names : s.entry[n] # 0}
WatchCount == Cardinality({i \in Ids : s.watch[i]})
PublicReplacementAllowed ==
    LET old == s.request.previous IN
    IF old = 0 THEN TRUE ELSE s.request.class # "Public" \/
        (s.class[old] = "Public" /\ s.registrar[old] # NoPrincipal /\
         s.registrar[old] = s.request.registrar)
RoleAllowed == s.request.class = "Public" \/
    (s.request.manager /\ s.request.registrar # NoPrincipal)
QuotaAllowed == Cardinality(Live) < LiveLimit \/
    (s.request.previous # 0 /\ Cardinality(Live) = LiveLimit)
CanPrepare == PublicReplacementAllowed /\ QuotaAllowed
\* Deliberately stress stale CQ matching: every real publication hint may alias
\* the current watch. MaxCandidates+1 is an unrelated hint. No hint is a result.
Matches == s.hint \in Ids

Begin(name, class, registrar, manager) ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ s.next \in Ids
    /\ name \in Names /\ class \in Classes
    /\ registrar \in Principals \cup {NoPrincipal} /\ manager \in BOOLEAN
    /\ s' = [s EXCEPT !.phase = "Proposed", !.candidate = s.next,
        !.next = @ + 1, !.owner[s.next] = "Source", !.source[s.next] = TRUE,
        !.class[s.next] = class, !.registrar[s.next] = registrar,
        !.request = [name |-> name, class |-> class, registrar |-> registrar,
            manager |-> manager, previous |-> s.entry[name],
            previousGeneration |-> s.policyGeneration[name]]]

RejectBeforeWatch ==
    /\ ~s.aborted /\ s.phase = "Proposed"
    /\ s' = [s EXCEPT !.phase = "Disposing", !.disposal = s.candidate]

PrepareWatch ==
    /\ ~s.aborted /\ s.phase = "Proposed" /\ CanPrepare
    /\ WatchCount < LiveLimit + 1
    /\ s' = [s EXCEPT !.phase = "Watched", !.owner[s.candidate] = "Watched",
        !.watch[s.candidate] = TRUE]

RejectPolicy ==
    /\ ~s.aborted /\ s.phase = "Watched"
    /\ s' = [s EXCEPT !.phase = "Disposing", !.disposal = s.candidate,
        !.owner[s.candidate] = "Detached"]

PublishPolicy ==
    /\ ~s.aborted /\ s.phase = "Watched" /\ RoleAllowed
    /\ s.policyGeneration[s.request.name] < MaxGeneration
    /\ s' = [s EXCEPT !.phase = "PolicyPublished",
        !.policyGeneration[s.request.name] = @ + 1,
        !.policyLive[s.request.name] = TRUE]

Install ==
    /\ ~s.aborted /\ s.phase = "PolicyPublished"
    /\ s' = [s EXCEPT !.phase = "Installed",
        !.entry[s.request.name] = s.candidate,
        !.generation[s.request.name] = s.policyGeneration[s.request.name],
        !.owner = [i \in Ids |-> IF i = s.candidate THEN "Live"
            ELSE IF i = s.request.previous THEN "Detached" ELSE s.owner[i]],
        !.badReplacement = (s.badReplacement \/ ~PublicReplacementAllowed)]

LookupResult(name, mode, authorized, deferred) ==
    LET id == s.entry[name]
        class == IF id = 0 THEN "Public" ELSE s.class[id]
        permitted == id # 0 /\ (mode # "Public" \/ class = "Public") /\
            (mode # "Authorized" \/ authorized)
    IN [state |-> IF permitted /\ ~s.closed[id] THEN "Granted" ELSE "Denied",
        mode |-> mode, class |-> class, generation |-> s.generation[name],
        policyGeneration |-> s.policyGeneration[name], authorized |-> authorized,
        deferred |-> deferred]

Lookup(name, mode, authorized) ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ name \in Live
    /\ mode \in {"Public", "Authorized"} /\ authorized \in BOOLEAN
    /\ s.result.state = "None"
    /\ s' = [s EXCEPT !.result = LookupResult(name, mode, authorized, FALSE)]

QueueLookup(name, mode) ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ name \in Names \ Live
    /\ s.waiter.state = "None" /\ s.result.state = "None"
    /\ mode \in {"Public", "Authorized"}
    /\ s' = [s EXCEPT !.waiter = [state |-> "Waiting", name |-> name, mode |-> mode]]

ResolveDeferred(authorized) ==
    /\ ~s.aborted /\ s.phase = "Installed" /\ authorized \in BOOLEAN
    /\ s.waiter.state = "Waiting" /\ s.waiter.name = s.request.name
    /\ s' = [s EXCEPT !.waiter.state = "Resolved",
        !.result = LookupResult(s.waiter.name, s.waiter.mode, authorized, TRUE)]

FinishInstall ==
    /\ ~s.aborted /\ s.phase = "Installed"
    /\ ~(s.waiter.state = "Waiting" /\ s.waiter.name = s.request.name)
    /\ s' = [s EXCEPT !.phase = IF s.request.previous = 0 THEN "Idle" ELSE "Disposing",
        !.disposal = s.request.previous]

CloseEndpoint(id) ==
    /\ id \in Ids /\ s.owner[id] # "Free" /\ ~s.closed[id]
    /\ s' = [s EXCEPT !.closed[id] = TRUE]

CqHint(id) ==
    /\ ~s.aborted /\ s.hint = 0 /\ id \in 1..(MaxCandidates + 1)
    /\ s' = [s EXCEPT !.hint = id]

ObservePending(name) ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ name \in Live /\ Matches
    /\ ~s.closed[s.entry[name]]
    /\ s' = [s EXCEPT !.hint = 0]

ObserveUnrelated ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ s.hint = MaxCandidates + 1
    /\ s' = [s EXCEPT !.hint = 0]

RetireDeath(name) ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ name \in Live /\ Matches
    /\ s.closed[s.entry[name]]
    /\ s' = [s EXCEPT !.phase = "Disposing", !.disposal = s.entry[name],
        !.owner[s.entry[name]] = "Detached", !.entry[name] = 0,
        !.policyLive[name] = FALSE, !.hint = 0,
        !.retiredByDeath = @ \cup {s.entry[name]}]

RetirementAllowed(name, expected, actor, manager) ==
    expected = s.generation[name] /\
    (manager \/ (actor # NoPrincipal /\ s.registrar[s.entry[name]] = actor))

RetireMutation(name, expected, actor, manager) ==
    s' = [s EXCEPT !.phase = "Disposing", !.disposal = s.entry[name],
        !.owner[s.entry[name]] = "Detached", !.entry[name] = 0,
        !.policyLive[name] = FALSE,
        !.badRetirement = (s.badRetirement \/ actor = NoPrincipal \/
            ~RetirementAllowed(name, expected, actor, manager))]

RetireExplicit(name, expected, actor, manager) ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ name \in Live
    /\ expected \in 1..MaxGeneration /\ actor \in Principals \cup {NoPrincipal}
    /\ manager \in BOOLEAN /\ actor # NoPrincipal
    /\ RetirementAllowed(name, expected, actor, manager)
    /\ RetireMutation(name, expected, actor, manager)

RejectRetirement(name, expected, actor, manager) ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ name \in Live
    /\ expected \in 1..MaxGeneration /\ actor \in Principals \cup {NoPrincipal}
    /\ manager \in BOOLEAN
    /\ actor = NoPrincipal \/ ~RetirementAllowed(name, expected, actor, manager)
    /\ s.result.state = "None"
    /\ s' = [s EXCEPT !.result = [NoResult EXCEPT !.state = "RetirementDenied"]]

CancelWatch ==
    /\ ~s.aborted /\ s.phase = "Disposing" /\ s.watch[s.disposal]
    /\ s' = [s EXCEPT !.watch[s.disposal] = FALSE, !.owner[s.disposal] = "WatchDisposed"]

CloseSource ==
    /\ ~s.aborted /\ s.phase = "Disposing" /\ ~s.watch[s.disposal]
    /\ s.source[s.disposal]
    /\ s' = [s EXCEPT !.source[s.disposal] = FALSE, !.owner[s.disposal] = "Released",
        !.phase = "Idle", !.disposal = 0]

Abort ==
    /\ ~s.aborted
    /\ s' = [s EXCEPT !.aborted = TRUE, !.frozenOwners = s.owner,
        !.frozenWatches = s.watch, !.frozenSources = s.source, !.frozenEntries = s.entry]

WatchError ==
    /\ s.phase = "Idle" /\ Matches /\ Live # {} /\ Abort
WaitRejected == /\ s.phase = "Idle" /\ Abort
LifecycleAbort == /\ s.phase = "Idle" /\ Abort
PublicationAbort == /\ s.phase # "Idle" /\ Abort

Next ==
    \/ \E n \in Names, c \in Classes, p \in Principals \cup {NoPrincipal}, m \in BOOLEAN : Begin(n,c,p,m)
    \/ RejectBeforeWatch \/ PrepareWatch \/ RejectPolicy \/ PublishPolicy \/ Install
    \/ \E n \in Names, m \in {"Public", "Authorized"}, a \in BOOLEAN : Lookup(n,m,a)
    \/ \E n \in Names, m \in {"Public", "Authorized"} : QueueLookup(n,m)
    \/ \E a \in BOOLEAN : ResolveDeferred(a)
    \/ FinishInstall \/ \E i \in Ids : CloseEndpoint(i)
    \/ \E i \in 1..(MaxCandidates+1) : CqHint(i)
    \/ \E n \in Names : ObservePending(n) \/ RetireDeath(n)
    \/ ObserveUnrelated
    \/ \E n \in Names, e \in 1..MaxGeneration, a \in Principals \cup {NoPrincipal}, m \in BOOLEAN :
        RetireExplicit(n,e,a,m) \/ RejectRetirement(n,e,a,m)
    \/ CancelWatch \/ CloseSource \/ WatchError \/ WaitRejected \/ LifecycleAbort \/ PublicationAbort

\* A separate tiny projection reaches saturation with three candidates, without
\* multiplying lookup/identity/watch interleavings already checked above.
ExhaustionRetire ==
    \E n \in Live : RetireExplicit(n, s.generation[n], CHOOSE p \in Principals : TRUE, TRUE)

ExhaustionNext ==
    \/ Begin(CHOOSE n \in Names : TRUE, "Public", CHOOSE p \in Principals : TRUE, FALSE)
    \/ RejectBeforeWatch \/ PrepareWatch \/ RejectPolicy \/ PublishPolicy \/ Install
    \/ FinishInstall \/ CancelWatch \/ CloseSource
    \/ ExhaustionRetire

RejectExhausted ==
    /\ ~s.aborted /\ s.phase = "Watched" /\ RoleAllowed
    /\ s.policyGeneration[s.request.name] = MaxGeneration
    /\ s' = [s EXCEPT !.phase = "Disposing", !.disposal = s.candidate,
        !.owner[s.candidate] = "Detached"]

Spec == Init /\ [][Next]_vars
ExhaustionSpec == Init /\ [][ExhaustionNext \/ RejectExhausted]_vars

UnsafePublicLookup ==
    /\ ~s.aborted /\ s.phase = "Idle"
    /\ \E n \in Live : s.class[s.entry[n]] = "Policy" /\ ~s.closed[s.entry[n]] /\
        s' = [s EXCEPT !.result = [LookupResult(n,"Public",FALSE,FALSE) EXCEPT !.state = "Granted"]]
UnsafeDeferredLookup ==
    /\ ~s.aborted /\ s.phase = "Installed" /\ s.request.class = "Policy"
    /\ s.waiter.state = "Waiting" /\ s.waiter.name = s.request.name /\ s.waiter.mode = "Public"
    /\ s' = [s EXCEPT !.waiter.state = "Resolved",
        !.result = [LookupResult(s.waiter.name,"Public",FALSE,TRUE) EXCEPT !.state = "Granted"]]
UnsafePublicReplacement ==
    /\ ~s.aborted /\ s.phase = "Proposed" /\ ~PublicReplacementAllowed /\ QuotaAllowed
    /\ s' = [s EXCEPT !.phase = "Watched", !.owner[s.candidate] = "Watched", !.watch[s.candidate] = TRUE]
UnsafeMutationBeforeWatch ==
    /\ ~s.aborted /\ s.phase = "Proposed"
    /\ s' = [s EXCEPT !.policyLive[s.request.name] = ~@]
UnsafeHintRetires ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ Matches
    /\ \E n \in Live : ~s.closed[s.entry[n]] /\
        s' = [s EXCEPT !.retiredByDeath = @ \cup {s.entry[n]}]
UnsafeStaleHint ==
    /\ ~s.aborted /\ s.phase = "Idle" /\ Matches /\ s.closed[s.hint]
    /\ \E n \in Live : s.hint # s.entry[n] /\ ~s.closed[s.entry[n]] /\
        s' = [s EXCEPT !.retiredByDeath = @ \cup {s.entry[n]}]
UnsafeCloseSourceFirst ==
    /\ ~s.aborted /\ s.phase = "Disposing" /\ s.watch[s.disposal]
    /\ s' = [s EXCEPT !.source[s.disposal] = FALSE]
UnsafeNewAtCapacity ==
    /\ ~s.aborted /\ s.phase = "Proposed" /\ RoleAllowed /\ PublicReplacementAllowed
    /\ ~QuotaAllowed
    /\ s' = [s EXCEPT !.phase = "Watched", !.owner[s.candidate] = "Watched", !.watch[s.candidate] = TRUE]
UnsafeAbortDisposes ==
    /\ s.aborted /\ Live # {}
    /\ \E n \in Live : s' = [s EXCEPT !.source[s.entry[n]] = FALSE]
UnsafeUnauthorizedRetire ==
    /\ ~s.aborted /\ s.phase = "Idle"
    /\ \E n \in Live, e \in 1..MaxGeneration, a \in Principals \cup {NoPrincipal}, m \in BOOLEAN :
        (a = NoPrincipal \/ ~RetirementAllowed(n,e,a,m)) /\ RetireMutation(n,e,a,m)

UnsafeGenerationWrap ==
    /\ ~s.aborted /\ s.phase = "Watched" /\ s.policyGeneration[s.request.name] = MaxGeneration
    /\ s' = [s EXCEPT !.policyGeneration[s.request.name] = 1,
        !.badGeneration = (s.badGeneration \/ 1 <= s.policyGeneration[s.request.name])]
UnsafePublishExhausted ==
    /\ ~s.aborted /\ s.phase = "Watched" /\ RoleAllowed
    /\ s.policyGeneration[s.request.name] = MaxGeneration
    /\ s' = [s EXCEPT !.phase = "PolicyPublished",
        !.policyGeneration[s.request.name] = @ + 1, !.policyLive[s.request.name] = TRUE]
UnsafeLookupDuringPublish ==
    /\ ~s.aborted /\ s.phase = "PolicyPublished" /\ s.request.name \in Live
    /\ ~s.closed[s.entry[s.request.name]]
    /\ s' = [s EXCEPT !.result = LookupResult(s.request.name,"Authorized",TRUE,FALSE)]

UnsafePublicSpec == Init /\ [][Next \/ UnsafePublicLookup]_vars
UnsafeDeferredSpec == Init /\ [][Next \/ UnsafeDeferredLookup]_vars
UnsafeReplacementSpec == Init /\ [][Next \/ UnsafePublicReplacement]_vars
UnsafeWatchSpec == Init /\ [][Next \/ UnsafeMutationBeforeWatch]_vars
UnsafeHintSpec == Init /\ [][Next \/ UnsafeHintRetires]_vars
UnsafeStaleSpec == Init /\ [][Next \/ UnsafeStaleHint]_vars
UnsafeDisposalSpec == Init /\ [][Next \/ UnsafeCloseSourceFirst]_vars
UnsafeQuotaSpec == Init /\ [][Next \/ UnsafeNewAtCapacity]_vars
UnsafeAbortSpec == Init /\ [][Next \/ UnsafeAbortDisposes]_vars
UnsafeRetireSpec == Init /\ [][Next \/ UnsafeUnauthorizedRetire]_vars
UnsafeWrapSpec == Init /\ [][ExhaustionNext \/ RejectExhausted \/ UnsafeGenerationWrap]_vars
UnsafeExhaustedSpec == Init /\ [][ExhaustionNext \/ RejectExhausted \/ UnsafePublishExhausted]_vars
UnsafeSerialSpec == Init /\ [][Next \/ UnsafeLookupDuringPublish]_vars

TypeOK ==
    /\ s.phase \in {"Idle", "Proposed", "Watched", "PolicyPublished", "Installed", "Disposing"}
    /\ s.next \in 1..(MaxCandidates+1) /\ s.candidate \in 0..MaxCandidates /\ s.disposal \in 0..MaxCandidates
    /\ s.entry \in [Names -> 0..MaxCandidates]
    /\ s.generation \in [Names -> 0..MaxGeneration] /\ s.policyGeneration \in [Names -> 0..MaxGeneration]
    /\ s.policyLive \in [Names -> BOOLEAN] /\ s.owner \in [Ids -> Owners]
    /\ s.watch \in [Ids -> BOOLEAN] /\ s.source \in [Ids -> BOOLEAN] /\ s.closed \in [Ids -> BOOLEAN]
    /\ s.class \in [Ids -> Classes] /\ s.registrar \in [Ids -> Principals \cup {NoPrincipal}]
    /\ s.hint \in 0..(MaxCandidates+1) /\ s.retiredByDeath \subseteq Ids
    /\ s.aborted \in BOOLEAN /\ s.badReplacement \in BOOLEAN /\ s.badRetirement \in BOOLEAN /\ s.badGeneration \in BOOLEAN
PublishedOwnsWatch == \A n \in Live : s.owner[s.entry[n]] = "Live" /\ s.watch[s.entry[n]] /\ s.source[s.entry[n]]
WatchRetainsSource == \A i \in Ids : s.watch[i] => s.source[i]
LiveBound == Cardinality(Live) <= LiveLimit
WatchBudget == WatchCount <= LiveLimit + 1
PublicIssuance == s.result.state = "Granted" /\ s.result.mode = "Public" => s.result.class = "Public"
AuthorizedIssuance == s.result.state = "Granted" /\ s.result.mode = "Authorized" => s.result.authorized
GrantGenerationCoherent == s.result.state = "Granted" => s.result.generation = s.result.policyGeneration
ReplacementProtected == ~s.badReplacement
RetirementAuthorized == ~s.badRetirement
GenerationNotReused == ~s.badGeneration
PolicyGenerationRepresentable == \A n \in Names : s.policyGeneration[n] <= MaxGeneration
DeathNeedsExactResult == \A i \in s.retiredByDeath : s.closed[i]
RegistryPolicyCoherent == s.phase # "PolicyPublished" =>
    \A n \in Names : s.generation[n] = s.policyGeneration[n] /\ (s.policyLive[n] = (s.entry[n] # 0))
BeforeWatchUnchanged == s.phase = "Proposed" =>
    s.entry[s.request.name] = s.request.previous /\
    s.policyGeneration[s.request.name] = s.request.previousGeneration /\
    (s.policyLive[s.request.name] = (s.request.previous # 0))
AbortRetainsOwners == s.aborted => s.owner = s.frozenOwners /\
    s.watch = s.frozenWatches /\ s.source = s.frozenSources /\ s.entry = s.frozenEntries
Invariants == TypeOK /\ PublishedOwnsWatch /\ WatchRetainsSource /\ LiveBound /\ WatchBudget
    /\ PublicIssuance /\ AuthorizedIssuance /\ GrantGenerationCoherent /\ ReplacementProtected
    /\ RetirementAuthorized /\ GenerationNotReused /\ PolicyGenerationRepresentable /\ DeathNeedsExactResult
    /\ RegistryPolicyCoherent /\ BeforeWatchUnchanged /\ AbortRetainsOwners
=============================================================================
