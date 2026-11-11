------------------------- MODULE BroadcastLifecycle -------------------------
(***************************************************************************)
(* One broadcast row and everyone who changes it at the same time:         *)
(* the owner (start, stop, delete), the MediaMTX hooks (available,         *)
(* unavailable), MediaMTX itself (auth callback, sessions, kicks) and the  *)
(* reaper pass. Each action is one SQL statement or one Control API call;  *)
(* SKILL.md maps every action to its function and SQL.                     *)
(*                                                                         *)
(* PostgreSQL READ COMMITTED is modeled for the one place it matters: the  *)
(* reaper pass is a single transaction. Its writes stay invisible to       *)
(* others until RCommit, and while it holds the row lock (rHolds) another  *)
(* UPDATE whose WHERE matches the committed row waits, then rechecks its   *)
(* WHERE on the newly committed row. An UPDATE whose WHERE does not match  *)
(* the committed row skips it without waiting.                             *)
(*                                                                         *)
(* Time is abstract: Tick ages live_since/interrupted_at past              *)
(* INTERRUPT_GRACE. The reaper compares against now() at the start of its  *)
(* pass, so it snapshots the ages at RBegin, and any timestamp written     *)
(* after RBegin (rTouched) is newer than that now().                       *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
  N,           \* publish attempts (WHIP sessions) over the broadcast's life
  MaxFailures \* probe/kick failures and pass rollbacks in one behavior

Sessions  == 1..N
Active    == {"LIVE", "INTERRUPTED"}       \* BroadcastStatus::ACTIVE
Deletable == {"READY", "ENDED"}            \* BroadcastStatus::DELETABLE
Edges     == {<<"READY", "LIVE">>, <<"LIVE", "INTERRUPTED">>,
              <<"INTERRUPTED", "LIVE">>, <<"LIVE", "ENDED">>,
              <<"INTERRUPTED", "ENDED">>}  \* BroadcastEvent::transition

VARIABLES
  status, deleted,   \* committed broadcasts.status, deleted_at IS NOT NULL
  liveOld, intOld,   \* live_since / interrupted_at older than INTERRUPT_GRACE
  sess,              \* MediaMTX view of publish attempt s
  av, lv,            \* runOnAvailable / runOnUnavailable delivery and handler
  avKick,            \* sessions the available-hook handler listed to kick
  stopPc, stopKick,  \* StopBroadcast::run after its UPDATE
  rPc,               \* Reaper::tick step
  rHolds,            \* the pass updated the row and holds its lock
  rStatus, rDeleted, \* the pass's own uncommitted row
  rTouched,          \* a timestamp was written after the pass began
  rLiveOld, rIntOld, \* ages as of the pass's now()
  rOnAir,            \* live_paths() named the path
  rKick, rKickSet,   \* the pass will kick this path / sessions it listed
  failures           \* infrastructure failures so far

vars == <<status, deleted, liveOld, intOld, sess, av, lv, avKick, stopPc,
          stopKick, rPc, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
          rIntOld, rOnAir, rKick, rKickSet, failures>>
reaperState == <<rPc, rHolds, rStatus, rDeleted, rTouched, rLiveOld, rIntOld,
                 rOnAir, rKick, rKickSet>>
reaperVars == <<reaperState, failures>>
pubVars == <<sess, av, lv, avKick>>

TypeOK ==
  /\ status \in {"READY", "LIVE", "INTERRUPTED", "ENDED"}
  /\ deleted \in BOOLEAN /\ liveOld \in BOOLEAN /\ intOld \in BOOLEAN
  /\ sess \in [Sessions -> {"idle", "authorized", "listed", "available", "closed"}]
  /\ av \in [Sessions -> {"none", "sent", "check", "kickList", "kickApply"}]
  /\ lv \in [Sessions -> {"none", "sent"}]
  /\ avKick \in [Sessions -> SUBSET Sessions]
  /\ stopPc \in {"idle", "kickList", "kickApply"}
  /\ rPc \in {"idle", "probe", "resume", "interrupt", "notActive", "expire",
              "demo", "commit", "kickList", "kickApply"}

Init ==
  /\ status = "READY" /\ deleted = FALSE /\ liveOld = FALSE /\ intOld = FALSE
  /\ sess = [s \in Sessions |-> "idle"]
  /\ av = [s \in Sessions |-> "none"] /\ lv = [s \in Sessions |-> "none"]
  /\ avKick = [s \in Sessions |-> {}]
  /\ stopPc = "idle" /\ stopKick = {}
  /\ rPc = "idle" /\ rHolds = FALSE /\ rStatus = "READY" /\ rDeleted = FALSE
  /\ rTouched = FALSE /\ rLiveOld = FALSE /\ rIntOld = FALSE
  /\ rOnAir = FALSE /\ rKick = FALSE /\ rKickSet = {}
  /\ failures = 0

OnAirNow   == \E s \in Sessions : sess[s] = "available"
Kickable   == {s \in Sessions : sess[s] \in {"listed", "available"}}

(* Another transaction's single-statement UPDATE with WHERE `match`:       *)
(* it waits only while the pass holds the row and the committed row        *)
(* matches; once enabled it acts on the committed row.                     *)
NotBlocked(match) == ~(rHolds /\ match)

(* A committed status write: the new status's timestamp is now(), younger  *)
(* than the grace and than any running pass's now().                       *)
PassOpen == rPc \in {"probe", "resume", "interrupt", "notActive", "expire",
                     "demo", "commit"}
SetStatus(to) ==
  /\ status' = to
  /\ liveOld' = FALSE /\ intOld' = FALSE
  /\ rTouched' = (rTouched \/ PassOpen)

(* MediaMTX closes the kicked sessions that are still there; an available  *)
(* one fires runOnUnavailable.                                             *)
Close(S) ==
  /\ sess' = [s \in Sessions |->
               IF s \in S /\ sess[s] \in {"listed", "available"}
               THEN "closed" ELSE sess[s]]
  /\ lv' = [s \in Sessions |->
             IF s \in S /\ sess[s] = "available" THEN "sent" ELSE lv[s]]

(* An infrastructure failure: the media server's Control API errs, or a   *)
(* pass is rolled back. At most MaxFailures per behavior, so liveness      *)
(* needs no fairness per failure point.                                    *)
Fail == failures < MaxFailures /\ failures' = failures + 1

---------------------------------------------------------------------------
(* Owner                                                                   *)

\* PgBroadcastRepository::start: UPDATE ... WHERE status = ANY('{READY}')
Start ==
  LET match == status = "READY" /\ ~deleted IN
  /\ NotBlocked(match) /\ match
  /\ SetStatus("LIVE")
  /\ UNCHANGED <<deleted, pubVars, stopPc, stopKick, rPc, rHolds, rStatus,
                 rDeleted, rLiveOld, rIntOld, rOnAir, rKick, rKickSet,
                 failures>>

\* PgBroadcastRepository::stop: UPDATE ... WHERE status = ANY(ACTIVE)
Stop ==
  LET match == status \in Active /\ ~deleted IN
  /\ stopPc = "idle" /\ NotBlocked(match) /\ match
  /\ SetStatus("ENDED")
  /\ stopPc' = "kickList"
  /\ UNCHANGED <<deleted, pubVars, stopKick, rPc, rHolds, rStatus, rDeleted,
                 rLiveOld, rIntOld, rOnAir, rKick, rKickSet,
                 failures>>

\* StopBroadcast::run -> MediaMtxControlApi::kick: sessions/list, then kick
StopKickList ==
  /\ stopPc = "kickList"
  /\ stopPc' = "kickApply" /\ stopKick' = Kickable
  /\ UNCHANGED <<status, deleted, liveOld, intOld, pubVars, reaperVars>>
StopKickFail ==
  /\ stopPc \in {"kickList", "kickApply"} /\ Fail
  /\ stopPc' = "idle" /\ stopKick' = {}
  /\ UNCHANGED <<status, deleted, liveOld, intOld, pubVars, reaperState>>
StopKickApply ==
  /\ stopPc = "kickApply"
  /\ Close(stopKick)
  /\ stopPc' = "idle" /\ stopKick' = {}
  /\ UNCHANGED <<status, deleted, liveOld, intOld, av, avKick, reaperVars>>

\* PgBroadcastRepository::soft_delete: WHERE status = ANY(DELETABLE)
Delete ==
  LET match == status \in Deletable /\ ~deleted IN
  /\ NotBlocked(match) /\ match
  /\ deleted' = TRUE
  /\ UNCHANGED <<status, liveOld, intOld, pubVars, stopPc, stopKick,
                 reaperVars>>

---------------------------------------------------------------------------
(* MediaMTX and the publisher                                              *)

\* /internal/media/auth -> PgPublishTicketLedger::redeem:
\* INSERT ... SELECT ... WHERE b.status = ANY(ACTIVE) AND b.deleted_at IS NULL
\* A plain read of the committed row; it never waits on the reaper's lock.
\* Attempts are used in order, which only removes renamings.
Auth(s) ==
  /\ sess[s] = "idle" /\ \A t \in 1..(s - 1) : sess[t] # "idle"
  /\ sess' = [sess EXCEPT ![s] =
               IF status \in Active /\ ~deleted THEN "authorized" ELSE "closed"]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, av, lv, avKick, stopPc,
                 stopKick, reaperVars>>

\* The WHIP offer is answered: the session joins webrtc/sessions/list.
Answer(s) ==
  /\ sess[s] = "authorized"
  /\ sess' = [sess EXCEPT ![s] = "listed"]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, av, lv, avKick, stopPc,
                 stopKick, reaperVars>>

\* ICE connects. overridePublisher: false refuses a second publisher.
Publish(s) ==
  /\ sess[s] = "listed"
  /\ IF OnAirNow
     THEN sess' = [sess EXCEPT ![s] = "closed"] /\ av' = av
     ELSE sess' = [sess EXCEPT ![s] = "available"] /\ av' = [av EXCEPT ![s] = "sent"]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, lv, avKick, stopPc,
                 stopKick, reaperVars>>

\* The phone backgrounds, the network drops, the tab closes.
Drop(s) ==
  /\ sess[s] \in {"authorized", "listed", "available"}
  /\ sess' = [sess EXCEPT ![s] = "closed"]
  /\ lv' = IF sess[s] = "available" THEN [lv EXCEPT ![s] = "sent"] ELSE lv
  /\ UNCHANGED <<status, deleted, liveOld, intOld, av, avKick, stopPc,
                 stopKick, reaperVars>>

\* A hook's wget fails or the API is down: the report is lost.
LoseAvailable(s) ==
  /\ av[s] = "sent" /\ av' = [av EXCEPT ![s] = "none"]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, sess, lv, avKick, stopPc,
                 stopKick, reaperVars>>
LoseUnavailable(s) ==
  /\ lv[s] = "sent" /\ lv' = [lv EXCEPT ![s] = "none"]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, sess, av, avKick, stopPc,
                 stopKick, reaperVars>>

---------------------------------------------------------------------------
(* Hook handlers: TrackPublisher::run                                      *)

\* Arrived -> PublisherPresence::resume: WHERE status = ANY('{INTERRUPTED}')
ArrivedResume(s) ==
  LET match == status = "INTERRUPTED" /\ ~deleted IN
  /\ av[s] = "sent" /\ NotBlocked(match)
  /\ IF match
     THEN SetStatus("LIVE") /\ av' = [av EXCEPT ![s] = "none"]
     ELSE av' = [av EXCEPT ![s] = "check"]
          /\ UNCHANGED <<status, liveOld, intOld, rTouched>>
  /\ UNCHANGED <<deleted, sess, lv, avKick, stopPc, stopKick, rPc, rHolds,
                 rStatus, rDeleted, rLiveOld, rIntOld, rOnAir, rKick, rKickSet,
                 failures>>

\* Arrived, unchanged -> PublisherPresence::is_active (plain read)
ArrivedCheck(s) ==
  /\ av[s] = "check"
  /\ av' = [av EXCEPT ![s] =
             IF status \in Active /\ ~deleted THEN "none" ELSE "kickList"]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, sess, lv, avKick, stopPc,
                 stopKick, reaperVars>>

\* Observed::NotActive -> StreamTerminator::kick
ArrivedKickList(s) ==
  /\ av[s] = "kickList"
  /\ av' = [av EXCEPT ![s] = "kickApply"]
  /\ avKick' = [avKick EXCEPT ![s] = Kickable]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, sess, lv, stopPc,
                 stopKick, reaperVars>>
ArrivedKickFail(s) ==
  /\ av[s] \in {"kickList", "kickApply"} /\ Fail
  /\ av' = [av EXCEPT ![s] = "none"] /\ avKick' = [avKick EXCEPT ![s] = {}]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, sess, lv, stopPc,
                 stopKick, reaperState>>
ArrivedKickApply(s) ==
  /\ av[s] = "kickApply"
  /\ Close(avKick[s])
  /\ av' = [av EXCEPT ![s] = "none"] /\ avKick' = [avKick EXCEPT ![s] = {}]
  /\ UNCHANGED <<status, deleted, liveOld, intOld, stopPc, stopKick,
                 reaperVars>>

\* Left -> PublisherPresence::interrupt: WHERE status = ANY('{LIVE}')
LeftInterrupt(s) ==
  LET match == status = "LIVE" /\ ~deleted IN
  /\ lv[s] = "sent" /\ NotBlocked(match)
  /\ IF match THEN SetStatus("INTERRUPTED")
              ELSE UNCHANGED <<status, liveOld, intOld, rTouched>>
  /\ lv' = [lv EXCEPT ![s] = "none"]
  /\ UNCHANGED <<deleted, sess, av, avKick, stopPc, stopKick, rPc, rHolds,
                 rStatus, rDeleted, rLiveOld, rIntOld, rOnAir, rKick, rKickSet,
                 failures>>

---------------------------------------------------------------------------
(* Reaper::tick, one transaction (PgReapPass)                              *)

SeenStatus  == IF rHolds THEN rStatus ELSE status
SeenDeleted == IF rHolds THEN rDeleted ELSE deleted
RWrite(to, del) ==
  /\ rStatus' = to /\ rDeleted' = del /\ rHolds' = TRUE

\* The pass is over: its scratch values go back to Init's (they mean
\* nothing outside a pass, and keeping them would only multiply states).
RClear ==
  /\ rHolds' = FALSE /\ rStatus' = "READY" /\ rDeleted' = FALSE
  /\ rTouched' = FALSE /\ rLiveOld' = FALSE /\ rIntOld' = FALSE
  /\ rOnAir' = FALSE /\ rKick' = FALSE

