-------------------------------- MODULE CharlotteIPC --------------------------------
\* TLA+ model of CharlotteOS IPC state machines.
\*
\* Covers: endpoints, connections, scalar send/call/receive/reply, reply tokens,
\*          pending calls, memory transfer (move/borrow-read/borrow-write),
\*          cancellation, endpoint close, domain teardown.
\*
\* All ID domains are bounded integer ranges for finite-state model checking.

EXTENDS Naturals, Sequences, FiniteSets, Integers

\* -----------------------------------------------------------------------------
\* 1. CONSTANTS
\* -----------------------------------------------------------------------------

CONSTANTS
    ASID,           \* E.g. {a1, a2}
    MaxCaps,        \* E.g. 12  -- max caps per AS
    MaxEps,         \* E.g. 2
    MaxTokens,      \* E.g. 6
    MaxCalls,       \* E.g. 6
    MaxMems,        \* E.g. 4
    MaxQueue,       \* E.g. 2
    NullAsid        \* distinguished non-member (e.g. a0)

ASSUME NullAsid \notin ASID
ASSUME MaxQueue > 0
ASSUME MaxCaps > 0 /\ MaxEps > 0 /\ MaxTokens > 0 /\ MaxCalls > 0 /\ MaxMems > 0

\* Integer ID domains.
CapId   == 1 .. MaxCaps
EpId    == 1 .. MaxEps
TokenId == 1 .. MaxTokens
CallId  == 1 .. MaxCalls
MemId   == 1 .. MaxMems

\* -----------------------------------------------------------------------------
\* 2. TYPES
\* -----------------------------------------------------------------------------

\* Rights bitmask as a record.
Rights == [send : BOOLEAN, call : BOOLEAN, receive : BOOLEAN, mint : BOOLEAN]

NoRights    == [send |-> FALSE, call |-> FALSE, receive |-> FALSE, mint |-> FALSE]
FullRights  == [send |-> TRUE,  call |-> TRUE,  receive |-> TRUE,  mint |-> TRUE]
SendCall    == [send |-> TRUE,  call |-> TRUE,  receive |-> FALSE, mint |-> FALSE]

Attenuate(r, allowed) == [
    send    |-> r.send    /\ allowed.send,
    call    |-> r.call    /\ allowed.call,
    receive |-> r.receive /\ allowed.receive,
    mint    |-> r.mint    /\ allowed.mint
]

TransferMode == {"Copy", "Move", "BorrowRead", "BorrowWrite", "None"}
MemState     == {"Owned", "Moved", "BorrowedR", "BorrowedW"}

\* Tagged capability union.
\* All have .kind; variant-specific fields are present only when needed.
\* A NullCap has kind="Null" and no other fields.
CapKind == {"EndpointCap", "ConnectionCap", "ReplyTokenCap", "PendingCallCap",
            "MemoryCap", "Null"}

Capability == [
    kind   : CapKind,
    ep     : EpId \cup {0},
    rights : Rights,
    token  : TokenId \cup {0},
    call   : CallId \cup {0},
    mem    : MemId \cup {0}
]

NullCap == [kind |-> "Null", ep |-> 0, rights |-> NoRights,
            token |-> 0, call |-> 0, mem |-> 0]

EndpointCap(epid, r) == [kind |-> "EndpointCap", ep |-> epid, rights |-> r,
                         token |-> 0, call |-> 0, mem |-> 0]

ConnectionCap(epid, r) == [kind |-> "ConnectionCap", ep |-> epid, rights |-> r,
                           token |-> 0, call |-> 0, mem |-> 0]

ReplyTokenCap(tokid) == [kind |-> "ReplyTokenCap", ep |-> 0, rights |-> NoRights,
                         token |-> tokid, call |-> 0, mem |-> 0]

PendingCallCap(callid) == [kind |-> "PendingCallCap", ep |-> 0, rights |-> NoRights,
                           token |-> 0, call |-> callid, mem |-> 0]

MemoryCap(mid) == [kind |-> "MemoryCap", ep |-> 0, rights |-> NoRights,
                   token |-> 0, call |-> 0, mem |-> mid]

\* Convenience: is a cap of the given kind?
IsKind(cap, k) == cap.kind = k

\* Authorized for an action?
Authorized(cap, action) ==
    CASE action = "send"    -> cap.rights.send
    [] action = "call"     -> cap.rights.call
    [] action = "receive"  -> cap.rights.receive
    [] action = "mint"     -> cap.rights.mint

\* A message in an endpoint queue.
Message == [
    sender  : ASID \cup {NullAsid},
    opcode  : 0..3,
    arg0    : 0..3,
    reply   : TokenId \cup {0},     \* 0 = no reply token
    mem     : MemId \cup {0},       \* 0 = no memory attachment
    memMode : TransferMode,
    conn    : CapId \cup {0}        \* 0 = no delegated connection
]

NoMsg == [sender |-> NullAsid, opcode |-> 0, arg0 |-> 0,
          reply |-> 0, mem |-> 0, memMode |-> "None", conn |-> 0]

\* Endpoint state.
Endpoint == [
    owner    : ASID \cup {NullAsid},
    capacity : 1 .. MaxQueue,
    queue    : Seq(Message),
    closed   : BOOLEAN
]

\* Reply token.
ReplyToken == [
    token    : TokenId \cup {0},
    server   : ASID \cup {NullAsid},
    call     : CallId \cup {0},
    delivered: BOOLEAN,
    consumed : BOOLEAN,
    borrow   : MemId \cup {0}
]

