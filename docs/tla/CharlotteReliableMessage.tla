--------------------- MODULE CharlotteReliableMessage ---------------------
\* Reliable-message session identity across retry abandonment and unilateral
\* service restart. A wire session is the pair (service generation, retry
\* epoch), packed monotonically by the implementation.

EXTENDS Naturals, FiniteSets

CONSTANTS MaxGeneration, MaxAttempt, UnsafeFlatIdentity, AllowSessionRegression

ASSUME MaxGeneration > 1
ASSUME MaxAttempt > 1

Generation == 1..MaxGeneration
Attempt == 1..MaxAttempt
MaxWireSession == MaxGeneration * MaxAttempt
WireSessions == 1..MaxWireSession

\* The repaired encoding is injective and ordered. The unsafe encoding is
\* the former `generation + retry` scheme: generation N's first retry aliases
\* generation N+1's initial session.
WireSession(g, a) ==
    IF UnsafeFlatIdentity
    THEN g + a - 1
    ELSE (g - 1) * MaxAttempt + a

VARIABLES generation, attempt, issuedSessions, activeReceiveSession,
          identityCollision, receiveRegressed

vars == <<generation, attempt, issuedSessions, activeReceiveSession,
          identityCollision, receiveRegressed>>

Init ==
    /\ generation = 1
    /\ attempt = 1
    /\ issuedSessions = {WireSession(1, 1)}
    /\ activeReceiveSession = 0
    /\ identityCollision = FALSE
    /\ receiveRegressed = FALSE

AbandonSession ==
    /\ attempt < MaxAttempt
    /\ LET replacement == WireSession(generation, attempt + 1)
       IN /\ identityCollision' = (identityCollision \/ replacement \in issuedSessions)
          /\ issuedSessions' = issuedSessions \cup {replacement}
    /\ attempt' = attempt + 1
    /\ UNCHANGED <<generation, activeReceiveSession, receiveRegressed>>

RestartService ==
    /\ generation < MaxGeneration
    /\ LET replacement == WireSession(generation + 1, 1)
       IN /\ identityCollision' = (identityCollision \/ replacement \in issuedSessions)
          /\ issuedSessions' = issuedSessions \cup {replacement}
    /\ generation' = generation + 1
    /\ attempt' = 1
    /\ UNCHANGED <<activeReceiveSession, receiveRegressed>>

AcceptCurrentSession ==
    /\ WireSession(generation, attempt) > activeReceiveSession
    /\ activeReceiveSession' = WireSession(generation, attempt)
    /\ UNCHANGED <<generation, attempt, issuedSessions,
                    identityCollision, receiveRegressed>>

\* Negative regression action for the former bounded retired-session list.
\* Once a sufficiently old session fell out of the list, a delayed SYN could
\* replace a newer receive session and reset sequence ordering backwards.
AcceptDelayedSession(s) ==
    /\ AllowSessionRegression
    /\ s \in issuedSessions
    /\ s # activeReceiveSession
    /\ activeReceiveSession' = s
    /\ receiveRegressed' = (receiveRegressed \/ s < activeReceiveSession)
    /\ UNCHANGED <<generation, attempt, issuedSessions, identityCollision>>

Next ==
    \/ AbandonSession
    \/ RestartService
    \/ AcceptCurrentSession
    \/ \E s \in WireSessions : AcceptDelayedSession(s)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ generation \in Generation
    /\ attempt \in Attempt
    /\ issuedSessions \subseteq WireSessions
    /\ activeReceiveSession \in 0..MaxWireSession
    /\ identityCollision \in BOOLEAN
    /\ receiveRegressed \in BOOLEAN

SessionIdentityUnique == ~identityCollision

ReceiveSessionMonotonic == ~receiveRegressed

Invariants ==
    /\ TypeOK
    /\ SessionIdentityUnique
    /\ ReceiveSessionMonotonic

=============================================================================