RShared == <<status, deleted, liveOld, intOld, pubVars, stopPc, stopKick>>
ROthers == <<RShared, failures>>

\* PgReaperStore::begin: pg_try_advisory_xact_lock; now() is fixed here.
\* Grace ages only reach "old" from the pass's view if they were old now.
RBegin ==
  /\ rPc = "idle"
  /\ rPc' = "probe" /\ rHolds' = FALSE /\ rTouched' = FALSE
  /\ rLiveOld' = liveOld /\ rIntOld' = intOld
  /\ rKick' = FALSE /\ rKickSet' = {} /\ rOnAir' = FALSE
  /\ UNCHANGED <<ROthers, rStatus, rDeleted>>

\* StreamProbe::live_paths (paths/list, available == true)
RProbeOk ==
  /\ rPc = "probe"
  /\ rPc' = "resume" /\ rOnAir' = OnAirNow
  /\ UNCHANGED <<ROthers, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rKick, rKickSet>>
\* Probe failed: skip reconcile and not_active, still expire.
RProbeFail ==
  /\ rPc = "probe" /\ Fail
  /\ rPc' = "expire"
  /\ UNCHANGED <<RShared, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKick, rKickSet>>

\* reconcile, first UPDATE: INTERRUPTED AND id = ANY(on_air)
\*   AND interrupted_at < now()  ->  LIVE, live_since = now()
RResume ==
  /\ rPc = "resume"
  /\ IF SeenStatus = "INTERRUPTED" /\ ~SeenDeleted /\ rOnAir /\ ~rTouched
     THEN RWrite("LIVE", SeenDeleted) /\ rTouched' = TRUE
     ELSE UNCHANGED <<rStatus, rDeleted, rHolds, rTouched>>
  /\ rPc' = "interrupt"
  /\ UNCHANGED <<ROthers, rLiveOld, rIntOld, rOnAir, rKick, rKickSet>>