\* Pending call.
PendingCall == [
    call     : CallId \cup {0},
    caller   : ASID \cup {NullAsid},
    result   : [value : Int, mem : MemId \cup {0}],
    observed : BOOLEAN
]

NoResult == [value |-> -100, mem |-> 0]   \* Sentinel: "no result yet"
CancelledResult == [value |-> -4, mem |-> 0]
ClosedResult == [value |-> -3, mem |-> 0]

\* Memory object.
MemoryObject == [
    obj     : MemId \cup {0},
    owner   : ASID \cup {NullAsid},
    state   : MemState,
    lender  : ASID \cup {NullAsid},
    borrows : SUBSET ASID
]

\* -----------------------------------------------------------------------------
\* 3. STATE VARIABLES
\* -----------------------------------------------------------------------------

VARIABLES
    capTable,       \* [ASID -> [CapId -> Capability]]
    endpoints,      \* [EpId -> Endpoint]
    replyTokens,    \* [TokenId -> ReplyToken]
    pendingCalls,   \* [CallId -> PendingCall]
    memObjects,     \* [MemId -> MemoryObject]
    nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId

vars == <<capTable, endpoints, replyTokens, pendingCalls, memObjects,
          nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -----------------------------------------------------------------------------
\* 4. HELPER MACROS
\* -----------------------------------------------------------------------------

\* Capability identifiers are drawn from one monotonic system-wide namespace,
\* matching CharlotteOS' unified object-capability handles.  The table remains
\* indexed by ASID because possession is address-space local.
CanAllocCaps(count) ==
    /\ count > 0
    /\ nextCapId \in CapId
    /\ nextCapId + count - 1 \in CapId

\* Can we allocate a new endpoint / token / call / mem?
CanAllocEp    == nextEpId \in EpId
CanAllocToken == nextTokenId \in TokenId
CanAllocCall  == nextCallId \in CallId
CanAllocMem   == nextMemId \in MemId

\* Convert a sequence to a set.
SeqToSet(seq) == { seq[i] : i \in 1..Len(seq) }

\* Place a capability at nextCapId in as's table.
AllocCap(as, cap) ==
    /\ CanAllocCaps(1)
    /\ capTable' = [capTable EXCEPT ![as][nextCapId] = cap]
    /\ nextCapId' = nextCapId + 1

\* Free a capability slot.
FreeCap(as, cid) ==
    /\ capTable' = [capTable EXCEPT ![as][cid] = NullCap]

\* Revoke every borrow represented by a set of reply tokens.  This operator
\* is used by endpoint close and domain teardown so completion and ownership
\* restoration remain one abstract atomic transition.
RevokeTokenBorrows(mid, tokenIds) ==
    LET mo == memObjects[mid]
        borrowers ==
            { replyTokens[t].server :
                t \in { candidate \in tokenIds :
                    replyTokens[candidate].borrow = mid } }
        hasBorrow ==
            \E t \in tokenIds : replyTokens[t].borrow = mid
        remaining == mo.borrows \ borrowers
    IN CASE mo.state = "BorrowedR" /\ hasBorrow ->
                [mo EXCEPT !.borrows = remaining,
                           !.state = IF remaining = {} THEN "Owned"
                                    ELSE "BorrowedR",
                           !.lender = IF remaining = {} THEN NullAsid
                                     ELSE mo.lender]
         [] mo.state = "BorrowedW" /\ hasBorrow ->
                [mo EXCEPT !.owner = mo.lender,
                           !.lender = NullAsid,
                           !.state = "Owned"]
         [] OTHER -> mo

\* -----------------------------------------------------------------------------
\* 5. INITIAL STATE
\* -----------------------------------------------------------------------------

Init ==
    /\ capTable  = [as \in ASID |-> [cid \in CapId |-> NullCap]]
    /\ endpoints = [e \in EpId |-> [
          owner    |-> NullAsid,
          capacity |-> MaxQueue,
          queue    |-> <<>>,
          closed   |-> FALSE
       ]]
    /\ replyTokens = [t \in TokenId |-> [
          token    |-> t,
          server   |-> NullAsid,
          call     |-> 0,
          delivered|-> TRUE,
          consumed |-> TRUE,
          borrow   |-> 0
       ]]
    /\ pendingCalls = [c \in CallId |-> [
          call     |-> c,
          caller   |-> NullAsid,
          result   |-> NoResult,
          observed |-> TRUE
       ]]
    /\ memObjects = [m \in MemId |-> [
          obj     |-> m,
          owner   |-> NullAsid,
          state   |-> "Owned",
          lender  |-> NullAsid,
          borrows |-> {}
       ]]
    /\ nextEpId    = 1
    /\ nextTokenId = 1
    /\ nextCallId  = 1
    /\ nextCapId   = 1
    /\ nextMemId   = 1

\* -----------------------------------------------------------------------------
\* 6. TRANSITIONS
\* -----------------------------------------------------------------------------

\* -- 6.0 Create a memory object ---------------------------------------------
MemoryCreate(owner) ==
    /\ CanAllocMem
    /\ CanAllocCaps(1)
    /\ LET mid == nextMemId
       IN /\ memObjects' = [memObjects EXCEPT ![mid] = [
                obj     |-> mid,
                owner   |-> owner,
                state   |-> "Owned",
                lender  |-> NullAsid,
                borrows |-> {}
             ]]
          /\ capTable' = [capTable EXCEPT
                ![owner][nextCapId] = MemoryCap(mid)]
          /\ nextMemId' = nextMemId + 1
          /\ nextCapId' = nextCapId + 1
    /\ UNCHANGED <<endpoints, replyTokens, pendingCalls,
                   nextEpId, nextTokenId, nextCallId>>

