-------------------------- MODULE CharlotteRemoteCall -------------------------
\* Bounded remote-call identity, generation fencing, uncertainty, and dedup.
\*
\* A Call denotes the complete (caller node, caller session, call id) identity.
\* The model deliberately does not claim transactional or global exactly-once
\* execution. It checks at-most-once execution while the protocol retains the
\* identity, and permits cache eviction only after a successful call's relmsg
\* exchange settles or an uncertain caller session is explicitly retired.

EXTENDS Naturals, FiniteSets

CONSTANTS Calls, MaxGeneration, CacheLimit

ASSUME Calls /= {}
ASSUME MaxGeneration > 1
ASSUME CacheLimit > 0

Phase == {"Idle", "Sent", "Executed", "ReplyQueued", "Completed",
          "Uncertain", "StaleRejected"}

VARIABLES phase, expectedGeneration, currentGeneration, executions,
          cache, replyDelivered, transportSettled, retired

vars == <<phase, expectedGeneration, currentGeneration, executions,
          cache, replyDelivered, transportSettled, retired>>

Init ==
    /\ phase = [c \in Calls |-> "Idle"]
    /\ expectedGeneration = [c \in Calls |-> 1]
    /\ currentGeneration = [c \in Calls |-> 1]
    /\ executions = [c \in Calls |-> 0]
    /\ cache = {}
    /\ replyDelivered = {}
    /\ transportSettled = {}
    /\ retired = {}

Start(c) ==
    /\ phase[c] = "Idle"
    /\ phase' = [phase EXCEPT ![c] = "Sent"]
    /\ expectedGeneration' =
         [expectedGeneration EXCEPT ![c] = currentGeneration[c]]
    /\ UNCHANGED <<currentGeneration, executions, cache, replyDelivered,
                    transportSettled, retired>>

ReplaceTarget(c) ==
    /\ currentGeneration[c] < MaxGeneration
    /\ currentGeneration' = [currentGeneration EXCEPT ![c] = @ + 1]
    /\ UNCHANGED <<phase, expectedGeneration, executions, cache,
                    replyDelivered, transportSettled, retired>>

Execute(c) ==
    /\ phase[c] = "Sent"
    /\ expectedGeneration[c] = currentGeneration[c]
    /\ executions[c] = 0
    /\ Cardinality(cache) < CacheLimit
    /\ phase' = [phase EXCEPT ![c] = "Executed"]
    /\ executions' = [executions EXCEPT ![c] = 1]
    /\ cache' = cache \cup {c}
    /\ UNCHANGED <<expectedGeneration, currentGeneration, replyDelivered,
                    transportSettled, retired>>

RejectStale(c) ==
    /\ phase[c] = "Sent"
    /\ expectedGeneration[c] /= currentGeneration[c]
    /\ phase' = [phase EXCEPT ![c] = "StaleRejected"]
    /\ UNCHANGED <<expectedGeneration, currentGeneration, executions, cache,
                    replyDelivered, transportSettled, retired>>

QueueReply(c) ==
    /\ phase[c] = "Executed"
    /\ c \in cache
    /\ phase' = [phase EXCEPT ![c] = "ReplyQueued"]
    /\ UNCHANGED <<expectedGeneration, currentGeneration, executions, cache,
                    replyDelivered, transportSettled, retired>>

\* A relmsg retransmission or application duplicate hits the retained result.
DuplicateRequest(c) ==
    /\ phase[c] \in {"Executed", "ReplyQueued"}
    /\ c \in cache
    /\ UNCHANGED vars

DeliverReply(c) ==
    /\ phase[c] = "ReplyQueued"
    /\ phase' = [phase EXCEPT ![c] = "Completed"]
    /\ replyDelivered' = replyDelivered \cup {c}
    /\ UNCHANGED <<expectedGeneration, currentGeneration, executions, cache,
                    transportSettled, retired>>

Timeout(c) ==
    /\ phase[c] \in {"Sent", "Executed", "ReplyQueued"}
    /\ phase' = [phase EXCEPT ![c] = "Uncertain"]
    /\ UNCHANGED <<expectedGeneration, currentGeneration, executions, cache,
                    replyDelivered, transportSettled, retired>>

SettleTransport(c) ==
    /\ phase[c] = "Completed"
    /\ transportSettled' = transportSettled \cup {c}
    /\ UNCHANGED <<phase, expectedGeneration, currentGeneration, executions,
                    cache, replyDelivered, retired>>

RetireUncertainSession(c) ==
    /\ phase[c] = "Uncertain"
    /\ retired' = retired \cup {c}
    /\ transportSettled' = transportSettled \cup {c}
    /\ UNCHANGED <<phase, expectedGeneration, currentGeneration, executions,
                    cache, replyDelivered>>

Evict(c) ==
    /\ c \in cache
    /\ c \in transportSettled
    /\ (phase[c] = "Completed" \/ c \in retired)
    /\ cache' = cache \ {c}
    /\ UNCHANGED <<phase, expectedGeneration, currentGeneration, executions,
                    replyDelivered, transportSettled, retired>>

Next ==
    \/ \E c \in Calls : Start(c)
    \/ \E c \in Calls : ReplaceTarget(c)
    \/ \E c \in Calls : Execute(c)
    \/ \E c \in Calls : RejectStale(c)
    \/ \E c \in Calls : QueueReply(c)
    \/ \E c \in Calls : DuplicateRequest(c)
    \/ \E c \in Calls : DeliverReply(c)
    \/ \E c \in Calls : Timeout(c)
    \/ \E c \in Calls : SettleTransport(c)
    \/ \E c \in Calls : RetireUncertainSession(c)
    \/ \E c \in Calls : Evict(c)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ phase \in [Calls -> Phase]
    /\ expectedGeneration \in [Calls -> 1..MaxGeneration]
    /\ currentGeneration \in [Calls -> 1..MaxGeneration]
    /\ executions \in [Calls -> 0..1]
    /\ cache \subseteq Calls
    /\ Cardinality(cache) <= CacheLimit
    /\ replyDelivered \subseteq Calls
    /\ transportSettled \subseteq Calls
    /\ retired \subseteq Calls

AtMostOnceWhileTracked == \A c \in Calls : executions[c] <= 1

CompletedHasExecutedReply ==
    \A c \in Calls : phase[c] = "Completed" =>
        /\ executions[c] = 1
        /\ c \in replyDelivered

StaleGenerationNeverExecutes ==
    \A c \in Calls : phase[c] = "StaleRejected" => executions[c] = 0

UncertainIsNotSuccess ==
    \A c \in Calls : phase[c] = "Uncertain" => c \notin replyDelivered

RetainUntilSafe ==
    \A c \in Calls :
        executions[c] = 1 /\ c \notin transportSettled => c \in cache

Invariants ==
    /\ TypeOK
    /\ AtMostOnceWhileTracked
    /\ CompletedHasExecutedReply
    /\ StaleGenerationNeverExecutes
    /\ UncertainIsNotSuccess
    /\ RetainUntilSafe

=============================================================================