\* reconcile, second UPDATE: LIVE AND NOT (id = ANY(on_air))
\*   AND live_since < now() - grace  ->  INTERRUPTED, interrupted_at = now()
RInterrupt ==
  /\ rPc = "interrupt"
  /\ IF SeenStatus = "LIVE" /\ ~SeenDeleted /\ ~rOnAir /\ ~rTouched /\ rLiveOld
     THEN RWrite("INTERRUPTED", SeenDeleted) /\ rTouched' = TRUE
     ELSE UNCHANGED <<rStatus, rDeleted, rHolds, rTouched>>
  /\ rPc' = "notActive"
  /\ UNCHANGED <<ROthers, rLiveOld, rIntOld, rOnAir, rKick, rKickSet>>

\* not_active: on_air ids with no ACTIVE, undeleted row
RNotActive ==
  /\ rPc = "notActive"
  /\ rKick' = (rOnAir /\ ~(SeenStatus \in Active /\ ~SeenDeleted))
  /\ rPc' = "expire"
  /\ UNCHANGED <<ROthers, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKickSet>>

\* expire: INTERRUPTED AND interrupted_at < now() - grace  ->  ENDED
RExpire ==
  /\ rPc = "expire"
  /\ IF SeenStatus = "INTERRUPTED" /\ ~SeenDeleted /\ ~rTouched /\ rIntOld
     THEN RWrite("ENDED", SeenDeleted) /\ rKick' = TRUE
     ELSE UNCHANGED <<rStatus, rDeleted, rHolds, rKick>>
  /\ rPc' = "demo"
  /\ UNCHANGED <<ROthers, rTouched, rLiveOld, rIntOld, rOnAir, rKickSet>>