\* -- 6.1 EndpointCreate -----------------------------------------------------
EndpointCreate(server) ==
    /\ CanAllocEp
    /\ CanAllocCaps(1)
    /\ LET epid == nextEpId
           ecap == EndpointCap(epid, FullRights)
       IN /\ endpoints' = [endpoints EXCEPT ![epid] = [
                owner |-> server, capacity |-> MaxQueue,
                queue |-> <<>>, closed |-> FALSE]]
          /\ nextEpId' = nextEpId + 1
          /\ AllocCap(server, ecap)
          /\ UNCHANGED <<replyTokens, pendingCalls, memObjects,
                         nextTokenId, nextCallId, nextMemId>>

\* -- 6.2 ConnectionMint -----------------------------------------------------
ConnectionMint(server, epCid, client, allowed) ==
    /\ capTable[server][epCid].kind = "EndpointCap"
    /\ Authorized(capTable[server][epCid], "mint")
    /\ CanAllocCaps(1)
    /\ LET epid == capTable[server][epCid].ep
           ccap == ConnectionCap(epid, Attenuate(
                        capTable[server][epCid].rights, allowed))
       IN /\ AllocCap(client, ccap)
    /\ UNCHANGED <<endpoints, replyTokens, pendingCalls, memObjects,
                   nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.3 ScalarSend ---------------------------------------------------------
ScalarSend(sender, connCid, opcode, arg0) ==
    /\ capTable[sender][connCid].kind = "ConnectionCap"
    /\ Authorized(capTable[sender][connCid], "send")
    /\ LET epid == capTable[sender][connCid].ep
           e    == endpoints[epid]
       IN /\ e.owner /= NullAsid
          /\ ~e.closed
          /\ Len(e.queue) < e.capacity
          /\ LET msg == [sender |-> sender, opcode |-> opcode, arg0 |-> arg0,
                         reply |-> 0, mem |-> 0, memMode |-> "None", conn |-> 0]
             IN /\ endpoints' = [endpoints EXCEPT ![epid].queue =
                                     Append(e.queue, msg)]
    /\ UNCHANGED <<capTable, replyTokens, pendingCalls, memObjects,
                   nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.4 ScalarCall (no memory attachment) ----------------------------------
ScalarCall(caller, connCid, opcode, arg0) ==
    /\ capTable[caller][connCid].kind = "ConnectionCap"
    /\ Authorized(capTable[caller][connCid], "call")
    /\ CanAllocCall /\ CanAllocToken /\ CanAllocCaps(2)
    /\ LET epid    == capTable[caller][connCid].ep
           e       == endpoints[epid]
           server  == e.owner
       IN /\ server /= NullAsid
          /\ ~e.closed
          /\ Len(e.queue) < e.capacity
          /\ LET callid    == nextCallId
                 tokid     == nextTokenId
                 pcapCid   == nextCapId
                 tokCapCid == nextCapId + 1
                 msg   == [sender |-> caller, opcode |-> opcode, arg0 |-> arg0,
                           reply |-> tokid, mem |-> 0, memMode |-> "None",
                           conn |-> 0]
                 token == [token |-> tokid, server |-> server, call |-> callid,
                           delivered |-> FALSE, consumed |-> FALSE, borrow |-> 0]
                 pcall == [call |-> callid, caller |-> caller,
                           result |-> NoResult, observed |-> FALSE]
             IN /\ endpoints'   = [endpoints EXCEPT ![epid].queue =
                                       Append(e.queue, msg)]
                /\ replyTokens' = [replyTokens EXCEPT ![tokid] = token]
                /\ pendingCalls' = [pendingCalls EXCEPT ![callid] = pcall]
                /\ capTable'    = [capTable EXCEPT
                       ![caller][pcapCid]   = PendingCallCap(callid),
                       ![server][tokCapCid] = ReplyTokenCap(tokid)]
                /\ nextCallId'  = nextCallId + 1
                /\ nextTokenId' = nextTokenId + 1
                /\ nextCapId'   = nextCapId + 2
                /\ UNCHANGED <<memObjects, nextEpId, nextMemId>>