\* reset_demo_broadcasts: a demo account's broadcast from before today, in
\* DELETABLE, is soft-deleted. Whether it is one is left open.
RDemoReset ==
  /\ rPc = "demo"
  /\ SeenStatus \in Deletable /\ ~SeenDeleted
  /\ RWrite(SeenStatus, TRUE)
  /\ rPc' = "commit"
  /\ UNCHANGED <<ROthers, rTouched, rLiveOld, rIntOld, rOnAir, rKick, rKickSet>>
RDemoSkip ==
  /\ rPc = "demo"
  /\ rPc' = "commit"
  /\ UNCHANGED <<ROthers, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKick, rKickSet>>

\* finish: COMMIT
RCommit ==
  /\ rPc = "commit"
  /\ IF rHolds
     THEN /\ status' = rStatus /\ deleted' = rDeleted
          /\ liveOld' = (liveOld /\ rStatus = status)
          /\ intOld' = (intOld /\ rStatus = status)
     ELSE UNCHANGED <<status, deleted, liveOld, intOld>>
  /\ rPc' = IF rKick THEN "kickList" ELSE "idle"
  /\ RClear
  /\ UNCHANGED <<pubVars, stopPc, stopKick, rKickSet, failures>>

\* The pass fails or is dropped before COMMIT: ROLLBACK.
RAbort ==
  /\ rPc \in {"probe", "resume", "interrupt", "notActive", "expire", "demo",
              "commit"} /\ Fail
  /\ rPc' = "idle" /\ RClear
  /\ UNCHANGED <<RShared, rKickSet>>