\* -- 6.5 ScalarCall with memory move -----------------------------------------
ScalarCallMove(caller, connCid, opcode, arg0, memCid) ==
    /\ capTable[caller][connCid].kind = "ConnectionCap"
    /\ Authorized(capTable[caller][connCid], "call")
    /\ capTable[caller][memCid].kind = "MemoryCap"
    /\ CanAllocCall /\ CanAllocToken /\ CanAllocCaps(3)
    /\ LET epid    == capTable[caller][connCid].ep
           e       == endpoints[epid]
           mid     == capTable[caller][memCid].mem
           mo      == memObjects[mid]
           server  == e.owner
       IN /\ server /= NullAsid
          /\ ~e.closed
          /\ Len(e.queue) < e.capacity
          /\ mo.state = "Owned" /\ mo.owner = caller
          /\ LET callid    == nextCallId
                 tokid     == nextTokenId
                 pcapCid   == nextCapId
                 tokCapCid == nextCapId + 1
                 cap3Cid   == nextCapId + 2
                 msg   == [sender |-> caller, opcode |-> opcode, arg0 |-> arg0,
                           reply |-> tokid, mem |-> mid, memMode |-> "Move",
                           conn |-> 0]
                 token == [token |-> tokid, server |-> server, call |-> callid,
                           delivered |-> FALSE, consumed |-> FALSE, borrow |-> 0]
                 pcall == [call |-> callid, caller |-> caller,
                           result |-> NoResult, observed |-> FALSE]
             IN /\ endpoints'   = [endpoints EXCEPT ![epid].queue =
                                       Append(e.queue, msg)]
                /\ replyTokens' = [replyTokens EXCEPT ![tokid] = token]
                /\ pendingCalls' = [pendingCalls EXCEPT ![callid] = pcall]
                /\ memObjects'  = [memObjects EXCEPT ![mid].owner = server,
                                                       ![mid].state = "Moved"]
                /\ capTable'    = [capTable EXCEPT
                       ![caller][memCid]    = NullCap,
                       ![caller][pcapCid]   = PendingCallCap(callid),
                       ![server][tokCapCid] = ReplyTokenCap(tokid),
                       ![server][cap3Cid]   = MemoryCap(mid)]
                /\ nextCallId'  = nextCallId + 1
                /\ nextTokenId' = nextTokenId + 1
                /\ nextCapId'   = nextCapId + 3
                /\ UNCHANGED <<nextEpId, nextMemId>>

\* -- 6.6 ScalarCall with borrow-read -----------------------------------------
ScalarCallBorrowRead(caller, connCid, opcode, arg0, memCid) ==
    /\ capTable[caller][connCid].kind = "ConnectionCap"
    /\ Authorized(capTable[caller][connCid], "call")
    /\ capTable[caller][memCid].kind = "MemoryCap"
    /\ CanAllocCall /\ CanAllocToken /\ CanAllocCaps(2)
    /\ LET epid    == capTable[caller][connCid].ep
           e       == endpoints[epid]
           mid     == capTable[caller][memCid].mem
           mo      == memObjects[mid]
           server  == e.owner
       IN /\ server /= NullAsid
          /\ server /= caller
          /\ ~e.closed
          /\ Len(e.queue) < e.capacity
          /\ mo.state \in {"Owned", "BorrowedR"}
          /\ mo.owner = caller
          /\ LET callid    == nextCallId
                 tokid     == nextTokenId
                 pcapCid   == nextCapId
                 tokCapCid == nextCapId + 1
                 msg   == [sender |-> caller, opcode |-> opcode, arg0 |-> arg0,
                           reply |-> tokid, mem |-> mid, memMode |-> "BorrowRead",
                           conn |-> 0]
                 token == [token |-> tokid, server |-> server, call |-> callid,
                           delivered |-> FALSE, consumed |-> FALSE, borrow |-> mid]
                 pcall == [call |-> callid, caller |-> caller,
                           result |-> NoResult, observed |-> FALSE]
             IN /\ endpoints'   = [endpoints EXCEPT ![epid].queue =
                                       Append(e.queue, msg)]
                /\ replyTokens' = [replyTokens EXCEPT ![tokid] = token]
                /\ pendingCalls' = [pendingCalls EXCEPT ![callid] = pcall]
                /\ memObjects'  = [memObjects EXCEPT
                       ![mid].state   = "BorrowedR",
                       ![mid].lender  = caller,
                       ![mid].borrows = mo.borrows \cup {server}]
                /\ capTable'    = [capTable EXCEPT
                       ![caller][pcapCid]   = PendingCallCap(callid),
                       ![server][tokCapCid] = ReplyTokenCap(tokid)]
                /\ nextCallId'  = nextCallId + 1
                /\ nextTokenId' = nextTokenId + 1
                /\ nextCapId'   = nextCapId + 2
                /\ UNCHANGED <<nextEpId, nextMemId>>

\* -- 6.7 ScalarCall with borrow-write ----------------------------------------
ScalarCallBorrowWrite(caller, connCid, opcode, arg0, memCid) ==
    /\ capTable[caller][connCid].kind = "ConnectionCap"
    /\ Authorized(capTable[caller][connCid], "call")
    /\ capTable[caller][memCid].kind = "MemoryCap"
    /\ CanAllocCall /\ CanAllocToken /\ CanAllocCaps(2)
    /\ LET epid    == capTable[caller][connCid].ep
           e       == endpoints[epid]
           mid     == capTable[caller][memCid].mem
           mo      == memObjects[mid]
           server  == e.owner
       IN /\ server /= NullAsid
          /\ server /= caller
          /\ ~e.closed
          /\ Len(e.queue) < e.capacity
          /\ mo.state = "Owned" /\ mo.owner = caller
          /\ mo.borrows = {}
          /\ LET callid    == nextCallId
                 tokid     == nextTokenId
                 pcapCid   == nextCapId
                 tokCapCid == nextCapId + 1
                 msg   == [sender |-> caller, opcode |-> opcode, arg0 |-> arg0,
                           reply |-> tokid, mem |-> mid, memMode |-> "BorrowWrite",
                           conn |-> 0]
                 token == [token |-> tokid, server |-> server, call |-> callid,
                           delivered |-> FALSE, consumed |-> FALSE, borrow |-> mid]
                 pcall == [call |-> callid, caller |-> caller,
                           result |-> NoResult, observed |-> FALSE]
             IN /\ endpoints'   = [endpoints EXCEPT ![epid].queue =
                                       Append(e.queue, msg)]
                /\ replyTokens' = [replyTokens EXCEPT ![tokid] = token]
                /\ pendingCalls' = [pendingCalls EXCEPT ![callid] = pcall]
                /\ memObjects'  = [memObjects EXCEPT ![mid].state  = "BorrowedW",
                                                       ![mid].lender = caller,
                                                       ![mid].owner  = server]
                /\ capTable'    = [capTable EXCEPT
                       ![caller][pcapCid]   = PendingCallCap(callid),
                       ![server][tokCapCid] = ReplyTokenCap(tokid)]
                /\ nextCallId'  = nextCallId + 1
                /\ nextTokenId' = nextTokenId + 1
                /\ nextCapId'   = nextCapId + 2
                /\ UNCHANGED <<nextEpId, nextMemId>>

\* -- 6.7b ScalarCall with memory copy ---------------------------------------
ScalarCallCopy(caller, connCid, opcode, arg0, memCid) ==
    /\ capTable[caller][connCid].kind = "ConnectionCap"
    /\ Authorized(capTable[caller][connCid], "call")
    /\ capTable[caller][memCid].kind = "MemoryCap"
    /\ CanAllocCall /\ CanAllocToken /\ CanAllocCaps(3)
    /\ CanAllocMem
    /\ LET epid    == capTable[caller][connCid].ep
           e       == endpoints[epid]
           mid     == capTable[caller][memCid].mem
           mo      == memObjects[mid]
           server  == e.owner
       IN /\ server /= NullAsid
          /\ ~e.closed
          /\ Len(e.queue) < e.capacity
          /\ mo.state = "Owned" /\ mo.owner = caller
          \* Deep-copy: allocate new memory object for the server.
          /\ LET newMid    == nextMemId
                 cmid      == newMid
                 callid    == nextCallId
                 tokid     == nextTokenId
                 pcapCid   == nextCapId
                 tokCapCid == nextCapId + 1
                 memCapCid == nextCapId + 2
                 msg   == [sender |-> caller, opcode |-> opcode, arg0 |-> arg0,
                           reply |-> tokid, mem |-> cmid, memMode |-> "Copy",
                           conn |-> 0]
                 token == [token |-> tokid, server |-> server, call |-> callid,
                           delivered |-> FALSE, consumed |-> FALSE, borrow |-> 0]
                 pcall == [call |-> callid, caller |-> caller,
                           result |-> NoResult, observed |-> FALSE]
             IN /\ endpoints'   = [endpoints EXCEPT ![epid].queue =
                                       Append(e.queue, msg)]
                /\ replyTokens' = [replyTokens EXCEPT ![tokid] = token]
                /\ pendingCalls' = [pendingCalls EXCEPT ![callid] = pcall]
                \* Original object unchanged; new copy owned by server.
                /\ memObjects'  = [memObjects EXCEPT
                       ![newMid] = [obj |-> newMid, owner |-> server,
                                    state |-> "Owned", lender |-> NullAsid,
                                    borrows |-> {}]]
                /\ capTable'    = [capTable EXCEPT
                       ![caller][pcapCid]   = PendingCallCap(callid),
                       ![server][tokCapCid] = ReplyTokenCap(tokid),
                       ![server][memCapCid] = MemoryCap(newMid)]
                /\ nextCallId'  = nextCallId + 1
                /\ nextTokenId' = nextTokenId + 1
                /\ nextCapId'   = nextCapId + 3
                /\ nextMemId'   = nextMemId + 1
                /\ UNCHANGED <<nextEpId>>

\* -- 6.8 Receive -------------------------------------------------------------
Receive(server, epCid) ==
    /\ capTable[server][epCid].kind = "EndpointCap"
    /\ Authorized(capTable[server][epCid], "receive")
    /\ LET epid == capTable[server][epCid].ep
           e    == endpoints[epid]
       IN /\ e.owner = server
          /\ e.queue /= <<>>
          /\ endpoints' = [endpoints EXCEPT ![epid].queue = Tail(e.queue)]
          /\ LET tokid == Head(e.queue).reply
             IN IF tokid /= 0
                THEN replyTokens' =
                    [replyTokens EXCEPT ![tokid].delivered = TRUE]
                ELSE UNCHANGED replyTokens
    /\ UNCHANGED <<capTable, pendingCalls, memObjects,
                   nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.9 Reply (scalar result only) ------------------------------------------
Reply(server, tokenCid, resultVal) ==
    /\ capTable[server][tokenCid].kind = "ReplyTokenCap"
    /\ LET tokid == capTable[server][tokenCid].token
           tok   == replyTokens[tokid]
       IN /\ tok.server = server
          /\ tok.delivered
          /\ ~tok.consumed
          /\ tok.borrow = 0
          /\ pendingCalls[tok.call].result = NoResult
          /\ replyTokens' = [replyTokens EXCEPT ![tokid].consumed = TRUE]
          /\ pendingCalls' = [pendingCalls EXCEPT ![tok.call].result =
                                  [value |-> resultVal, mem |-> 0]]
          /\ capTable'    = [capTable EXCEPT ![server][tokenCid] = NullCap]
    /\ UNCHANGED <<endpoints, memObjects,
                   nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.10 Reply with memory return (move or copy back) -----------------------