\* After COMMIT: kick expired and not_active paths.
RKickList ==
  /\ rPc = "kickList"
  /\ rPc' = "kickApply" /\ rKickSet' = Kickable
  /\ UNCHANGED <<ROthers, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKick>>
RKickFail ==
  /\ rPc \in {"kickList", "kickApply"} /\ Fail
  /\ rPc' = "idle" /\ rKickSet' = {}
  /\ UNCHANGED <<RShared, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKick>>
RKickApply ==
  /\ rPc = "kickApply"
  /\ Close(rKickSet)
  /\ rPc' = "idle" /\ rKickSet' = {}
  /\ UNCHANGED <<status, deleted, liveOld, intOld, av, avKick, stopPc,
                 stopKick, rHolds, rStatus, rDeleted, rTouched, rLiveOld,
                 rIntOld, rOnAir, rKick,
                 failures>>

---------------------------------------------------------------------------
(* Time: INTERRUPT_GRACE passes for the current timestamp.                 *)
Tick ==
  /\ \/ status = "LIVE" /\ ~liveOld /\ liveOld' = TRUE /\ intOld' = intOld
     \/ status = "INTERRUPTED" /\ ~intOld /\ intOld' = TRUE /\ liveOld' = liveOld
  /\ UNCHANGED <<status, deleted, pubVars, stopPc, stopKick, reaperVars>>