ReplyReturnMemory(server, tokenCid, memCid, resultVal) ==
    /\ capTable[server][tokenCid].kind = "ReplyTokenCap"
    /\ capTable[server][memCid].kind = "MemoryCap"
    /\ LET tokid == capTable[server][tokenCid].token
           tok   == replyTokens[tokid]
           mid   == capTable[server][memCid].mem
           mo    == memObjects[mid]
       IN /\ tok.server = server /\ ~tok.consumed
          /\ tok.delivered
          /\ tok.borrow = 0
          /\ mo.state \in {"Moved", "Owned"} /\ mo.owner = server
          /\ pendingCalls[tok.call].result = NoResult
          /\ LET caller == pendingCalls[tok.call].caller
             IN /\ replyTokens'   = [replyTokens EXCEPT ![tokid].consumed = TRUE]
                /\ pendingCalls'  = [pendingCalls EXCEPT ![tok.call].result =
                                         [value |-> resultVal, mem |-> mid]]
                /\ memObjects'    = [memObjects EXCEPT ![mid].owner = caller,
                                                         ![mid].state = "Owned"]
                /\ capTable'      = [capTable EXCEPT ![server][tokenCid] = NullCap,
                                                      ![server][memCid]   = NullCap]
    /\ UNCHANGED <<endpoints,
                   nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.11 Reply that revokes a borrow ---------------------------------------
ReplyRevokeBorrow(server, tokenCid, resultVal) ==
    /\ capTable[server][tokenCid].kind = "ReplyTokenCap"
    /\ LET tokid == capTable[server][tokenCid].token
           tok   == replyTokens[tokid]
       IN /\ tok.server = server /\ ~tok.consumed
          /\ tok.delivered
          /\ tok.borrow /= 0
          /\ pendingCalls[tok.call].result = NoResult
          /\ LET mid == tok.borrow
                 mo  == memObjects[mid]
             IN /\ replyTokens' = [replyTokens EXCEPT ![tokid].consumed = TRUE]
                /\ pendingCalls' = [pendingCalls EXCEPT ![tok.call].result =
                                        [value |-> resultVal, mem |-> 0]]
                /\ memObjects'  = [memObjects EXCEPT ![mid] =
                       CASE mo.state = "BorrowedR" ->
                           [mo EXCEPT !.borrows = mo.borrows \ {server},
                                      !.state   = IF mo.borrows \ {server} = {}
                                                  THEN "Owned" ELSE "BorrowedR"]
                       [] mo.state = "BorrowedW" ->
                           [mo EXCEPT !.owner  = mo.lender,
                                      !.lender = NullAsid,
                                      !.state  = "Owned"]
                       [] OTHER -> mo]
                /\ capTable'    = [capTable EXCEPT ![server][tokenCid] = NullCap]
    /\ UNCHANGED <<endpoints,
                   nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.12 Cancel pending call ------------------------------------------------
CancelPendingCall(caller, callCid) ==
    /\ capTable[caller][callCid].kind = "PendingCallCap"
    /\ LET callid == capTable[caller][callCid].call
           pcall  == pendingCalls[callid]
       IN /\ pcall.caller = caller
          /\ pcall.result = NoResult
          /\ \E tokid \in TokenId :
               /\ replyTokens[tokid].call = callid
               /\ ~replyTokens[tokid].consumed
               /\ LET tok == replyTokens[tokid]
                  IN /\ replyTokens' = [replyTokens EXCEPT ![tokid].consumed = TRUE]
                     /\ pendingCalls' = [pendingCalls EXCEPT ![callid].result =
                                             CancelledResult]
                     /\ IF tok.borrow /= 0
                        THEN LET mid == tok.borrow
                                 mo  == memObjects[mid]
                             IN /\ memObjects' = [memObjects EXCEPT ![mid] =
                                    CASE mo.state = "BorrowedR" ->
                                        [mo EXCEPT !.borrows = mo.borrows \ {tok.server},
                                                   !.state   = IF mo.borrows \ {tok.server} = {}
                                                               THEN "Owned" ELSE "BorrowedR"]
                                    [] mo.state = "BorrowedW" ->
                                        [mo EXCEPT !.owner  = mo.lender,
                                                   !.lender = NullAsid,
                                                   !.state  = "Owned"]
                                    [] OTHER -> mo]
                        ELSE UNCHANGED memObjects
                     /\ capTable' = [capTable EXCEPT ![caller][callCid] = NullCap]
    /\ UNCHANGED <<endpoints,
                   nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.13 Endpoint close ----------------------------------------------------
EndpointClose(owner, epCid) ==
    /\ capTable[owner][epCid].kind = "EndpointCap"
    /\ LET epid == capTable[owner][epCid].ep
           e    == endpoints[epid]
       IN /\ e.owner = owner
          /\ ~e.closed
          /\ \* Collect reply tokens from queued messages and complete their calls.
              LET TokenIdsFromQueue ==
                      { msg.reply : msg \in SeqToSet(e.queue) } \ {0}
             IN /\ endpoints' = [endpoints EXCEPT ![epid].closed = TRUE,
                                                    ![epid].queue  = <<>>]
                /\ replyTokens' = [t \in TokenId |->
                       IF t \in TokenIdsFromQueue
                       THEN [replyTokens[t] EXCEPT !.consumed = TRUE]
                       ELSE replyTokens[t]]
                /\ pendingCalls' = [c \in CallId |->
                       IF \E t \in TokenIdsFromQueue :
                              replyTokens[t].call = c /\ pendingCalls[c].result = NoResult
                       THEN [pendingCalls[c] EXCEPT !.result = ClosedResult]
                       ELSE pendingCalls[c]]
                /\ memObjects' = [m \in MemId |->
                       RevokeTokenBorrows(m, TokenIdsFromQueue)]
    /\ UNCHANGED <<capTable,
                   nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.14 Domain teardown ----------------------------------------------------