---------------------------------------------------------------------------
HookStep(s) ==
  \/ ArrivedResume(s) \/ ArrivedCheck(s) \/ ArrivedKickList(s)
  \/ ArrivedKickApply(s) \/ LeftInterrupt(s)
ReaperStep ==
  \/ RBegin \/ RProbeOk \/ RResume \/ RInterrupt \/ RNotActive \/ RExpire
  \/ RDemoReset \/ RDemoSkip \/ RCommit \/ RKickList \/ RKickApply
StopStep == StopKickList \/ StopKickApply

Next ==
  \/ Start \/ Stop \/ Delete \/ StopStep \/ StopKickFail \/ Tick
  \/ ReaperStep \/ RProbeFail \/ RAbort \/ RKickFail
  \/ \E s \in Sessions :
       \/ Auth(s) \/ Answer(s) \/ Publish(s) \/ Drop(s)
       \/ LoseAvailable(s) \/ LoseUnavailable(s)
       \/ HookStep(s) \/ ArrivedKickFail(s)

(* The reaper runs every REAP_INTERVAL, a started kick finishes, a hook    *)
(* that is not lost gets handled once the row lock is free, and time       *)
(* passes. Nothing is assumed about the owner, the publisher, or whether a *)
(* hook is delivered.                                                      *)
Fairness ==
  /\ WF_vars(Tick)
  /\ WF_vars(ReaperStep)
  /\ WF_vars(StopStep)
  /\ SF_vars(\E s \in Sessions : HookStep(s))

Spec == Init /\ [][Next]_vars /\ Fairness

---------------------------------------------------------------------------
(* Properties                                                              *)

Inactive    == status = "ENDED" \/ deleted
NoPublisher == ~OnAirNow

\* P1 ENDED is final: no order of events brings an ENDED broadcast back.
EndedIsFinal == [][status = "ENDED" => status' = "ENDED"]_vars

\* P2 Every status change is an edge of BroadcastEvent::transition.
LegalTransitions == [][status' # status => <<status, status'>> \in Edges]_vars

\* P3 A deletion commits only in READY or ENDED, and a deleted broadcast
\*    never changes again. (One reaper commit may expire and then
\*    demo-reset the same row: INTERRUPTED -> ENDED + deleted.)
DeleteOnlyReadyOrEnded == [][~deleted /\ deleted' => status' \in Deletable]_vars
DeletedIsFrozen == [][deleted => deleted' /\ status' = status]_vars

\* P4 The media server admits no publisher to a broadcast that is not active.
NoAdmitWhenInactive ==
  [][\A s \in Sessions :
       sess[s] = "idle" /\ sess'[s] = "authorized" =>
         status \in Active /\ ~deleted]_vars

\* P5 A pass never expires a broadcast whose path it saw on air
\*    (reconcile runs before expire).
NoExpireOnAir ==
  [][rPc = "expire" /\ rOnAir /\ SeenStatus = "INTERRUPTED" =>
       ~(rHolds' /\ rStatus' = "ENDED")]_vars

\* L1 Once a broadcast is ENDED or deleted, its stream eventually stops
\*    for good: no publisher outlives the end.
EndedStreamStops == [](Inactive => <>[]NoPublisher)

\* L2 A broadcast does not stay INTERRUPTED forever.
InterruptedResolves == [](status = "INTERRUPTED" => <>(status # "INTERRUPTED"))

\* L3 A LIVE broadcast with no publisher does not stay that way forever.
GhostLiveResolves ==
  [](status = "LIVE" /\ NoPublisher => <>(status # "LIVE" \/ ~NoPublisher))
=============================================================================