DomainTeardown(as) ==
    /\ \* Collect reply tokens from queues of endpoints owned by this AS.
       LET \* Calls belonging to the dying AS.
           MyCalls ==
               { c \in CallId :
                   pendingCalls[c].caller = as
                   /\ pendingCalls[c].result = NoResult }
           \* Reply tokens whose pending call is owned by the dying AS.
           TokensForMyCalls ==
               { t \in TokenId :
                   replyTokens[t].call \in MyCalls
                   /\ ~replyTokens[t].consumed }
           \* Tokens to consume: those owned by the dying AS, those for its calls,
           \* plus those in the queues of its endpoints.
           QueueTokens ==
               { msg.reply : msg \in UNION { SeqToSet(endpoints[eid].queue) :
                       eid \in { e \in EpId : endpoints[e].owner = as } } } \ {0}
           TokensToConsume ==
               { t \in TokenId :
                   (replyTokens[t].server = as /\ ~replyTokens[t].consumed)
                   \/ t \in TokensForMyCalls
                   \/ t \in QueueTokens }
           \* Calls to close: those with consumed tokens, plus calls owned
           \* by the dying AS.
           CallsToClose ==
               { replyTokens[t].call : t \in TokensToConsume }
               \cup MyCalls
       IN /\ endpoints' = [e \in EpId |->
               IF endpoints[e].owner = as /\ ~endpoints[e].closed
               THEN [endpoints[e] EXCEPT !.closed = TRUE, !.queue = <<>>]
               ELSE endpoints[e]]
          /\ replyTokens' = [t \in TokenId |->
               IF t \in TokensToConsume
               THEN [replyTokens[t] EXCEPT !.consumed = TRUE]
               ELSE replyTokens[t]]
          /\ pendingCalls' = [c \in CallId |->
               IF c \in CallsToClose
               THEN [pendingCalls[c] EXCEPT !.result = ClosedResult]
               ELSE pendingCalls[c]]
          \* Revoke borrows involving this AS.
          /\ memObjects' = [m \in MemId |->
               LET revoked == RevokeTokenBorrows(m, TokensToConsume)
               IN IF revoked.state = "BorrowedR" /\ as \in revoked.borrows
                  THEN LET remaining == revoked.borrows \ {as}
                       IN [revoked EXCEPT !.borrows = remaining,
                                          !.state = IF remaining = {}
                                                   THEN "Owned" ELSE "BorrowedR",
                                          !.lender = IF remaining = {}
                                                    THEN NullAsid
                                                    ELSE revoked.lender]
                  ELSE IF revoked.state = "BorrowedW" /\ revoked.owner = as
                  THEN [revoked EXCEPT !.owner  = revoked.lender,
                                       !.lender = NullAsid,
                                       !.state  = "Owned"]
                  ELSE revoked]
          \* Remove all caps owned by this AS.
          /\ capTable' = [capTable EXCEPT ![as] =
                              [cid \in CapId |-> NullCap]]
    /\ UNCHANGED <<nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -- 6.15 Observe a pending call result --------------------------------------
ObserveResult(caller, callCid) ==
    /\ capTable[caller][callCid].kind = "PendingCallCap"
    /\ LET callid == capTable[caller][callCid].call
           pcall  == pendingCalls[callid]
       IN /\ pcall.caller = caller
          /\ pcall.result /= NoResult
          /\ ~pcall.observed
          /\ pendingCalls' = [pendingCalls EXCEPT ![callid].observed = TRUE]
    /\ UNCHANGED <<capTable, endpoints, replyTokens, memObjects,
                   nextCapId, nextEpId, nextTokenId, nextCallId, nextMemId>>

\* -----------------------------------------------------------------------------
\* 7. NEXT-STATE RELATION
\* -----------------------------------------------------------------------------

Next ==
    \/ \E as \in ASID : MemoryCreate(as)
    \/ \E as \in ASID : EndpointCreate(as)
    \/ \E as1, as2 \in ASID : \E cid \in CapId :
        \E allowed \in {SendCall, FullRights} :
            ConnectionMint(as1, cid, as2, allowed)
    \/ \E as \in ASID : \E cid \in CapId : \E o \in {0,1} : \E a \in {0,1} :
            ScalarSend(as, cid, o, a)
    \/ \E as \in ASID : \E cid \in CapId : \E o \in {0,1} : \E a \in {0,1} :
            ScalarCall(as, cid, o, a)
    \/ \E as \in ASID : \E cid, cid2 \in CapId : \E o \in {0,1} : \E a \in {0,1} :
            ScalarCallMove(as, cid, o, a, cid2)
    \/ \E as \in ASID : \E cid, cid2 \in CapId : \E o \in {0,1} : \E a \in {0,1} :
            ScalarCallBorrowRead(as, cid, o, a, cid2)
    \/ \E as \in ASID : \E cid, cid2 \in CapId : \E o \in {0,1} : \E a \in {0,1} :
            ScalarCallBorrowWrite(as, cid, o, a, cid2)
    \/ \E as \in ASID : \E cid, cid2 \in CapId : \E o \in {0,1} : \E a \in {0,1} :
            ScalarCallCopy(as, cid, o, a, cid2)
    \/ \E as \in ASID : \E cid \in CapId : Receive(as, cid)
    \/ \E as \in ASID : \E cid \in CapId : \E r \in {0,1} :
            Reply(as, cid, r)
    \/ \E as \in ASID : \E cid, cid2 \in CapId : \E r \in {0,1} :
            ReplyReturnMemory(as, cid, cid2, r)
    \/ \E as \in ASID : \E cid \in CapId : \E r \in {0,1} :
            ReplyRevokeBorrow(as, cid, r)
    \/ \E as \in ASID : \E cid \in CapId : CancelPendingCall(as, cid)
    \/ \E as \in ASID : \E cid \in CapId : EndpointClose(as, cid)
    \/ \E as \in ASID : DomainTeardown(as)
    \/ \E as \in ASID : \E cid \in CapId : ObserveResult(as, cid)

Spec == Init /\ [][Next]_vars

\* -----------------------------------------------------------------------------
\* 8. INVARIANTS
\* -----------------------------------------------------------------------------

\* I0: Every state variable remains within its declared abstract type.
TypeOK ==
    /\ capTable \in [ASID -> [CapId -> Capability]]
    /\ endpoints \in [EpId -> Endpoint]
    /\ replyTokens \in [TokenId -> ReplyToken]
    /\ pendingCalls \in [CallId -> PendingCall]
    /\ memObjects \in [MemId -> MemoryObject]
    /\ nextCapId \in 1 .. (MaxCaps + 1)
    /\ nextEpId \in 1 .. (MaxEps + 1)
    /\ nextTokenId \in 1 .. (MaxTokens + 1)
    /\ nextCallId \in 1 .. (MaxCalls + 1)
    /\ nextMemId \in 1 .. (MaxMems + 1)

\* I1: Token consumption => call completed (only for live tokens).
TokenImpliesCall ==
    \A t \in TokenId :
        (replyTokens[t].call /= 0 /\ replyTokens[t].consumed) =>
           (pendingCalls[replyTokens[t].call].result /= NoResult)

\* I2: Active (unconsumed) tokens have live pending calls.
ActiveTokenHasCall ==
    \A t \in TokenId :
        (~replyTokens[t].consumed) =>
           (replyTokens[t].call /= 0
            /\ pendingCalls[replyTokens[t].call].caller /= NullAsid
            /\ pendingCalls[replyTokens[t].call].result = NoResult)

\* I3: Queue boundedness.
QueueBounded ==
    \A e \in EpId :
        Len(endpoints[e].queue) <= endpoints[e].capacity

\* I4: Closed endpoints have empty queues.
ClosedEmpty ==
    \A e \in EpId :
        endpoints[e].closed => endpoints[e].queue = <<>>

\* I5: Exclusive write borrow -- at most one AS has a write-borrow.
ExclusiveBorrowWrite ==
    \A m \in MemId :
        memObjects[m].state = "BorrowedW" =>
            /\ memObjects[m].borrows = {}
            /\ memObjects[m].owner /= NullAsid
            /\ memObjects[m].lender /= NullAsid
            /\ memObjects[m].owner /= memObjects[m].lender

\* I6: Read-borrow and write-borrow are mutually exclusive.
NoMixedBorrows ==
    \A m \in MemId :
        ~(memObjects[m].state = "BorrowedW" /\ memObjects[m].borrows /= {})

\* I7: A non-Null reply token cap points to the correct server.
TokenServer ==
    \A as \in ASID :
        \A cid \in CapId :
            (capTable[as][cid].kind = "ReplyTokenCap"
             /\ capTable[as][cid].token /= 0)
            => (replyTokens[capTable[as][cid].token].server = as)

\* I9: No dangling memory caps. A cap to a memory object is only held by
\*     address spaces that have a valid relationship to it.
NoDanglingMemCaps ==
    \A as \in ASID :
        \A cid \in CapId :
            (capTable[as][cid].kind = "MemoryCap" /\ capTable[as][cid].mem /= 0) =>
                LET mid == capTable[as][cid].mem
                    mo  == memObjects[mid]
                IN ~(mo.state = "Moved" /\ mo.owner /= as)

\* I10: Token call references are valid.
TokenCallValid ==
    \A tok \in TokenId :
        (~replyTokens[tok].consumed) =>
            (replyTokens[tok].call /= 0)
    /\ \A tok2 \in TokenId :
        (replyTokens[tok2].call /= 0 /\ ~replyTokens[tok2].consumed) =>
            (pendingCalls[replyTokens[tok2].call].caller /= NullAsid)

\* I11: Borrow revocation on completion. If a pending call completes (result set)
\*      and its reply token had an attached borrow, the borrowed memory object
\*      must be back in Owned state.
BorrowRevokedOnCompletion ==
    \A c \in CallId :
        LET pcall == pendingCalls[c]
        IN (pcall.result /= NoResult /\ pcall.caller /= NullAsid)
           => \A t \in TokenId :
                (replyTokens[t].call = c /\ replyTokens[t].borrow /= 0)
                => memObjects[replyTokens[t].borrow].state = "Owned"

Invariants ==
    /\ TypeOK
    /\ TokenImpliesCall
    /\ ActiveTokenHasCall
    /\ QueueBounded
    /\ ClosedEmpty
    /\ ExclusiveBorrowWrite
    /\ NoMixedBorrows
    /\ TokenServer
    /\ NoDanglingMemCaps
    /\ TokenCallValid
    /\ BorrowRevokedOnCompletion

=============================================================================
